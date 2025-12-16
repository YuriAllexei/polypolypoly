use crate::config::ClientConfig;
use crate::connection_state::{AtomicConnectionState, AtomicMetrics, ConnectionState};
use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::{SinkExt, StreamExt};
use crate::traits::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http;
use tracing::{debug, error, info, warn};

/// Internal command messages for client control
#[derive(Debug)]
enum ClientCommand {
    /// Send a message to the WebSocket
    Send(WsMessage),
    /// Shutdown the client
    Shutdown,
    /// Get current metrics
    GetMetrics(Sender<Metrics>),
}

/// Internal event messages from the client
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Connected to the server
    Connected,
    /// Disconnected from the server
    Disconnected,
    /// Reconnecting (attempt number)
    Reconnecting(usize),
    /// Error occurred
    Error(String),
}

/// Client metrics snapshot
#[derive(Debug, Clone)]
pub struct Metrics {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub reconnect_count: u64,
    pub connection_state: ConnectionState,
}

/// High-performance WebSocket client with message routing
///
/// This client is designed for maximum performance and modularity:
/// - Lock-free state management using atomics
/// - Unbounded crossbeam channels for message passing
/// - Conditional routing with per-message-type ordering
/// - Cross-type parallel processing
/// - Configurable heartbeat, auth, and reconnection
///
/// # Type Parameters
/// - `R`: MessageRouter implementation
/// - `M`: Message type (determined by router)
pub struct WebSocketClient<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    /// Client configuration (kept for potential future API access)
    #[allow(dead_code)]
    config: Arc<ClientConfig<R, M>>,
    /// Atomic connection state
    state: Arc<AtomicConnectionState>,
    /// Atomic metrics
    metrics: Arc<AtomicMetrics>,
    /// Command channel sender
    command_tx: Sender<ClientCommand>,
    /// Event channel receiver
    event_rx: Receiver<ClientEvent>,
    /// Main task handle (tokio task for async I/O)
    task_handle: Option<tokio::task::JoinHandle<()>>,
    /// Handler thread handles (dedicated OS threads for message processing)
    pub(crate) handler_handles: Vec<std::thread::JoinHandle<()>>,
    /// Shutdown flag reference (for external access if needed)
    shutdown_flag: Arc<AtomicBool>,
    /// Optional halted flag reference (for connection state tracking)
    halted_flag: Option<Arc<AtomicBool>>,
}

impl<R, M> WebSocketClient<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    /// Create a new WebSocket client from configuration
    ///
    /// This is called by the builder's `build()` method.
    /// Use `hypersockets::builder()` to create a client.
    pub(crate) async fn new(config: ClientConfig<R, M>) -> Result<Self> {
        let config = Arc::new(config);
        let state = Arc::new(AtomicConnectionState::new(ConnectionState::Disconnected));
        let metrics = Arc::new(AtomicMetrics::new());
        let shutdown_flag = Arc::clone(&config.shutdown_flag);
        let halted_flag = config.halted_flag.as_ref().map(Arc::clone);

        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();

        // Note: Handler tasks will be spawned by the builder
        // The builder creates the channels and handlers, then passes them here

        // Spawn the main client task
        let task_handle = {
            let config = Arc::clone(&config);
            let state = Arc::clone(&state);
            let metrics = Arc::clone(&metrics);

            tokio::spawn(async move {
                run_client(config, state, metrics, command_rx, event_tx).await;
            })
        };

        Ok(Self {
            config,
            state,
            metrics,
            command_tx,
            event_rx,
            task_handle: Some(task_handle),
            handler_handles: Vec::new(), // Builder will populate this
            shutdown_flag,
            halted_flag,
        })
    }

    /// Send a message through the WebSocket
    pub fn send(&self, message: WsMessage) -> Result<()> {
        self.command_tx
            .send(ClientCommand::Send(message))
            .map_err(|e| HyperSocketError::ChannelSend(e.to_string()))
    }

    /// Get current connection state
    #[inline]
    pub fn connection_state(&self) -> ConnectionState {
        self.state.get()
    }

    /// Check if connected
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.state.is_connected()
    }

    /// Get the halted flag reference (if configured)
    ///
    /// Returns `None` if no halted flag was configured via the builder.
    /// Returns `Some(Arc<AtomicBool>)` if a halted flag was set.
    ///
    /// The halted flag is typically managed by ClientManager and indicates
    /// when the connection is temporarily down (disconnected but reconnecting).
    #[inline]
    pub fn halted_flag(&self) -> Option<Arc<AtomicBool>> {
        self.halted_flag.as_ref().map(Arc::clone)
    }

    /// Check if the connection is halted (informational)
    ///
    /// Returns `false` if no halted flag is configured.
    /// Returns the halted flag value if configured.
    ///
    /// A halted connection is one that is disconnected but still attempting
    /// to reconnect (not shutting down).
    #[inline]
    pub fn is_halted(&self) -> bool {
        self.halted_flag
            .as_ref()
            .map(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
            .unwrap_or(false)
    }

    /// Get current metrics
    pub fn metrics(&self) -> Metrics {
        let (tx, rx) = unbounded();
        if self.command_tx.send(ClientCommand::GetMetrics(tx)).is_ok() {
            // Use try_recv to avoid blocking - if not immediately available, use atomic values
            rx.try_recv().unwrap_or_else(|_| Metrics {
                messages_sent: self.metrics.messages_sent(),
                messages_received: self.metrics.messages_received(),
                reconnect_count: self.metrics.reconnect_count(),
                connection_state: self.state.get(),
            })
        } else {
            Metrics {
                messages_sent: self.metrics.messages_sent(),
                messages_received: self.metrics.messages_received(),
                reconnect_count: self.metrics.reconnect_count(),
                connection_state: self.state.get(),
            }
        }
    }

    /// Try to receive an event (non-blocking)
    pub fn try_recv_event(&self) -> Option<ClientEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Receive an event (blocking)
    pub fn recv_event(&self) -> std::result::Result<ClientEvent, crossbeam_channel::RecvError> {
        self.event_rx.recv()
    }

    /// Get a reference to the shutdown flag
    ///
    /// This allows external code to trigger graceful shutdown by setting
    /// the flag to false: `client.shutdown_flag().store(false, Ordering::Release)`
    ///
    /// The flag is checked before each reconnection attempt. When false,
    /// the client will not reconnect and will exit gracefully.
    pub fn shutdown_flag(&self) -> &Arc<AtomicBool> {
        &self.shutdown_flag
    }

    /// Shutdown the client
    pub async fn shutdown(mut self) -> Result<()> {
        info!("Shutting down WebSocket client");

        // Set shutdown flag to prevent reconnection
        self.shutdown_flag.store(false, std::sync::atomic::Ordering::Release);

        // Set state for immediate shutdown of active connection
        self.state.set(ConnectionState::ShuttingDown);

        // Send shutdown command to main WebSocket I/O task
        let _ = self.command_tx.send(ClientCommand::Shutdown);

        // Wait for main WebSocket I/O task to complete
        // This stops receiving new messages
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        // Give in-flight parse tasks 100ms to complete or be discarded
        // Parse tasks spawned before shutdown will either:
        // 1. Complete parsing and check shutdown flag before routing (discarded)
        // 2. Finish within this grace period
        // This prevents blocking handler threads with late-arriving work
        debug!("Waiting 100ms for in-flight parse tasks to complete");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Drop config to close handler channels
        // When all Arc references are dropped, route_senders are dropped
        // This closes the channels, causing handler threads to exit
        drop(self.config);

        // Wait for all handler threads to finish processing remaining messages
        // Handler threads will exit within 50ms (recv_timeout) after detecting shutdown flag
        debug!("Waiting for {} handler threads to complete", self.handler_handles.len());
        for handle in self.handler_handles {
            let _ = handle.join();
        }

        info!("All handlers shut down successfully");
        Ok(())
    }
}

/// Main client task loop
async fn run_client<R, M>(
    config: Arc<ClientConfig<R, M>>,
    state: Arc<AtomicConnectionState>,
    metrics: Arc<AtomicMetrics>,
    command_rx: Receiver<ClientCommand>,
    event_tx: Sender<ClientEvent>,
) where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    let mut reconnect_attempt = 0;
    let shutdown_flag = &config.shutdown_flag;

    loop {
        // Check shutdown flag FIRST - highest priority check
        if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            debug!("Shutdown flag is false, exiting main loop");
            break;
        }

        // Check if shutting down (legacy check for state-based shutdown)
        if state.is_shutting_down() {
            debug!("Client is shutting down, exiting main loop");
            break;
        }

        // Attempt connection
        state.set(if reconnect_attempt == 0 {
            ConnectionState::Connecting
        } else {
            ConnectionState::Reconnecting
        });

        if reconnect_attempt > 0 {
            let _ = event_tx.send(ClientEvent::Reconnecting(reconnect_attempt));
        }

        // Build request with headers if configured
        let connection_result = if let Some(ref header_provider) = config.headers {
            // Generate headers dynamically
            let headers = header_provider.get_headers().await;

            match config.url.as_str().into_client_request() {
                Ok(mut request) => {
                    // Apply headers to request
                    for (key, value) in headers {
                        match key.parse::<http::header::HeaderName>() {
                            Ok(header_name) => {
                                match value.parse::<http::header::HeaderValue>() {
                                    Ok(header_value) => {
                                        request.headers_mut().insert(header_name, header_value);
                                    }
                                    Err(_) => {
                                        warn!("Invalid header value for key '{}': {}", key, value);
                                    }
                                }
                            }
                            Err(_) => {
                                warn!("Invalid header name: {}", key);
                            }
                        }
                    }

                    debug!("Connecting with custom headers");
                    connect_async(request).await
                }
                Err(e) => {
                    error!("Failed to create request: {}", e);
                    // Fall back to connecting without headers
                    connect_async(&config.url).await
                }
            }
        } else {
            // Connect without custom headers
            connect_async(&config.url).await
        };

        match connection_result {
            Ok((ws_stream, _)) => {
                info!("Connected to {}", config.url);
                state.set(ConnectionState::Connected);
                let _ = event_tx.send(ClientEvent::Connected);

                reconnect_attempt = 0;

                // Handle the connection
                if let Err(e) = handle_connection(
                    ws_stream,
                    Arc::clone(&config),
                    Arc::clone(&state),
                    Arc::clone(&metrics),
                    &command_rx,
                    &event_tx,
                )
                .await
                {
                    error!("Connection error: {}", e);
                    let _ = event_tx.send(ClientEvent::Error(e.to_string()));
                }

                state.set(ConnectionState::Disconnected);
                let _ = event_tx.send(ClientEvent::Disconnected);
            }
            Err(e) => {
                error!("Failed to connect: {}", e);
                let _ = event_tx.send(ClientEvent::Error(e.to_string()));
                state.set(ConnectionState::Disconnected);
            }
        }

        // Check if we should reconnect
        if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            debug!("Shutdown flag set during connection, stopping reconnection");
            break;
        }

        if state.is_shutting_down() {
            break;
        }

        // Apply reconnection delay offset (immediate wait after disconnect)
        if config.reconnection_delay_offset.as_secs() > 0 || config.reconnection_delay_offset.as_millis() > 0 {
            debug!("Waiting reconnection delay offset: {:?}", config.reconnection_delay_offset);

            // Check shutdown flag periodically during the wait
            let sleep_duration = config.reconnection_delay_offset;
            let check_interval = std::time::Duration::from_millis(100); // Check every 100ms
            let mut elapsed = std::time::Duration::ZERO;

            while elapsed < sleep_duration {
                if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
                    debug!("Shutdown flag set during reconnection delay offset");
                    return; // Exit the function early
                }

                let sleep_time = std::cmp::min(check_interval, sleep_duration - elapsed);
                tokio::time::sleep(sleep_time).await;
                elapsed += sleep_time;
            }
        }

        // Use reconnection strategy
        if let Some(delay) = config.reconnect_strategy.next_delay(reconnect_attempt) {
            info!(
                "Reconnecting in {:?} (attempt {})",
                delay,
                reconnect_attempt + 1
            );

            // Check shutdown flag periodically during reconnection delay
            let check_interval = std::time::Duration::from_millis(100); // Check every 100ms
            let mut elapsed = std::time::Duration::ZERO;

            while elapsed < delay {
                if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
                    debug!("Shutdown flag set during reconnection delay");
                    return; // Exit the function early
                }

                let sleep_time = std::cmp::min(check_interval, delay - elapsed);
                tokio::time::sleep(sleep_time).await;
                elapsed += sleep_time;
            }

            reconnect_attempt += 1;
            metrics.increment_reconnects();
        } else {
            warn!("Reconnection strategy exhausted, stopping");
            break;
        }
    }

    info!("Client task exiting");
}

/// Handle an active WebSocket connection
async fn handle_connection<R, M>(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    config: Arc<ClientConfig<R, M>>,
    state: Arc<AtomicConnectionState>,
    metrics: Arc<AtomicMetrics>,
    command_rx: &Receiver<ClientCommand>,
    _event_tx: &Sender<ClientEvent>,
) -> Result<()>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    let (mut write, mut read) = ws_stream.split();

    // Send auth message if configured
    if let Some(ref auth) = config.auth {
        if let Some(auth_msg) = auth.get_auth_message().await? {
            let msg = ws_message_to_tungstenite(&auth_msg);
            write.send(msg).await.map_err(|e| {
                HyperSocketError::WebSocket(format!("Failed to send auth: {}", e))
            })?;
            metrics.increment_sent();
            debug!("Sent authentication message");
        }
    }

    if let Some(ref barrier) = config.handlers_ready {
        barrier.wait();
    }

    // Send subscription messages if configured
    for sub in &config.subscriptions {
        let msg = ws_message_to_tungstenite(sub);
        write.send(msg).await.map_err(|e| {
            HyperSocketError::WebSocket(format!("Failed to send subscription: {}", e))
        })?;
        metrics.increment_sent();
        debug!("Sent subscription message");
    }

    // Spawn heartbeat task if configured
    let heartbeat_handle = if let Some((interval, payload)) = &config.heartbeat {
        let interval = *interval;
        let payload = payload.clone();

        let (handle, shutdown_tx, heartbeat_rx) =
            crate::heartbeat::spawn_heartbeat(interval, payload);

        Some((handle, shutdown_tx, heartbeat_rx))
    } else {
        None
    };

    // Main message loop
    let result = message_loop(
        &mut write,
        &mut read,
        config,
        state,
        metrics,
        command_rx,
        heartbeat_handle.as_ref().map(|(_, _, rx)| rx),
    )
    .await;

    // Cleanup heartbeat task
    // Send shutdown signal and let it exit gracefully via signal check
    if let Some((_handle, shutdown_tx, _)) = heartbeat_handle {
        let _ = shutdown_tx.send(());
        // Heartbeat task checks shutdown_rx in its select loop and will exit cleanly
        // No need to abort - it will finish naturally
    }

    result
}

/// Main message processing loop
async fn message_loop<R, M>(
    write: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >,
    read: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
    config: Arc<ClientConfig<R, M>>,
    state: Arc<AtomicConnectionState>,
    metrics: Arc<AtomicMetrics>,
    command_rx: &Receiver<ClientCommand>,
    heartbeat_rx: Option<&Receiver<WsMessage>>,
) -> Result<()>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    let shutdown_flag = &config.shutdown_flag;

    loop {
        // Check shutdown flag FIRST - exit immediately if shutdown requested
        if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            debug!("Shutdown flag detected in message loop, closing connection");
            // Close the WebSocket write sink to stop the connection
            let _ = write.close().await;
            return Ok(());
        }

        // Check if shutting down via state
        if state.is_shutting_down() {
            debug!("Shutting down state detected in message loop, closing connection");
            // Close the WebSocket write sink to stop the connection
            let _ = write.close().await;
            return Ok(());
        }

        tokio::select! {
            // Handle incoming messages
            msg = read.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        metrics.increment_received();

                        if let Some(ws_msg) = tungstenite_to_ws_message(msg) {
                            // Check EVERY message for passive ping (if configured)
                            if let Some(ref detector) = config.passive_ping {
                                if detector.is_ping(&ws_msg) {
                                    debug!("Passive ping detected from server");

                                    // Get the required pong response
                                    let pong = detector.get_pong_response();

                                    // Send pong response immediately
                                    let msg = ws_message_to_tungstenite(&pong);
                                    write.send(msg).await.map_err(|e| {
                                        HyperSocketError::WebSocket(format!(
                                            "Failed to send passive pong: {}", e
                                        ))
                                    })?;
                                    metrics.increment_sent();
                                    debug!("Passive pong sent successfully");

                                    // Don't parse this message - it was a ping
                                    continue;
                                }
                            }

                            // Check shutdown flag before spawning parse task
                            // Don't queue new work if shutting down
                            if !shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
                                debug!("Shutdown detected, skipping message parsing");
                                continue;
                            }

                            // Parse and route message
                            let router = Arc::clone(&config.router);
                            let route_senders = config.route_senders.clone();
                            let shutdown_flag_parse = Arc::clone(&shutdown_flag);

                            tokio::spawn(async move {
                                // Parse the WebSocket message
                                match router.parse(ws_msg).await {
                                    Ok(message) => {
                                        // Check shutdown flag before routing (atomic load, ~1ns)
                                        // Don't route messages if shutdown was triggered during parse
                                        if !shutdown_flag_parse.load(std::sync::atomic::Ordering::Acquire) {
                                            debug!("Shutdown detected after parse, discarding message");
                                            return;
                                        }

                                        // Get route key
                                        let route_key = router.route_key(&message);

                                        // Route to appropriate handler channel
                                        if let Some(sender) = route_senders.get(&route_key) {
                                            // Send message to handler
                                            // If send fails, channel is closed which only happens during shutdown
                                            // We silently ignore these errors as they're expected during graceful shutdown
                                            let _ = sender.send(message);
                                        } else {
                                            warn!("No handler configured for route key: {:?}", route_key);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Parse error: {}", e);
                                    }
                                }
                            });
                        }
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        return Err(HyperSocketError::WebSocket(e.to_string()));
                    }
                    None => {
                        warn!("WebSocket stream closed");
                        return Err(HyperSocketError::ConnectionClosed("Stream ended".into()));
                    }
                }
            }

            // Handle commands (use spawn_blocking with timeout to avoid blocking select)
            cmd = async {
                let rx = command_rx.clone();
                tokio::task::spawn_blocking(move || {
                    rx.recv_timeout(std::time::Duration::from_millis(100))
                }).await.ok()
            } => {
                match cmd {
                    Some(Ok(ClientCommand::Send(msg))) => {
                        let tung_msg = ws_message_to_tungstenite(&msg);
                        write.send(tung_msg).await.map_err(|e| {
                            HyperSocketError::WebSocket(e.to_string())
                        })?;
                        metrics.increment_sent();
                    }
                    Some(Ok(ClientCommand::Shutdown)) => {
                        info!("Received shutdown command");
                        state.set(ConnectionState::ShuttingDown);
                        return Ok(());
                    }
                    Some(Ok(ClientCommand::GetMetrics(tx))) => {
                        let _ = tx.send(Metrics {
                            messages_sent: metrics.messages_sent(),
                            messages_received: metrics.messages_received(),
                            reconnect_count: metrics.reconnect_count(),
                            connection_state: state.get(),
                        });
                    }
                    Some(Err(_)) => {
                        // Timeout is normal, just continue the loop
                    }
                    None => {
                        debug!("Command channel closed");
                        return Ok(());
                    }
                }
            }

            // Handle heartbeat messages from dedicated heartbeat task
            hb = async {
                if let Some(rx) = heartbeat_rx {
                    let rx_clone = rx.clone();
                    tokio::task::spawn_blocking(move || {
                        rx_clone.recv_timeout(std::time::Duration::from_millis(100))
                    }).await.ok().and_then(|r| r.ok())
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(msg) = hb {
                    debug!("Received heartbeat from heartbeat task, sending to server");
                    let tung_msg = ws_message_to_tungstenite(&msg);
                    write.send(tung_msg).await.map_err(|e| {
                        HyperSocketError::WebSocket(format!("Failed to send heartbeat: {}", e))
                    })?;
                    metrics.increment_sent();
                    debug!("Heartbeat sent successfully");
                }
                // Timeout is normal, continue loop
            }
        }
    }
}

/// Convert WsMessage to tungstenite Message
fn ws_message_to_tungstenite(msg: &WsMessage) -> Message {
    match msg {
        WsMessage::Text(text) => Message::Text(text.clone()),
        WsMessage::Binary(data) => Message::Binary(data.clone()),
    }
}

/// Convert tungstenite Message to WsMessage
fn tungstenite_to_ws_message(msg: Message) -> Option<WsMessage> {
    match msg {
        Message::Text(text) => Some(WsMessage::Text(text)),
        Message::Binary(data) => Some(WsMessage::Binary(data)),
        Message::Ping(_) | Message::Pong(_) | Message::Close(_) | Message::Frame(_) => None,
    }
}
