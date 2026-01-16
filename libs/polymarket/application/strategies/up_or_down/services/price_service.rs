//! Price service for the Up or Down strategy.
//!
//! Handles fetching prices from ChainLink Candlestick API (primary)
//! or Polymarket's crypto price API (fallback).
//!
//! Uses dedicated OS threads for HTTP requests to avoid blocking the tokio runtime.

use crate::domain::DbMarket;
use crate::infrastructure::{CandlestickApiClient, SharedOraclePrices};
use crate::application::strategies::up_or_down::types::{CryptoAsset, OracleSource, Timeframe};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, info, warn};

// =============================================================================
// API Types
// =============================================================================

/// Response from Polymarket's crypto price API
#[derive(Debug, Deserialize)]
struct CryptoPriceResponse {
    #[serde(rename = "openPrice")]
    open_price: Option<f64>,
    /// True when the market just started and price isn't recorded yet
    #[serde(default)]
    incomplete: bool,
}

// =============================================================================
// Price to Beat (ChainLink Candlestick API - Primary)
// =============================================================================

/// Get the opening price ("price to beat") from ChainLink Candlestick API.
///
/// For a market ending at time T with duration D, the price_to_beat is the
/// candle close price at time T-D (the market start time).
///
/// # Arguments
/// * `timeframe` - The market timeframe (15M, 1H, 4H, Daily)
/// * `crypto_asset` - The cryptocurrency being tracked (BTC, ETH, SOL, XRP)
/// * `market` - The market containing the end_date
///
/// # Returns
/// The candle close price as f64, or an error if the request fails
fn get_price_to_beat_from_chainlink(
    timeframe: Timeframe,
    crypto_asset: CryptoAsset,
    market: &DbMarket,
) -> anyhow::Result<f64> {
    // Get symbol for ChainLink API (e.g., "BTCUSD")
    let symbol = crypto_asset
        .oracle_symbol()
        .ok_or_else(|| anyhow::anyhow!("Cannot get price for unknown crypto asset"))?;
    let chainlink_symbol = format!("{}USD", symbol);

    // Get resolution string for ChainLink API
    let resolution = match timeframe {
        Timeframe::FiveMin => "5m",
        Timeframe::FifteenMin => "15m",
        Timeframe::OneHour => "1h",
        Timeframe::FourHour => "4h",
        Timeframe::Daily => "1d",
        Timeframe::Unknown => return Err(anyhow::anyhow!("Unknown timeframe")),
    };

    // Get duration from timeframe
    let duration = timeframe
        .duration()
        .ok_or_else(|| anyhow::anyhow!("Cannot calculate duration for timeframe: {}", timeframe))?;

    // Parse end_date from market
    let end_date = DateTime::parse_from_rfc3339(&market.end_date)
        .map_err(|e| anyhow::anyhow!("Failed to parse market end_date: {}", e))?
        .with_timezone(&Utc);

    // Calculate event start time (this is when the candle closes that we need)
    let event_start_time = end_date - duration;
    let start_timestamp = event_start_time.timestamp();

    info!(
        "[ChainLink] Fetching price_to_beat for {} {} market starting at {}",
        crypto_asset, timeframe, event_start_time
    );

    // Create client and fetch
    let client = CandlestickApiClient::from_env()?;
    let price = client.get_open_price_at(&chainlink_symbol, resolution, start_timestamp)?;

    info!(
        "[ChainLink] Got price_to_beat for {}: ${:.2}",
        chainlink_symbol, price
    );

    Ok(price)
}

// =============================================================================
// Price to Beat (Polymarket API - Fallback)
// =============================================================================

/// Get the opening price from Polymarket's crypto price API (fallback).
fn get_price_to_beat_from_polymarket(
    timeframe: Timeframe,
    crypto_asset: CryptoAsset,
    market: &DbMarket,
) -> anyhow::Result<f64> {
    // Get symbol from crypto asset
    let symbol = crypto_asset
        .oracle_symbol()
        .ok_or_else(|| anyhow::anyhow!("Cannot get price for unknown crypto asset"))?;

    // Get API variant from timeframe
    let variant = timeframe
        .api_variant()
        .ok_or_else(|| anyhow::anyhow!("Cannot get price for unsupported timeframe: {}", timeframe))?;

    // Get duration from timeframe
    let duration = timeframe
        .duration()
        .ok_or_else(|| anyhow::anyhow!("Cannot calculate duration for timeframe: {}", timeframe))?;

    // Parse end_date from market
    let end_date = DateTime::parse_from_rfc3339(&market.end_date)
        .map_err(|e| anyhow::anyhow!("Failed to parse market end_date: {}", e))?
        .with_timezone(&Utc);

    // Calculate event start time by subtracting timeframe duration
    let event_start_time = end_date - duration;

    // Format dates as ISO 8601 for URL
    let end_date_str = end_date.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let event_start_time_str = event_start_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Build the API URL
    let url = format!(
        "https://polymarket.com/api/crypto/crypto-price?symbol={}&eventStartTime={}&variant={}&endDate={}",
        symbol, event_start_time_str, variant, end_date_str
    );

    debug!(
        symbol = symbol,
        variant = variant,
        event_start_time = %event_start_time_str,
        end_date = %end_date_str,
        "Fetching price to beat from Polymarket API (fallback)"
    );

    info!("⏳ [Polymarket] Fetching price to beat from {}", url);

    let response = ureq::get(&url)
        .set("User-Agent", "polymarket-strategy")
        .set("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| anyhow::anyhow!("HTTP request failed: {}", e))?;

    let status = response.status();
    let response_body = response
        .into_string()
        .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

    info!("📥 [Polymarket] Got response: status={}, body_len={}", status, response_body.len());

    if status == 200 {
        let data: CryptoPriceResponse = serde_json::from_str(&response_body)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {} - body: {}", e, response_body))?;

        match data.open_price {
            Some(price) => {
                debug!(open_price = price, "Retrieved price to beat from Polymarket");
                Ok(price)
            }
            None => {
                if data.incomplete {
                    Err(anyhow::anyhow!("Market just started - openPrice not yet recorded (incomplete=true)"))
                } else {
                    Err(anyhow::anyhow!("API returned null openPrice - body: {}", response_body))
                }
            }
        }
    } else {
        Err(anyhow::anyhow!("API returned error status {}: {}", status, response_body))
    }
}

// =============================================================================
// Price to Beat (Public API)
// =============================================================================

/// Get the opening price ("price to beat") for a market.
///
/// Tries ChainLink Candlestick API first (more reliable), then falls back
/// to Polymarket's crypto price API if ChainLink fails.
///
/// # Arguments
/// * `timeframe` - The market timeframe (15M, 1H, 4H, Daily)
/// * `crypto_asset` - The cryptocurrency being tracked (BTC, ETH, SOL, XRP)
/// * `market` - The market containing the end_date
///
/// # Returns
/// The opening price as f64, or an error if both sources fail
pub async fn get_price_to_beat(
    timeframe: Timeframe,
    crypto_asset: CryptoAsset,
    market: &DbMarket,
) -> anyhow::Result<f64> {
    // Clone data for the blocking thread
    let tf = timeframe;
    let asset = crypto_asset;
    let market_end_date = market.end_date.clone();
    let market_clone = DbMarket {
        end_date: market_end_date,
        ..market.clone()
    };

    // Use dedicated OS thread to avoid blocking tokio runtime
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        // Try ChainLink first
        let result = match get_price_to_beat_from_chainlink(tf, asset, &market_clone) {
            Ok(price) => {
                info!("✅ [Price] Got price_to_beat from ChainLink: ${:.2}", price);
                Ok(price)
            }
            Err(chainlink_err) => {
                warn!(
                    "⚠️ [Price] ChainLink failed: {}. Trying Polymarket fallback...",
                    chainlink_err
                );

                // Fall back to Polymarket
                match get_price_to_beat_from_polymarket(tf, asset, &market_clone) {
                    Ok(price) => {
                        info!("✅ [Price] Got price_to_beat from Polymarket fallback: ${:.2}", price);
                        Ok(price)
                    }
                    Err(polymarket_err) => {
                        Err(anyhow::anyhow!(
                            "Both sources failed - ChainLink: {}, Polymarket: {}",
                            chainlink_err,
                            polymarket_err
                        ))
                    }
                }
            }
        };

        let _ = tx.send(result);
    });

    // Wait for the dedicated thread to complete
    rx.await
        .map_err(|_| anyhow::anyhow!("Price thread channel closed unexpectedly"))?
}

// =============================================================================
// Oracle Price (Real-time)
// =============================================================================

/// Get the current oracle price for a crypto asset.
///
/// Fetches the real-time price from either Binance or ChainLink oracle
/// depending on the market's oracle source.
///
/// # Arguments
/// * `oracle_source` - Which oracle to use (Binance or ChainLink)
/// * `crypto_asset` - The cryptocurrency (BTC, ETH, SOL, XRP)
/// * `oracle_prices` - Shared oracle price manager
///
/// # Returns
/// The current price as f64, or None if unavailable
pub fn get_oracle_price(
    oracle_source: OracleSource,
    crypto_asset: CryptoAsset,
    oracle_prices: &SharedOraclePrices,
) -> Option<f64> {
    // Get oracle type from source
    let oracle_type = oracle_source.to_oracle_type()?;

    // Get symbol from crypto asset
    let symbol = crypto_asset.oracle_symbol()?;

    // Get price from oracle manager
    let manager = oracle_prices.read();
    manager
        .get_price(oracle_type, symbol)
        .map(|entry| entry.value)
}

// =============================================================================
// Oracle Health Tracking
// =============================================================================

/// Check if the SPECIFIC oracle for this market is fresh enough for trading.
///
/// Each market uses only one oracle (ChainLink OR Binance) for resolution.
/// This checks if that specific oracle has received data recently.
///
/// # Arguments
/// * `oracle_prices` - Shared oracle price manager
/// * `oracle_source` - Which oracle to check (from market context)
/// * `max_age_secs` - Maximum allowed age of oracle data in seconds
///
/// # Returns
/// True if the oracle has received data within max_age_secs, false otherwise
pub fn is_market_oracle_fresh(
    oracle_prices: &Option<SharedOraclePrices>,
    oracle_source: OracleSource,
    max_age_secs: u64,
) -> bool {
    let Some(prices) = oracle_prices else {
        return false;
    };

    // Skip check for unknown oracle sources
    let Some(oracle_type) = oracle_source.to_oracle_type() else {
        return true;
    };

    let prices = prices.read();
    let max_age = Duration::from_secs(max_age_secs);
    prices.is_oracle_healthy(oracle_type, max_age)
}

/// Get the age of the last update for the specific oracle this market uses.
///
/// # Arguments
/// * `oracle_prices` - Shared oracle price manager
/// * `oracle_source` - Which oracle to check (from market context)
///
/// # Returns
/// Duration since last update, or None if oracle source is unknown
pub fn get_market_oracle_age(
    oracle_prices: &Option<SharedOraclePrices>,
    oracle_source: OracleSource,
) -> Option<Duration> {
    let prices = oracle_prices.as_ref()?;

    let oracle_type = oracle_source.to_oracle_type()?;

    Some(prices.read().oracle_age(oracle_type))
}
