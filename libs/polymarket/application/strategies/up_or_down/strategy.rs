//! Up or Down Strategy implementation.
//!
//! The main strategy struct that implements the Strategy trait and orchestrates
//! market discovery, tracking, and cleanup.

use super::tracker::run_market_tracker;
use super::types::{CryptoAsset, OracleSource, Timeframe, REQUIRED_TAGS};
use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::domain::DbMarket;
use crate::infrastructure::config::UpOrDownConfig;
use crate::infrastructure::{spawn_oracle_trackers, SharedOraclePrices};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

// =============================================================================
// Tracked Market
// =============================================================================

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

impl UpOrDownStrategy {
    /// Create a new Up or Down strategy with the given configuration
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
            .filter(|m| match DateTime::parse_from_rfc3339(&m.end_date) {
                Ok(end_time) => end_time.with_timezone(&Utc) > now,
                Err(_) => {
                    warn!(
                        "Failed to parse end_date for market {}: {}",
                        m.id, m.end_date
                    );
                    false
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

    /// Check markets within delta_t window and spawn WebSocket trackers.
    /// Returns list of markets that need trackers spawned.
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

                // Get token_ids and outcomes for logging
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
            let balance_manager = Arc::clone(&ctx.balance_manager);
            let position_tracker = Some(ctx.position_tracker.clone());
            let order_state = Some(ctx.order_state.clone());

            // Register token pair for this market (enables merge detection)
            if let Some(ref condition_id) = tracked.market.condition_id {
                let token_ids = tracked.market.parse_token_ids().unwrap_or_default();
                if token_ids.len() >= 2 {
                    ctx.position_tracker.write().register_token_pair(
                        &token_ids[0],
                        &token_ids[1],
                        condition_id,
                    );
                    debug!(
                        "Registered token pair for market {}: {} <-> {}",
                        tracked.market.id, token_ids[0], token_ids[1]
                    );
                }
            }

            info!(
                "[Tracker] Spawning WebSocket tracker for market {}",
                market.id
            );

            // Spawn the tracker task
            let handle = tokio::spawn(async move {
                match run_market_tracker(
                    market,
                    shutdown_flag,
                    config,
                    trading,
                    oracle_prices,
                    balance_manager,
                    position_tracker,
                    order_state,
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("[Tracker] Market tracker failed: {}", e);
                    }
                }
            });

            self.tracker_tasks.insert(tracked.market.id.clone(), handle);
        }
    }

    /// Remove completed tracker tasks from the tasks map.
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

        // Position tracker and order state are now provided by StrategyContext
        info!(
            "Using shared order state ({} orders) and position tracker from context",
            ctx.order_state.read().order_count()
        );

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
