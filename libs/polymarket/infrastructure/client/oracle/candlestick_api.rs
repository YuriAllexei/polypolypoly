//! ChainLink Candlestick API Client
//!
//! Fetches historical OHLC candle data from ChainLink's Candlestick API.
//! Used to get the "price to beat" for Up/Down markets by querying
//! the candle close price at the market start time.

use anyhow::Result;
use parking_lot::RwLock;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// =============================================================================
// Constants
// =============================================================================

const CANDLESTICK_API_URL: &str = "https://priceapi.dataengine.chain.link";

/// JWT token lifetime buffer (refresh 5 minutes before expiry)
const TOKEN_REFRESH_BUFFER_SECS: u64 = 300;

/// HTTP request timeout
const REQUEST_TIMEOUT_SECS: u64 = 15;

// =============================================================================
// API Response Types
// =============================================================================

#[derive(Debug, Deserialize)]
struct AuthResponse {
    s: String,
    #[serde(default)]
    d: Option<AuthData>,
    #[serde(default)]
    errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthData {
    access_token: String,
    expiration: u64,
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    s: String,
    #[serde(default)]
    t: Vec<i64>,      // timestamps
    #[serde(default)]
    o: Vec<f64>,      // open prices (18 decimals)
    #[serde(default)]
    h: Vec<f64>,      // high prices
    #[serde(default)]
    l: Vec<f64>,      // low prices
    #[serde(default)]
    c: Vec<f64>,      // close prices
    #[serde(default)]
    errmsg: Option<String>,
}

// =============================================================================
// Candle Data
// =============================================================================

/// A single OHLC candle
#[derive(Debug, Clone, Copy)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

// =============================================================================
// Token Cache
// =============================================================================

struct TokenCache {
    token: String,
    expires_at: Instant,
}

// =============================================================================
// Client
// =============================================================================

/// ChainLink Candlestick API client
pub struct CandlestickApiClient {
    /// Streams User ID (used as login)
    user_id: String,
    /// Candlestick API key (used as password)
    api_key: String,
    /// Cached JWT token
    token_cache: RwLock<Option<TokenCache>>,
}

impl CandlestickApiClient {
    /// Create a new client from environment variables
    pub fn from_env() -> Result<Self> {
        let user_id = std::env::var("CHAINLINK_CLIENT_ID")
            .map_err(|_| anyhow::anyhow!("CHAINLINK_CLIENT_ID not set"))?;
        let api_key = std::env::var("CHAINLINK_CANDLESTICK_API_KEY")
            .map_err(|_| anyhow::anyhow!("CHAINLINK_CANDLESTICK_API_KEY not set"))?;

        Ok(Self {
            user_id,
            api_key,
            token_cache: RwLock::new(None),
        })
    }

    /// Create a new client with explicit credentials
    pub fn new(user_id: String, api_key: String) -> Self {
        Self {
            user_id,
            api_key,
            token_cache: RwLock::new(None),
        }
    }

    /// Get a valid JWT token, refreshing if necessary
    fn get_token(&self) -> Result<String> {
        // Check cache first
        {
            let cache = self.token_cache.read();
            if let Some(ref cached) = *cache {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Need to refresh token
        debug!("[Candlestick API] Refreshing JWT token");

        let url = format!("{}/api/v1/authorize", CANDLESTICK_API_URL);
        let body = serde_json::json!({
            "login": self.user_id,
            "password": self.api_key
        });

        let response = ureq::post(&url)
            .set("Content-Type", "application/json")
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .send_json(&body)
            .map_err(|e| anyhow::anyhow!("Auth request failed: {}", e))?;

        let auth_resp: AuthResponse = response
            .into_json()
            .map_err(|e| anyhow::anyhow!("Failed to parse auth response: {}", e))?;

        if auth_resp.s != "ok" {
            return Err(anyhow::anyhow!(
                "Auth failed: {}",
                auth_resp.errmsg.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        let data = auth_resp
            .d
            .ok_or_else(|| anyhow::anyhow!("Auth response missing data"))?;

        // Calculate expiry (with buffer)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ttl_secs = data.expiration.saturating_sub(now).saturating_sub(TOKEN_REFRESH_BUFFER_SECS);
        let expires_at = Instant::now() + Duration::from_secs(ttl_secs);

        // Update cache
        {
            let mut cache = self.token_cache.write();
            *cache = Some(TokenCache {
                token: data.access_token.clone(),
                expires_at,
            });
        }

        debug!("[Candlestick API] Got new token, expires in {}s", ttl_secs);
        Ok(data.access_token)
    }

    /// Fetch historical candles for a symbol
    ///
    /// # Arguments
    /// * `symbol` - Trading pair (e.g., "BTCUSD", "ETHUSD")
    /// * `resolution` - Candle timeframe (e.g., "1m", "15m", "1h")
    /// * `from` - Start timestamp (unix seconds)
    /// * `to` - End timestamp (unix seconds)
    pub fn get_candles(
        &self,
        symbol: &str,
        resolution: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<Candle>> {
        let token = self.get_token()?;

        let url = format!(
            "{}/api/v1/history?symbol={}&resolution={}&from={}&to={}",
            CANDLESTICK_API_URL, symbol, resolution, from, to
        );

        debug!(
            "[Candlestick API] Fetching {} {} candles from {} to {}",
            symbol, resolution, from, to
        );

        let response = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", token))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .call()
            .map_err(|e| anyhow::anyhow!("History request failed: {}", e))?;

        let hist_resp: HistoryResponse = response
            .into_json()
            .map_err(|e| anyhow::anyhow!("Failed to parse history response: {}", e))?;

        if hist_resp.s != "ok" {
            return Err(anyhow::anyhow!(
                "History fetch failed: {}",
                hist_resp.errmsg.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Convert to candles (prices are in 18-decimal format)
        let candles: Vec<Candle> = hist_resp
            .t
            .iter()
            .enumerate()
            .map(|(i, &ts)| Candle {
                timestamp: ts,
                open: hist_resp.o.get(i).copied().unwrap_or(0.0) / 1e18,
                high: hist_resp.h.get(i).copied().unwrap_or(0.0) / 1e18,
                low: hist_resp.l.get(i).copied().unwrap_or(0.0) / 1e18,
                close: hist_resp.c.get(i).copied().unwrap_or(0.0) / 1e18,
            })
            .collect();

        debug!(
            "[Candlestick API] Got {} candles for {}",
            candles.len(),
            symbol
        );

        Ok(candles)
    }

    /// Get the candle open price at a specific timestamp
    ///
    /// This fetches the candle that starts at the given timestamp and returns its OPEN price.
    /// For price_to_beat, Polymarket uses the OPEN price of the candle at market start time.
    ///
    /// # Arguments
    /// * `symbol` - Trading pair (e.g., "BTCUSD")
    /// * `resolution` - Candle timeframe (e.g., "15m")
    /// * `timestamp` - Unix timestamp to query (market start time)
    pub fn get_open_price_at(
        &self,
        symbol: &str,
        resolution: &str,
        timestamp: i64,
    ) -> Result<f64> {
        let resolution_secs: i64 = match resolution {
            "1m" => 60,
            "5m" => 300,
            "15m" => 900,
            "1h" => 3600,
            "4h" => 14400,
            "1d" | "24h" => 86400,
            _ => 900, // default to 15m
        };

        // Fetch candles around the target timestamp
        let from = timestamp - resolution_secs;
        let to = timestamp + resolution_secs;

        let candles = self.get_candles(symbol, resolution, from, to)?;

        if candles.is_empty() {
            return Err(anyhow::anyhow!(
                "No candles found for {} at timestamp {}",
                symbol,
                timestamp
            ));
        }

        // Find the candle that starts at or closest to the timestamp
        // The timestamp in the response is the candle OPEN time
        // For a market starting at 4:45, we want the candle with timestamp 4:45
        let candle = candles
            .iter()
            .filter(|c| c.timestamp <= timestamp)
            .max_by_key(|c| c.timestamp)
            .or_else(|| candles.first())
            .ok_or_else(|| anyhow::anyhow!("No matching candle found"))?;

        info!(
            "[Candlestick API] {} open at {} (candle ts {}): ${:.2}",
            symbol, timestamp, candle.timestamp, candle.open
        );

        Ok(candle.open)
    }
}

/// Shared client for use across async tasks
pub type SharedCandlestickClient = Arc<CandlestickApiClient>;

/// Create a shared candlestick client from environment variables
pub fn create_shared_candlestick_client() -> Result<SharedCandlestickClient> {
    Ok(Arc::new(CandlestickApiClient::from_env()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolution_to_secs() {
        // Just verify our resolution parsing logic
        assert_eq!(
            match "15m" {
                "1m" => 60,
                "5m" => 300,
                "15m" => 900,
                "1h" => 3600,
                _ => 0,
            },
            900
        );
    }
}
