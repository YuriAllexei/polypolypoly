pub mod states;

use crate::client::WebSocketClient;
use crate::config::ClientConfig;
use crate::traits::*;
use states::*;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// Type-state builder for WebSocketClient with routing
///
/// This builder uses Rust's type system to enforce that required
/// fields (URL and router) are set before the client can be built.
///
/// Handlers can be added for each route key that the router produces.
pub struct WebSocketClientBuilder<U, Ro, R, M>
where
    U: UrlState,
    Ro: RouterState,
{
    _state: TypeState<U, Ro>,
    _router_type: PhantomData<R>,
    _message_type: PhantomData<M>,
    url: Option<String>,
    router: Option<R>,
    // Closure to build handlers - uses Box<dyn Any> to avoid trait bound issues
    handler_builder: Option<Box<dyn std::any::Any + Send>>,
    auth: Option<Arc<dyn AuthProvider>>,
    headers: Option<Arc<dyn HeaderProvider>>,
    heartbeat: Option<(Duration, WsMessage)>,
    passive_ping: Option<Arc<dyn PassivePingDetector>>,
    pong_detector: Option<Arc<dyn PongDetector>>,
    pong_timeout: Option<Duration>,
    reconnect_strategy: Option<Box<dyn ReconnectionStrategy>>,
    reconnection_delay_offset: Duration,
    subscriptions: Vec<WsMessage>,
    shutdown_flag: Option<Arc<AtomicBool>>,
    halted_flag: Option<Arc<AtomicBool>>,
}

impl WebSocketClientBuilder<NoUrl, NoRouter, (), ()> {
    /// Create a new builder instance
    pub fn new() -> Self {
        Self {
            _state: TypeState::new(),
            _router_type: PhantomData,
            _message_type: PhantomData,
            url: None,
            router: None,
            handler_builder: None,
            auth: None,
            headers: None,
            heartbeat: None,
            passive_ping: None,
            pong_detector: None,
            pong_timeout: None,
            reconnect_strategy: None,
            reconnection_delay_offset: Duration::from_secs(0), // Default: no offset
            subscriptions: Vec::new(),
            shutdown_flag: None,
            halted_flag: None,
        }
    }
}

impl Default for WebSocketClientBuilder<NoUrl, NoRouter, (), ()> {
    fn default() -> Self {
        Self::new()
    }
}

// URL setting
impl<Ro, R, M> WebSocketClientBuilder<NoUrl, Ro, R, M>
where
    Ro: RouterState,
{
    pub fn url(self, url: impl Into<String>) -> WebSocketClientBuilder<HasUrl, Ro, R, M> {
        WebSocketClientBuilder {
            _state: TypeState::new(),
            _router_type: PhantomData,
            _message_type: PhantomData,
            url: Some(url.into()),
            router: self.router,
            handler_builder: self.handler_builder,
            auth: self.auth,
            headers: self.headers,
            heartbeat: self.heartbeat,
            passive_ping: self.passive_ping,
            pong_detector: self.pong_detector,
            pong_timeout: self.pong_timeout,
            reconnect_strategy: self.reconnect_strategy,
            reconnection_delay_offset: self.reconnection_delay_offset,
            subscriptions: self.subscriptions,
            shutdown_flag: self.shutdown_flag,
            halted_flag: self.halted_flag,
        }
    }
}

/// Routing builder helper
///
/// This helper allows adding handlers for different route keys.
pub struct RoutingBuilder<R>
where
    R: MessageRouter,
{
    handlers: HashMap<R::RouteKey, (crossbeam_channel::Sender<R::Message>, crossbeam_channel::Receiver<R::Message>, Box<dyn MessageHandler<R::Message>>)>,
}

impl<R> RoutingBuilder<R>
where
    R: MessageRouter,
{
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Add a handler for a specific route key
    pub fn handler<H>(mut self, route_key: R::RouteKey, handler: H) -> Self
    where
        H: MessageHandler<R::Message>,
    {
        let (sender, receiver) = crossbeam_channel::unbounded();
        self.handlers.insert(route_key, (sender, receiver, Box::new(handler)));
        self
    }

    fn build(self, _router: Arc<R>, shutdown_flag: Arc<std::sync::atomic::AtomicBool>) -> (HashMap<R::RouteKey, crossbeam_channel::Sender<R::Message>>, Vec<std::thread::JoinHandle<()>>, Option<Arc<std::sync::atomic::AtomicUsize>>) {
        let mut senders = HashMap::new();
        let mut handles = Vec::new();

        let handler_count = self.handlers.len();
        let handlers_not_ready = if handler_count > 0 {
            Some(Arc::new(std::sync::atomic::AtomicUsize::new(handler_count)))
        } else {
            None
        };

        for (route_key, (sender, receiver, handler)) in self.handlers {
            senders.insert(route_key.clone(), sender);

            let shutdown_flag = Arc::clone(&shutdown_flag);
            let counter = handlers_not_ready.clone();

            let handle = std::thread::spawn(move || {
                let mut handler = handler;

                if let Some(c) = counter {
                    c.fetch_sub(1, std::sync::atomic::Ordering::Release);
                }

                loop {
                    match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(message) => {
                            if let Err(e) = handler.handle(message) {
                                tracing::error!("Handler error for route {:?}: {}", route_key, e);
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
                                tracing::debug!("Shutdown flag detected, handler thread for route {:?} exiting", route_key);
                                break;
                            }
                            continue;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            tracing::debug!("Handler channel closed for route {:?}, thread exiting", route_key);
                            break;
                        }
                    }
                }
            });

            handles.push(handle);
        }

        (senders, handles, handlers_not_ready)
    }
}

// Router setting
impl<U> WebSocketClientBuilder<U, NoRouter, (), ()>
where
    U: UrlState,
{
    pub fn router<NewR, F>(
        self,
        router: NewR,
        configure_routing: F,
    ) -> WebSocketClientBuilder<U, HasRouter, NewR, NewR::Message>
    where
        NewR: MessageRouter,
        F: FnOnce(RoutingBuilder<NewR>) -> RoutingBuilder<NewR> + Send + 'static,
    {
        // We'll build the routing immediately and store the results
        let routing = RoutingBuilder::<NewR>::new();
        let routing = configure_routing(routing);

        // Store the routing builder as a closure that can be called later
        type HandlerBuilderFn<R> = Box<dyn FnOnce(Arc<R>, Arc<std::sync::atomic::AtomicBool>) -> (HashMap<<R as MessageRouter>::RouteKey, crossbeam_channel::Sender<<R as MessageRouter>::Message>>, Vec<std::thread::JoinHandle<()>>, Option<Arc<std::sync::atomic::AtomicUsize>>) + Send>;

        let handler_builder: HandlerBuilderFn<NewR> = Box::new(move |router_arc: Arc<NewR>, shutdown_flag: Arc<std::sync::atomic::AtomicBool>| {
            routing.build(router_arc, shutdown_flag)
        });

        // Box it as Any for storage
        let handler_builder_any = Box::new(handler_builder) as Box<dyn std::any::Any + Send>;

        WebSocketClientBuilder {
            _state: TypeState::new(),
            _router_type: PhantomData,
            _message_type: PhantomData,
            url: self.url,
            router: Some(router),
            handler_builder: Some(handler_builder_any),
            auth: self.auth,
            headers: self.headers,
            heartbeat: self.heartbeat,
            passive_ping: self.passive_ping,
            pong_detector: self.pong_detector,
            pong_timeout: self.pong_timeout,
            reconnect_strategy: self.reconnect_strategy,
            reconnection_delay_offset: self.reconnection_delay_offset,
            subscriptions: self.subscriptions,
            shutdown_flag: self.shutdown_flag,
            halted_flag: self.halted_flag,
        }
    }
}

// Optional configuration methods
impl<U, R> WebSocketClientBuilder<U, HasRouter, R, R::Message>
where
    U: UrlState,
    R: MessageRouter,
{
    pub fn auth(mut self, auth: impl AuthProvider + 'static) -> Self {
        self.auth = Some(Arc::new(auth));
        self
    }

    pub fn headers(mut self, provider: impl HeaderProvider + 'static) -> Self {
        self.headers = Some(Arc::new(provider));
        self
    }

    pub fn heartbeat(mut self, interval: Duration, payload: WsMessage) -> Self {
        self.heartbeat = Some((interval, payload));
        self
    }

    pub fn passive_ping(mut self, detector: impl PassivePingDetector + 'static) -> Self {
        self.passive_ping = Some(Arc::new(detector));
        self
    }

    /// Set a PONG detector for tracking PONG responses
    ///
    /// The PONG detector is used to identify PONG messages in the WebSocket stream.
    /// When a PONG is detected, it's recorded for health tracking.
    ///
    /// Should be used together with `pong_timeout()` for full PONG tracking.
    pub fn pong_detector(mut self, detector: Arc<dyn PongDetector>) -> Self {
        self.pong_detector = Some(detector);
        self
    }

    /// Set the PONG timeout for connection health tracking
    ///
    /// If no PONG is received within this duration after a PING was sent,
    /// the connection is considered unhealthy and will trigger a reconnection.
    ///
    /// Recommended value: 3x the heartbeat interval (e.g., 15s for 5s heartbeat)
    ///
    /// Should be used together with `pong_detector()` for full PONG tracking.
    pub fn pong_timeout(mut self, timeout: Duration) -> Self {
        self.pong_timeout = Some(timeout);
        self
    }

    pub fn reconnect_strategy(mut self, strategy: impl ReconnectionStrategy + 'static) -> Self {
        self.reconnect_strategy = Some(Box::new(strategy));
        self
    }

    /// Set the delay offset to wait after disconnection before reconnection
    ///
    /// This delay is applied BEFORE the reconnection strategy's delay.
    /// Useful for giving servers time to clean up resources after disconnect.
    ///
    /// # Arguments
    /// * `offset` - Duration to wait after disconnect (e.g., Duration::from_secs(2))
    ///
    /// # Example
    /// ```ignore
    /// .reconnection_delay_offset(Duration::from_secs(2))  // Wait 2s after disconnect
    /// .reconnect_strategy(ExponentialBackoff::new(...))   // Then apply strategy delay
    /// ```
    pub fn reconnection_delay_offset(mut self, offset: Duration) -> Self {
        self.reconnection_delay_offset = offset;
        self
    }

    pub fn subscription(mut self, message: WsMessage) -> Self {
        self.subscriptions.push(message);
        self
    }

    pub fn subscriptions(mut self, messages: Vec<WsMessage>) -> Self {
        self.subscriptions.extend(messages);
        self
    }

    /// Set a custom shutdown flag for coordinated shutdown across components
    ///
    /// By default, the client creates an internal shutdown flag. Use this method
    /// if you want to control shutdown externally or coordinate shutdown across
    /// multiple clients.
    ///
    /// When the flag is set to `false`, the client will not attempt reconnection
    /// and will gracefully shut down.
    ///
    /// # Example
    /// ```ignore
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// let shutdown_flag = Arc::new(AtomicBool::new(true));
    ///
    /// let client = hypersockets::builder()
    ///     .url("wss://api.example.com")
    ///     .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    ///     .shutdown_flag(Arc::clone(&shutdown_flag))
    ///     .build()
    ///     .await?;
    ///
    /// // Later, to trigger graceful shutdown from another thread:
    /// shutdown_flag.store(false, Ordering::Release);
    /// ```
    pub fn shutdown_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.shutdown_flag = Some(flag);
        self
    }

    /// Set a custom halted flag for connection state tracking
    ///
    /// The halted flag is informational and indicates when the client is
    /// disconnected but not shutting down (i.e., actively trying to reconnect).
    ///
    /// This is typically used with ClientManager to coordinate state across
    /// multiple connections. When any managed client disconnects, the manager
    /// sets the halted flag to true. When all clients reconnect, it's set to false.
    ///
    /// # Example
    /// ```ignore
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicBool, Ordering};
    ///
    /// let halted_flag = Arc::new(AtomicBool::new(false));
    ///
    /// let client = hypersockets::builder()
    ///     .url("wss://api.example.com")
    ///     .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    ///     .halted_flag(Arc::clone(&halted_flag))
    ///     .build()
    ///     .await?;
    ///
    /// // Check if connection is halted (disconnected but reconnecting):
    /// if halted_flag.load(Ordering::Acquire) {
    ///     println!("Connection temporarily down, reconnecting...");
    /// }
    /// ```
    pub fn halted_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.halted_flag = Some(flag);
        self
    }
}

// Build method - only available when all required fields are set
impl<R> WebSocketClientBuilder<HasUrl, HasRouter, R, R::Message>
where
    R: MessageRouter,
{
    pub async fn build(self) -> Result<WebSocketClient<R, R::Message>> {
        let url = self.url.expect("URL must be set");
        let router = Arc::new(self.router.expect("Router must be set"));

        // Create shutdown flag if not provided
        let shutdown_flag = self.shutdown_flag
            .unwrap_or_else(|| Arc::new(AtomicBool::new(true)));

        let reconnect_strategy = self.reconnect_strategy.unwrap_or_else(|| {
            Box::new(ExponentialBackoff::new(
                Duration::from_secs(1),
                Duration::from_secs(60),
                Some(10),
            ))
        });

        // Build handlers using the closure
        let (route_senders, handler_handles, handlers_not_ready) = if let Some(builder_any) = self.handler_builder {
            // Downcast from Any back to the concrete closure type
            type HandlerBuilderFn<R> = Box<dyn FnOnce(Arc<R>, Arc<std::sync::atomic::AtomicBool>) -> (HashMap<<R as MessageRouter>::RouteKey, crossbeam_channel::Sender<<R as MessageRouter>::Message>>, Vec<std::thread::JoinHandle<()>>, Option<Arc<std::sync::atomic::AtomicUsize>>) + Send>;

            let builder = builder_any
                .downcast::<HandlerBuilderFn<R>>()
                .expect("Handler builder type mismatch");

            (*builder)(Arc::clone(&router), Arc::clone(&shutdown_flag))
        } else {
            (HashMap::new(), Vec::new(), None)
        };

        let config = ClientConfig {
            url,
            router,
            route_senders,
            auth: self.auth,
            headers: self.headers,
            heartbeat: self.heartbeat,
            passive_ping: self.passive_ping,
            pong_detector: self.pong_detector,
            pong_timeout: self.pong_timeout,
            reconnect_strategy,
            reconnection_delay_offset: self.reconnection_delay_offset,
            subscriptions: self.subscriptions,
            shutdown_flag,
            halted_flag: self.halted_flag,
            handlers_not_ready,
        };

        let mut client = WebSocketClient::new(config).await?;
        client.handler_handles = handler_handles;

        Ok(client)
    }
}
