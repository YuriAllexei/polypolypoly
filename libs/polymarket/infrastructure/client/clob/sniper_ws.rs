//! WebSocket client for market sniper orderbook tracking
//!
//! This module provides reusable WebSocket components for tracking Polymarket orderbooks.
//! The types are designed to be used by strategies that need real-time orderbook data.

use super::orderbook::Orderbook;
use super::sniper_ws_types::{
    BookSnapshot, LastTradePriceEvent, MarketSubscription, PriceChangeEvent, SniperMessage,
    TickSizeChangeEvent,
};
use super::types::PriceLevel;
use anyhow::Result;
use chrono::{DateTime, Utc};
use crossbeam_channel::Sender;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, TextPongDetector, WsMessage};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use tracing::{debug, info, warn};

/// Shared orderbooks accessible by both handler and main loop
pub type SharedOrderbooks = Arc<RwLock<HashMap<String, Orderbook>>>;

/// Shared precisions per token (number of decimal places)
pub type SharedPrecisions = Arc<RwLock<HashMap<String, u8>>>;

// =============================================================================
// Precision Helper Functions
// =============================================================================

/// Get the number of decimal places from a price/tick_size string
/// "0.01" -> 2, "0.001" -> 3, "0.55" -> 2, "0.555" -> 3, "1" -> 0
pub fn decimal_places(s: &str) -> u8 {
    match s.find('.') {
        Some(pos) => (s.len() - pos - 1) as u8,
        None => 0,
    }
}

/// Get the maximum precision found in a slice of price levels
/// Returns 2 as default if no levels provided
pub fn max_precision_in_levels(levels: &[PriceLevel]) -> u8 {
    levels
        .iter()
        .map(|l| decimal_places(&l.price))
        .max()
        .unwrap_or(2)
}

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
    /// Shared precisions per token (number of decimal places)
    precisions: SharedPrecisions,
    /// Optional channel to forward tick_size_change events to main loop
    tick_size_tx: Option<Sender<TickSizeChangeEvent>>,
    message_count: u64,
    /// Track last trade prices per asset
    last_trade_prices: HashMap<String, (String, String)>, // asset_id -> (price, size)

    first_snapshot_received: Arc<AtomicBool>,
}

impl SniperHandler {
    pub fn new(
        market_id: String,
        orderbooks: SharedOrderbooks,
        precisions: SharedPrecisions,
        tick_size_tx: Option<Sender<TickSizeChangeEvent>>,
        first_snapshot_received: Arc<AtomicBool>,
    ) -> Self {
        Self {
            market_id,
            orderbooks,
            precisions,
            tick_size_tx,
            message_count: 0,
            last_trade_prices: HashMap::new(),
            first_snapshot_received,
        }
    }

    /// Process orderbook snapshots and update shared orderbooks
    /// Also detects precision from price levels if current precision is 2 (default)
    fn handle_snapshot(&mut self, snapshots: &[BookSnapshot]) {
        if snapshots.is_empty() {
            return;
        }

        // First pass: Calculate all precision updates without holding any locks
        // This minimizes lock contention by doing computation outside the critical section
        let mut precision_updates: Vec<(String, u8)> = Vec::new();
        {
            let precs = self.precisions.read();
            for snapshot in snapshots {
                let current = *precs.get(&snapshot.asset_id).unwrap_or(&2);
                if current == 2 {
                    let bid_max = max_precision_in_levels(&snapshot.bids);
                    let ask_max = max_precision_in_levels(&snapshot.asks);
                    let detected = bid_max.max(ask_max);
                    if detected > 2 {
                        debug!(
                            "[WS {}] Detected higher precision {} for {} from book snapshot",
                            self.market_id, detected, snapshot.asset_id
                        );
                        precision_updates.push((snapshot.asset_id.clone(), detected));
                    } else {
                        precision_updates.push((snapshot.asset_id.clone(), 2));
                    }
                }
            }
        } // Read lock released here

        // Second pass: Update orderbooks (separate lock)
        {
            let mut obs = self.orderbooks.write();
            for snapshot in snapshots {
                let orderbook = obs
                    .entry(snapshot.asset_id.clone())
                    .or_insert_with(|| Orderbook::new(snapshot.asset_id.clone()));
                orderbook.process_snapshot(&snapshot.bids, &snapshot.asks);
            }
        } // Write lock released here

        // Third pass: Apply precision updates (separate lock, brief hold)
        if !precision_updates.is_empty() {
            let mut precs = self.precisions.write();
            for (asset_id, precision) in precision_updates {
                precs.insert(asset_id, precision);
            }
        }

        self.first_snapshot_received.swap(true, Ordering::Release);
    }

    /// Process price change events and update shared orderbooks
    /// Also detects precision upgrades from price levels
    fn handle_price_change(&mut self, event: &PriceChangeEvent) {
        // First, detect any precision changes from incoming prices
        for change in &event.price_changes {
            let price_precision = decimal_places(&change.price);
            let current_precision = *self.precisions.read().get(&change.asset_id).unwrap_or(&2);

            // If incoming price has higher precision than what we know, upgrade
            if price_precision > current_precision {
                info!(
                    "[WS {}] Detected precision upgrade from price_change: {} -> {} for {}",
                    self.market_id, current_precision, price_precision, change.asset_id
                );

                // Calculate old and new tick sizes for the event
                let old_tick_size = format!("{}", 10_f64.powi(-(current_precision as i32)));
                let new_tick_size = format!("{}", 10_f64.powi(-(price_precision as i32)));

                // Update precision
                self.precisions
                    .write()
                    .insert(change.asset_id.clone(), price_precision);

                // Emit synthetic tick_size_change event to trigger order upgrade
                if let Some(ref tx) = self.tick_size_tx {
                    let synthetic_event = TickSizeChangeEvent {
                        event_type: "tick_size_change".to_string(),
                        asset_id: change.asset_id.clone(),
                        market: event.market.clone(),
                        old_tick_size,
                        new_tick_size,
                        side: Some(change.side.clone()),
                        timestamp: event.timestamp.clone(),
                    };
                    let _ = tx.send(synthetic_event);
                }
            }
        }

        // Then update the orderbooks using authoritative best_bid/best_ask from exchange
        let mut obs = self.orderbooks.write();
        for change in &event.price_changes {
            let orderbook = obs
                .entry(change.asset_id.clone())
                .or_insert_with(|| Orderbook::new(change.asset_id.clone()));
            orderbook.process_update_with_best(
                &change.side,
                &change.price,
                &change.size,
                &change.best_bid,
                &change.best_ask,
            );
        }
    }

    /// Process tick size change events and update precision
    fn handle_tick_size_change(&mut self, event: &TickSizeChangeEvent) {
        let precision = decimal_places(&event.new_tick_size);
        debug!(
            "[WS {}] Tick size change for {}: {} -> {} (precision: {})",
            self.market_id, event.asset_id, event.old_tick_size, event.new_tick_size, precision
        );

        // Update precision in shared state
        self.precisions
            .write()
            .insert(event.asset_id.clone(), precision);

        // Forward event to main loop if channel configured
        if let Some(ref tx) = self.tick_size_tx {
            let _ = tx.send(event.clone()); // Non-blocking, ignore if receiver dropped
        }
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
///
/// # Arguments
/// * `tick_size_tx` - Optional channel sender for forwarding tick_size_change events to main loop
pub async fn build_ws_client(
    config: &MarketTrackerConfig,
    orderbooks: SharedOrderbooks,
    precisions: SharedPrecisions,
    tick_size_tx: Option<Sender<TickSizeChangeEvent>>,
    first_snapshot_received: Arc<AtomicBool>,
) -> Result<WebSocketClient<SniperRouter, SniperMessage>> {
    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SniperRouter::new(config.market_id.clone());
    let handler = SniperHandler::new(
        config.market_id.clone(),
        orderbooks,
        precisions,
        tick_size_tx,
        first_snapshot_received,
    );

    let subscription = MarketSubscription::new(config.token_ids.clone());
    let subscription_json = serde_json::to_string(&subscription)?;

    // Create PONG detector for "PONG" text messages
    // Timeout is 15s (3x heartbeat interval of 5s)
    let pong_detector = Arc::new(TextPongDetector::new("PONG".to_string()));

    let market_id_for_route = config.market_id.clone();
    let market_id_for_log = config.market_id.clone();
    let client = WebSocketClientBuilder::new()
        .url("wss://ws-subscriptions-clob.polymarket.com/ws/market")
        .router(router, move |routing| {
            routing.handler(SniperRoute::Market(market_id_for_route.clone()), handler)
        })
        .heartbeat(Duration::from_secs(5), WsMessage::Text("PING".to_string()))
        .pong_detector(pong_detector)
        .pong_timeout(Duration::from_secs(15))
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    // Yield to allow the spawned client task to start running
    // This ensures the WebSocket connection begins before we return
    tokio::task::yield_now().await;

    // Wait briefly for the connection to be established (up to 5 seconds)
    // This prevents returning before the WebSocket actually connects
    let start = std::time::Instant::now();
    while !client.is_connected() && start.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    if !client.is_connected() {
        tracing::warn!("[WS {}] Client not connected after 5s wait, proceeding anyway", market_id_for_log);
    }

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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decimal_places_tick_sizes() {
        // Common tick sizes
        assert_eq!(decimal_places("0.01"), 2);
        assert_eq!(decimal_places("0.001"), 3);
        assert_eq!(decimal_places("0.0001"), 4);
    }

    #[test]
    fn test_decimal_places_prices() {
        // Normal 2-decimal prices
        assert_eq!(decimal_places("0.55"), 2);
        assert_eq!(decimal_places("0.99"), 2);
        assert_eq!(decimal_places("0.01"), 2);

        // 3-decimal prices
        assert_eq!(decimal_places("0.555"), 3);
        assert_eq!(decimal_places("0.991"), 3);
        assert_eq!(decimal_places("0.015"), 3);
    }

    #[test]
    fn test_decimal_places_edge_cases() {
        // No decimal point
        assert_eq!(decimal_places("1"), 0);
        assert_eq!(decimal_places("100"), 0);

        // One decimal place
        assert_eq!(decimal_places("0.5"), 1);

        // Many decimal places
        assert_eq!(decimal_places("0.123456"), 6);
    }

    #[test]
    fn test_max_precision_in_levels_detects_higher() {
        let levels = vec![
            PriceLevel {
                price: "0.55".to_string(),
                size: "100".to_string(),
            },
            PriceLevel {
                price: "0.555".to_string(),
                size: "50".to_string(),
            },
            PriceLevel {
                price: "0.56".to_string(),
                size: "75".to_string(),
            },
        ];
        assert_eq!(max_precision_in_levels(&levels), 3);
    }

    #[test]
    fn test_max_precision_in_levels_stays_at_2() {
        let levels = vec![
            PriceLevel {
                price: "0.55".to_string(),
                size: "100".to_string(),
            },
            PriceLevel {
                price: "0.56".to_string(),
                size: "50".to_string(),
            },
            PriceLevel {
                price: "0.99".to_string(),
                size: "75".to_string(),
            },
        ];
        assert_eq!(max_precision_in_levels(&levels), 2);
    }

    #[test]
    fn test_max_precision_in_levels_empty() {
        let levels: Vec<PriceLevel> = vec![];
        assert_eq!(max_precision_in_levels(&levels), 2);
    }

    #[test]
    fn test_max_precision_in_levels_4_decimals() {
        let levels = vec![
            PriceLevel {
                price: "0.9901".to_string(),
                size: "100".to_string(),
            },
            PriceLevel {
                price: "0.99".to_string(),
                size: "50".to_string(),
            },
        ];
        assert_eq!(max_precision_in_levels(&levels), 4);
    }
}
