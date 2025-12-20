//! Price service for the Up or Down strategy.
//!
//! Handles fetching prices from Polymarket's crypto price API
//! and reading oracle prices from the shared price manager.
//!
//! Uses dedicated OS threads for HTTP requests to avoid blocking the tokio runtime.

use crate::domain::DbMarket;
use crate::infrastructure::SharedOraclePrices;
use crate::application::strategies::up_or_down::types::{CryptoAsset, OracleSource, Timeframe};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, info};

// =============================================================================
// API Types
// =============================================================================

/// Response from Polymarket's crypto price API
#[derive(Debug, Deserialize)]
struct CryptoPriceResponse {
    #[serde(rename = "openPrice")]
    open_price: f64,
}

// =============================================================================
// Price to Beat (Polymarket API)
// =============================================================================

/// Get the opening price ("price to beat") from Polymarket's crypto price API.
///
/// This fetches the reference price used to determine if the crypto asset
/// went "up" or "down" during the market's timeframe.
///
/// # Arguments
/// * `timeframe` - The market timeframe (15M, 1H, 4H, Daily)
/// * `crypto_asset` - The cryptocurrency being tracked (BTC, ETH, SOL, XRP)
/// * `market` - The market containing the end_date
///
/// # Returns
/// The opening price as f64, or an error if the request fails
pub async fn get_price_to_beat(
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
        "Fetching price to beat from Polymarket API"
    );

    // Use dedicated OS thread with ureq to avoid blocking tokio runtime
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        info!("⏳ [Price thread] Fetching price to beat from {}", url);

        let result = (|| -> std::result::Result<f64, String> {
            let response = ureq::get(&url)
                .set("User-Agent", "polymarket-strategy")
                .set("Accept", "application/json")
                .timeout(Duration::from_secs(15))
                .call()
                .map_err(|e| format!("HTTP request failed: {}", e))?;

            let status = response.status();
            let response_body = response
                .into_string()
                .map_err(|e| format!("Failed to read response: {}", e))?;

            info!("📥 [Price thread] Got response: status={}, body_len={}", status, response_body.len());

            if status == 200 {
                let data: CryptoPriceResponse = serde_json::from_str(&response_body)
                    .map_err(|e| format!("Failed to parse response: {} - body: {}", e, response_body))?;
                Ok(data.open_price)
            } else {
                Err(format!("API returned error status {}: {}", status, response_body))
            }
        })();

        let _ = tx.send(result);
    });

    // Wait for the dedicated thread to complete
    let result = rx.await
        .map_err(|_| anyhow::anyhow!("Price thread channel closed unexpectedly"))?;

    match result {
        Ok(open_price) => {
            debug!(open_price = open_price, "Retrieved price to beat");
            Ok(open_price)
        }
        Err(e) => Err(anyhow::anyhow!("Failed to fetch crypto price: {}", e)),
    }
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
