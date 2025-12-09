//! WebSocket client for Oracle price tracking
//!
//! Connects to Polymarket's live data WebSocket to receive real-time
//! crypto price updates from ChainLink and Binance oracles.

use super::price_manager::{OraclePriceManager, SharedOraclePrices};
use super::types::{OracleMessage, OraclePriceUpdate, OracleSubscription, OracleType};
use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// WebSocket URL for Polymarket live data
const ORACLE_WS_URL: &str = "wss://ws-live-data.polymarket.com";

/// Heartbeat interval in seconds
const HEARTBEAT_INTERVAL_SECS: u64 = 8;

// =============================================================================
// Symbol Parsing
// =============================================================================

/// Parse ChainLink symbol format ("eth/usd" -> "ETH")
pub fn parse_chainlink_symbol(symbol: &str) -> String {
    // ChainLink format: "eth/usd", "btc/usd", etc.
    // Extract the base currency (before the slash)
    symbol
        .split('/')
        .next()
        .unwrap_or(symbol)
        .to_uppercase()
}

/// Parse Binance symbol format ("solusdt" -> "SOL")
pub fn parse_binance_symbol(symbol: &str) -> String {
    // Binance format: "solusdt", "btcusdt", etc.
    // Strip the "usdt" suffix
    let lower = symbol.to_lowercase();
    if lower.ends_with("usdt") {
        lower[..lower.len() - 4].to_uppercase()
    } else {
        symbol.to_uppercase()
    }
}

// =============================================================================
// Router - Parses WebSocket messages
// =============================================================================

/// Route key for oracle messages
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum OracleRoute {
    Oracle(OracleType),
}

/// Router for parsing WebSocket messages
pub struct OracleRouter {
    oracle_type: OracleType,
}

impl OracleRouter {
    pub fn new(oracle_type: OracleType) -> Self {
        Self { oracle_type }
    }
}

#[async_trait::async_trait]
impl MessageRouter for OracleRouter {
    type Message = OracleMessage;
    type RouteKey = OracleRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(OracleMessage::Unknown("Binary data".to_string())),
        };

        // Check for PONG response
        if text == "PONG" {
            return Ok(OracleMessage::Pong);
        }

        // Try to parse as price update
        if let Ok(update) = serde_json::from_str::<OraclePriceUpdate>(text) {
            if update.msg_type == "update" {
                return Ok(OracleMessage::PriceUpdate(update));
            }
        }

        // Unknown message
        debug!("[Oracle {}] Unknown message: {}", self.oracle_type, text);
        Ok(OracleMessage::Unknown(text.to_string()))
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        OracleRoute::Oracle(self.oracle_type)
    }
}

// =============================================================================
// Handler - Processes and stores price updates
// =============================================================================

/// Handler for processing oracle price messages
pub struct OracleHandler {
    oracle_type: OracleType,
    prices: SharedOraclePrices,
    message_count: u64,
}

impl OracleHandler {
    pub fn new(oracle_type: OracleType, prices: SharedOraclePrices) -> Self {
        Self {
            oracle_type,
            prices,
            message_count: 0,
        }
    }

    /// Process a price update and store it
    fn handle_price_update(&mut self, update: &OraclePriceUpdate) {
        // Parse the symbol based on oracle type
        let symbol = match self.oracle_type {
            OracleType::ChainLink => parse_chainlink_symbol(&update.payload.symbol),
            OracleType::Binance => parse_binance_symbol(&update.payload.symbol),
        };

        // Update the price in shared state
        {
            let mut prices = self.prices.write().unwrap();
            prices.update_price(
                self.oracle_type,
                &symbol,
                update.payload.value,
                update.payload.timestamp,
            );
        }

        debug!(
            "[Oracle {}] {} = {} (ts: {})",
            self.oracle_type, symbol, update.payload.value, update.payload.timestamp
        );
    }
}

impl MessageHandler<OracleMessage> for OracleHandler {
    fn handle(&mut self, message: OracleMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            OracleMessage::PriceUpdate(update) => self.handle_price_update(&update),
            OracleMessage::Pong => debug!("[Oracle {}] Pong received", self.oracle_type),
            OracleMessage::Unknown(_) => {}
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build a WebSocket client for the given oracle type.
///
/// Each WebSocket client uses a local shutdown flag because hypersockets
/// sets the flag to false during `client.shutdown()`, which would
/// inadvertently trigger global shutdown if shared.
async fn build_oracle_ws_client(
    oracle_type: OracleType,
    prices: SharedOraclePrices,
) -> Result<WebSocketClient<OracleRouter, OracleMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = OracleRouter::new(oracle_type);
    let handler = OracleHandler::new(oracle_type, prices);

    let subscription = OracleSubscription::new(oracle_type);
    let subscription_json = serde_json::to_string(&subscription)?;

    let client = WebSocketClientBuilder::new()
        .url(ORACLE_WS_URL)
        .router(router, move |routing| {
            routing.handler(OracleRoute::Oracle(oracle_type), handler)
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

// =============================================================================
// Main Tracking Loop
// =============================================================================

/// Handle a WebSocket client event
fn handle_client_event(event: ClientEvent, oracle_type: OracleType) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[Oracle {}] WebSocket connected", oracle_type);
            true
        }
        ClientEvent::Disconnected => {
            warn!("[Oracle {}] WebSocket disconnected", oracle_type);
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[Oracle {}] Reconnecting (attempt {})", oracle_type, attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[Oracle {}] Error: {}", oracle_type, err);
            true
        }
    }
}

/// Spawn a tracker for a single oracle type (internal use)
async fn spawn_single_oracle_tracker(
    oracle_type: OracleType,
    prices: SharedOraclePrices,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    info!("[Oracle {}] Connecting to price feed...", oracle_type);

    let client = build_oracle_ws_client(oracle_type, prices).await?;
    info!("[Oracle {}] Connected and subscribed", oracle_type);

    // Main tracking loop
    loop {
        // Check shutdown flag first (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[Oracle {}] Shutdown signal received", oracle_type);
            break;
        }

        // Handle WebSocket events
        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event, oracle_type) {
                    break;
                }
            }
            None => {
                // No event available, sleep briefly before checking again
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[Oracle {}] Closing connection", oracle_type);
    if let Err(e) = client.shutdown().await {
        warn!("[Oracle {}] Error during shutdown: {}", oracle_type, e);
    }
    info!("[Oracle {}] Tracker stopped", oracle_type);
    Ok(())
}

// =============================================================================
// Public Entry Point
// =============================================================================

/// Spawn both ChainLink and Binance oracle WebSocket connections.
///
/// Returns the shared price manager for reading prices.
/// Both connections run in background tasks and update the shared state.
pub async fn spawn_oracle_trackers(
    shutdown_flag: Arc<AtomicBool>,
) -> Result<SharedOraclePrices> {
    // Create shared price manager
    let prices: SharedOraclePrices = Arc::new(RwLock::new(OraclePriceManager::new()));

    info!("════════════════════════════════════════════════════════════════");
    info!("🔮 STARTING ORACLE PRICE TRACKERS");
    info!("════════════════════════════════════════════════════════════════");
    info!("  ChainLink: crypto_prices_chainlink");
    info!("  Binance:   crypto_prices");
    info!("  Heartbeat: {} seconds", HEARTBEAT_INTERVAL_SECS);
    info!("════════════════════════════════════════════════════════════════");

    // Spawn ChainLink tracker
    let chainlink_prices = Arc::clone(&prices);
    let chainlink_shutdown = Arc::clone(&shutdown_flag);
    tokio::spawn(async move {
        if let Err(e) = spawn_single_oracle_tracker(
            OracleType::ChainLink,
            chainlink_prices,
            chainlink_shutdown,
        )
        .await
        {
            warn!("[Oracle ChainLink] Tracker failed: {}", e);
        }
    });

    // Spawn Binance tracker
    let binance_prices = Arc::clone(&prices);
    let binance_shutdown = Arc::clone(&shutdown_flag);
    tokio::spawn(async move {
        if let Err(e) =
            spawn_single_oracle_tracker(OracleType::Binance, binance_prices, binance_shutdown).await
        {
            warn!("[Oracle Binance] Tracker failed: {}", e);
        }
    });

    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chainlink_symbol() {
        assert_eq!(parse_chainlink_symbol("eth/usd"), "ETH");
        assert_eq!(parse_chainlink_symbol("btc/usd"), "BTC");
        assert_eq!(parse_chainlink_symbol("sol/usd"), "SOL");
        assert_eq!(parse_chainlink_symbol("ETH/USD"), "ETH");
    }

    #[test]
    fn test_parse_binance_symbol() {
        assert_eq!(parse_binance_symbol("solusdt"), "SOL");
        assert_eq!(parse_binance_symbol("btcusdt"), "BTC");
        assert_eq!(parse_binance_symbol("ethusdt"), "ETH");
        assert_eq!(parse_binance_symbol("SOLUSDT"), "SOL");
        // Edge case: no usdt suffix
        assert_eq!(parse_binance_symbol("btc"), "BTC");
    }

    #[test]
    fn test_oracle_route_equality() {
        let route1 = OracleRoute::Oracle(OracleType::ChainLink);
        let route2 = OracleRoute::Oracle(OracleType::ChainLink);
        let route3 = OracleRoute::Oracle(OracleType::Binance);

        assert_eq!(route1, route2);
        assert_ne!(route1, route3);
    }
}
