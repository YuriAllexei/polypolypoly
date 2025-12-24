//! User Channel WebSocket Client
//!
//! Connects to the Polymarket user WebSocket channel for real-time
//! order and trade updates. Automatically hydrates from REST API on startup.
//!
//! See: https://docs.polymarket.com/developers/CLOB/websocket/user-channel

use super::super::auth::PolymarketAuth;
use super::super::clob::rest::RestClient;
use super::order_manager::{OrderEvent, OrderEventCallback, OrderStateStore, SharedOrderState};
use super::types::{OrderMessage, TradeMessage, UserMessage, UserSubscription};
use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, TextPongDetector, WsMessage};
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
    state: SharedOrderState,
    callback: Arc<dyn OrderEventCallback>,
}

impl UserHandler {
    pub fn new(state: SharedOrderState) -> Self {
        let callback = state.read().callback().clone();
        Self { state, callback }
    }

    fn handle_trade(&mut self, trade: &TradeMessage) {
        let trader_side = trade.trader_side.as_deref().unwrap_or("UNKNOWN");

        // Process the trade first to get the corrected Fill (with our perspective for MAKER trades)
        let event = self.state.write().process_trade(trade);

        if let Some(OrderEvent::Trade(ref fill)) = event {
            // Log with our corrected perspective (asset_id, price, side are adjusted for MAKER)
            info!(
                "[UserWS] Trade: {} {} {}... @ {} (size: {:.2}, status: {}, you: {})",
                fill.side, fill.outcome, &fill.asset_id[..8.min(fill.asset_id.len())],
                fill.price, fill.size, fill.status, trader_side
            );
            self.fire_callback(&event.unwrap());
        } else if event.is_none() {
            // Log the raw message for debugging when trade was filtered (duplicate/zero-size)
            debug!(
                "[UserWS] Trade filtered: {} {} {}... @ {} (raw_size: {}, you: {})",
                trade.side, trade.outcome, &trade.asset_id[..8.min(trade.asset_id.len())],
                trade.price, trade.size, trader_side
            );
        }
    }

    fn handle_order(&mut self, order: &OrderMessage) {
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

        let event = self.state.write().process_order(order);
        if let Some(event) = event {
            self.fire_callback(&event);
        }
    }

    fn fire_callback(&self, event: &OrderEvent) {
        match event {
            OrderEvent::Placed(order) => self.callback.on_order_placed(order),
            OrderEvent::Updated(order) => self.callback.on_order_updated(order),
            OrderEvent::Filled(order) => self.callback.on_order_filled(order),
            OrderEvent::Cancelled(order) => self.callback.on_order_cancelled(order),
            OrderEvent::Trade(fill) => self.callback.on_trade(fill),
        }
    }
}

impl MessageHandler<UserMessage> for UserHandler {
    fn handle(&mut self, message: UserMessage) -> hypersockets::Result<()> {
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
    state: SharedOrderState,
) -> Result<WebSocketClient<UserRouter, UserMessage>> {
    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = UserRouter;
    let handler = UserHandler::new(state);

    let subscription = config.subscription();
    let subscription_json = serde_json::to_string(&subscription)?;

    // Create PONG detector for "PONG" text messages
    let pong_detector = Arc::new(TextPongDetector::new("PONG".to_string()));

    let client = WebSocketClientBuilder::new()
        .url(USER_WS_URL)
        .router(router, move |routing| {
            routing.handler(UserRoute::User, handler)
        })
        .heartbeat(
            Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
            WsMessage::Text("PING".to_string()),
        )
        .pong_detector(pong_detector)
        .pong_timeout(Duration::from_secs(15))
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
// Public Entry Points
// =============================================================================

/// Spawn a user order tracker with automatic REST hydration
///
/// 1. Hydrates existing orders from REST API
/// 2. Hydrates recent trades from REST API
/// 3. Connects to WebSocket for real-time updates
///
/// # Arguments
/// * `shutdown_flag` - Shared shutdown flag for graceful termination
/// * `rest_client` - REST client for hydration
/// * `auth` - Authentication for REST API calls
/// * `callback` - Optional callback for order/trade events
///
/// # Returns
/// * `SharedOrderState` - Thread-safe order state store
pub async fn spawn_user_order_tracker(
    shutdown_flag: Arc<AtomicBool>,
    rest_client: &RestClient,
    auth: &PolymarketAuth,
    callback: Option<Arc<dyn OrderEventCallback>>,
) -> Result<SharedOrderState> {
    // Load WebSocket configuration from environment
    let config = UserConfig::from_env()?;

    info!("[UserWS] Starting user order tracker...");
    info!(
        "[UserWS] API Key: {}...",
        &config.api_key[..8.min(config.api_key.len())]
    );

    // Create shared order state store
    let state: SharedOrderState = Arc::new(RwLock::new(match callback {
        Some(cb) => OrderStateStore::with_callback(cb),
        None => OrderStateStore::new(),
    }));

    // Hydrate from REST API
    info!("[UserWS] Hydrating orders from REST API...");
    match rest_client.get_all_orders(auth, None).await {
        Ok(orders) => {
            state.write().hydrate_orders(&orders);
            info!("[UserWS] Hydrated {} orders", orders.len());
        }
        Err(e) => {
            warn!("[UserWS] Failed to hydrate orders: {}", e);
        }
    }

    info!("[UserWS] Hydrating trades from REST API...");
    match rest_client.get_all_trades(auth, None).await {
        Ok(trades) => {
            state.write().hydrate_trades(&trades);
            info!("[UserWS] Hydrated {} trades", trades.len());
        }
        Err(e) => {
            warn!("[UserWS] Failed to hydrate trades: {}", e);
        }
    }

    // Clone for the spawned task
    let state_clone = Arc::clone(&state);
    let shutdown_clone = Arc::clone(&shutdown_flag);

    // Spawn WebSocket tracker task
    tokio::spawn(async move {
        if let Err(e) = run_user_tracker(config, state_clone, shutdown_clone).await {
            warn!("[UserWS] User tracker error: {}", e);
        }
    });

    // Give the connection a moment to establish
    sleep(Duration::from_millis(100)).await;

    Ok(state)
}

/// Spawn a user order tracker without REST hydration (WebSocket only)
///
/// Use this when you don't have a REST client available or want to start
/// with an empty state.
pub async fn spawn_user_order_tracker_ws_only(
    shutdown_flag: Arc<AtomicBool>,
    callback: Option<Arc<dyn OrderEventCallback>>,
) -> Result<SharedOrderState> {
    let config = UserConfig::from_env()?;

    info!("[UserWS] Starting user order tracker (WebSocket only)...");
    info!(
        "[UserWS] API Key: {}...",
        &config.api_key[..8.min(config.api_key.len())]
    );

    let state: SharedOrderState = Arc::new(RwLock::new(match callback {
        Some(cb) => OrderStateStore::with_callback(cb),
        None => OrderStateStore::new(),
    }));

    let state_clone = Arc::clone(&state);
    let shutdown_clone = Arc::clone(&shutdown_flag);

    tokio::spawn(async move {
        if let Err(e) = run_user_tracker(config, state_clone, shutdown_clone).await {
            warn!("[UserWS] User tracker error: {}", e);
        }
    });

    sleep(Duration::from_millis(100)).await;

    Ok(state)
}

/// Internal function to run the user tracker
async fn run_user_tracker(
    config: UserConfig,
    state: SharedOrderState,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    // Build and connect WebSocket client
    let client = build_ws_client(&config, state).await?;
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
