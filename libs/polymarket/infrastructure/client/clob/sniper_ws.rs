//! WebSocket client for market sniper orderbook tracking
//!
//! This module provides reusable WebSocket components for tracking Polymarket orderbooks.
//! The types are designed to be used by strategies that need real-time orderbook data.

use super::orderbook::Orderbook;
use super::sniper_ws_types::{
    BookSnapshot, LastTradePriceEvent, MarketSubscription, PriceChangeEvent, SniperMessage,
    TickSizeChangeEvent,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Shared orderbooks accessible by both handler and main loop
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for a market tracker
pub struct MarketTrackerConfig {
    pub market_id: String,
    pub market_question: String,
    pub slug: Option<String>,
    pub token_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub resolution_time: DateTime<Utc>,
}

impl MarketTrackerConfig {
    /// Create a new tracker configuration
    pub fn new(
        market_id: String,
        market_question: String,
        slug: Option<String>,
        token_ids: Vec<String>,
        outcomes: Vec<String>,
        resolution_time_str: &str,
    ) -> Result<Self> {
        let resolution_time =
            DateTime::parse_from_rfc3339(resolution_time_str)?.with_timezone(&Utc);

        Ok(Self {
            market_id,
            market_question,
            slug,
            token_ids,
            outcomes,
            resolution_time,
        })
    }

    /// Build a mapping from token_id to outcome name (e.g., "Yes", "No")
    pub fn build_outcome_map(&self) -> HashMap<String, String> {
        self.token_ids
            .iter()
            .zip(self.outcomes.iter())
            .map(|(t, o)| (t.clone(), o.clone()))
            .collect()
    }
}

// =============================================================================
// Router - Parses WebSocket messages
// =============================================================================

/// Route key for sniper messages
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum SniperRoute {
    Market(String),
}

/// Router for parsing WebSocket messages
pub struct SniperRouter {
    market_id: String,
}

impl SniperRouter {
    pub fn new(market_id: String) -> Self {
        Self { market_id }
    }
}

#[async_trait::async_trait]
impl MessageRouter for SniperRouter {
    type Message = SniperMessage;
    type RouteKey = SniperRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(SniperMessage::Unknown("Binary data".to_string())),
        };

        // Check for PONG response
        if text == "PONG" {
            return Ok(SniperMessage::Pong);
        }

        // Try to parse as JSON array (book snapshots - initial subscription)
        if let Ok(snapshots) = serde_json::from_str::<Vec<BookSnapshot>>(text) {
            if snapshots.first().map(|s| s.event_type.as_str()) == Some("book") {
                return Ok(SniperMessage::BookSnapshots(snapshots));
            }
        }

        // Try to parse as single book snapshot (sent after trades that affect the book)
        if let Ok(snapshot) = serde_json::from_str::<BookSnapshot>(text) {
            if snapshot.event_type == "book" {
                return Ok(SniperMessage::BookSnapshots(vec![snapshot]));
            }
        }

        // Try to parse as price_change event
        if let Ok(price_change) = serde_json::from_str::<PriceChangeEvent>(text) {
            if price_change.event_type == "price_change" {
                return Ok(SniperMessage::PriceChange(price_change));
            }
        }

        // Try to parse as tick_size_change event
        if let Ok(tick_change) = serde_json::from_str::<TickSizeChangeEvent>(text) {
            if tick_change.event_type == "tick_size_change" {
                return Ok(SniperMessage::TickSizeChange(tick_change));
            }
        }

        // Try to parse as last_trade_price event
        if let Ok(trade) = serde_json::from_str::<LastTradePriceEvent>(text) {
            if trade.event_type == "last_trade_price" {
                return Ok(SniperMessage::LastTradePrice(trade));
            }
        }

        // Unknown message
        debug!("[WS {}] Unknown message: {}", self.market_id, text);
        Ok(SniperMessage::Unknown(text.to_string()))
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        SniperRoute::Market(self.market_id.clone())
    }
}

// =============================================================================
// Handler - Processes and logs messages
// =============================================================================

/// Handler for processing sniper messages
pub struct SniperHandler {
    market_id: String,
    orderbooks: SharedOrderbooks,
    message_count: u64,
    /// Track last trade prices per asset
    last_trade_prices: HashMap<String, (String, String)>, // asset_id -> (price, size)
    /// Track tick sizes per asset
    tick_sizes: HashMap<String, String>, // asset_id -> tick_size
    first_snapshot_received: Arc<AtomicBool>,
}

impl SniperHandler {
    pub fn new(
        market_id: String,
        orderbooks: SharedOrderbooks,
        first_snapshot_received: Arc<AtomicBool>,
    ) -> Self {
        Self {
            market_id,
            orderbooks,
            message_count: 0,
            last_trade_prices: HashMap::new(),
            tick_sizes: HashMap::new(),
            first_snapshot_received,
        }
    }

    /// Process orderbook snapshots and update shared orderbooks
    fn handle_snapshot(&mut self, snapshots: &[BookSnapshot]) {
        if snapshots.is_empty() {
            return;
        }

        let mut obs = self.orderbooks.write().unwrap();
        for snapshot in snapshots {
            let orderbook = obs
                .entry(snapshot.asset_id.clone())
                .or_insert_with(|| Orderbook::new(snapshot.asset_id.clone()));
            orderbook.process_snapshot(&snapshot.bids, &snapshot.asks);
        }

        self.first_snapshot_received.swap(true, Ordering::Release);
    }

    /// Process price change events and update shared orderbooks
    fn handle_price_change(&mut self, event: &PriceChangeEvent) {
        let mut obs = self.orderbooks.write().unwrap();
        for change in &event.price_changes {
            let orderbook = obs
                .entry(change.asset_id.clone())
                .or_insert_with(|| Orderbook::new(change.asset_id.clone()));
            orderbook.process_update(&change.side, &change.price, &change.size);
        }
    }

    /// Process tick size change events
    fn handle_tick_size_change(&mut self, event: &TickSizeChangeEvent) {
        debug!(
            "[WS {}] Tick size change for {}: {} -> {}",
            self.market_id, event.asset_id, event.old_tick_size, event.new_tick_size
        );
        self.tick_sizes
            .insert(event.asset_id.clone(), event.new_tick_size.clone());
    }

    /// Process last trade price events
    fn handle_last_trade_price(&mut self, event: &LastTradePriceEvent) {
        debug!(
            "[WS {}] Trade: {} {} @ {} (size: {})",
            self.market_id, event.side, event.asset_id, event.price, event.size
        );
        self.last_trade_prices.insert(
            event.asset_id.clone(),
            (event.price.clone(), event.size.clone()),
        );
    }
}

impl MessageHandler<SniperMessage> for SniperHandler {
    fn handle(&mut self, message: SniperMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            SniperMessage::BookSnapshots(snapshots) => self.handle_snapshot(&snapshots),
            SniperMessage::PriceChange(event) => self.handle_price_change(&event),
            SniperMessage::TickSizeChange(event) => self.handle_tick_size_change(&event),
            SniperMessage::LastTradePrice(event) => self.handle_last_trade_price(&event),
            SniperMessage::Pong => debug!("[WS {}] Pong received", self.market_id),
            SniperMessage::Unknown(_) => {}
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build a WebSocket client for the given market configuration.
///
/// Note: Each WebSocket client uses a local shutdown flag because hypersockets
/// sets the flag to false during `client.shutdown()`, which would inadvertently
/// trigger global shutdown if shared. The global flag should be checked separately.
pub async fn build_ws_client(
    config: &MarketTrackerConfig,
    orderbooks: SharedOrderbooks,
    first_snapshot_received: Arc<AtomicBool>,
) -> Result<WebSocketClient<SniperRouter, SniperMessage>> {
    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SniperRouter::new(config.market_id.clone());
    let handler = SniperHandler::new(config.market_id.clone(), orderbooks, first_snapshot_received);

    let subscription = MarketSubscription::new(config.token_ids.clone());
    let subscription_json = serde_json::to_string(&subscription)?;

    let market_id_for_route = config.market_id.clone();
    let client = WebSocketClientBuilder::new()
        .url("wss://ws-subscriptions-clob.polymarket.com/ws/market")
        .router(router, move |routing| {
            routing.handler(SniperRoute::Market(market_id_for_route.clone()), handler)
        })
        .heartbeat(Duration::from_secs(5), WsMessage::Text("PING".to_string()))
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

// =============================================================================
// Client Event Handling
// =============================================================================

/// Handle a WebSocket client event, returning false if tracking should stop
pub fn handle_client_event(event: ClientEvent, market_id: &str) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[WS {}] WebSocket connected", market_id);
            true
        }
        ClientEvent::Disconnected => {
            warn!("[WS {}] WebSocket disconnected", market_id);
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[WS {}] Reconnecting (attempt {})", market_id, attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[WS {}] Error: {}", market_id, err);
            true
        }
    }
}
