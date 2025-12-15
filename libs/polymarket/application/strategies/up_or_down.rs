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
    SharedOraclePrices, SharedOrderbooks,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Required tags for Up or Down markets
const REQUIRED_TAGS: &[&str] = &["Up or Down", "Crypto Prices", "Recurring", "Crypto"];

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
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
    /// Whether this is the first orderbook check (used for initial delay)
    first_orderbook_check: bool,
}

impl TrackerState {
    fn new() -> Self {
        Self {
            no_asks_timers: HashMap::new(),
            threshold_triggered: HashSet::new(),
            order_placed: HashMap::new(),
            first_orderbook_check: true,
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
            info!(
                "⏹️  Timer RESET for {} ({}) - asks appeared in orderbook",
                token_id, outcome_name
            );
        }
        return OrderbookCheckResult::HasAsks;
    }

    // No asks - start timer if not running
    if !state.no_asks_timers.contains_key(token_id) {
        log_no_asks_started(ctx, token_id, &outcome_name);
        state
            .no_asks_timers
            .insert(token_id.to_string(), Instant::now());
    }

    // Check if threshold exceeded using dynamic threshold
    if !state.threshold_triggered.contains(token_id) && !state.order_placed.contains_key(token_id) {
        if let Some(timer_start) = state.no_asks_timers.get(token_id) {
            let elapsed = timer_start.elapsed().as_secs_f64();
            let dynamic_threshold = calculate_dynamic_threshold(ctx);
            if elapsed >= dynamic_threshold {
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
    // Sleep on first call to allow orderbook data to populate
    if state.first_orderbook_check {
        sleep(StdDuration::from_secs(2)).await;
        state.first_orderbook_check = false;
    }

    let mut tokens_to_order = Vec::new();
    let mut all_empty = true;

    let obs = orderbooks.read().unwrap();
    for token_id in &ctx.token_ids {
        if let Some(orderbook) = obs.get(token_id) {
            let has_asks = !orderbook.asks.is_empty();
            let has_bids = !orderbook.bids.is_empty();

            if has_asks || has_bids {
                all_empty = false;
            }

            match check_token_orderbook(token_id, has_asks, state, ctx) {
                OrderbookCheckResult::ThresholdExceeded { elapsed_secs } => {
                    let outcome_name = ctx.get_outcome_name(token_id);
                    let dynamic_threshold = calculate_dynamic_threshold(ctx);
                    log_threshold_exceeded(
                        ctx,
                        token_id,
                        &outcome_name,
                        elapsed_secs,
                        dynamic_threshold,
                    );
                    tokens_to_order.push((token_id.clone(), outcome_name, elapsed_secs));
                }
                _ => {}
            }
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
) -> Option<String> {
    let dynamic_threshold = calculate_dynamic_threshold(ctx);
    log_placing_order(ctx, token_id, outcome_name, elapsed, dynamic_threshold);

    match trading.buy(token_id, 0.99, 18.0).await {
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
/// 1. Average of other bids (excluding top) < 0.90
/// 2. |price_to_beat - oracle_price| in bps < oracle_bps_price_threshold
///
/// When risk is detected, cancels all placed orders and clears the order tracking state.
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

    // Signal 1: Check bid levels (existing logic)
    let mut signal_1_active = false;
    let mut avg_bid_price = 0.0;
    let mut other_bids: Vec<f64> = Vec::new();

    {
        let obs = orderbooks.read().unwrap();
        for token_id in state.order_placed.keys() {
            if let Some(orderbook) = obs.get(token_id) {
                let bid_levels = orderbook.bids.levels();

                // Need at least 2 bids to analyze (skip top bid, check others)
                if bid_levels.len() > 1 {
                    other_bids = bid_levels
                        .iter()
                        .skip(1)
                        .take(4)
                        .map(|(price, _)| *price)
                        .collect();

                    if !other_bids.is_empty() {
                        avg_bid_price = other_bids.iter().sum::<f64>() / other_bids.len() as f64;
                        if avg_bid_price < 0.90 {
                            signal_1_active = true;
                        }
                    }
                }
            }
        }
    }

    // Signal 2: Check oracle price difference
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

    // Both signals must be active to indicate risk
    if signal_1_active && signal_2_active {
        // Log risk detection for each token
        for token_id in state.order_placed.keys() {
            let outcome_name = ctx.get_outcome_name(token_id);
            log_risk_detected(
                ctx,
                token_id,
                &outcome_name,
                avg_bid_price,
                &other_bids,
                bps_diff,
                oracle_price,
            );
        }

        // Cancel all placed orders
        let order_ids = state.get_order_ids();
        if !order_ids.is_empty() {
            cancel_orders(trading, &order_ids, ctx).await;
            state.order_placed.clear();
        }

        return true;
    }

    false
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
            error!(
                "[WS {}] ❌ Failed to cancel orders: {}",
                ctx.market_id, e
            );
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

        // Create shared orderbooks and connect WebSocket
        let orderbooks: SharedOrderbooks = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let client = build_ws_client(&ws_config, Arc::clone(&orderbooks)).await?;
        info!("[WS {}] Connected and subscribed", ctx.market_id);

        // Main tracking loop
        loop {
            // Check shutdown flag (highest priority)
            if !shutdown_flag.load(Ordering::Acquire) {
                info!("[WS {}] Shutdown signal received", ctx.market_id);
                break;
            }

            // Handle WebSocket events
            if let Some(event) = client.try_recv_event() {
                if !handle_client_event(event, &ctx.market_id) {
                    break;
                }
            }

            // Check orderbooks and get tokens needing orders
            let (tokens_to_order, all_empty) =
                check_all_orderbooks(&orderbooks, &mut state, &ctx).await;

            // Exit if market has ended
            if all_empty {
                log_market_ended(&ctx);
                break;
            }

            // Place orders for tokens that exceeded threshold
            for (token_id, outcome_name, elapsed) in tokens_to_order {
                if let Some(order_id) = place_order(&trading, &token_id, &outcome_name, elapsed, &ctx).await {
                    state.order_placed.insert(token_id, order_id);
                }
            }

            // Monitor for risk on placed orders and cancel if detected
            check_risk(&orderbooks, &mut state, &ctx, &oracle_prices, &trading).await;

            // Brief sleep before next iteration
            sleep(StdDuration::from_millis(100)).await;
        }

        // Cleanup: Cancel any remaining open orders before shutdown
        if !state.order_placed.is_empty() {
            info!(
                "[WS {}] Cancelling {} remaining orders before shutdown",
                ctx.market_id,
                state.order_placed.len()
            );
            let order_ids = state.get_order_ids();
            cancel_orders(&trading, &order_ids, &ctx).await;
        }

        // Close WebSocket connection
        info!("[WS {}] Closing connection", ctx.market_id);
        if let Err(e) = client.shutdown().await {
            warn!("[WS {}] Error during shutdown: {}", ctx.market_id, e);
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
