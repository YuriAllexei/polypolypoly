use crate::traits::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for WebSocketClient with routing support
///
/// This struct holds all the configuration needed to create and run
/// a WebSocket client with message routing. It is built using the
/// type-state builder pattern.
pub struct ClientConfig<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    /// WebSocket URL (wss:// or ws://)
    pub(crate) url: String,

    /// Message router for parsing and routing messages
    pub(crate) router: Arc<R>,

    /// Channel senders mapped by route key (for routing messages)
    pub(crate) route_senders: HashMap<R::RouteKey, crossbeam_channel::Sender<M>>,

    /// Optional authentication provider
    pub(crate) auth: Option<Arc<dyn AuthProvider>>,

    /// Optional header provider for dynamic HTTP headers
    pub(crate) headers: Option<Arc<dyn HeaderProvider>>,

    /// Optional heartbeat configuration (interval, payload)
    pub(crate) heartbeat: Option<(Duration, WsMessage)>,

    /// Optional passive ping detector
    pub(crate) passive_ping: Option<Arc<dyn PassivePingDetector>>,

    /// Reconnection strategy
    pub(crate) reconnect_strategy: Box<dyn ReconnectionStrategy>,

    /// Delay to wait after disconnection before attempting reconnection
    /// This is applied BEFORE the reconnection strategy delay
    pub(crate) reconnection_delay_offset: Duration,

    /// Subscription messages to send after connection/auth
    pub(crate) subscriptions: Vec<WsMessage>,

    /// Shutdown flag - when false, prevents reconnection attempts
    /// This allows graceful shutdown and external shutdown coordination
    pub(crate) shutdown_flag: Arc<AtomicBool>,

    /// Optional halted flag - informational flag indicating temporary disconnection
    /// Used by ClientManager to track when connections are down but reconnecting
    pub(crate) halted_flag: Option<Arc<AtomicBool>>,
}

impl<R, M> ClientConfig<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    /// Get a reference to the URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Check if authentication is configured
    pub fn has_auth(&self) -> bool {
        self.auth.is_some()
    }

    /// Check if heartbeat is configured
    pub fn has_heartbeat(&self) -> bool {
        self.heartbeat.is_some()
    }

    /// Check if passive ping detection is configured
    pub fn has_passive_ping(&self) -> bool {
        self.passive_ping.is_some()
    }

    /// Get the number of configured subscriptions
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Get the number of configured handlers
    pub fn handler_count(&self) -> usize {
        self.route_senders.len()
    }
}
