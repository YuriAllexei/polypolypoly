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
    build_ws_client, handle_client_event, spawn_oracle_trackers, MarketTrackerConfig,
    SharedOraclePrices, SharedOrderbooks,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
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
                    tracked.market.id, tracked.market.question, market_url, oracle_source, crypto_asset, timeframe,
                    time_until_end.num_seconds(), tracked.end_time.format("%Y-%m-%d %H:%M:%S UTC"), token_ids, outcomes
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
        _oracle_prices: Option<SharedOraclePrices>,
    ) -> anyhow::Result<()> {
        let market_id = market.id.clone();
        let market_question = market.question.clone();

        // Parse market metadata from description and tags
        let tags = market
            .parse_tags()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let oracle_source = OracleSource::from_description(&market.description);
        let crypto_asset = CryptoAsset::from_tags(&tags);
        let timeframe = Timeframe::from_tags(&tags);

        // Parse token_ids and outcomes from DbMarket
        let token_ids = market.parse_token_ids()?;
        let outcomes = market.parse_outcomes()?;

        // Build WebSocket configuration from DbMarket
        let ws_config = MarketTrackerConfig::new(
            market_id.clone(),
            market_question.clone(),
            market.slug.clone(),
            token_ids.clone(),
            outcomes.clone(),
            &market.end_date,
        )?;

        info!("[WS {}] Connecting to orderbook stream...", market_id);
        info!("[WS {}] Market: {}", market_id, market_question);
        info!("[WS {}] Oracle: {}", market_id, oracle_source);
        info!("[WS {}] Asset: {}", market_id, crypto_asset);
        info!("[WS {}] Timeframe: {}", market_id, timeframe);
        info!(
            "[WS {}] Resolution time: {}",
            market_id, ws_config.resolution_time
        );

        // Create shared orderbooks - handler writes, this loop reads
        let orderbooks: SharedOrderbooks = Arc::new(std::sync::RwLock::new(HashMap::new()));

        // Build and connect WebSocket client with shared orderbooks
        let client = build_ws_client(&ws_config, Arc::clone(&orderbooks)).await?;
        info!("[WS {}] Connected and subscribed", market_id);

        // Timer state for tracking no-asks condition
        let outcome_map = ws_config.build_outcome_map();
        let market_url = market
            .slug
            .as_ref()
            .map(|s| format!("https://polymarket.com/event/{}", s))
            .unwrap_or_else(|| "N/A".to_string());
        let mut no_asks_timers: HashMap<String, Instant> = HashMap::new();
        let mut _bid_triggered: HashSet<String> = HashSet::new();
        let mut threshold_triggered: HashSet<String> = HashSet::new();
        let mut order_placed: HashSet<String> = HashSet::new();

        // Get threshold from config
        let no_ask_threshold_secs = config.no_ask_time_threshold;

        const _MIN_LIQUIDITY: f64 = 100_000.0; // 10k tokens
        const _TARGET_BID_PRICE: f64 = 0.99;

        // Main tracking loop
        loop {
            // Check shutdown flag first (highest priority)
            if !shutdown_flag.load(Ordering::Acquire) {
                info!("[WS {}] Shutdown signal received", market_id);
                break;
            }

            // Handle WebSocket events
            if let Some(event) = client.try_recv_event() {
                if !handle_client_event(event, &market_id) {
                    break;
                }
            }

            // Check orderbook state for all tracked tokens
            // Collect tokens that need orders (to avoid holding lock across await)
            let mut tokens_to_order: Vec<(String, String, f64)> = Vec::new();
            let mut all_orderbooks_empty = true; // Track if all orderbooks are empty (market ended)
            {
                let obs = orderbooks.read().unwrap();
                for token_id in &token_ids {
                    if let Some(orderbook) = obs.get(token_id) {
                        let has_asks = !orderbook.asks.is_empty();
                        let has_bids = !orderbook.bids.is_empty();
                        let outcome_name = outcome_map
                            .get(token_id)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string());

                        // Check if this orderbook has any activity
                        if has_asks || has_bids {
                            all_orderbooks_empty = false;
                        }

                        if has_asks {
                            // Asks exist - reset timer and threshold state if they were running
                            if no_asks_timers.remove(token_id).is_some() {
                                threshold_triggered.remove(token_id);
                                info!(
                                    "⏹️  Timer RESET for {} ({}) - asks appeared in orderbook",
                                    token_id, outcome_name
                                );
                            }
                        } else {
                            // No asks - start timer if not already running
                            if !no_asks_timers.contains_key(token_id) {
                                info!(
                                    "════════════════════════════════════════════════════════════════\n\
                                     🎯 NO ASKS IN ORDERBOOK - STARTING TIMER\n\
                                     ════════════════════════════════════════════════════════════════\n\
                                       Market ID:    {}\n\
                                       Market:       {}\n\
                                       URL:          {}\n\
                                       Oracle:       {}\n\
                                       Asset:        {}\n\
                                       Timeframe:    {}\n\
                                       Outcome:      {}\n\
                                       Token ID:     {}\n\
                                     ════════════════════════════════════════════════════════════════",
                                    market_id, market_question, market_url, oracle_source, crypto_asset, timeframe, outcome_name, token_id
                                );
                                no_asks_timers.insert(token_id.clone(), Instant::now());
                            }

                            // Check if no-asks time threshold exceeded (only log once per token)
                            if !threshold_triggered.contains(token_id)
                                && !order_placed.contains(token_id)
                            {
                                if let Some(timer_start) = no_asks_timers.get(token_id) {
                                    let elapsed = timer_start.elapsed().as_secs_f64();
                                    if elapsed >= no_ask_threshold_secs {
                                        info!(
                                            "════════════════════════════════════════════════════════════════\n\
                                             ⚡ NO-ASK TIME THRESHOLD EXCEEDED\n\
                                             ════════════════════════════════════════════════════════════════\n\
                                               Market ID:      {}\n\
                                               Market:         {}\n\
                                               URL:            {}\n\
                                               Oracle:         {}\n\
                                               Asset:          {}\n\
                                               Timeframe:      {}\n\
                                               Outcome:        {}\n\
                                               Token ID:       {}\n\
                                               Elapsed Time:   {:.3} seconds\n\
                                               Threshold:      {:.3} seconds\n\
                                             ════════════════════════════════════════════════════════════════",
                                            market_id, market_question, market_url, oracle_source, crypto_asset, timeframe, outcome_name, token_id, elapsed, no_ask_threshold_secs
                                        );

                                        // Collect token for ordering (will place order after releasing lock)
                                        tokens_to_order.push((
                                            token_id.clone(),
                                            outcome_name.clone(),
                                            elapsed,
                                        ));
                                        threshold_triggered.insert(token_id.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            } // Lock released here

            // Check if all orderbooks are empty (market has ended)
            if all_orderbooks_empty {
                info!(
                    "════════════════════════════════════════════════════════════════\n\
                     🏁 MARKET ENDED - NO BIDS OR ASKS IN ANY ORDERBOOK\n\
                     ════════════════════════════════════════════════════════════════\n\
                       Market ID:    {}\n\
                       Market:       {}\n\
                       URL:          {}\n\
                     ════════════════════════════════════════════════════════════════",
                    market_id, market_question, market_url
                );
                break;
            }

            // Place orders outside the lock scope
            for (token_id, outcome_name, elapsed) in tokens_to_order {
                info!(
                    "════════════════════════════════════════════════════════════════\n\
                     🚀 PLACING BUY ORDER\n\
                     ════════════════════════════════════════════════════════════════\n\
                       Market ID:      {}\n\
                       Market:         {}\n\
                       URL:            {}\n\
                       Oracle:         {}\n\
                       Asset:          {}\n\
                       Timeframe:      {}\n\
                       Outcome:        {}\n\
                       Token ID:       {}\n\
                       Elapsed Time:   {:.3} seconds\n\
                       Threshold:      {:.3} seconds\n\
                     ════════════════════════════════════════════════════════════════",
                    market_id, market_question, market_url, oracle_source, crypto_asset, timeframe, outcome_name, token_id, elapsed, no_ask_threshold_secs
                );

                match trading.buy(&token_id, 0.99, 18.0).await {
                    Ok(order_id) => {
                        info!(
                            "════════════════════════════════════════════════════════════════\n\
                             ✅ ORDER PLACED SUCCESSFULLY\n\
                             ════════════════════════════════════════════════════════════════\n\
                               Order ID:       {:?}\n\
                               Market ID:      {}\n\
                               Market:         {}\n\
                               URL:            {}\n\
                               Outcome:        {}\n\
                               Timeframe:      {}\n\
                               Token ID:       {}\n\
                             ════════════════════════════════════════════════════════════════",
                            order_id, market_id, market_question, market_url, outcome_name, timeframe, token_id
                        );
                    }
                    Err(e) => {
                        error!(
                            "════════════════════════════════════════════════════════════════\n\
                             ❌ ORDER PLACEMENT FAILED\n\
                             ════════════════════════════════════════════════════════════════\n\
                               Error:          {}\n\
                               Market ID:      {}\n\
                               Market:         {}\n\
                               URL:            {}\n\
                               Outcome:        {}\n\
                               Token ID:       {}\n\
                             ════════════════════════════════════════════════════════════════",
                            e, market_id, market_question, market_url, outcome_name, token_id
                        );
                    }
                }
                order_placed.insert(token_id.clone());
            }

            // Check for false positive risk on all tokens with placed orders
            if !order_placed.is_empty() {
                let obs = orderbooks.read().unwrap();
                for token_id in &order_placed {
                    if let Some(orderbook) = obs.get(token_id) {
                        let outcome_name = outcome_map
                            .get(token_id)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string());

                        // Bids are already sorted descending (highest price first) from OrderbookSide
                        let bid_levels = orderbook.bids.levels();

                        // Skip top bid and take next 4 bids
                        if bid_levels.len() > 1 {
                            let other_bids: Vec<f64> = bid_levels.iter().skip(1).take(4).map(|(price, _)| *price).collect();
                            if !other_bids.is_empty() {
                                let avg_price: f64 = other_bids.iter().sum::<f64>() / other_bids.len() as f64;

                                if avg_price < 0.90 {
                                    warn!(
                                        "════════════════════════════════════════════════════════════════\n\
                                         ⚠️  FALSE POSITIVE RISK DETECTED\n\
                                         ════════════════════════════════════════════════════════════════\n\
                                           Market ID:      {}\n\
                                           Market:         {}\n\
                                           URL:            {}\n\
                                           Outcome:        {}\n\
                                           Token ID:       {}\n\
                                           Avg Bid (excl. top): {:.4}\n\
                                           Other Bids:     {:?}\n\
                                           Threshold:      0.90\n\
                                         ════════════════════════════════════════════════════════════════",
                                        market_id, market_question, market_url, outcome_name, token_id, avg_price, other_bids
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Sleep briefly before next iteration
            sleep(StdDuration::from_millis(100)).await;
        }

        info!("[WS {}] Closing connection", market_id);
        if let Err(e) = client.shutdown().await {
            warn!("[WS {}] Error during shutdown: {}", market_id, e);
        }
        info!("[WS {}] Tracker stopped", market_id);
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
