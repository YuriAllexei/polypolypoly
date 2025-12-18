//! Up or Down Strategy
//!
//! Monitors recurring crypto price prediction markets
//! with tags: 'Up or Down', 'Crypto Prices', 'Recurring', 'Crypto'
//!
//! When a market enters the delta_t window (time before end), this strategy
//! spawns a WebSocket tracker to monitor the orderbook in real-time.

use super::traits::{Strategy, StrategyContext, StrategyResult};
use crate::domain::DbMarket;
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::config::UpOrDownConfig;
use crate::infrastructure::{
    build_ws_client, handle_client_event, spawn_oracle_trackers, MarketTrackerConfig, OracleType,
    SharedOraclePrices, SharedOrderbooks, SharedPrecisions, TickSizeChangeEvent,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use crossbeam_channel::unbounded;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Required tags for Up or Down markets
const REQUIRED_TAGS: &[&str] = &["Up or Down", "Crypto Prices", "Recurring", "Crypto"];
const STALENESS_THRESHOLD_SECS: f64 = 60.0;
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

// =============================================================================
// Market Metadata Types
// =============================================================================

/// Oracle source detected from market description
#[derive(Debug, Clone, Copy, PartialEq)]
enum OracleSource {
    Binance,
    ChainLink,
    Unknown,
}

impl OracleSource {
    fn from_description(description: &Option<String>) -> Self {
        match description {
            Some(desc) => {
                if desc.contains("www.binance.com") {
                    OracleSource::Binance
                } else if desc.contains("data.chain.link") {
                    OracleSource::ChainLink
                } else {
                    OracleSource::Unknown
                }
            }
            None => OracleSource::Unknown,
        }
    }
}

impl std::fmt::Display for OracleSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleSource::Binance => write!(f, "Binance"),
            OracleSource::ChainLink => write!(f, "ChainLink"),
            OracleSource::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Cryptocurrency tracked by the market
#[derive(Debug, Clone, Copy, PartialEq)]
enum CryptoAsset {
    Bitcoin,
    Ethereum,
    Solana,
    Xrp,
    Unknown,
}

impl CryptoAsset {
    fn from_tags(tags: &serde_json::Value) -> Self {
        if let serde_json::Value::Array(arr) = tags {
            for tag in arr {
                if let Some(label) = tag.get("label").and_then(|l| l.as_str()) {
                    match label {
                        "Bitcoin" => return CryptoAsset::Bitcoin,
                        "Ethereum" => return CryptoAsset::Ethereum,
                        "Solana" => return CryptoAsset::Solana,
                        "XRP" => return CryptoAsset::Xrp,
                        _ => {}
                    }
                }
            }
        }
        CryptoAsset::Unknown
    }

    /// Get the symbol used for oracle price lookup
    fn oracle_symbol(&self) -> &'static str {
        match self {
            CryptoAsset::Bitcoin => "BTC",
            CryptoAsset::Ethereum => "ETH",
            CryptoAsset::Solana => "SOL",
            CryptoAsset::Xrp => "XRP",
            CryptoAsset::Unknown => "",
        }
    }
}

impl std::fmt::Display for CryptoAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoAsset::Bitcoin => write!(f, "Bitcoin (BTC)"),
            CryptoAsset::Ethereum => write!(f, "Ethereum (ETH)"),
            CryptoAsset::Solana => write!(f, "Solana (SOL)"),
            CryptoAsset::Xrp => write!(f, "XRP"),
            CryptoAsset::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Timeframe of the market
#[derive(Debug, Clone, Copy, PartialEq)]
enum Timeframe {
    FiveMin,    // 5M - NOT officially live, should be skipped
    FifteenMin, // 15M
    OneHour,    // 1H
    FourHour,   // 4H
    Daily,
    Unknown,
}

impl Timeframe {
    fn from_tags(tags: &serde_json::Value) -> Self {
        if let serde_json::Value::Array(arr) = tags {
            for tag in arr {
                if let Some(label) = tag.get("label").and_then(|l| l.as_str()) {
                    match label {
                        "5M" => return Timeframe::FiveMin,
                        "15M" => return Timeframe::FifteenMin,
                        "1H" => return Timeframe::OneHour,
                        "4H" => return Timeframe::FourHour,
                        "Daily" => return Timeframe::Daily,
                        _ => {}
                    }
                }
            }
        }
        Timeframe::Unknown
    }

    /// Check if this timeframe is officially supported/live
    /// 5M markets are not officially live yet and should be skipped
    fn is_supported(&self) -> bool {
        match self {
            Timeframe::FiveMin => false,  // Not officially live
            Timeframe::Unknown => false,  // Unknown timeframes should be skipped
            _ => true,
        }
    }
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Timeframe::FiveMin => write!(f, "5M"),
            Timeframe::FifteenMin => write!(f, "15M"),
            Timeframe::OneHour => write!(f, "1H"),
            Timeframe::FourHour => write!(f, "4H"),
            Timeframe::Daily => write!(f, "Daily"),
            Timeframe::Unknown => write!(f, "Unknown"),
        }
    }
}

// =============================================================================
// Market Tracker Types
// =============================================================================

/// Context holding immutable market information for the tracker
struct MarketTrackerContext {
    market_id: String,
    market_question: String,
    market_url: String,
    oracle_source: OracleSource,
    crypto_asset: CryptoAsset,
    timeframe: Timeframe,
    token_ids: Vec<String>,
    outcome_map: HashMap<String, String>,
    /// Market end time for dynamic threshold calculation
    market_end_time: DateTime<Utc>,
    /// Minimum threshold in seconds (when close to market end)
    threshold_min: f64,
    /// Maximum threshold in seconds (when far from market end)
    threshold_max: f64,
    /// Decay time constant in seconds
    threshold_tau: f64,
    /// The opening price that determines "up" or "down" outcome
    price_to_beat: Option<f64>,
    /// Oracle price difference threshold in basis points
    oracle_bps_price_threshold: f64,
}

impl MarketTrackerContext {
    fn new(
        market: &DbMarket,
        config: &UpOrDownConfig,
        outcomes: Vec<String>,
    ) -> anyhow::Result<Self> {
        let tags = market
            .parse_tags()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let token_ids = market.parse_token_ids()?;

        // Build outcome map (token_id -> outcome name)
        let outcome_map: HashMap<String, String> = token_ids
            .iter()
            .zip(outcomes.iter())
            .map(|(id, outcome)| (id.clone(), outcome.clone()))
            .collect();

        let market_url = market
            .slug
            .as_ref()
            .map(|s| format!("https://polymarket.com/event/{}", s))
            .unwrap_or_else(|| "N/A".to_string());

        // Parse market end time
        let market_end_time = DateTime::parse_from_rfc3339(&market.end_date)
            .map_err(|e| anyhow::anyhow!("Failed to parse market end_date: {}", e))?
            .with_timezone(&Utc);

        Ok(Self {
            market_id: market.id.clone(),
            market_question: market.question.clone(),
            market_url,
            oracle_source: OracleSource::from_description(&market.description),
            crypto_asset: CryptoAsset::from_tags(&tags),
            timeframe: Timeframe::from_tags(&tags),
            token_ids,
            outcome_map,
            market_end_time,
            threshold_min: config.threshold_min,
            threshold_max: config.threshold_max,
            threshold_tau: config.threshold_tau,
            price_to_beat: None,
            oracle_bps_price_threshold: config.oracle_bps_price_threshold,
        })
    }

    fn get_outcome_name(&self, token_id: &str) -> String {
        self.outcome_map
            .get(token_id)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn set_price_to_beat(&mut self, price: Option<f64>) {
        self.price_to_beat = price;
    }

    fn format_price_to_beat(&self) -> String {
        match self.price_to_beat {
            Some(price) => format!("${:.2}", price),
            None => "N/A".to_string(),
        }
    }
}

/// Mutable state for the market tracker
struct TrackerState {
    /// Timers tracking how long each token has had no asks
    no_asks_timers: HashMap<String, Instant>,
    /// Tokens that have exceeded the no-asks threshold
    threshold_triggered: HashSet<String>,
    /// Orders placed: token_id → order_id (for cancellation tracking)
    order_placed: HashMap<String, String>,
}

impl TrackerState {
    fn new() -> Self {
        Self {
            no_asks_timers: HashMap::new(),
            threshold_triggered: HashSet::new(),
            order_placed: HashMap::new(),
        }
    }

    /// Get all order IDs for cancellation
    fn get_order_ids(&self) -> Vec<String> {
        self.order_placed.values().cloned().collect()
    }
}

/// Result of checking orderbook state for a single token
enum OrderbookCheckResult {
    /// Asks exist - market is active
    HasAsks,
    /// No asks - timer started or continuing
    NoAsks,
    /// No asks and threshold exceeded - should place order
    ThresholdExceeded { elapsed_secs: f64 },
}

/// Reason for exiting the tracking loop
enum TrackingLoopExit {
    Shutdown,
    MarketEnded,
    AllOrderbooksEmpty,
    WebSocketDisconnected,
    StaleOrderbook,
}

// =============================================================================
// Market Tracker Helper Functions
// =============================================================================

/// Calculate dynamic no-ask threshold based on time remaining until market end.
///
/// Uses exponential decay formula:
/// threshold = min + (max - min) * (1 - exp(-time_remaining / tau))
///
/// - When far from market end (large time_remaining): threshold approaches max (conservative)
/// - When close to market end (small time_remaining): threshold approaches min (aggressive)
fn calculate_dynamic_threshold(ctx: &MarketTrackerContext) -> f64 {
    let now = Utc::now();
    let time_remaining = ctx
        .market_end_time
        .signed_duration_since(now)
        .num_milliseconds() as f64
        / 1000.0;

    // If past market end or at market end, use minimum threshold
    if time_remaining <= 0.0 {
        return ctx.threshold_min;
    }

    // Exponential decay formula
    ctx.threshold_min
        + (ctx.threshold_max - ctx.threshold_min)
            * (1.0 - (-time_remaining / ctx.threshold_tau).exp())
}

/// Check a single token's orderbook and update timer state
fn check_token_orderbook(
    token_id: &str,
    has_asks: bool,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
) -> OrderbookCheckResult {
    let outcome_name = ctx.get_outcome_name(token_id);

    if has_asks {
        // Asks exist - reset timer and threshold state
        if state.no_asks_timers.remove(token_id).is_some() {
            state.threshold_triggered.remove(token_id);
            debug!(
                "⏹️  Timer RESET for {} ({}) - asks appeared in orderbook",
                token_id, outcome_name
            );
        }
        return OrderbookCheckResult::HasAsks;
    }

    // No asks - start timer if not running
    // Only log "STARTING TIMER" if:
    // 1. Timer doesn't exist yet
    // 2. We haven't already triggered threshold (prevents spam after order cycle)
    // 3. We don't already have an order placed (prevents spam after order placed)
    if !state.no_asks_timers.contains_key(token_id) {
        // Only log if this is truly a new detection (not a restart after order cycle)
        if !state.threshold_triggered.contains(token_id) && !state.order_placed.contains_key(token_id) {
            log_no_asks_started(ctx, token_id, &outcome_name);
        }
        state
            .no_asks_timers
            .insert(token_id.to_string(), Instant::now());
    }

    // Check if threshold exceeded using dynamic threshold
    if !state.threshold_triggered.contains(token_id) {
        if let Some(timer_start) = state.no_asks_timers.get(token_id) {
            let elapsed = timer_start.elapsed().as_secs_f64();
            let dynamic_threshold = calculate_dynamic_threshold(ctx);
            if elapsed >= dynamic_threshold {
                // Check if order already placed for this token
                if state.order_placed.contains_key(token_id) {
                    // Order already exists - silently skip (don't remove timer to prevent restart)
                    return OrderbookCheckResult::NoAsks;
                }
                state.threshold_triggered.insert(token_id.to_string());
                return OrderbookCheckResult::ThresholdExceeded {
                    elapsed_secs: elapsed,
                };
            }
        }
    }

    OrderbookCheckResult::NoAsks
}

/// Check all orderbooks and return tokens that need orders placed
async fn check_all_orderbooks(
    orderbooks: &SharedOrderbooks,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
) -> (Vec<(String, String, f64)>, bool) {
    let mut tokens_to_order = Vec::new();
    let mut all_empty = true;

    let token_data: Vec<(String, bool, bool)> = {
        let obs = orderbooks.read().unwrap();
        ctx.token_ids
            .iter()
            .filter_map(|token_id| {
                obs.get(token_id).map(|orderbook| {
                    (
                        token_id.clone(),
                        !orderbook.asks.is_empty(),
                        !orderbook.bids.is_empty(),
                    )
                })
            })
            .collect()
    };

    for (token_id, has_asks, has_bids) in token_data {
        if has_asks || has_bids {
            all_empty = false;
        }

        match check_token_orderbook(&token_id, has_asks, state, ctx) {
            OrderbookCheckResult::ThresholdExceeded { elapsed_secs } => {
                let outcome_name = ctx.get_outcome_name(&token_id);
                let dynamic_threshold = calculate_dynamic_threshold(ctx);
                log_threshold_exceeded(
                    ctx,
                    &token_id,
                    &outcome_name,
                    elapsed_secs,
                    dynamic_threshold,
                );
                tokens_to_order.push((token_id.clone(), outcome_name, elapsed_secs));
            }
            _ => {}
        }
    }

    (tokens_to_order, all_empty)
}

/// Place a buy order for a token
/// Returns the order_id if successful, None if failed
async fn place_order(
    trading: &TradingClient,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    ctx: &MarketTrackerContext,
    precisions: &SharedPrecisions,
) -> Option<String> {
    let dynamic_threshold = calculate_dynamic_threshold(ctx);
    log_placing_order(ctx, token_id, outcome_name, elapsed, dynamic_threshold);

    // Get precision for this token (default to 2)
    let precision = {
        let precs = precisions.read().unwrap();
        *precs.get(token_id).unwrap_or(&2)
    };

    // Calculate price: 0.99 for precision 2, 0.999 for precision 3, etc.
    let price = 1.0 - 10_f64.powi(-(precision as i32));

    match trading.buy(token_id, price, 18.0).await {
        Ok(response) => {
            log_order_success(ctx, token_id, outcome_name, &response);
            response.order_id
        }
        Err(e) => {
            log_order_failed(ctx, token_id, outcome_name, &e);
            None
        }
    }
}

/// Check for risk on tokens with placed orders and cancel if risk detected.
///
/// Two signals must both be active to indicate risk:
/// 1. Average of other bids (excluding top) < 0.85
/// 2. |price_to_beat - oracle_price| in bps < oracle_bps_price_threshold
///
/// Returns false early if no orders are placed or if the market has ended.
/// Only cancels the specific token(s) where risk is detected, not all orders.
async fn check_risk(
    orderbooks: &SharedOrderbooks,
    state: &mut TrackerState,
    ctx: &MarketTrackerContext,
    oracle_prices: &Option<SharedOraclePrices>,
    trading: &TradingClient,
) -> bool {
    if state.order_placed.is_empty() {
        return false;
    }

    // If market has ended, no point in checking risk
    if Utc::now() > ctx.market_end_time {
        return false;
    }

    // Signal 2: Check oracle price difference (applies to whole market)
    let mut signal_2_active = false;
    let mut bps_diff = 0.0;
    let mut oracle_price = 0.0;

    if let (Some(price_to_beat), Some(oracle_prices)) = (ctx.price_to_beat, oracle_prices) {
        if let Some(current_price) =
            get_oracle_price(ctx.oracle_source, ctx.crypto_asset, oracle_prices)
        {
            oracle_price = current_price;
            bps_diff = ((price_to_beat - current_price).abs() / price_to_beat) * 10000.0;
            if bps_diff < ctx.oracle_bps_price_threshold {
                signal_2_active = true;
            }
        }
    }

    // If oracle signal not active, no risk
    if !signal_2_active {
        return false;
    }

    // Signal 1: Check bid levels per token
    let bid_data: Vec<(String, Vec<f64>)> = {
        let obs = orderbooks.read().unwrap();
        state
            .order_placed
            .keys()
            .filter_map(|token_id| {
                obs.get(token_id).and_then(|orderbook| {
                    let bid_levels = orderbook.bids.levels();
                    if bid_levels.len() > 1 {
                        let other_bids: Vec<f64> = bid_levels
                            .iter()
                            .skip(1)
                            .take(4)
                            .map(|(price, _)| *price)
                            .collect();
                        if !other_bids.is_empty() {
                            return Some((token_id.clone(), other_bids));
                        }
                    }
                    None
                })
            })
            .collect()
    };

    let mut tokens_at_risk: Vec<(String, f64, Vec<f64>)> = Vec::new();
    for (token_id, other_bids) in bid_data {
        let avg_bid_price = other_bids.iter().sum::<f64>() / other_bids.len() as f64;
        if avg_bid_price < 0.85 {
            tokens_at_risk.push((token_id, avg_bid_price, other_bids));
        }
    }

    if tokens_at_risk.is_empty() {
        return false;
    }

    // Cancel only the specific tokens at risk
    for (token_id, avg_bid_price, other_bids) in tokens_at_risk {
        let outcome_name = ctx.get_outcome_name(&token_id);
        log_risk_detected(
            ctx,
            &token_id,
            &outcome_name,
            avg_bid_price,
            &other_bids,
            bps_diff,
            oracle_price,
        );

        // Cancel only this token's order
        if let Some(order_id) = state.order_placed.remove(&token_id) {
            cancel_order(trading, &order_id, &token_id, ctx).await;
        }
    }

    true
}

/// Cancel a single order and log the result
async fn cancel_order(
    trading: &TradingClient,
    order_id: &str,
    token_id: &str,
    ctx: &MarketTrackerContext,
) {
    let outcome_name = ctx.get_outcome_name(token_id);
    info!(
        "[WS {}] 🚨 Cancelling order {} for {}",
        ctx.market_id, order_id, outcome_name
    );

    match trading.cancel_order(order_id).await {
        Ok(response) => {
            if !response.canceled.is_empty() {
                info!(
                    "[WS {}] ✅ Cancelled order for {}",
                    ctx.market_id, outcome_name
                );
            }
            if !response.not_canceled.is_empty() {
                warn!(
                    "[WS {}] ⚠️ Failed to cancel order {}: {:?}",
                    ctx.market_id, order_id, response.not_canceled
                );
            }
        }
        Err(e) => {
            error!(
                "[WS {}] ❌ Failed to cancel order {}: {}",
                ctx.market_id, order_id, e
            );
        }
    }
}

/// Cancel orders and log the result
async fn cancel_orders(trading: &TradingClient, order_ids: &[String], ctx: &MarketTrackerContext) {
    info!(
        "[WS {}] 🚨 CANCELLING {} orders due to risk detection",
        ctx.market_id,
        order_ids.len()
    );

    match trading.cancel_orders(order_ids).await {
        Ok(response) => {
            if !response.canceled.is_empty() {
                info!(
                    "[WS {}] ✅ Successfully cancelled {} orders: {:?}",
                    ctx.market_id,
                    response.canceled.len(),
                    response.canceled
                );
            }
            if !response.not_canceled.is_empty() {
                warn!(
                    "[WS {}] ⚠️ Failed to cancel {} orders: {:?}",
                    ctx.market_id,
                    response.not_canceled.len(),
                    response.not_canceled
                );
            }
        }
        Err(e) => {
            error!("[WS {}] ❌ Failed to cancel orders: {}", ctx.market_id, e);
        }
    }
}

// =============================================================================
// Price API Helper Functions
// =============================================================================

/// Response from Polymarket's crypto price API
#[derive(Debug, Deserialize)]
struct CryptoPriceResponse {
    #[serde(rename = "openPrice")]
    open_price: f64,
}

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
async fn get_price_to_beat(
    timeframe: Timeframe,
    crypto_asset: CryptoAsset,
    market: &DbMarket,
) -> anyhow::Result<f64> {
    // Map CryptoAsset to API symbol
    let symbol = match crypto_asset {
        CryptoAsset::Bitcoin => "BTC",
        CryptoAsset::Ethereum => "ETH",
        CryptoAsset::Solana => "SOL",
        CryptoAsset::Xrp => "XRP",
        CryptoAsset::Unknown => anyhow::bail!("Cannot get price for unknown crypto asset"),
    };

    // Map Timeframe to API variant
    let variant = match timeframe {
        Timeframe::FiveMin => anyhow::bail!("5M markets are not supported"),
        Timeframe::FifteenMin => "fifteen",
        Timeframe::OneHour => "hourly",
        Timeframe::FourHour => "fourhour",
        Timeframe::Daily => "daily",
        Timeframe::Unknown => anyhow::bail!("Cannot get price for unknown timeframe"),
    };

    // Parse end_date from market
    let end_date = DateTime::parse_from_rfc3339(&market.end_date)
        .map_err(|e| anyhow::anyhow!("Failed to parse market end_date: {}", e))?
        .with_timezone(&Utc);

    // Calculate event start time by subtracting timeframe duration
    let duration = match timeframe {
        Timeframe::FiveMin => Duration::minutes(5),
        Timeframe::FifteenMin => Duration::minutes(15),
        Timeframe::OneHour => Duration::hours(1),
        Timeframe::FourHour => Duration::hours(4),
        Timeframe::Daily => Duration::days(1),
        Timeframe::Unknown => anyhow::bail!("Cannot calculate duration for unknown timeframe"),
    };
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

    // Make the HTTP request
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch crypto price: {}", e))?;

    // Check for successful response
    if !response.status().is_success() {
        anyhow::bail!(
            "Crypto price API returned error status: {}",
            response.status()
        );
    }

    // Parse the JSON response
    let data: CryptoPriceResponse = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse crypto price response: {}", e))?;

    debug!(open_price = data.open_price, "Retrieved price to beat");

    Ok(data.open_price)
}

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
fn get_oracle_price(
    oracle_source: OracleSource,
    crypto_asset: CryptoAsset,
    oracle_prices: &SharedOraclePrices,
) -> Option<f64> {
    // Map OracleSource to OracleType
    let oracle_type = match oracle_source {
        OracleSource::Binance => OracleType::Binance,
        OracleSource::ChainLink => OracleType::ChainLink,
        OracleSource::Unknown => return None,
    };

    // Map CryptoAsset to symbol string
    let symbol = match crypto_asset {
        CryptoAsset::Bitcoin => "BTC",
        CryptoAsset::Ethereum => "ETH",
        CryptoAsset::Solana => "SOL",
        CryptoAsset::Xrp => "XRP",
        CryptoAsset::Unknown => return None,
    };

    // Get price from oracle manager
    let manager = oracle_prices.read().unwrap();
    manager
        .get_price(oracle_type, symbol)
        .map(|entry| entry.value)
}

// =============================================================================
// Logging Helper Functions
// =============================================================================

fn log_no_asks_started(ctx: &MarketTrackerContext, token_id: &str, outcome_name: &str) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🎯 NO ASKS IN ORDERBOOK - STARTING TIMER\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:    {}\n\
           Market:       {}\n\
           URL:          {}\n\
           Price to Beat:{}\n\
           Oracle:       {}\n\
           Asset:        {}\n\
           Timeframe:    {}\n\
           Outcome:      {}\n\
           Token ID:     {}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id
    );
}

fn log_threshold_exceeded(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    dynamic_threshold: f64,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         ⚡ NO-ASK TIME THRESHOLD EXCEEDED\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle:         {}\n\
           Asset:          {}\n\
           Timeframe:      {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Elapsed Time:   {:.3} seconds\n\
           Threshold:      {:.3} seconds (dynamic)\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id,
        elapsed,
        dynamic_threshold
    );
}

fn log_placing_order(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    elapsed: f64,
    dynamic_threshold: f64,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🚀 PLACING BUY ORDER\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle:         {}\n\
           Asset:          {}\n\
           Timeframe:      {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Elapsed Time:   {:.3} seconds\n\
           Threshold:      {:.3} seconds (dynamic)\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        ctx.oracle_source,
        ctx.crypto_asset,
        ctx.timeframe,
        outcome_name,
        token_id,
        elapsed,
        dynamic_threshold
    );
}

fn log_order_success<T: std::fmt::Debug>(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    order_id: &T,
) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         ✅ ORDER PLACED SUCCESSFULLY\n\
         ════════════════════════════════════════════════════════════════\n\
           Order ID:       {:?}\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Outcome:        {}\n\
           Timeframe:      {}\n\
           Token ID:       {}\n\
         ════════════════════════════════════════════════════════════════",
        order_id,
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        outcome_name,
        ctx.timeframe,
        token_id
    );
}

fn log_order_failed<E: std::fmt::Display>(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    error: &E,
) {
    error!(
        "════════════════════════════════════════════════════════════════\n\
         ❌ ORDER PLACEMENT FAILED\n\
         ════════════════════════════════════════════════════════════════\n\
           Error:          {}\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
         ════════════════════════════════════════════════════════════════",
        error,
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        outcome_name,
        token_id
    );
}

fn log_risk_detected(
    ctx: &MarketTrackerContext,
    token_id: &str,
    outcome_name: &str,
    avg_bid_price: f64,
    other_bids: &[f64],
    bps_diff: f64,
    oracle_price: f64,
) {
    warn!(
        "════════════════════════════════════════════════════════════════\n\
         ⚠️  RISK DETECTED - BOTH SIGNALS ACTIVE\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:      {}\n\
           Market:         {}\n\
           URL:            {}\n\
           Price to Beat:  {}\n\
           Oracle Price:   ${:.4}\n\
           BPS Difference: {:.4} bps (threshold: {:.4})\n\
           Outcome:        {}\n\
           Token ID:       {}\n\
           Avg Bid (excl top): {:.4}\n\
           Other Bids:     {:?}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat(),
        oracle_price,
        bps_diff,
        ctx.oracle_bps_price_threshold,
        outcome_name,
        token_id,
        avg_bid_price,
        other_bids
    );
}

fn log_market_ended(ctx: &MarketTrackerContext) {
    info!(
        "════════════════════════════════════════════════════════════════\n\
         🏁 MARKET ENDED - NO BIDS OR ASKS IN ANY ORDERBOOK\n\
         ════════════════════════════════════════════════════════════════\n\
           Market ID:    {}\n\
           Market:       {}\n\
           URL:          {}\n\
           Price to Beat:{}\n\
         ════════════════════════════════════════════════════════════════",
        ctx.market_id,
        ctx.market_question,
        ctx.market_url,
        ctx.format_price_to_beat()
    );
}

// =============================================================================
// Strategy Implementation
// =============================================================================

/// Up or Down strategy implementation
pub struct UpOrDownStrategy {
    config: UpOrDownConfig,
    /// Set of market IDs we've already seen (to avoid duplicates)
    tracked_market_ids: HashSet<String>,
    /// Markets we're actively monitoring for ending
    active_markets: Vec<TrackedMarket>,
    /// Spawned tracker tasks (market_id -> JoinHandle)
    tracker_tasks: HashMap<String, JoinHandle<()>>,
    /// Oracle prices (ChainLink and Binance) - strategy-owned
    oracle_prices: Option<SharedOraclePrices>,
}

/// A market being tracked with its parsed end time
#[derive(Debug, Clone)]
struct TrackedMarket {
    /// The database market record
    market: DbMarket,
    /// Parsed end time for quick access
    end_time: DateTime<Utc>,
    /// Whether we've already spawned a WebSocket tracker for this market
    tracker_spawned: bool,
}

impl UpOrDownStrategy {
    pub fn new(config: UpOrDownConfig) -> Self {
        Self {
            config,
            tracked_market_ids: HashSet::new(),
            active_markets: Vec::new(),
            tracker_tasks: HashMap::new(),
            oracle_prices: None,
        }
    }

    /// Fetch markets matching the required tags
    async fn fetch_matching_markets(&self, ctx: &StrategyContext) -> StrategyResult<Vec<DbMarket>> {
        let markets = ctx.database.get_markets_by_tags(REQUIRED_TAGS).await?;
        Ok(markets)
    }

    /// Filter markets that haven't ended yet
    fn filter_active_markets(&self, markets: Vec<DbMarket>) -> Vec<DbMarket> {
        let now = Utc::now();
        markets
            .into_iter()
            .filter(|m| {
                // Parse end_date and check if it's in the future
                match DateTime::parse_from_rfc3339(&m.end_date) {
                    Ok(end_time) => end_time.with_timezone(&Utc) > now,
                    Err(_) => {
                        warn!(
                            "Failed to parse end_date for market {}: {}",
                            m.id, m.end_date
                        );
                        false
                    }
                }
            })
            .collect()
    }

    /// Add new markets to tracking, returns count of newly added
    fn add_new_markets(&mut self, markets: Vec<DbMarket>) -> usize {
        let mut added = 0;
        for market in markets {
            // Only add if we haven't seen this market ID before
            if self.tracked_market_ids.insert(market.id.clone()) {
                // Parse end time
                if let Ok(end_time) = DateTime::parse_from_rfc3339(&market.end_date) {
                    // Validate token_ids and outcomes can be parsed
                    let token_ids = match market.parse_token_ids() {
                        Ok(ids) => ids,
                        Err(e) => {
                            warn!("Failed to parse token_ids for market {}: {}", market.id, e);
                            continue;
                        }
                    };

                    if let Err(e) = market.parse_outcomes() {
                        warn!("Failed to parse outcomes for market {}: {}", market.id, e);
                        continue;
                    }

                    // Skip markets without valid token pairs
                    if token_ids.len() < 2 {
                        warn!("Market {} has less than 2 token_ids, skipping", market.id);
                        continue;
                    }

                    // Skip unsupported timeframes (5M markets are not officially live)
                    let tags = market
                        .parse_tags()
                        .unwrap_or(serde_json::Value::Array(vec![]));
                    let timeframe = Timeframe::from_tags(&tags);
                    if !timeframe.is_supported() {
                        debug!(
                            "Skipping market {} with unsupported timeframe: {} ({})",
                            market.id, timeframe, market.question
                        );
                        continue;
                    }

                    let tracked = TrackedMarket {
                        end_time: end_time.with_timezone(&Utc),
                        market,
                        tracker_spawned: false,
                    };
                    debug!(
                        "Added market to tracking: {} - {} (ends at {})",
                        tracked.market.id, tracked.market.question, tracked.market.end_date
                    );
                    self.active_markets.push(tracked);
                    added += 1;
                }
            }
        }
        added
    }

    /// Check markets within delta_t window and spawn WebSocket trackers
    /// Returns list of markets that need trackers spawned
    fn check_markets_for_tracking(&mut self) -> Vec<TrackedMarket> {
        let now = Utc::now();
        let delta_t = Duration::seconds(self.config.delta_t_seconds as i64);
        let mut markets_to_track = Vec::new();

        for tracked in &mut self.active_markets {
            // Skip if we've already spawned a tracker for this market
            if tracked.tracker_spawned {
                continue;
            }

            let time_until_end = tracked.end_time.signed_duration_since(now);

            // Check if within delta_t window and hasn't ended yet
            if time_until_end > Duration::zero() && time_until_end <= delta_t {
                let market_url = tracked
                    .market
                    .slug
                    .as_ref()
                    .map(|s| format!("https://polymarket.com/event/{}", s))
                    .unwrap_or_else(|| "N/A".to_string());

                // Get token_ids and outcomes for logging (already validated in add_new_markets)
                let token_ids = tracked.market.parse_token_ids().unwrap_or_default();
                let outcomes = tracked.market.parse_outcomes().unwrap_or_default();

                // Parse metadata for logging
                let tags = tracked
                    .market
                    .parse_tags()
                    .unwrap_or(serde_json::Value::Array(vec![]));
                let oracle_source = OracleSource::from_description(&tracked.market.description);
                let crypto_asset = CryptoAsset::from_tags(&tags);
                let timeframe = Timeframe::from_tags(&tags);

                info!(
                    "════════════════════════════════════════════════════════════════\n\
                     ⏰ MARKET ENTERING TRACKING WINDOW!\n\
                     ════════════════════════════════════════════════════════════════\n\
                       Market ID:      {}\n\
                       Question:       {}\n\
                       URL:            {}\n\
                       Oracle:         {}\n\
                       Asset:          {}\n\
                       Timeframe:      {}\n\
                       Time Remaining: {} seconds\n\
                       End Time:       {}\n\
                       Token IDs:      {:?}\n\
                       Outcomes:       {:?}\n\
                     ════════════════════════════════════════════════════════════════",
                    tracked.market.id,
                    tracked.market.question,
                    market_url,
                    oracle_source,
                    crypto_asset,
                    timeframe,
                    time_until_end.num_seconds(),
                    tracked.end_time.format("%Y-%m-%d %H:%M:%S UTC"),
                    token_ids,
                    outcomes
                );

                tracked.tracker_spawned = true;
                markets_to_track.push(tracked.clone());
            }
        }

        markets_to_track
    }

    /// Spawn WebSocket trackers for the given markets
    async fn spawn_trackers(&mut self, markets: Vec<TrackedMarket>, ctx: &StrategyContext) {
        for tracked in markets {
            let market = tracked.market.clone();
            let shutdown_flag = Arc::clone(&ctx.shutdown_flag);
            let config = self.config.clone();
            let trading = Arc::clone(&ctx.trading);
            let oracle_prices = self.oracle_prices.clone();

            info!(
                "[Tracker] Spawning WebSocket tracker for market {}",
                market.id
            );

            // Spawn the tracker task with the tracking loop inline
            let handle = tokio::spawn(async move {
                match UpOrDownStrategy::run_market_tracker(
                    market,
                    shutdown_flag,
                    config,
                    trading,
                    oracle_prices,
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        error!("[Tracker] Market tracker failed: {}", e);
                    }
                }
            });

            self.tracker_tasks.insert(tracked.market.id.clone(), handle);
        }
    }

    /// Run the WebSocket market tracker for a single market
    ///
    /// Connects to Polymarket WebSocket, subscribes to orderbook updates,
    /// and monitors for trading signals until shutdown or market ends.
    async fn run_market_tracker(
        market: DbMarket,
        shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
        config: UpOrDownConfig,
        trading: Arc<TradingClient>,
        oracle_prices: Option<SharedOraclePrices>,
    ) -> anyhow::Result<()> {
        // Initialize context and state
        let outcomes = market.parse_outcomes()?;
        let mut ctx = MarketTrackerContext::new(&market, &config, outcomes.clone())?;
        let mut state = TrackerState::new();

        // Build WebSocket configuration
        let ws_config = MarketTrackerConfig::new(
            ctx.market_id.clone(),
            ctx.market_question.clone(),
            market.slug.clone(),
            ctx.token_ids.clone(),
            outcomes,
            &market.end_date,
        )?;

        // Fetch the price to beat for this market
        let price_to_beat = match get_price_to_beat(ctx.timeframe, ctx.crypto_asset, &market).await
        {
            Ok(price) => {
                info!("[WS {}] Price to beat: ${:.2}", ctx.market_id, price);
                Some(price)
            }
            Err(e) => {
                warn!(
                    "[WS {}] Failed to fetch price to beat: {}",
                    ctx.market_id, e
                );
                None
            }
        };
        ctx.set_price_to_beat(price_to_beat);

        // Log startup info
        info!("[WS {}] Connecting to orderbook stream...", ctx.market_id);
        info!("[WS {}] Market: {}", ctx.market_id, ctx.market_question);
        info!("[WS {}] Oracle: {}", ctx.market_id, ctx.oracle_source);
        info!("[WS {}] Asset: {}", ctx.market_id, ctx.crypto_asset);
        info!("[WS {}] Timeframe: {}", ctx.market_id, ctx.timeframe);
        info!(
            "[WS {}] Resolution time: {}",
            ctx.market_id, ws_config.resolution_time
        );

        // Track reconnection attempts
        let mut reconnect_attempts: u32 = 0;

        // Outer reconnection loop - handles WebSocket reconnection on staleness
        'reconnect: loop {
            // Check shutdown before attempting connection
            if !shutdown_flag.load(Ordering::Acquire) {
                info!("[WS {}] Shutdown signal received before connect", ctx.market_id);
                break 'reconnect;
            }

            // Check if market has ended before attempting connection
            if Utc::now() > ctx.market_end_time {
                info!(
                    "[WS {}] Market already ended, not connecting",
                    ctx.market_id
                );
                break 'reconnect;
            }

            if reconnect_attempts > 0 {
                info!(
                    "[WS {}] Reconnection attempt {} of {}",
                    ctx.market_id, reconnect_attempts, MAX_RECONNECT_ATTEMPTS
                );
                // Brief delay before reconnection to avoid hammering the server
                sleep(StdDuration::from_secs(2)).await;
            }

            // Create shared orderbooks, precisions, and connect WebSocket
            let orderbooks: SharedOrderbooks = Arc::new(std::sync::RwLock::new(HashMap::new()));
            let precisions: SharedPrecisions = Arc::new(std::sync::RwLock::new(HashMap::new()));
            let first_snapshot_received = Arc::new(AtomicBool::new(false));

            // Create channel for tick_size_change events
            let (tick_size_tx, tick_size_rx) = unbounded::<TickSizeChangeEvent>();

            let client = match build_ws_client(
                &ws_config,
                Arc::clone(&orderbooks),
                Arc::clone(&precisions),
                Some(tick_size_tx),
                Arc::clone(&first_snapshot_received),
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    error!("[WS {}] Failed to connect: {}", ctx.market_id, e);
                    reconnect_attempts += 1;
                    continue 'reconnect;
                }
            };
            info!("[WS {}] Connected and subscribed", ctx.market_id);

            // Wait for first snapshot
            let start = std::time::Instant::now();
            let snapshot_received = loop {
                if first_snapshot_received.load(Ordering::Acquire) {
                    break true;
                }
                if start.elapsed() > StdDuration::from_secs(10) {
                    error!("[WS {}] Timeout waiting for first snapshot", ctx.market_id);
                    break false;
                }
                if !shutdown_flag.load(Ordering::Acquire) {
                    info!("[WS {}] Shutdown during snapshot wait", ctx.market_id);
                    let _ = client.shutdown().await;
                    break 'reconnect;
                }
                sleep(StdDuration::from_millis(10)).await;
            };

            if !snapshot_received {
                let _ = client.shutdown().await;
                reconnect_attempts += 1;
                continue 'reconnect;
            }

            // Validate all expected tokens have orderbooks
            let has_missing_tokens = {
                let obs = orderbooks.read().unwrap();
                let missing_count = ctx.token_ids.iter().filter(|t| !obs.contains_key(*t)).count();
                if missing_count > 0 {
                    error!("[WS {}] Missing orderbooks for {} tokens", ctx.market_id, missing_count);
                }
                missing_count > 0
            };
            if has_missing_tokens {
                let _ = client.shutdown().await;
                reconnect_attempts += 1;
                continue 'reconnect;
            }

            // Track when this connection started (for reconnect counter reset logic)
            let connection_start = std::time::Instant::now();

            // Track if we've seen any updates (price_change) since connecting
            // Used to distinguish "inactive market" from "silently disconnected"
            let mut seen_updates_since_connect = false;

            // Main tracking loop
            let exit_reason = loop {
                // Check shutdown flag (highest priority)
                if !shutdown_flag.load(Ordering::Acquire) {
                    info!("[WS {}] Shutdown signal received", ctx.market_id);
                    break TrackingLoopExit::Shutdown;
                }

                // Check if market resolution time has passed (exit gracefully)
                if Utc::now() > ctx.market_end_time {
                    info!(
                        "[WS {}] Market resolution time passed ({}), stopping tracker",
                        ctx.market_id,
                        ctx.market_end_time.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    break TrackingLoopExit::MarketEnded;
                }

                // Handle WebSocket events
                if let Some(event) = client.try_recv_event() {
                    if !handle_client_event(event, &ctx.market_id) {
                        break TrackingLoopExit::WebSocketDisconnected;
                    }
                }

                // Handle tick_size_change events from the channel
                while let Ok(event) = tick_size_rx.try_recv() {
                    info!(
                        "[WS {}] Tick size changed for {}: {} -> {} (detected in main loop)",
                        ctx.market_id,
                        ctx.get_outcome_name(&event.asset_id),
                        event.old_tick_size,
                        event.new_tick_size
                    );
                }

                // Check for stale orderbooks (WebSocket may have silently disconnected)
                // But only if we've seen updates since connecting - inactive markets shouldn't trigger staleness
                let (is_stale, market_has_activity) = {
                    let obs = orderbooks.read().unwrap();
                    let connection_age = connection_start.elapsed().as_secs_f64();
                    let mut stale = false;
                    let mut has_activity = seen_updates_since_connect;

                    for (token_id, orderbook) in obs.iter() {
                        let staleness = orderbook.seconds_since_update();

                        // Check if we've received updates beyond the initial snapshot
                        // If staleness is significantly less than connection age, we've had updates
                        if !has_activity && staleness < (connection_age - 5.0) {
                            has_activity = true;
                        }

                        // Only consider staleness if we've seen activity
                        // (otherwise the market is just inactive, not disconnected)
                        if has_activity && staleness > STALENESS_THRESHOLD_SECS {
                            warn!(
                                "[WS {}] Orderbook for {} is stale ({:.1}s since last update) - triggering reconnection",
                                ctx.market_id,
                                ctx.get_outcome_name(token_id),
                                staleness
                            );
                            stale = true;
                            break;
                        }
                    }
                    (stale, has_activity)
                };

                // Update our tracking of whether we've seen activity
                if market_has_activity {
                    seen_updates_since_connect = true;
                }

                if is_stale {
                    break TrackingLoopExit::StaleOrderbook;
                }

                // Check orderbooks and get tokens needing orders
                let (tokens_to_order, all_empty) =
                    check_all_orderbooks(&orderbooks, &mut state, &ctx).await;

                // Exit if market has ended (all orderbooks empty)
                if all_empty {
                    log_market_ended(&ctx);
                    break TrackingLoopExit::AllOrderbooksEmpty;
                }

                // Place orders for tokens that exceeded threshold
                for (token_id, outcome_name, elapsed) in tokens_to_order {
                    // Re-check orderbook before placing order (ensures fresh data)
                    let still_no_asks = {
                        let obs = orderbooks.read().unwrap();
                        obs.get(&token_id).map(|ob| ob.asks.is_empty()).unwrap_or(false)
                    };

                    if !still_no_asks {
                        info!(
                            "[WS {}] Skipping order for {} - asks appeared during processing",
                            ctx.market_id, outcome_name
                        );
                        state.threshold_triggered.remove(&token_id);
                        state.no_asks_timers.remove(&token_id);
                        continue;
                    }

                    if let Some(order_id) =
                        place_order(&trading, &token_id, &outcome_name, elapsed, &ctx, &precisions).await
                    {
                        state.order_placed.insert(token_id, order_id);
                    }
                }

                // Monitor for risk on placed orders and cancel if detected
                check_risk(&orderbooks, &mut state, &ctx, &oracle_prices, &trading).await;

                // Brief sleep before next iteration
                sleep(StdDuration::from_millis(100)).await;
            };

            // Close current WebSocket connection
            info!("[WS {}] Closing connection (reason: {:?})", ctx.market_id,
                match &exit_reason {
                    TrackingLoopExit::Shutdown => "shutdown",
                    TrackingLoopExit::MarketEnded => "market_ended",
                    TrackingLoopExit::AllOrderbooksEmpty => "all_empty",
                    TrackingLoopExit::WebSocketDisconnected => "ws_disconnected",
                    TrackingLoopExit::StaleOrderbook => "stale_orderbook",
                }
            );
            if let Err(e) = client.shutdown().await {
                warn!("[WS {}] Error during shutdown: {}", ctx.market_id, e);
            }

            // Decide whether to reconnect or exit
            match exit_reason {
                TrackingLoopExit::StaleOrderbook | TrackingLoopExit::WebSocketDisconnected => {
                    // Check if connection was stable (ran longer than staleness threshold)
                    // If so, this is likely a real disconnect, not a pattern of repeated staleness
                    let connection_duration = connection_start.elapsed().as_secs_f64();
                    if connection_duration > STALENESS_THRESHOLD_SECS * 2.0 {
                        // Connection was stable for a while, reset the counter
                        reconnect_attempts = 0;
                        info!(
                            "[WS {}] Connection was stable for {:.1}s, resetting reconnect counter",
                            ctx.market_id, connection_duration
                        );
                    }

                    // These are recoverable - try to reconnect
                    reconnect_attempts += 1;

                    // Check if we've exceeded max attempts AFTER incrementing
                    if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                        error!(
                            "[WS {}] Exceeded max reconnection attempts ({}) due to repeated staleness/disconnects, giving up",
                            ctx.market_id, MAX_RECONNECT_ATTEMPTS
                        );
                        break 'reconnect;
                    }

                    info!(
                        "[WS {}] Will attempt reconnection (attempt {} of {})",
                        ctx.market_id, reconnect_attempts, MAX_RECONNECT_ATTEMPTS
                    );
                    // Clear timer state on reconnect to avoid false triggers from stale timers
                    state.no_asks_timers.clear();
                    state.threshold_triggered.clear();
                    continue 'reconnect;
                }
                TrackingLoopExit::Shutdown
                | TrackingLoopExit::MarketEnded
                | TrackingLoopExit::AllOrderbooksEmpty => {
                    // These are terminal - exit completely
                    break 'reconnect;
                }
            }
        }
        if !state.order_placed.is_empty() {
            info!(
                "[WS {}] Tracker stopping with {} orders still open (leaving them for potential fills)",
                ctx.market_id,
                state.order_placed.len()
            );
        }

        info!("[WS {}] Tracker stopped", ctx.market_id);
        Ok(())
    }

    /// Remove completed tracker tasks from the tasks map.
    ///
    /// Called periodically to clean up finished async tasks and free resources.
    /// Tasks complete either when the market resolves or when shutdown is signaled.
    fn cleanup_tracker_tasks(&mut self) {
        self.tracker_tasks.retain(|market_id, handle| {
            if handle.is_finished() {
                debug!(market_id = %market_id, "Cleaned up completed tracker task");
                false
            } else {
                true
            }
        });
    }

    /// Remove markets whose end time has passed from the active markets list.
    ///
    /// Called periodically to clean up expired markets. Markets are kept even after
    /// their trackers complete so we can track their IDs to avoid re-adding.
    fn cleanup_ended_markets(&mut self) {
        let now = Utc::now();
        let initial_count = self.active_markets.len();

        self.active_markets.retain(|m| m.end_time > now);

        let removed = initial_count - self.active_markets.len();
        if removed > 0 {
            debug!("Removed {} ended markets from active tracking", removed);
        }
    }
}

#[async_trait]
impl Strategy for UpOrDownStrategy {
    fn name(&self) -> &str {
        "up_or_down"
    }

    fn description(&self) -> &str {
        "Monitors Up or Down crypto price prediction markets"
    }

    async fn initialize(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!(
            tags = ?REQUIRED_TAGS,
            delta_t_seconds = self.config.delta_t_seconds,
            poll_interval_secs = self.config.poll_interval_secs,
            "Initializing Up or Down strategy"
        );

        // Spawn oracle price trackers (lives for strategy lifetime)
        info!("Starting oracle price trackers (ChainLink + Binance)");
        self.oracle_prices = Some(spawn_oracle_trackers(ctx.shutdown_flag.clone()).await?);
        info!("Oracle price trackers started successfully");

        // Initial market fetch
        let markets = self.fetch_matching_markets(ctx).await?;
        let active = self.filter_active_markets(markets.clone());
        let added = self.add_new_markets(active.clone());

        info!(
            total_matching = markets.len(),
            active_count = active.len(),
            added_to_tracking = added,
            "Initial market discovery completed"
        );

        Ok(())
    }

    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("Starting Up or Down strategy main loop");

        let poll_interval = StdDuration::from_secs_f64(self.config.poll_interval_secs);

        while ctx.is_running() {
            // 1. Fetch new markets from database
            match self.fetch_matching_markets(ctx).await {
                Ok(markets) => {
                    let active = self.filter_active_markets(markets);
                    let new_count = self.add_new_markets(active);

                    if new_count > 0 {
                        info!(
                            "Added {} new markets to tracking (total active: {})",
                            new_count,
                            self.active_markets.len()
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch markets: {}", e);
                }
            }

            // 2. Check which markets are within delta_t and need WebSocket tracking
            let markets_to_track = self.check_markets_for_tracking();
            if !markets_to_track.is_empty() {
                info!(
                    "Spawning WebSocket trackers for {} markets",
                    markets_to_track.len()
                );
                self.spawn_trackers(markets_to_track, ctx).await;
            }

            // 3. Cleanup completed tracker tasks
            self.cleanup_tracker_tasks();

            // 4. Cleanup ended markets
            self.cleanup_ended_markets();

            // 5. Log status periodically
            if !self.tracker_tasks.is_empty() {
                debug!(
                    "Active trackers: {}, Active markets: {}",
                    self.tracker_tasks.len(),
                    self.active_markets.len()
                );
            }

            // 6. Wait for next iteration (interruptible by shutdown)
            ctx.shutdown.interruptible_sleep(poll_interval).await;
        }

        info!("Up or Down strategy loop ended (shutdown requested)");
        Ok(())
    }

    async fn stop(&mut self) -> StrategyResult<()> {
        info!(
            total_tracked = self.tracked_market_ids.len(),
            active_markets = self.active_markets.len(),
            active_trackers = self.tracker_tasks.len(),
            "Stopping Up or Down strategy"
        );

        // Wait for all tracker tasks to complete (they will stop due to shutdown flag)
        if !self.tracker_tasks.is_empty() {
            info!(
                count = self.tracker_tasks.len(),
                "Waiting for WebSocket trackers to shut down"
            );

            for (market_id, handle) in self.tracker_tasks.drain() {
                match handle.await {
                    Ok(()) => debug!(market_id = %market_id, "Tracker task completed"),
                    Err(e) => {
                        warn!(market_id = %market_id, error = %e, "Tracker task failed to join")
                    }
                }
            }

            info!("All WebSocket trackers shut down");
        }

        Ok(())
    }
}
