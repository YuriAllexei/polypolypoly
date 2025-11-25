//! WebSocket client for market sniper orderbook tracking
//!
//! This module provides real-time orderbook tracking for markets approaching resolution.

use super::orderbook::{micros_to_f64, Orderbook};
use super::sniper_ws_types::*;
use crate::database::{DbOpportunity, MarketDatabase};
use anyhow::Result;
use chrono::{DateTime, Utc};
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use std::collections::{HashMap, HashSet};
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
    pub token_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub resolution_time: DateTime<Utc>,
}

impl MarketTrackerConfig {
    /// Create a new tracker configuration
    pub fn new(
        market_id: String,
        token_ids: Vec<String>,
        outcomes: Vec<String>,
        resolution_time_str: &str,
    ) -> Result<Self> {
        let resolution_time = DateTime::parse_from_rfc3339(resolution_time_str)?
            .with_timezone(&Utc);

        Ok(Self {
            market_id,
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

/// Handler for processing sniper messages (only updates orderbooks, no logging)
pub struct SniperHandler {
    #[allow(dead_code)]
    market_id: String,
    orderbooks: SharedOrderbooks,
    #[allow(dead_code)]
    message_count: u64,
}

impl SniperHandler {
    pub fn new(market_id: String, orderbooks: SharedOrderbooks) -> Self {
        Self {
            market_id,
            orderbooks,
            message_count: 0,
        }
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
    fn handle_price_change(&mut self, event: &PriceChangeEvent) {
        let mut obs = self.orderbooks.write().unwrap();
        for change in &event.price_changes {
            let orderbook = obs
                .entry(change.asset_id.clone())
                .or_insert_with(|| Orderbook::new(change.asset_id.clone()));
            orderbook.process_update(&change.side, &change.price, &change.size);
        }
    }
}

impl MessageHandler<SniperMessage> for SniperHandler {
    fn handle(&mut self, message: SniperMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            SniperMessage::BookSnapshots(snapshots) => self.handle_snapshot(&snapshots),
            SniperMessage::PriceChange(event) => self.handle_price_change(&event),
            SniperMessage::Pong => debug!("[WS {}] Pong received", self.market_id),
            SniperMessage::Unknown(_) => {}
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build a WebSocket client for the given market configuration
async fn build_ws_client(
    config: &MarketTrackerConfig,
    shutdown_flag: Arc<AtomicBool>,
    orderbooks: SharedOrderbooks,
) -> Result<WebSocketClient<SniperRouter, SniperMessage>> {
    let router = SniperRouter::new(config.market_id.clone());
    let handler = SniperHandler::new(config.market_id.clone(), orderbooks);

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
        .shutdown_flag(shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

// =============================================================================
// Resolution Time Checking
// =============================================================================

/// Check if the market resolution time has been reached
fn is_market_resolved(resolution_time: DateTime<Utc>) -> bool {
    Utc::now() >= resolution_time
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
/// the shutdown flag is set. It also detects opportunities when ask >= probability_threshold.
pub async fn spawn_market_tracker(
    market_id: String,
    token_ids: Vec<String>,
    outcomes: Vec<String>,
    resolution_time: String,
    shutdown_flag: Arc<AtomicBool>,
    db: Arc<MarketDatabase>,
    probability_threshold: f64,
    event_id: Option<String>,
) -> Result<()> {
    // Build configuration
    let config = MarketTrackerConfig::new(market_id.clone(), token_ids, outcomes, &resolution_time)?;

    info!("[WS {}] Connecting to orderbook stream...", market_id);
    info!("[WS {}] Resolution time: {}", market_id, config.resolution_time);
    info!("[WS {}] Probability threshold: {:.2}", market_id, probability_threshold);

    // Create shared orderbooks - handler writes, this loop reads
    let orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

    // Build and connect WebSocket client with shared orderbooks
    let client = build_ws_client(&config, Arc::clone(&shutdown_flag), Arc::clone(&orderbooks)).await?;
    info!("[WS {}] Connected and subscribed", market_id);

    // Track which opportunities have been recorded (by token_id)
    let mut recorded_opportunities: HashSet<String> = HashSet::new();
    let outcome_map = config.build_outcome_map();

    // Main tracking loop with opportunity detection
    loop {
        // Check shutdown flag first (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[WS {}] Shutdown signal received", market_id);
            break;
        }

        // Check if market has resolved
        if is_market_resolved(config.resolution_time) {
            info!("[WS {}] Market resolution time reached!", market_id);
            break;
        }

        // === OPPORTUNITY DETECTION ===
        // First, collect potential opportunities while holding read lock
        let mut opportunity_to_record: Option<(String, String, f64, f64)> = None;
        {
            let obs = orderbooks.read().unwrap();
            for (token_id, orderbook) in obs.iter() {
                // Skip if already recorded for this token
                if recorded_opportunities.contains(token_id) {
                    continue;
                }

                if let Some((price_micros, size_micros)) = orderbook.best_ask() {
                    let ask_price = micros_to_f64(price_micros);

                    // Check if ask >= threshold (opportunity!)
                    if ask_price >= probability_threshold {
                        let outcome = outcome_map
                            .get(token_id)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string());
                        let liquidity = micros_to_f64(size_micros);

                        // Store opportunity data for processing after releasing lock
                        opportunity_to_record = Some((token_id.clone(), outcome, ask_price, liquidity));
                        break;
                    }
                }
            }
        } // Read lock released here

        // Process opportunity outside of lock
        if let Some((token_id, outcome, ask_price, liquidity)) = opportunity_to_record {
            let opp = DbOpportunity::new(
                market_id.clone(),
                event_id.clone(),
                token_id.clone(),
                outcome.clone(),
                ask_price,
                liquidity,
                resolution_time.clone(),
                Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            );

            if let Err(e) = db.insert_opportunity(&opp).await {
                warn!("[WS {}] Failed to record opportunity: {}", market_id, e);
            } else {
                info!(
                    "[WS {}] OPPORTUNITY: {} ask={:.4} liq={:.2}",
                    market_id, outcome, ask_price, liquidity
                );
                recorded_opportunities.insert(token_id);
            }
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
                sleep(Duration::from_millis(100)).await;
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
