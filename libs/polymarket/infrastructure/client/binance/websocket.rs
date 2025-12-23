//! WebSocket client for direct Binance price tracking
//!
//! Connects directly to Binance's WebSocket stream for lowest latency
//! crypto price feeds. Designed for HFT trading applications.

use super::price_manager::{BinancePriceManager, SharedBinancePrices};
use super::types::{BinanceAsset, BinanceMessage, BinanceRoute, BinanceStreamWrapper};
use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Base WebSocket URL for Binance combined streams
const BINANCE_WS_BASE: &str = "wss://stream.binance.com:9443/stream";

/// Maximum reconnection attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Delay between reconnection attempts in seconds
const RECONNECT_DELAY_SECS: u64 = 5;

/// Data flow staleness threshold - if no trade received for this duration,
/// trigger a reconnection. Binance streams are very active, so this can be short.
const DATA_FLOW_STALENESS_SECS: u64 = 10;

/// How often to check data flow staleness
const STALENESS_CHECK_INTERVAL_SECS: u64 = 5;

// =============================================================================
// URL Builder
// =============================================================================

/// Build the combined stream URL for all supported assets
fn build_stream_url() -> String {
    let streams: Vec<&str> = BinanceAsset::all()
        .iter()
        .map(|asset| asset.stream_name())
        .collect();

    format!("{}?streams={}", BINANCE_WS_BASE, streams.join("/"))
}

// =============================================================================
// Router - Parses WebSocket messages
// =============================================================================

/// Router for parsing Binance WebSocket messages
pub struct BinanceRouter;

impl BinanceRouter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BinanceRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MessageRouter for BinanceRouter {
    type Message = BinanceMessage;
    type RouteKey = BinanceRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(BinanceMessage::Unknown("Binary data".to_string())),
        };

        // Try to parse as combined stream wrapper
        match serde_json::from_str::<BinanceStreamWrapper>(text) {
            Ok(wrapper) => {
                // Verify it's a trade event
                if wrapper.data.event_type == "trade" {
                    return Ok(BinanceMessage::Trade(wrapper));
                }
                debug!("[Binance WS] Non-trade event: {}", wrapper.data.event_type);
                Ok(BinanceMessage::Unknown(text.to_string()))
            }
            Err(e) => {
                debug!("[Binance WS] Parse error: {} - {}", e, text);
                Ok(BinanceMessage::Unknown(text.to_string()))
            }
        }
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        BinanceRoute::Trades
    }
}

// =============================================================================
// Handler - Processes and stores price updates
// =============================================================================

/// Handler for processing Binance trade messages
pub struct BinanceHandler {
    prices: SharedBinancePrices,
    message_count: u64,
}

impl BinanceHandler {
    pub fn new(prices: SharedBinancePrices) -> Self {
        Self {
            prices,
            message_count: 0,
        }
    }

    /// Process a trade and store the price
    fn handle_trade(&mut self, wrapper: &BinanceStreamWrapper) {
        let data = &wrapper.data;

        // Parse symbol to asset
        let asset = match BinanceAsset::from_symbol(&data.symbol) {
            Some(a) => a,
            None => {
                debug!("[Binance WS] Unknown symbol: {}", data.symbol);
                return;
            }
        };

        // Parse price
        let price = match data.price_f64() {
            Some(p) => p,
            None => {
                warn!("[Binance WS] Invalid price: {}", data.price);
                return;
            }
        };

        // Calculate latency for logging
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let latency_ms = now_ms - (data.event_time as i64);

        // Update the price in shared state
        {
            let mut prices = self.prices.write();
            prices.update_price(
                asset.symbol(),
                price,
                data.event_time,
                data.trade_id,
                data.is_buyer_maker,
            );
        }

        // Log periodically (every 1000 trades to avoid spam)
        if self.message_count % 1000 == 0 {
            debug!(
                "[Binance WS] {} = ${:.2} (latency: {}ms, trade_id: {}, count: {})",
                asset, price, latency_ms, data.trade_id, self.message_count
            );
        }
    }
}

impl MessageHandler<BinanceMessage> for BinanceHandler {
    fn handle(&mut self, message: BinanceMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            BinanceMessage::Trade(wrapper) => self.handle_trade(&wrapper),
            BinanceMessage::Unknown(_) => {}
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build the Binance WebSocket client.
///
/// Uses a local shutdown flag because hypersockets sets the flag to false
/// during `client.shutdown()`.
async fn build_binance_ws_client(
    prices: SharedBinancePrices,
) -> Result<WebSocketClient<BinanceRouter, BinanceMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = BinanceRouter::new();
    let handler = BinanceHandler::new(prices);

    let url = build_stream_url();
    debug!("[Binance WS] Connecting to: {}", url);

    // Binance uses standard WebSocket ping/pong frames, not custom text messages.
    // The underlying tokio-tungstenite handles these automatically.
    // No custom heartbeat/pong_detector needed.

    let client = WebSocketClientBuilder::new()
        .url(&url)
        .router(router, move |routing| {
            routing.handler(BinanceRoute::Trades, handler)
        })
        // No subscription message needed - streams are specified in URL
        // No custom heartbeat needed - Binance uses standard WS ping/pong
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

// =============================================================================
// Main Tracking Loop
// =============================================================================

/// Handle a WebSocket client event
fn handle_client_event(event: ClientEvent) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[Binance WS] Connected to price feed");
            true
        }
        ClientEvent::Disconnected => {
            warn!("[Binance WS] Disconnected from price feed");
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[Binance WS] Reconnecting (attempt {})", attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[Binance WS] Error: {}", err);
            true
        }
    }
}

/// Spawn the Binance price tracker with automatic reconnection.
///
/// Returns the shared price manager for reading prices.
/// The tracker runs in a background task and updates shared state.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use std::sync::atomic::AtomicBool;
///
/// let shutdown = Arc::new(AtomicBool::new(true));
/// let prices = spawn_binance_tracker(shutdown).await?;
///
/// // Read prices
/// let btc = prices.read().get_price("BTC");
/// ```
pub async fn spawn_binance_tracker(shutdown_flag: Arc<AtomicBool>) -> Result<SharedBinancePrices> {
    // Create shared price manager
    let prices: SharedBinancePrices = Arc::new(RwLock::new(BinancePriceManager::new()));

    info!("================================================================");
    info!("  STARTING BINANCE DIRECT PRICE TRACKER");
    info!("================================================================");
    info!("  URL: {}", build_stream_url());
    info!("  Assets: BTC, ETH, SOL, XRP");
    info!("  Staleness threshold: {}s", DATA_FLOW_STALENESS_SECS);
    info!("================================================================");

    // Spawn the tracker task
    let prices_clone = Arc::clone(&prices);
    let shutdown_clone = Arc::clone(&shutdown_flag);

    tokio::spawn(async move {
        if let Err(e) = run_binance_tracker(prices_clone, shutdown_clone).await {
            warn!("[Binance WS] Tracker failed: {}", e);
        }
    });

    // Brief sleep to allow connection to establish
    sleep(Duration::from_millis(100)).await;

    Ok(prices)
}

/// Internal tracker loop with reconnection logic
async fn run_binance_tracker(
    prices: SharedBinancePrices,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    let mut reconnect_attempts: u32 = 0;

    'reconnect: loop {
        // Check shutdown before attempting connection
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[Binance WS] Shutdown signal received before connect");
            break 'reconnect;
        }

        if reconnect_attempts > 0 {
            info!(
                "[Binance WS] Reconnection attempt {} of {}",
                reconnect_attempts, MAX_RECONNECT_ATTEMPTS
            );
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        }

        // Build WebSocket client
        let client = match build_binance_ws_client(Arc::clone(&prices)).await {
            Ok(c) => c,
            Err(e) => {
                warn!("[Binance WS] Failed to connect: {}", e);
                reconnect_attempts += 1;
                if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                    warn!(
                        "[Binance WS] Exceeded max reconnection attempts ({}), giving up",
                        MAX_RECONNECT_ATTEMPTS
                    );
                    break 'reconnect;
                }
                continue 'reconnect;
            }
        };

        // Reset health state to avoid immediate staleness detection
        prices.write().reset_health();

        // Track connection start time (for stable connection detection)
        let connection_start = Instant::now();

        // Track if we should reconnect after this loop
        let mut should_reconnect = false;

        // Track last time we checked for data flow staleness
        let mut last_staleness_check = Instant::now();

        // Main tracking loop
        loop {
            // Check shutdown flag first (highest priority)
            if !shutdown_flag.load(Ordering::Acquire) {
                info!("[Binance WS] Shutdown signal received");
                break;
            }

            // Handle WebSocket events
            match client.try_recv_event() {
                Some(event) => {
                    if !handle_client_event(event) {
                        // Disconnected - mark for reconnection
                        should_reconnect = true;
                        break;
                    }
                }
                None => {
                    // No event available, sleep briefly before checking again
                    sleep(Duration::from_millis(10)).await;
                }
            }

            // Periodically check for data flow staleness (zombie connection detection)
            if last_staleness_check.elapsed().as_secs() >= STALENESS_CHECK_INTERVAL_SECS {
                last_staleness_check = Instant::now();

                let staleness = prices.read().age();
                if staleness.as_secs() >= DATA_FLOW_STALENESS_SECS {
                    warn!(
                        "[Binance WS] Data flow STALE for {:.1}s (threshold: {}s) - triggering reconnection",
                        staleness.as_secs_f64(),
                        DATA_FLOW_STALENESS_SECS
                    );
                    should_reconnect = true;
                    break;
                }

                // Log health stats periodically
                let manager = prices.read();
                if manager.trade_count() > 0 && manager.trade_count() % 10000 == 0 {
                    info!(
                        "[Binance WS] Health: trades={}, avg_latency={:.1}ms, min={}, max={}",
                        manager.trade_count(),
                        manager.avg_latency_ms(),
                        manager.min_latency_ms(),
                        manager.max_latency_ms()
                    );
                }
            }
        }

        // Shutdown current client
        info!("[Binance WS] Closing connection");
        if let Err(e) = client.shutdown().await {
            warn!("[Binance WS] Error during shutdown: {}", e);
        }

        // Decide whether to reconnect or exit
        if should_reconnect {
            // Check if connection was stable (ran for more than 2x reconnect delay)
            let connection_duration = connection_start.elapsed().as_secs();
            if connection_duration > RECONNECT_DELAY_SECS * 2 {
                reconnect_attempts = 0;
                info!(
                    "[Binance WS] Connection was stable for {}s, resetting reconnect counter",
                    connection_duration
                );
            }

            reconnect_attempts += 1;

            if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                warn!(
                    "[Binance WS] Exceeded max reconnection attempts ({}), giving up",
                    MAX_RECONNECT_ATTEMPTS
                );
                break 'reconnect;
            }

            info!(
                "[Binance WS] Will attempt reconnection (attempt {} of {})",
                reconnect_attempts, MAX_RECONNECT_ATTEMPTS
            );
            continue 'reconnect;
        } else {
            // Normal shutdown
            break 'reconnect;
        }
    }

    info!("[Binance WS] Tracker stopped");
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stream_url() {
        let url = build_stream_url();
        assert!(url.starts_with("wss://stream.binance.com:9443/stream?streams="));
        assert!(url.contains("btcusdt@trade"));
        assert!(url.contains("ethusdt@trade"));
        assert!(url.contains("solusdt@trade"));
        assert!(url.contains("xrpusdt@trade"));
    }

    #[test]
    fn test_router_creation() {
        let router = BinanceRouter::new();
        let default_router = BinanceRouter::default();
        // Just verify they create without panic
        let _ = router;
        let _ = default_router;
    }

    #[test]
    fn test_binance_route_equality() {
        let route1 = BinanceRoute::Trades;
        let route2 = BinanceRoute::Trades;
        assert_eq!(route1, route2);
    }
}
