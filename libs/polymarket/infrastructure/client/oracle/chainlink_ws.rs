//! ChainLink Data Streams WebSocket Client
//!
//! Connects directly to ChainLink's Data Streams WebSocket API with HMAC authentication
//! to receive real-time crypto price updates.

use super::chainlink_types::{ChainLinkMessage, ChainLinkWsMessage, DecodedPrice, FeedIdMap};
use super::price_manager::SharedOraclePrices;
use super::types::OracleType;
use anyhow::Result;
use hmac::{Hmac, Mac};
use hypersockets::core::*;
use hypersockets::{HeaderProvider, Headers, MessageHandler, MessageRouter, WsMessage};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

// =============================================================================
// Constants
// =============================================================================

/// ChainLink WebSocket URL (mainnet)
const CHAINLINK_WS_URL: &str = "wss://ws.dataengine.chain.link";

/// WebSocket endpoint path
const WS_PATH: &str = "/api/v1/ws";

/// Maximum reconnection attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Delay between reconnection attempts in seconds
const RECONNECT_DELAY_SECS: u64 = 5;

/// Data flow staleness threshold
const DATA_FLOW_STALENESS_SECS: u64 = 30;

/// Staleness check interval
const STALENESS_CHECK_INTERVAL_SECS: u64 = 10;

// =============================================================================
// HMAC Authentication
// =============================================================================

type HmacSha256 = Hmac<Sha256>;

/// ChainLink HMAC authentication handler
#[derive(Clone)]
pub struct ChainLinkAuth {
    client_id: String,
    client_secret: String,
}

impl ChainLinkAuth {
    /// Create a new ChainLink auth handler from environment variables
    pub fn from_env() -> Result<Self> {
        let client_id = std::env::var("CHAINLINK_CLIENT_ID")
            .map_err(|_| anyhow::anyhow!("CHAINLINK_CLIENT_ID not set"))?;
        let client_secret = std::env::var("STREAMS_SECRET")
            .map_err(|_| anyhow::anyhow!("STREAMS_SECRET not set"))?;

        Ok(Self {
            client_id,
            client_secret,
        })
    }

    /// Create a new ChainLink auth handler with explicit credentials
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }

    /// Generate HMAC-SHA256 signature for a request
    ///
    /// Format: `METHOD FULL_PATH BODY_HASH API_KEY TIMESTAMP`
    fn generate_hmac(&self, method: &str, path: &str, body: &[u8], timestamp: u128) -> String {
        // SHA256 hash of the body (empty for GET/WebSocket)
        let mut hasher = Sha256::new();
        hasher.update(body);
        let body_hash = hex::encode(hasher.finalize());

        // Build the string to sign
        let message = format!(
            "{} {} {} {} {}",
            method, path, body_hash, self.client_id, timestamp
        );

        // HMAC-SHA256 with the secret
        let mut mac =
            HmacSha256::new_from_slice(self.client_secret.as_bytes()).expect("HMAC can take key");
        mac.update(message.as_bytes());
        let result = mac.finalize();

        hex::encode(result.into_bytes())
    }

    /// Generate authentication headers for a request
    pub fn generate_headers(&self, method: &str, path: &str, body: &[u8]) -> Headers {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis();

        let signature = self.generate_hmac(method, path, body, timestamp);

        let mut headers = Headers::new();
        headers.insert("Authorization".to_string(), self.client_id.clone());
        headers.insert(
            "X-Authorization-Timestamp".to_string(),
            timestamp.to_string(),
        );
        headers.insert("X-Authorization-Signature-SHA256".to_string(), signature);

        headers
    }
}

// =============================================================================
// Header Provider for WebSocket Connection
// =============================================================================

/// Header provider that generates fresh HMAC auth headers on each connection
pub struct ChainLinkHeaderProvider {
    auth: ChainLinkAuth,
    path: String,
}

impl ChainLinkHeaderProvider {
    pub fn new(auth: ChainLinkAuth, feed_ids: &str) -> Self {
        let path = format!("{}?feedIDs={}", WS_PATH, feed_ids);
        Self { auth, path }
    }
}

#[async_trait::async_trait]
impl HeaderProvider for ChainLinkHeaderProvider {
    async fn get_headers(&self) -> Headers {
        // Generate fresh headers with current timestamp
        self.auth.generate_headers("GET", &self.path, &[])
    }
}

// =============================================================================
// Report Decoding
// =============================================================================

/// Decode a ChainLink report to extract price data
pub fn decode_report(
    report: &ChainLinkWsMessage,
    feed_map: &FeedIdMap,
) -> Result<DecodedPrice> {
    use chainlink_data_streams_report::report::{decode_full_report, v3::ReportDataV3};

    let feed_id = &report.report.feed_id;
    let full_report = &report.report.full_report;

    // Get symbol from feed ID
    let symbol = feed_map
        .get_symbol(feed_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown feed ID: {}", feed_id))?
        .clone();

    // Decode hex report (skip 0x prefix if present)
    let hex_data = if full_report.starts_with("0x") {
        &full_report[2..]
    } else {
        full_report
    };
    let bytes = hex::decode(hex_data)?;

    // Decode the full report structure
    let (_, report_blob) = decode_full_report(&bytes)?;

    // Decode as V3 report
    let data = ReportDataV3::decode(&report_blob)?;

    // Convert prices from BigInt (18 decimals) to f64
    // benchmark_price, bid, ask are all in 18 decimal format
    let price = bigint_to_f64(&data.benchmark_price);
    let bid = bigint_to_f64(&data.bid);
    let ask = bigint_to_f64(&data.ask);

    Ok(DecodedPrice {
        symbol,
        feed_id: feed_id.clone(),
        price,
        bid,
        ask,
        timestamp: data.observations_timestamp as u64,
        valid_from: data.valid_from_timestamp as u64,
        expires_at: data.expires_at as u64,
    })
}

/// Convert a BigInt to f64 with 18 decimal places
fn bigint_to_f64(value: &num_bigint::BigInt) -> f64 {
    use num_traits::ToPrimitive;

    // For prices, we can safely convert to f64 first then divide
    // The precision loss is acceptable for display purposes
    value.to_f64().unwrap_or(0.0) / 1e18
}

// =============================================================================
// Router - Parses WebSocket Messages
// =============================================================================

/// Route key for ChainLink messages
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ChainLinkRoute {
    Reports,
}

/// Router for parsing ChainLink WebSocket messages
pub struct ChainLinkRouter;

impl ChainLinkRouter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChainLinkRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MessageRouter for ChainLinkRouter {
    type Message = ChainLinkMessage;
    type RouteKey = ChainLinkRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        // ChainLink sends binary WebSocket messages containing JSON data
        match message {
            WsMessage::Binary(data) => {
                // Parse binary data as JSON
                if let Ok(report) = serde_json::from_slice::<ChainLinkWsMessage>(&data) {
                    return Ok(ChainLinkMessage::Report(report));
                }
                // Try to decode as UTF-8 text for debugging
                if let Ok(text) = String::from_utf8(data.clone()) {
                    debug!("[ChainLink WS] Binary message (as text): {}", text);
                }
                Ok(ChainLinkMessage::Unknown("Failed to parse binary message".to_string()))
            }
            WsMessage::Text(text) => {
                // Text messages are typically control messages
                if let Ok(report) = serde_json::from_str::<ChainLinkWsMessage>(&text) {
                    return Ok(ChainLinkMessage::Report(report));
                }
                debug!("[ChainLink WS] Text message: {}", text);
                Ok(ChainLinkMessage::Unknown(text))
            }
            _ => Ok(ChainLinkMessage::Unknown("Other message type".to_string())),
        }
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        ChainLinkRoute::Reports
    }
}

// =============================================================================
// Handler - Processes and Stores Price Updates
// =============================================================================

/// Handler for processing ChainLink price messages
pub struct ChainLinkHandler {
    prices: SharedOraclePrices,
    feed_map: FeedIdMap,
    message_count: u64,
}

impl ChainLinkHandler {
    pub fn new(prices: SharedOraclePrices) -> Self {
        Self {
            prices,
            feed_map: FeedIdMap::new(),
            message_count: 0,
        }
    }
}

impl MessageHandler<ChainLinkMessage> for ChainLinkHandler {
    fn handle(&mut self, message: ChainLinkMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            ChainLinkMessage::Report(report) => {
                match decode_report(&report, &self.feed_map) {
                    Ok(decoded) => {
                        // Update the price in shared state
                        {
                            let mut prices = self.prices.write();
                            prices.update_price(
                                OracleType::ChainLink,
                                &decoded.symbol,
                                decoded.price,
                                decoded.timestamp,
                            );
                        }

                        debug!(
                            "[ChainLink WS] {} = ${:.2} (bid: ${:.2}, ask: ${:.2}, ts: {})",
                            decoded.symbol, decoded.price, decoded.bid, decoded.ask, decoded.timestamp
                        );
                    }
                    Err(e) => {
                        warn!("[ChainLink WS] Failed to decode report: {}", e);
                    }
                }
            }
            ChainLinkMessage::Pong => {
                debug!("[ChainLink WS] Pong received");
            }
            ChainLinkMessage::Unknown(msg) => {
                if self.message_count <= 5 {
                    debug!("[ChainLink WS] Unknown message ({}): {}", self.message_count, msg);
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build a WebSocket client for ChainLink Data Streams
async fn build_chainlink_ws_client(
    prices: SharedOraclePrices,
) -> Result<WebSocketClient<ChainLinkRouter, ChainLinkMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    // Load auth from environment
    let auth = ChainLinkAuth::from_env()?;

    // Create feed ID map and get the feed IDs parameter
    let feed_map = FeedIdMap::new();
    let feed_ids = feed_map.get_feed_ids_param();

    // Build full WebSocket URL
    let ws_url = format!("{}{}?feedIDs={}", CHAINLINK_WS_URL, WS_PATH, feed_ids);

    // Create header provider
    let header_provider = ChainLinkHeaderProvider::new(auth, &feed_ids);

    let router = ChainLinkRouter::new();
    let handler = ChainLinkHandler::new(prices);

    let client = WebSocketClientBuilder::new()
        .url(&ws_url)
        .router(router, move |routing| {
            routing.handler(ChainLinkRoute::Reports, handler)
        })
        .headers(header_provider)
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
            info!("[ChainLink WS] Connected to Data Streams");
            true
        }
        ClientEvent::Disconnected => {
            warn!("[ChainLink WS] Disconnected");
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[ChainLink WS] Reconnecting (attempt {})", attempt);
            true
        }
        ClientEvent::Error(err) => {
            error!("[ChainLink WS] Error: {}", err);
            true
        }
    }
}

/// Spawn a tracker for ChainLink Data Streams
pub async fn spawn_chainlink_tracker(
    prices: SharedOraclePrices,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
    let mut reconnect_attempts: u32 = 0;

    // Outer reconnection loop
    'reconnect: loop {
        // Check shutdown before attempting connection
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[ChainLink WS] Shutdown signal received before connect");
            break 'reconnect;
        }

        if reconnect_attempts > 0 {
            info!(
                "[ChainLink WS] Reconnection attempt {} of {}",
                reconnect_attempts, MAX_RECONNECT_ATTEMPTS
            );
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        } else {
            info!("[ChainLink WS] Connecting to Data Streams...");
        }

        // Build WebSocket client (generates fresh auth headers)
        let client = match build_chainlink_ws_client(Arc::clone(&prices)).await {
            Ok(c) => c,
            Err(e) => {
                warn!("[ChainLink WS] Failed to connect: {}", e);
                reconnect_attempts += 1;
                if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                    error!(
                        "[ChainLink WS] Exceeded max reconnection attempts ({})",
                        MAX_RECONNECT_ATTEMPTS
                    );
                    break 'reconnect;
                }
                continue 'reconnect;
            }
        };
        info!("[ChainLink WS] Connected successfully");

        // Reset oracle health state
        prices.write().reset_oracle_health(OracleType::ChainLink);

        // Track connection start time
        let connection_start = std::time::Instant::now();
        let mut should_reconnect = false;
        let mut last_staleness_check = std::time::Instant::now();

        // Main tracking loop
        loop {
            // Check shutdown flag
            if !shutdown_flag.load(Ordering::Acquire) {
                info!("[ChainLink WS] Shutdown signal received");
                break;
            }

            // Handle WebSocket events
            match client.try_recv_event() {
                Some(event) => {
                    if !handle_client_event(event) {
                        should_reconnect = true;
                        break;
                    }
                }
                None => {
                    sleep(Duration::from_millis(10)).await;
                }
            }

            // Periodically check for data flow staleness
            if last_staleness_check.elapsed().as_secs() >= STALENESS_CHECK_INTERVAL_SECS {
                last_staleness_check = std::time::Instant::now();

                let staleness = prices.read().oracle_age(OracleType::ChainLink);
                if staleness.as_secs() >= DATA_FLOW_STALENESS_SECS {
                    warn!(
                        "[ChainLink WS] Data flow STALE for {:.1}s - triggering reconnection",
                        staleness.as_secs_f64()
                    );
                    should_reconnect = true;
                    break;
                }
            }
        }

        // Shutdown current client
        info!("[ChainLink WS] Closing connection");
        if let Err(e) = client.shutdown().await {
            warn!("[ChainLink WS] Error during shutdown: {}", e);
        }

        // Decide whether to reconnect
        if should_reconnect {
            let connection_duration = connection_start.elapsed().as_secs();
            if connection_duration > RECONNECT_DELAY_SECS * 2 {
                reconnect_attempts = 0;
                info!(
                    "[ChainLink WS] Connection was stable for {}s, resetting counter",
                    connection_duration
                );
            }

            reconnect_attempts += 1;

            if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                error!(
                    "[ChainLink WS] Exceeded max reconnection attempts ({})",
                    MAX_RECONNECT_ATTEMPTS
                );
                break 'reconnect;
            }

            continue 'reconnect;
        } else {
            break 'reconnect;
        }
    }

    info!("[ChainLink WS] Tracker stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_signature_generation() {
        let auth = ChainLinkAuth::new(
            "test-client-id".to_string(),
            "test-secret".to_string(),
        );

        // Test that signature is generated
        let headers = auth.generate_headers("GET", "/api/v1/ws?feedIDs=0x123", &[]);

        assert!(headers.contains_key("Authorization"));
        assert!(headers.contains_key("X-Authorization-Timestamp"));
        assert!(headers.contains_key("X-Authorization-Signature-SHA256"));

        assert_eq!(headers.get("Authorization").unwrap(), "test-client-id");
    }

    #[test]
    fn test_header_provider_path() {
        let auth = ChainLinkAuth::new("id".to_string(), "secret".to_string());
        let provider = ChainLinkHeaderProvider::new(auth, "0x123,0x456");

        assert_eq!(provider.path, "/api/v1/ws?feedIDs=0x123,0x456");
    }
}
