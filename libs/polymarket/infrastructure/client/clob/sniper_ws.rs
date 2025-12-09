//! WebSocket client for market sniper orderbook tracking
//!
//! This module provides real-time orderbook tracking for markets approaching resolution.
//! When a price_change message indicates best_ask = 1 for any token, it logs
//! the event with full market details.

use super::orderbook::Orderbook;
use super::sniper_ws_types::{
    BookSnapshot, LastTradePriceEvent, MarketSubscription, PriceChangeEvent, SniperMessage,
    TickSizeChangeEvent,
};
use crate::infrastructure::SharedOraclePrices;
use anyhow::Result;
use chrono::{DateTime, Utc};
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time::sleep;
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
    pub token_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub resolution_time: DateTime<Utc>,
}

impl MarketTrackerConfig {
    /// Create a new tracker configuration
    pub fn new(
        market_id: String,
        market_question: String,
        token_ids: Vec<String>,
        outcomes: Vec<String>,
        resolution_time_str: &str,
    ) -> Result<Self> {
        let resolution_time =
            DateTime::parse_from_rfc3339(resolution_time_str)?.with_timezone(&Utc);

        Ok(Self {
            market_id,
            market_question,
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

        // Try to parse as JSON array (book snapshots)
        if let Ok(snapshots) = serde_json::from_str::<Vec<BookSnapshot>>(text) {
            if snapshots.first().map(|s| s.event_type.as_str()) == Some("book") {
                return Ok(SniperMessage::BookSnapshots(snapshots));
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
    /// Market question/name
    market_question: String,
    /// Map from token_id to outcome name (e.g., "Yes", "No", "Up", "Down")
    outcome_map: HashMap<String, String>,
    orderbooks: SharedOrderbooks,
    message_count: u64,
    /// Track last trade prices per asset
    last_trade_prices: HashMap<String, (String, String)>, // asset_id -> (price, size)
    /// Track tick sizes per asset
    tick_sizes: HashMap<String, String>, // asset_id -> tick_size
}

impl SniperHandler {
    pub fn new(
        market_id: String,
        market_question: String,
        outcome_map: HashMap<String, String>,
        orderbooks: SharedOrderbooks,
    ) -> Self {
        Self {
            market_id,
            market_question,
            outcome_map,
            orderbooks,
            message_count: 0,
            last_trade_prices: HashMap::new(),
            tick_sizes: HashMap::new(),
        }
    }

    /// Get outcome name for a token_id
    fn get_outcome_name(&self, token_id: &str) -> String {
        self.outcome_map
            .get(token_id)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Process orderbook snapshots and update shared orderbooks
    fn handle_snapshot(&mut self, snapshots: &[BookSnapshot]) {
        let mut obs = self.orderbooks.write().unwrap();
        for snapshot in snapshots {
            let orderbook = obs
                .entry(snapshot.asset_id.clone())
                .or_insert_with(|| Orderbook::new(snapshot.asset_id.clone()));
            orderbook.process_snapshot(&snapshot.bids, &snapshot.asks);
        }
    }

    /// Process price change events and update shared orderbooks
    /// Also checks for best_ask = "1" and logs it
    fn handle_price_change(&mut self, event: &PriceChangeEvent) {
        // First update the orderbooks
        {
            let mut obs = self.orderbooks.write().unwrap();
            for change in &event.price_changes {
                let orderbook = obs
                    .entry(change.asset_id.clone())
                    .or_insert_with(|| Orderbook::new(change.asset_id.clone()));
                orderbook.process_update(&change.side, &change.price, &change.size);
            }
        }

        // Check for best_ask = "1" (100% probability) with active market (best_bid > 0)
        for change in &event.price_changes {
            let best_ask_value: f64 = change.best_ask.parse().unwrap_or(0.0);
            let best_bid_value: f64 = change.best_bid.parse().unwrap_or(0.0);

            // Only log if best_ask = 1 AND best_bid > 0 (active market)
            // Skip if best_bid = 0 (no liquidity on bid side)
            if (best_ask_value - 1.0).abs() < 0.0001 && best_bid_value > 0.0001 {
                let outcome_name = self.get_outcome_name(&change.asset_id);

                info!("════════════════════════════════════════════════════════════════");
                info!("🎯 BEST ASK = 1 DETECTED!");
                info!("════════════════════════════════════════════════════════════════");
                info!("  Market ID:    {}", self.market_id);
                info!("  Market:       {}", self.market_question);
                info!("  Outcome:      {}", outcome_name);
                info!("  Token ID:     {}", change.asset_id);
                info!("  Best Ask:     {}", change.best_ask);
                info!("  Best Bid:     {}", change.best_bid);
                info!(
                    "  Last Change:  {} {} @ {} (size: {})",
                    change.side, outcome_name, change.price, change.size
                );
                info!("  Timestamp:    {}", event.timestamp);
                info!("════════════════════════════════════════════════════════════════");
            }
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
/// Note: The global shutdown flag is intentionally unused here. Each WebSocket client
/// uses a local shutdown flag because hypersockets sets the flag to false during
/// `client.shutdown()`, which would inadvertently trigger global shutdown if shared.
/// The global flag is checked in the main tracking loop instead.
async fn build_ws_client(
    config: &MarketTrackerConfig,
    orderbooks: SharedOrderbooks,
) -> Result<WebSocketClient<SniperRouter, SniperMessage>> {
    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SniperRouter::new(config.market_id.clone());
    let outcome_map = config.build_outcome_map();
    let handler = SniperHandler::new(
        config.market_id.clone(),
        config.market_question.clone(),
        outcome_map,
        orderbooks,
    );

    let subscription = MarketSubscription::new(config.token_ids.clone());
    let subscription_json = serde_json::to_string(&subscription)?;

    let market_id_for_route = config.market_id.clone();
    let client = WebSocketClientBuilder::new()
        .url("wss://ws-subscriptions-clob.polymarket.com/ws/market")
        .router(router, move |routing| {
            routing.handler(SniperRoute::Market(market_id_for_route.clone()), handler)
        })
        .heartbeat(Duration::from_secs(10), WsMessage::Text("PING".to_string()))
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

// =============================================================================
// Main Tracking Loop
// =============================================================================

/// Handle a WebSocket client event
fn handle_client_event(event: ClientEvent, market_id: &str) -> bool {
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

// =============================================================================
// Public Entry Point
// =============================================================================

/// Spawn a WebSocket tracker for a specific market
///
/// This function connects to the Polymarket WebSocket, subscribes to the market's
/// orderbook updates, and tracks until the market resolution time is reached or
/// the shutdown flag is set.
///
/// When a price_change message indicates best_ask = 1 for any token, it logs
/// the event with full market details.
pub async fn spawn_market_tracker(
    market_id: String,
    market_question: String,
    token_ids: Vec<String>,
    outcomes: Vec<String>,
    resolution_time: String,
    shutdown_flag: Arc<AtomicBool>,
    _oracle_prices: Option<SharedOraclePrices>,
) -> Result<()> {
    // Build configuration
    let config = MarketTrackerConfig::new(
        market_id.clone(),
        market_question.clone(),
        token_ids,
        outcomes,
        &resolution_time,
    )?;

    info!("[WS {}] Connecting to orderbook stream...", market_id);
    info!("[WS {}] Market: {}", market_id, market_question);
    info!(
        "[WS {}] Resolution time: {}",
        market_id, config.resolution_time
    );

    // Create shared orderbooks - handler writes, this loop reads
    let orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

    // Build and connect WebSocket client with shared orderbooks
    let client = build_ws_client(&config, Arc::clone(&orderbooks)).await?;
    info!("[WS {}] Connected and subscribed", market_id);

    // Main tracking loop
    loop {
        // Check shutdown flag first (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[WS {}] Shutdown signal received", market_id);
            break;
        }

        // Handle WebSocket events
        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event, &market_id) {
                    break;
                }
            }
            None => {
                // No event available, sleep briefly before checking again
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[WS {}] Closing connection", market_id);
    if let Err(e) = client.shutdown().await {
        warn!("[WS {}] Error during shutdown: {}", market_id, e);
    }
    info!("[WS {}] Tracker stopped", market_id);
    Ok(())
}
