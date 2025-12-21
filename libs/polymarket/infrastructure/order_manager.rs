//! Order Manager
//!
//! Connects to Polymarket user WebSocket channel to receive real-time
//! order and trade updates. Organizes orders per asset_id with separate
//! tracking for bids (BUY), asks (SELL), and fills (trades).
//!
//! See: https://docs.polymarket.com/developers/CLOB/websocket/user-channel

use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// WebSocket URL for user channel
const USER_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";

/// Heartbeat interval in seconds
const HEARTBEAT_INTERVAL_SECS: u64 = 5;

// =============================================================================
// Message Types (Wire Format)
// =============================================================================

/// Subscription message for the user channel
#[derive(Debug, Clone, Serialize)]
pub struct UserSubscription {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub auth: AuthPayload,
}

impl UserSubscription {
    pub fn new(api_key: String, secret: String, passphrase: String) -> Self {
        Self {
            msg_type: "user".to_string(),
            auth: AuthPayload {
                api_key,
                secret,
                passphrase,
            },
        }
    }
}

/// Authentication payload
#[derive(Debug, Clone, Serialize)]
pub struct AuthPayload {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

/// Order message from WebSocket
#[derive(Debug, Clone, Deserialize)]
pub struct OrderMessage {
    pub id: String,
    pub owner: String,
    pub market: String,
    pub asset_id: String,
    pub side: String,
    #[serde(default)]
    pub order_owner: Option<String>,
    pub original_size: String,
    pub size_matched: String,
    pub price: String,
    #[serde(default)]
    pub associate_trades: Vec<String>,
    pub outcome: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub expiration: Option<String>,
    #[serde(default)]
    pub order_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub maker_address: Option<String>,
    pub timestamp: String,
    pub event_type: String,
}

/// Trade message from WebSocket
#[derive(Debug, Clone, Deserialize)]
pub struct TradeMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    #[serde(default)]
    pub taker_order_id: Option<String>,
    pub market: String,
    pub asset_id: String,
    pub side: String,
    pub size: String,
    #[serde(default)]
    pub fee_rate_bps: Option<String>,
    pub price: String,
    pub status: String,
    #[serde(default)]
    pub match_time: Option<String>,
    #[serde(default)]
    pub last_update: Option<String>,
    pub outcome: String,
    pub owner: String,
    #[serde(default)]
    pub trade_owner: Option<String>,
    #[serde(default)]
    pub maker_address: Option<String>,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub bucket_index: Option<u32>,
    #[serde(default)]
    pub maker_orders: Vec<MakerOrderMsg>,
    #[serde(default)]
    pub trader_side: Option<String>,
    pub timestamp: String,
    pub event_type: String,
}

/// Maker order in trade message
#[derive(Debug, Clone, Deserialize)]
pub struct MakerOrderMsg {
    pub order_id: String,
    pub owner: String,
    #[serde(default)]
    pub maker_address: Option<String>,
    pub matched_amount: String,
    pub price: String,
    #[serde(default)]
    pub fee_rate_bps: Option<String>,
    pub asset_id: String,
    pub outcome: String,
    #[serde(default)]
    pub outcome_index: Option<u32>,
    pub side: String,
}

/// Union type for WebSocket messages
#[derive(Debug, Clone)]
pub enum UserMessage {
    Order(OrderMessage),
    Trade(TradeMessage),
    Pong,
    Unknown(String),
}

// =============================================================================
// Domain Types
// =============================================================================

/// Side of an order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => Side::Buy,
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

/// Order status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    /// Order is active (LIVE)
    Open,
    /// Order is partially filled (size_matched > 0 but < original_size)
    PartiallyFilled,
    /// Order is fully filled (size_matched >= original_size)
    Filled,
    /// Order was cancelled
    Cancelled,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Open => write!(f, "OPEN"),
            OrderStatus::PartiallyFilled => write!(f, "PARTIAL"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

/// Trade/Fill status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStatus {
    Matched,
    Mined,
    Confirmed,
    Retrying,
    Failed,
}

impl TradeStatus {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "MATCHED" => TradeStatus::Matched,
            "MINED" => TradeStatus::Mined,
            "CONFIRMED" => TradeStatus::Confirmed,
            "RETRYING" => TradeStatus::Retrying,
            "FAILED" => TradeStatus::Failed,
            _ => TradeStatus::Matched,
        }
    }
}

impl std::fmt::Display for TradeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeStatus::Matched => write!(f, "MATCHED"),
            TradeStatus::Mined => write!(f, "MINED"),
            TradeStatus::Confirmed => write!(f, "CONFIRMED"),
            TradeStatus::Retrying => write!(f, "RETRYING"),
            TradeStatus::Failed => write!(f, "FAILED"),
        }
    }
}

/// Order state
#[derive(Debug, Clone)]
pub struct Order {
    pub order_id: String,
    pub asset_id: String,
    pub market: String,
    pub side: Side,
    pub outcome: String,
    pub price: f64,
    pub original_size: f64,
    pub size_matched: f64,
    pub status: OrderStatus,
    pub order_type: String,
    pub maker_address: String,
    pub associate_trades: Vec<String>,
    pub created_at: String,
    pub timestamp: String,
}

impl Order {
    /// Remaining size to be filled
    pub fn remaining_size(&self) -> f64 {
        self.original_size - self.size_matched
    }

    /// Check if order is open (can still be filled or cancelled)
    pub fn is_open(&self) -> bool {
        matches!(self.status, OrderStatus::Open | OrderStatus::PartiallyFilled)
    }
}

/// Fill/Trade state
#[derive(Debug, Clone)]
pub struct Fill {
    pub trade_id: String,
    pub asset_id: String,
    pub market: String,
    pub side: Side,
    pub outcome: String,
    pub price: f64,
    pub size: f64,
    pub status: TradeStatus,
    pub taker_order_id: String,
    pub trader_side: String,
    pub transaction_hash: Option<String>,
    pub fee_rate_bps: f64,
    pub maker_orders: Vec<MakerOrderInfo>,
    pub timestamp: String,
}

/// Maker order info in a fill
#[derive(Debug, Clone)]
pub struct MakerOrderInfo {
    pub order_id: String,
    pub asset_id: String,
    pub matched_amount: f64,
    pub price: f64,
    pub owner: String,
    pub side: Side,
}

// =============================================================================
// Asset Order Book
// =============================================================================

/// Order book for a single asset
#[derive(Debug, Default)]
pub struct AssetOrderBook {
    pub asset_id: String,
    /// BUY orders (bids)
    bids: HashMap<String, Order>,
    /// SELL orders (asks)
    asks: HashMap<String, Order>,
    /// All fills/trades for this asset
    fills: Vec<Fill>,
}

impl AssetOrderBook {
    pub fn new(asset_id: String) -> Self {
        Self {
            asset_id,
            bids: HashMap::new(),
            asks: HashMap::new(),
            fills: Vec::new(),
        }
    }

    /// Add or update an order
    pub fn upsert_order(&mut self, order: Order) {
        let map = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        map.insert(order.order_id.clone(), order);
    }

    /// Remove an order (on cancellation or full fill)
    pub fn remove_order(&mut self, order_id: &str, side: Side) {
        let map = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        map.remove(order_id);
    }

    /// Get an order by ID (checks both bids and asks)
    pub fn get_order(&self, order_id: &str) -> Option<&Order> {
        self.bids.get(order_id).or_else(|| self.asks.get(order_id))
    }

    /// Get mutable order by ID
    pub fn get_order_mut(&mut self, order_id: &str) -> Option<&mut Order> {
        if self.bids.contains_key(order_id) {
            self.bids.get_mut(order_id)
        } else {
            self.asks.get_mut(order_id)
        }
    }

    /// Add a fill
    pub fn add_fill(&mut self, fill: Fill) {
        self.fills.push(fill);
    }

    /// Get all bids
    pub fn bids(&self) -> Vec<&Order> {
        self.bids.values().collect()
    }

    /// Get all asks
    pub fn asks(&self) -> Vec<&Order> {
        self.asks.values().collect()
    }

    /// Get all fills
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    /// Get open orders (bids + asks that are still open)
    pub fn open_orders(&self) -> Vec<&Order> {
        self.bids
            .values()
            .chain(self.asks.values())
            .filter(|o| o.is_open())
            .collect()
    }

    /// Total bid size (sum of remaining sizes)
    pub fn total_bid_size(&self) -> f64 {
        self.bids.values().filter(|o| o.is_open()).map(|o| o.remaining_size()).sum()
    }

    /// Total ask size (sum of remaining sizes)
    pub fn total_ask_size(&self) -> f64 {
        self.asks.values().filter(|o| o.is_open()).map(|o| o.remaining_size()).sum()
    }

    /// Total fill volume
    pub fn total_fill_volume(&self) -> f64 {
        self.fills.iter().map(|f| f.size).sum()
    }

    /// Bid count
    pub fn bid_count(&self) -> usize {
        self.bids.len()
    }

    /// Ask count
    pub fn ask_count(&self) -> usize {
        self.asks.len()
    }

    /// Fill count
    pub fn fill_count(&self) -> usize {
        self.fills.len()
    }
}

// =============================================================================
// Order State Store
// =============================================================================

/// Central state store for all assets
#[derive(Debug, Default)]
pub struct OrderStateStore {
    /// Per-asset order books
    assets: HashMap<String, AssetOrderBook>,
    /// Quick lookup: order_id -> asset_id
    order_to_asset: HashMap<String, String>,
}

impl OrderStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create an asset order book
    fn get_or_create_asset(&mut self, asset_id: &str) -> &mut AssetOrderBook {
        if !self.assets.contains_key(asset_id) {
            self.assets
                .insert(asset_id.to_string(), AssetOrderBook::new(asset_id.to_string()));
        }
        self.assets.get_mut(asset_id).unwrap()
    }

    /// Process an order message
    pub fn process_order(&mut self, msg: &OrderMessage) {
        let order_type = msg.msg_type.to_uppercase();
        let side = Side::from_str(&msg.side);
        let original_size: f64 = msg.original_size.parse().unwrap_or(0.0);
        let size_matched: f64 = msg.size_matched.parse().unwrap_or(0.0);
        let price: f64 = msg.price.parse().unwrap_or(0.0);

        // Determine order status
        let status = match order_type.as_str() {
            "CANCELLATION" => OrderStatus::Cancelled,
            _ => {
                if size_matched >= original_size && original_size > 0.0 {
                    OrderStatus::Filled
                } else if size_matched > 0.0 {
                    OrderStatus::PartiallyFilled
                } else {
                    OrderStatus::Open
                }
            }
        };

        let order = Order {
            order_id: msg.id.clone(),
            asset_id: msg.asset_id.clone(),
            market: msg.market.clone(),
            side,
            outcome: msg.outcome.clone(),
            price,
            original_size,
            size_matched,
            status,
            order_type: msg.order_type.clone().unwrap_or_default(),
            maker_address: msg.maker_address.clone().unwrap_or_default(),
            associate_trades: msg.associate_trades.clone(),
            created_at: msg.created_at.clone().unwrap_or_default(),
            timestamp: msg.timestamp.clone(),
        };

        // Update lookup map
        self.order_to_asset
            .insert(msg.id.clone(), msg.asset_id.clone());

        // Update asset order book
        let book = self.get_or_create_asset(&msg.asset_id);
        book.upsert_order(order);
    }

    /// Process a trade message
    pub fn process_trade(&mut self, msg: &TradeMessage) {
        let fill = Fill {
            trade_id: msg.id.clone(),
            asset_id: msg.asset_id.clone(),
            market: msg.market.clone(),
            side: Side::from_str(&msg.side),
            outcome: msg.outcome.clone(),
            price: msg.price.parse().unwrap_or(0.0),
            size: msg.size.parse().unwrap_or(0.0),
            status: TradeStatus::from_str(&msg.status),
            taker_order_id: msg.taker_order_id.clone().unwrap_or_default(),
            trader_side: msg.trader_side.clone().unwrap_or_default(),
            transaction_hash: msg.transaction_hash.clone(),
            fee_rate_bps: msg
                .fee_rate_bps
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            maker_orders: msg
                .maker_orders
                .iter()
                .map(|m| MakerOrderInfo {
                    order_id: m.order_id.clone(),
                    asset_id: m.asset_id.clone(),
                    matched_amount: m.matched_amount.parse().unwrap_or(0.0),
                    price: m.price.parse().unwrap_or(0.0),
                    owner: m.owner.clone(),
                    side: Side::from_str(&m.side),
                })
                .collect(),
            timestamp: msg.timestamp.clone(),
        };

        let book = self.get_or_create_asset(&msg.asset_id);
        book.add_fill(fill);
    }

    // =========================================================================
    // Query Methods - Per Asset
    // =========================================================================

    /// Get bids for an asset
    pub fn get_bids(&self, asset_id: &str) -> Vec<Order> {
        self.assets
            .get(asset_id)
            .map(|b| b.bids().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get asks for an asset
    pub fn get_asks(&self, asset_id: &str) -> Vec<Order> {
        self.assets
            .get(asset_id)
            .map(|b| b.asks().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get fills for an asset
    pub fn get_fills(&self, asset_id: &str) -> Vec<Fill> {
        self.assets
            .get(asset_id)
            .map(|b| b.fills().to_vec())
            .unwrap_or_default()
    }

    /// Get open orders for an asset
    pub fn get_open_orders(&self, asset_id: &str) -> Vec<Order> {
        self.assets
            .get(asset_id)
            .map(|b| b.open_orders().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    // =========================================================================
    // Query Methods - By Order ID
    // =========================================================================

    /// Get order by ID
    pub fn get_order(&self, order_id: &str) -> Option<Order> {
        self.order_to_asset
            .get(order_id)
            .and_then(|asset_id| self.assets.get(asset_id))
            .and_then(|book| book.get_order(order_id))
            .cloned()
    }

    // =========================================================================
    // Query Methods - Aggregations
    // =========================================================================

    /// Total bid size for an asset
    pub fn total_bid_size(&self, asset_id: &str) -> f64 {
        self.assets
            .get(asset_id)
            .map(|b| b.total_bid_size())
            .unwrap_or(0.0)
    }

    /// Total ask size for an asset
    pub fn total_ask_size(&self, asset_id: &str) -> f64 {
        self.assets
            .get(asset_id)
            .map(|b| b.total_ask_size())
            .unwrap_or(0.0)
    }

    /// Total fill volume for an asset
    pub fn total_fill_volume(&self, asset_id: &str) -> f64 {
        self.assets
            .get(asset_id)
            .map(|b| b.total_fill_volume())
            .unwrap_or(0.0)
    }

    // =========================================================================
    // Global Stats
    // =========================================================================

    /// Total order count across all assets
    pub fn order_count(&self) -> usize {
        self.assets
            .values()
            .map(|b| b.bid_count() + b.ask_count())
            .sum()
    }

    /// Total fill count across all assets
    pub fn fill_count(&self) -> usize {
        self.assets.values().map(|b| b.fill_count()).sum()
    }

    /// Number of assets being tracked
    pub fn asset_count(&self) -> usize {
        self.assets.len()
    }

    /// Get all asset IDs
    pub fn asset_ids(&self) -> Vec<String> {
        self.assets.keys().cloned().collect()
    }

    /// Get asset order book
    pub fn get_asset_book(&self, asset_id: &str) -> Option<&AssetOrderBook> {
        self.assets.get(asset_id)
    }
}

pub type SharedOrderState = Arc<RwLock<OrderStateStore>>;

// =============================================================================
// Router & Handler
// =============================================================================

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum OrderRoute {
    User,
}

pub struct OrderRouter;

#[async_trait::async_trait]
impl MessageRouter for OrderRouter {
    type Message = UserMessage;
    type RouteKey = OrderRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(UserMessage::Unknown("Binary data".to_string())),
        };

        if text == "PONG" {
            return Ok(UserMessage::Pong);
        }

        // Try to parse as order message
        if let Ok(order) = serde_json::from_str::<OrderMessage>(text) {
            if order.event_type == "order" {
                return Ok(UserMessage::Order(order));
            }
        }

        // Try to parse as trade message
        if let Ok(trade) = serde_json::from_str::<TradeMessage>(text) {
            if trade.event_type == "trade" {
                return Ok(UserMessage::Trade(trade));
            }
        }

        debug!("[OrderManager] Unknown message: {}", text);
        Ok(UserMessage::Unknown(text.to_string()))
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        OrderRoute::User
    }
}

pub struct OrderHandler {
    state: SharedOrderState,
    message_count: u64,
    order_count: u64,
    trade_count: u64,
}

impl OrderHandler {
    pub fn new(state: SharedOrderState) -> Self {
        Self {
            state,
            message_count: 0,
            order_count: 0,
            trade_count: 0,
        }
    }

    fn handle_order(&mut self, order: &OrderMessage) {
        self.order_count += 1;

        let status = order.status.as_deref().unwrap_or("UNKNOWN");
        info!(
            "[OrderManager] Order {}: {} {} {} @ {} (matched: {}/{}) [{}]",
            order.msg_type,
            order.side,
            order.outcome,
            &order.asset_id[..12.min(order.asset_id.len())],
            order.price,
            order.size_matched,
            order.original_size,
            status
        );

        if let Ok(mut store) = self.state.write() {
            store.process_order(order);
        }
    }

    fn handle_trade(&mut self, trade: &TradeMessage) {
        self.trade_count += 1;

        let trader_side = trade.trader_side.as_deref().unwrap_or("UNKNOWN");
        info!(
            "[OrderManager] Trade: {} {} {} @ {} (size: {}, status: {}) [{}]",
            trade.side,
            trade.outcome,
            &trade.asset_id[..12.min(trade.asset_id.len())],
            trade.price,
            trade.size,
            trade.status,
            trader_side
        );

        if let Ok(mut store) = self.state.write() {
            store.process_trade(trade);
        }
    }
}

impl MessageHandler<UserMessage> for OrderHandler {
    fn handle(&mut self, message: UserMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            UserMessage::Order(order) => self.handle_order(&order),
            UserMessage::Trade(trade) => self.handle_trade(&trade),
            UserMessage::Pong => debug!("[OrderManager] Pong received"),
            UserMessage::Unknown(msg) => {
                if !msg.is_empty() && msg != "PONG" {
                    debug!("[OrderManager] Unknown message: {}", msg);
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// OrderManager
// =============================================================================

/// Order Manager - Tracks orders and trades via WebSocket
///
/// Organizes orders per asset_id with separate tracking for:
/// - Bids (BUY orders)
/// - Asks (SELL orders)
/// - Fills (trades)
///
/// Lifecycle: `new()` → `start(shutdown_flag)` → `stop()`
pub struct OrderManager {
    state: SharedOrderState,
    task_handle: Option<JoinHandle<()>>,
}

impl OrderManager {
    /// Create a new OrderManager instance
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(OrderStateStore::new())),
            task_handle: None,
        }
    }

    /// Start the order tracking WebSocket connection
    ///
    /// Loads credentials from environment variables:
    /// - `API_KEY` - Polymarket API key
    /// - `API_SECRET` - Polymarket API secret
    /// - `API_PASSPHRASE` - Polymarket API passphrase
    pub async fn start(&mut self, shutdown_flag: Arc<AtomicBool>) -> Result<()> {
        let api_key = std::env::var("API_KEY")
            .map_err(|_| anyhow::anyhow!("API_KEY environment variable not set"))?;
        let api_secret = std::env::var("API_SECRET")
            .map_err(|_| anyhow::anyhow!("API_SECRET environment variable not set"))?;
        let api_passphrase = std::env::var("API_PASSPHRASE")
            .map_err(|_| anyhow::anyhow!("API_PASSPHRASE environment variable not set"))?;

        info!("[OrderManager] Starting...");
        info!(
            "[OrderManager] API Key: {}...",
            &api_key[..8.min(api_key.len())]
        );

        let state = Arc::clone(&self.state);
        let shutdown_clone = Arc::clone(&shutdown_flag);

        let handle = tokio::spawn(async move {
            if let Err(e) = run_order_tracker(
                api_key,
                api_secret,
                api_passphrase,
                state,
                shutdown_clone,
            )
            .await
            {
                warn!("[OrderManager] Tracker error: {}", e);
            }
        });

        self.task_handle = Some(handle);

        // Give the connection a moment to establish
        sleep(Duration::from_millis(100)).await;

        info!("[OrderManager] Started");
        Ok(())
    }

    /// Stop the order tracking WebSocket connection
    pub async fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
            let _ = handle.await;
            info!("[OrderManager] Stopped");
        }
    }

    /// Get the shared state for direct access
    pub fn state(&self) -> SharedOrderState {
        Arc::clone(&self.state)
    }

    // =========================================================================
    // Query Methods - Per Asset
    // =========================================================================

    /// Get all bids (BUY orders) for an asset
    pub fn get_bids(&self, asset_id: &str) -> Vec<Order> {
        self.state
            .read()
            .ok()
            .map(|s| s.get_bids(asset_id))
            .unwrap_or_default()
    }

    /// Get all asks (SELL orders) for an asset
    pub fn get_asks(&self, asset_id: &str) -> Vec<Order> {
        self.state
            .read()
            .ok()
            .map(|s| s.get_asks(asset_id))
            .unwrap_or_default()
    }

    /// Get all fills (trades) for an asset
    pub fn get_fills(&self, asset_id: &str) -> Vec<Fill> {
        self.state
            .read()
            .ok()
            .map(|s| s.get_fills(asset_id))
            .unwrap_or_default()
    }

    /// Get all open orders for an asset (bids + asks that are still open)
    pub fn get_open_orders(&self, asset_id: &str) -> Vec<Order> {
        self.state
            .read()
            .ok()
            .map(|s| s.get_open_orders(asset_id))
            .unwrap_or_default()
    }

    // =========================================================================
    // Query Methods - By Order ID
    // =========================================================================

    /// Get an order by its ID
    pub fn get_order(&self, order_id: &str) -> Option<Order> {
        self.state.read().ok().and_then(|s| s.get_order(order_id))
    }

    // =========================================================================
    // Query Methods - Aggregations
    // =========================================================================

    /// Total bid size for an asset (sum of remaining sizes)
    pub fn total_bid_size(&self, asset_id: &str) -> f64 {
        self.state
            .read()
            .ok()
            .map(|s| s.total_bid_size(asset_id))
            .unwrap_or(0.0)
    }

    /// Total ask size for an asset (sum of remaining sizes)
    pub fn total_ask_size(&self, asset_id: &str) -> f64 {
        self.state
            .read()
            .ok()
            .map(|s| s.total_ask_size(asset_id))
            .unwrap_or(0.0)
    }

    /// Total fill volume for an asset
    pub fn total_fill_volume(&self, asset_id: &str) -> f64 {
        self.state
            .read()
            .ok()
            .map(|s| s.total_fill_volume(asset_id))
            .unwrap_or(0.0)
    }

    // =========================================================================
    // Global Stats
    // =========================================================================

    /// Total order count across all assets
    pub fn order_count(&self) -> usize {
        self.state.read().ok().map(|s| s.order_count()).unwrap_or(0)
    }

    /// Total fill count across all assets
    pub fn fill_count(&self) -> usize {
        self.state.read().ok().map(|s| s.fill_count()).unwrap_or(0)
    }

    /// Number of assets being tracked
    pub fn asset_count(&self) -> usize {
        self.state.read().ok().map(|s| s.asset_count()).unwrap_or(0)
    }

    /// Get all tracked asset IDs
    pub fn asset_ids(&self) -> Vec<String> {
        self.state
            .read()
            .ok()
            .map(|s| s.asset_ids())
            .unwrap_or_default()
    }
}

impl Default for OrderManager {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Internal Functions
// =============================================================================

async fn build_ws_client(
    api_key: String,
    api_secret: String,
    api_passphrase: String,
    state: SharedOrderState,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<WebSocketClient<OrderRouter, UserMessage>> {
    let subscription = UserSubscription::new(api_key, api_secret, api_passphrase);
    let subscription_json = serde_json::to_string(&subscription)?;

    let router = OrderRouter;
    let handler = OrderHandler::new(state);

    let client = WebSocketClientBuilder::new()
        .url(USER_WS_URL)
        .router(router, move |routing| {
            routing.handler(OrderRoute::User, handler)
        })
        .heartbeat(
            Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
            WsMessage::Text("PING".to_string()),
        )
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

fn handle_client_event(event: ClientEvent) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[OrderManager] Connected to user channel");
            true
        }
        ClientEvent::Disconnected => {
            warn!("[OrderManager] Disconnected from user channel");
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[OrderManager] Reconnecting (attempt {})", attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[OrderManager] Error: {}", err);
            true
        }
    }
}

async fn run_order_tracker(
    api_key: String,
    api_secret: String,
    api_passphrase: String,
    state: SharedOrderState,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    let client = build_ws_client(
        api_key,
        api_secret,
        api_passphrase,
        state,
        Arc::clone(&shutdown_flag),
    )
    .await?;

    info!("[OrderManager] Connected and authenticated");

    loop {
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[OrderManager] Shutdown signal received");
            break;
        }

        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event) {
                    break;
                }
            }
            None => {
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[OrderManager] Closing connection");
    if let Err(e) = client.shutdown().await {
        warn!("[OrderManager] Error during shutdown: {}", e);
    }
    info!("[OrderManager] Tracker stopped");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order_msg(
        id: &str,
        asset_id: &str,
        side: &str,
        msg_type: &str,
        original_size: &str,
        size_matched: &str,
        price: &str,
    ) -> OrderMessage {
        OrderMessage {
            id: id.to_string(),
            owner: "owner".to_string(),
            market: "market-1".to_string(),
            asset_id: asset_id.to_string(),
            side: side.to_string(),
            order_owner: None,
            original_size: original_size.to_string(),
            size_matched: size_matched.to_string(),
            price: price.to_string(),
            associate_trades: vec![],
            outcome: "Up".to_string(),
            msg_type: msg_type.to_string(),
            created_at: None,
            expiration: None,
            order_type: Some("GTC".to_string()),
            status: Some("LIVE".to_string()),
            maker_address: None,
            timestamp: "123456789".to_string(),
            event_type: "order".to_string(),
        }
    }

    #[test]
    fn test_order_placement_bid() {
        let mut store = OrderStateStore::new();
        let msg = make_order_msg("order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5");
        store.process_order(&msg);

        let bids = store.get_bids("asset-1");
        assert_eq!(bids.len(), 1);
        assert_eq!(bids[0].side, Side::Buy);
        assert_eq!(bids[0].status, OrderStatus::Open);
    }

    #[test]
    fn test_order_placement_ask() {
        let mut store = OrderStateStore::new();
        let msg = make_order_msg("order-1", "asset-1", "SELL", "PLACEMENT", "100", "0", "0.5");
        store.process_order(&msg);

        let asks = store.get_asks("asset-1");
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].side, Side::Sell);
    }

    #[test]
    fn test_partial_fill() {
        let mut store = OrderStateStore::new();
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5",
        ));
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "UPDATE", "100", "50", "0.5",
        ));

        let order = store.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::PartiallyFilled);
        assert_eq!(order.size_matched, 50.0);
    }

    #[test]
    fn test_full_fill() {
        let mut store = OrderStateStore::new();
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5",
        ));
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "UPDATE", "100", "100", "0.5",
        ));

        let order = store.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_cancellation() {
        let mut store = OrderStateStore::new();
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5",
        ));
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "CANCELLATION", "100", "0", "0.5",
        ));

        let order = store.get_order("order-1").unwrap();
        assert_eq!(order.status, OrderStatus::Cancelled);
    }

    #[test]
    fn test_multiple_assets() {
        let mut store = OrderStateStore::new();
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5",
        ));
        store.process_order(&make_order_msg(
            "order-2", "asset-1", "SELL", "PLACEMENT", "50", "0", "0.6",
        ));
        store.process_order(&make_order_msg(
            "order-3", "asset-2", "BUY", "PLACEMENT", "200", "0", "0.4",
        ));

        assert_eq!(store.asset_count(), 2);
        assert_eq!(store.get_bids("asset-1").len(), 1);
        assert_eq!(store.get_asks("asset-1").len(), 1);
        assert_eq!(store.get_bids("asset-2").len(), 1);
    }

    #[test]
    fn test_aggregations() {
        let mut store = OrderStateStore::new();
        store.process_order(&make_order_msg(
            "order-1", "asset-1", "BUY", "PLACEMENT", "100", "0", "0.5",
        ));
        store.process_order(&make_order_msg(
            "order-2", "asset-1", "BUY", "PLACEMENT", "50", "0", "0.4",
        ));

        assert_eq!(store.total_bid_size("asset-1"), 150.0);
    }
}
