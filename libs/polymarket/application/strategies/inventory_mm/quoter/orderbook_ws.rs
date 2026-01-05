//! Per-quoter orderbook WebSocket handling.
//!
//! Provides a simplified interface for quoters to subscribe to orderbook updates
//! for their specific token pair (up_token, down_token).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use tracing::{debug, info, warn, trace};

use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, TextPongDetector, WsMessage};

use crate::infrastructure::SharedOrderbooks;
use crate::infrastructure::client::clob::orderbook::Orderbook;
use crate::infrastructure::client::clob::sniper_ws_types::{
    BookSnapshot, MarketSubscription, PriceChangeEvent, SniperMessage,
};

/// Configuration for quoter orderbook WebSocket.
#[derive(Debug, Clone)]
pub struct QuoterWsConfig {
    /// Market ID for logging
    pub market_id: String,
    /// UP token ID
    pub up_token_id: String,
    /// DOWN token ID
    pub down_token_id: String,
}

impl QuoterWsConfig {
    pub fn new(market_id: String, up_token_id: String, down_token_id: String) -> Self {
        Self {
            market_id,
            up_token_id,
            down_token_id,
        }
    }

    /// Get token IDs as a vector for subscription.
    pub fn token_ids(&self) -> Vec<String> {
        vec![self.up_token_id.clone(), self.down_token_id.clone()]
    }
}

// =============================================================================
// Router - Parses WebSocket messages
// =============================================================================

/// Route key for quoter messages.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum QuoterRoute {
    Market(String),
}

/// Router for parsing WebSocket messages (simplified from SniperRouter).
pub struct QuoterRouter {
    market_id: String,
}

impl QuoterRouter {
    pub fn new(market_id: String) -> Self {
        Self { market_id }
    }
}

#[async_trait::async_trait]
impl MessageRouter for QuoterRouter {
    type Message = SniperMessage;
    type RouteKey = QuoterRoute;

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

        // Try to parse as single book snapshot
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

        // Unknown message (tick_size_change, last_trade_price, etc. ignored for quoter)
        debug!("[QuoterWS {}] Unknown message: {}", self.market_id, text);
        Ok(SniperMessage::Unknown(text.to_string()))
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        QuoterRoute::Market(self.market_id.clone())
    }
}

// =============================================================================
// Handler - Updates orderbooks
// =============================================================================

/// Handler for processing orderbook updates (simplified from SniperHandler).
pub struct QuoterHandler {
    market_id: String,
    orderbooks: SharedOrderbooks,
    first_snapshot_received: Arc<AtomicBool>,
    message_count: u64,
}

impl QuoterHandler {
    pub fn new(
        market_id: String,
        orderbooks: SharedOrderbooks,
        first_snapshot_received: Arc<AtomicBool>,
    ) -> Self {
        Self {
            market_id,
            orderbooks,
            first_snapshot_received,
            message_count: 0,
        }
    }

    /// Process orderbook snapshots.
    fn handle_snapshot(&mut self, snapshots: &[BookSnapshot]) {
        if snapshots.is_empty() {
            return;
        }

        trace!(
            "[QuoterWS {}] Received {} snapshots: {:?}",
            self.market_id,
            snapshots.len(),
            snapshots.iter().map(|s| format!("{}...(bids={},asks={})", &s.asset_id[..8.min(s.asset_id.len())], s.bids.len(), s.asks.len())).collect::<Vec<_>>()
        );

        let mut obs = self.orderbooks.write();
        for snapshot in snapshots {
            let orderbook = obs
                .entry(snapshot.asset_id.clone())
                .or_insert_with(|| Orderbook::new(snapshot.asset_id.clone()));
            orderbook.process_snapshot(&snapshot.bids, &snapshot.asks);

            // Log orderbook state after processing
            debug!(
                "[QuoterWS {}] Orderbook for {}...: bid={:?}, ask={:?}",
                self.market_id,
                &snapshot.asset_id[..8.min(snapshot.asset_id.len())],
                orderbook.best_bid(),
                orderbook.best_ask()
            );
        }

        self.first_snapshot_received.store(true, Ordering::Release);
    }

    /// Process price change events.
    /// CRITICAL: Uses best_bid/best_ask from exchange instead of computing ourselves
    fn handle_price_change(&mut self, event: &PriceChangeEvent) {
        let mut obs = self.orderbooks.write();
        for change in &event.price_changes {
            let orderbook = obs
                .entry(change.asset_id.clone())
                .or_insert_with(|| Orderbook::new(change.asset_id.clone()));
            // Use authoritative best_bid/best_ask from exchange - prevents stale data
            orderbook.process_update_with_best(
                &change.side,
                &change.price,
                &change.size,
                &change.best_bid,
                &change.best_ask,
            );
        }
    }
}

impl MessageHandler<SniperMessage> for QuoterHandler {
    fn handle(&mut self, message: SniperMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            SniperMessage::BookSnapshots(snapshots) => self.handle_snapshot(&snapshots),
            SniperMessage::PriceChange(event) => self.handle_price_change(&event),
            SniperMessage::Pong => debug!("[QuoterWS {}] Pong received", self.market_id),
            _ => {} // Ignore tick_size_change, last_trade_price, unknown
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Result of building a quoter WebSocket client.
pub struct QuoterWsClient {
    /// The underlying WebSocket client
    pub client: WebSocketClient<QuoterRouter, SniperMessage>,
    /// Flag that becomes true once first snapshot is received
    pub first_snapshot_received: Arc<AtomicBool>,
}

impl QuoterWsClient {
    /// Check if the WebSocket is connected.
    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// Check if the first orderbook snapshot has been received.
    pub fn has_snapshot(&self) -> bool {
        self.first_snapshot_received.load(Ordering::Acquire)
    }

    /// Shutdown the WebSocket client.
    pub async fn shutdown(self) -> Result<()> {
        self.client.shutdown().await?;
        Ok(())
    }
}

/// Build a WebSocket client for a quoter's token pair.
///
/// This is a simplified version of `build_ws_client` from `sniper_ws.rs`,
/// tailored for quoter use cases (no precision tracking, tick_size forwarding, etc.).
pub async fn build_quoter_ws_client(
    config: &QuoterWsConfig,
    orderbooks: SharedOrderbooks,
) -> Result<QuoterWsClient> {
    let first_snapshot_received = Arc::new(AtomicBool::new(false));

    let router = QuoterRouter::new(config.market_id.clone());
    let handler = QuoterHandler::new(
        config.market_id.clone(),
        orderbooks,
        Arc::clone(&first_snapshot_received),
    );

    let token_ids = config.token_ids();
    info!(
        "[QuoterWS {}] Subscribing to tokens: UP={}..., DOWN={}...",
        config.market_id,
        &token_ids[0][..16.min(token_ids[0].len())],
        &token_ids[1][..16.min(token_ids[1].len())]
    );

    let subscription = MarketSubscription::new(token_ids);
    let subscription_json = serde_json::to_string(&subscription)?;

    let pong_detector = Arc::new(TextPongDetector::new("PONG".to_string()));

    // Local shutdown flag for this WebSocket client only
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let market_id_for_route = config.market_id.clone();
    let market_id_for_log = config.market_id.clone();

    let client = WebSocketClientBuilder::new()
        .url("wss://ws-subscriptions-clob.polymarket.com/ws/market")
        .router(router, move |routing| {
            routing.handler(QuoterRoute::Market(market_id_for_route.clone()), handler)
        })
        .heartbeat(Duration::from_secs(5), WsMessage::Text("PING".to_string()))
        .pong_detector(pong_detector)
        .pong_timeout(Duration::from_secs(15))
        .subscription(WsMessage::Text(subscription_json))
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    // Yield to allow the spawned client task to start running
    tokio::task::yield_now().await;

    // Wait briefly for the connection to be established (up to 5 seconds)
    let start = std::time::Instant::now();
    while !client.is_connected() && start.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    if !client.is_connected() {
        warn!("[QuoterWS {}] Client not connected after 5s wait", market_id_for_log);
    }

    Ok(QuoterWsClient {
        client,
        first_snapshot_received,
    })
}

/// Wait for the first orderbook snapshot to be received.
///
/// Returns true if snapshot received, false if shutdown requested or timeout.
pub async fn wait_for_snapshot(
    ws_client: &QuoterWsClient,
    shutdown_flag: &Arc<AtomicBool>,
    market_id: &str,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();

    while !ws_client.has_snapshot() {
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[QuoterWS {}] Shutdown during snapshot wait", market_id);
            return false;
        }

        if start.elapsed() >= timeout {
            warn!("[QuoterWS {}] Timeout waiting for snapshot", market_id);
            return false;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    info!("[QuoterWS {}] First snapshot received", market_id);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quoter_ws_config() {
        let config = QuoterWsConfig::new(
            "market-123".to_string(),
            "up-token".to_string(),
            "down-token".to_string(),
        );

        assert_eq!(config.token_ids(), vec!["up-token", "down-token"]);
    }
}
