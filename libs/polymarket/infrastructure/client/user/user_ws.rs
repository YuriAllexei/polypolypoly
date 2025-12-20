//! User Channel WebSocket Client
//!
//! Connects to the Polymarket user WebSocket channel for real-time
//! order and trade updates.
//!
//! See: https://docs.polymarket.com/developers/CLOB/websocket/user-channel

use super::order_manager::{OrderManager, SharedOrderManager};
use super::types::{OrderMessage, TradeMessage, UserMessage, UserSubscription};
use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// WebSocket URL for user channel
const USER_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";

/// Heartbeat interval in seconds
const HEARTBEAT_INTERVAL_SECS: u64 = 5;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for user WebSocket connection
pub struct UserConfig {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
}

impl UserConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("API_KEY")
            .map_err(|_| anyhow::anyhow!("API_KEY environment variable not set"))?;
        let api_secret = std::env::var("API_SECRET")
            .map_err(|_| anyhow::anyhow!("API_SECRET environment variable not set"))?;
        let api_passphrase = std::env::var("API_PASSPHRASE")
            .map_err(|_| anyhow::anyhow!("API_PASSPHRASE environment variable not set"))?;

        Ok(Self {
            api_key,
            api_secret,
            api_passphrase,
        })
    }

    /// Create subscription message
    pub fn subscription(&self) -> UserSubscription {
        UserSubscription::new(
            self.api_key.clone(),
            self.api_secret.clone(),
            self.api_passphrase.clone(),
        )
    }
}

// =============================================================================
// Router
// =============================================================================

/// Route key for user messages
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum UserRoute {
    User,
}

/// Router for parsing user channel WebSocket messages
pub struct UserRouter;

#[async_trait::async_trait]
impl MessageRouter for UserRouter {
    type Message = UserMessage;
    type RouteKey = UserRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(UserMessage::Unknown("Binary data".to_string())),
        };

        // Check for PONG response
        if text == "PONG" {
            return Ok(UserMessage::Pong);
        }

        // Try to parse as trade message (check event_type field)
        if let Ok(trade) = serde_json::from_str::<TradeMessage>(text) {
            if trade.event_type == "trade" {
                return Ok(UserMessage::Trade(trade));
            }
        }

        // Try to parse as order message
        if let Ok(order) = serde_json::from_str::<OrderMessage>(text) {
            if order.event_type == "order" {
                return Ok(UserMessage::Order(order));
            }
        }

        // Unknown message
        debug!("[UserWS] Unknown message: {}", text);
        Ok(UserMessage::Unknown(text.to_string()))
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        UserRoute::User
    }
}

// =============================================================================
// Handler
// =============================================================================

/// Handler for processing user channel messages
pub struct UserHandler {
    orders: SharedOrderManager,
    message_count: u64,
    order_count: u64,
    trade_count: u64,
}

impl UserHandler {
    pub fn new(orders: SharedOrderManager) -> Self {
        Self {
            orders,
            message_count: 0,
            order_count: 0,
            trade_count: 0,
        }
    }

    fn handle_trade(&mut self, trade: &TradeMessage) {
        self.trade_count += 1;

        info!(
            "[UserWS] Trade: {} {} {} @ {} (size: {}, status: {})",
            trade.side, trade.outcome, trade.asset_id, trade.price, trade.size, trade.status
        );

        // Update order manager
        self.orders.write().process_trade(trade);
    }

    fn handle_order(&mut self, order: &OrderMessage) {
        self.order_count += 1;

        info!(
            "[UserWS] Order {}: {} {} {} @ {} (matched: {}/{})",
            order.msg_type,
            order.side,
            order.outcome,
            order.asset_id,
            order.price,
            order.size_matched,
            order.original_size
        );

        // Update order manager
        self.orders.write().process_order(order);
    }
}

impl MessageHandler<UserMessage> for UserHandler {
    fn handle(&mut self, message: UserMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            UserMessage::Trade(trade) => self.handle_trade(&trade),
            UserMessage::Order(order) => self.handle_order(&order),
            UserMessage::Pong => debug!("[UserWS] Pong received"),
            UserMessage::Unknown(msg) => {
                if !msg.is_empty() && msg != "PONG" {
                    debug!("[UserWS] Unknown message: {}", msg);
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client
// =============================================================================

/// Build a WebSocket client for the user channel
async fn build_ws_client(
    config: &UserConfig,
    orders: SharedOrderManager,
) -> Result<WebSocketClient<UserRouter, UserMessage>> {
    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = UserRouter;
    let handler = UserHandler::new(orders);

    let subscription = config.subscription();
    let subscription_json = serde_json::to_string(&subscription)?;

    let client = WebSocketClientBuilder::new()
        .url(USER_WS_URL)
        .router(router, move |routing| {
            routing.handler(UserRoute::User, handler)
        })
        .heartbeat(
            Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
            WsMessage::Text("PING".to_string()),
        )
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

/// Handle a WebSocket client event
fn handle_client_event(event: ClientEvent) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[UserWS] Connected to user channel");
            true
        }
        ClientEvent::Disconnected => {
            warn!("[UserWS] Disconnected from user channel");
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[UserWS] Reconnecting (attempt {})", attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[UserWS] Error: {}", err);
            true
        }
    }
}

// =============================================================================
// Public Entry Point
// =============================================================================

/// Spawn a user order tracker
///
/// Connects to the Polymarket user WebSocket channel and tracks order/trade
/// updates in real-time. Returns a shared OrderManager that can be queried
/// for current order state.
///
/// # Arguments
/// * `shutdown_flag` - Shared shutdown flag for graceful termination
///
/// # Returns
/// * `SharedOrderManager` - Thread-safe order state manager
///
/// # Example
/// ```ignore
/// let orders = spawn_user_order_tracker(shutdown.flag()).await?;
///
/// // Later, query order state
/// let mgr = orders.read().unwrap();
/// if let Some(order) = mgr.get_order("order-123") {
///     println!("Order status: {:?}", order.status);
/// }
/// ```
pub async fn spawn_user_order_tracker(
    shutdown_flag: Arc<AtomicBool>,
) -> Result<SharedOrderManager> {
    // Load configuration from environment
    let config = UserConfig::from_env()?;

    info!("[UserWS] Starting user order tracker...");
    info!("[UserWS] API Key: {}...", &config.api_key[..8.min(config.api_key.len())]);

    // Create shared order manager
    let orders: SharedOrderManager = Arc::new(RwLock::new(OrderManager::new()));
    let orders_clone = Arc::clone(&orders);

    // Clone shutdown flag for the spawned task
    let shutdown_clone = Arc::clone(&shutdown_flag);

    // Spawn tracker task
    tokio::spawn(async move {
        if let Err(e) = run_user_tracker(config, orders_clone, shutdown_clone).await {
            warn!("[UserWS] User tracker error: {}", e);
        }
    });

    // Give the connection a moment to establish
    sleep(Duration::from_millis(100)).await;

    Ok(orders)
}

/// Internal function to run the user tracker
async fn run_user_tracker(
    config: UserConfig,
    orders: SharedOrderManager,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    // Build and connect WebSocket client
    let client = build_ws_client(&config, orders).await?;
    info!("[UserWS] Connected and authenticated");

    // Main tracking loop
    loop {
        // Check shutdown flag (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[UserWS] Shutdown signal received");
            break;
        }

        // Handle WebSocket events
        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event) {
                    break;
                }
            }
            None => {
                // No event available, sleep briefly
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[UserWS] Closing connection");
    if let Err(e) = client.shutdown().await {
        warn!("[UserWS] Error during shutdown: {}", e);
    }
    info!("[UserWS] User tracker stopped");

    Ok(())
}
