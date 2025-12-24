//! Market Merger Strategy implementation.
//!
//! The main strategy struct that implements the Strategy trait and orchestrates
//! market discovery, accumulator spawning, and cleanup.

use super::config::MarketMergerConfig;
use super::tracker::{run_accumulator, AccumulatorContext};
use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::application::strategies::up_or_down::{CryptoAsset, Timeframe};
use crate::domain::DbMarket;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Required tags for market filtering
const REQUIRED_TAGS: &[&str] = &["Up or Down", "Crypto Prices", "Recurring"];

// =============================================================================
// Tracked Market
// =============================================================================

/// A market being tracked with its parsed metadata
#[derive(Debug, Clone)]
struct TrackedMarket {
    /// The database market record
    market: DbMarket,
    /// Parsed end time for quick access
    end_time: DateTime<Utc>,
    /// Crypto asset being tracked
    crypto_asset: CryptoAsset,
    /// Timeframe of the market
    timeframe: Timeframe,
    /// Whether we've already spawned an accumulator for this market
    accumulator_spawned: bool,
}

// =============================================================================
// Strategy Implementation
// =============================================================================

/// Market Merger strategy implementation
pub struct MarketMergerStrategy {
    config: MarketMergerConfig,
    /// Set of market IDs we've already seen (to avoid duplicates)
    tracked_market_ids: HashSet<String>,
    /// Markets we're actively monitoring
    active_markets: Vec<TrackedMarket>,
    /// Spawned accumulator tasks (market_id -> JoinHandle)
    accumulator_tasks: HashMap<String, JoinHandle<()>>,
}

impl MarketMergerStrategy {
    /// Create a new Market Merger strategy with the given configuration
    pub fn new(config: MarketMergerConfig) -> Self {
        Self {
            config,
            tracked_market_ids: HashSet::new(),
            active_markets: Vec::new(),
            accumulator_tasks: HashMap::new(),
        }
    }

    /// Fetch markets matching the required tags
    async fn fetch_matching_markets(&self, ctx: &StrategyContext) -> StrategyResult<Vec<DbMarket>> {
        let markets = ctx.database.get_markets_by_tags(REQUIRED_TAGS).await?;
        Ok(markets)
    }

    /// Filter markets that are eligible for accumulation
    fn filter_eligible_markets(&self, markets: Vec<DbMarket>) -> Vec<DbMarket> {
        let now = Utc::now();

        markets
            .into_iter()
            .filter(|m| {
                // Must have valid end time in the future
                let end_time = match DateTime::parse_from_rfc3339(&m.end_date) {
                    Ok(dt) => dt.with_timezone(&Utc),
                    Err(_) => {
                        warn!("Failed to parse end_date for market {}: {}", m.id, m.end_date);
                        return false;
                    }
                };

                // Market must end in the future
                if end_time <= now {
                    return false;
                }

                // Parse tags to get asset and timeframe
                let tags = m.parse_tags().unwrap_or_default();
                let crypto_asset = CryptoAsset::from_tags(&tags);
                let timeframe = Timeframe::from_tags(&tags);

                // Check if asset is in our configured list
                if !self.config.is_asset_enabled(&crypto_asset.to_string()) {
                    debug!(
                        "Skipping market {} - asset {} not enabled",
                        m.id, crypto_asset
                    );
                    return false;
                }

                // Check if timeframe is in our configured list
                if !self.config.is_timeframe_enabled(&timeframe.to_string()) {
                    debug!(
                        "Skipping market {} - timeframe {} not enabled",
                        m.id, timeframe
                    );
                    return false;
                }

                // Must have at least some time left to accumulate
                // Skip markets ending in less than 5 minutes
                let time_remaining = end_time.signed_duration_since(now);
                if time_remaining < Duration::minutes(5) {
                    debug!(
                        "Skipping market {} - only {} seconds remaining",
                        m.id,
                        time_remaining.num_seconds()
                    );
                    return false;
                }

                true
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

                    // Parse metadata
                    let tags = market.parse_tags().unwrap_or_default();
                    let crypto_asset = CryptoAsset::from_tags(&tags);
                    let timeframe = Timeframe::from_tags(&tags);

                    let tracked = TrackedMarket {
                        end_time: end_time.with_timezone(&Utc),
                        market,
                        crypto_asset,
                        timeframe,
                        accumulator_spawned: false,
                    };

                    info!(
                        "Added market to tracking: {} ({} {}) - ends at {}",
                        tracked.market.id,
                        tracked.crypto_asset,
                        tracked.timeframe,
                        tracked.market.end_date
                    );

                    self.active_markets.push(tracked);
                    added += 1;
                }
            }
        }

        added
    }

    /// Check markets and spawn accumulators for those that need it
    fn check_markets_for_accumulation(&mut self) -> Vec<TrackedMarket> {
        let mut markets_to_spawn = Vec::new();

        for tracked in &mut self.active_markets {
            // Skip if we've already spawned an accumulator for this market
            if tracked.accumulator_spawned {
                continue;
            }

            // For market merger, we want to start accumulating right away
            // (not waiting for end time like sniping)
            info!(
                "════════════════════════════════════════════════════════════════\n\
                 🎯 STARTING ACCUMULATION FOR MARKET!\n\
                 ════════════════════════════════════════════════════════════════\n\
                   Market ID:   {}\n\
                   Question:    {}\n\
                   Asset:       {}\n\
                   Timeframe:   {}\n\
                   End Time:    {}\n\
                 ════════════════════════════════════════════════════════════════",
                tracked.market.id,
                tracked.market.question,
                tracked.crypto_asset,
                tracked.timeframe,
                tracked.end_time.format("%Y-%m-%d %H:%M:%S UTC")
            );

            tracked.accumulator_spawned = true;
            markets_to_spawn.push(tracked.clone());
        }

        markets_to_spawn
    }

    /// Spawn accumulators for the given markets
    async fn spawn_accumulators(&mut self, markets: Vec<TrackedMarket>, ctx: &StrategyContext) {
        for tracked in markets {
            let market = tracked.market.clone();
            let shutdown_flag = Arc::clone(&ctx.shutdown_flag);
            let config = self.config.clone();
            let trading = Arc::clone(&ctx.trading);
            let balance_manager = Arc::clone(&ctx.balance_manager);
            let order_state = ctx.order_state.clone();

            info!(
                "[Accumulator] Spawning for market {} ({} {})",
                market.id, tracked.crypto_asset, tracked.timeframe
            );

            // Create accumulator context - share the same balance_manager
            let acc_ctx = AccumulatorContext {
                shutdown_flag,
                trading,
                balance_manager,
                order_state,
            };

            // Spawn the accumulator task
            let handle = tokio::spawn(async move {
                match run_accumulator(market, config, acc_ctx).await {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("[Accumulator] Failed: {}", e);
                    }
                }
            });

            self.accumulator_tasks.insert(tracked.market.id.clone(), handle);
        }
    }

    /// Remove completed accumulator tasks
    fn cleanup_accumulator_tasks(&mut self) {
        self.accumulator_tasks.retain(|market_id, handle| {
            if handle.is_finished() {
                debug!(market_id = %market_id, "Cleaned up completed accumulator task");
                false
            } else {
                true
            }
        });
    }

    /// Remove markets whose end time has passed
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
impl Strategy for MarketMergerStrategy {
    fn name(&self) -> &str {
        "market_merger"
    }

    fn description(&self) -> &str {
        "Accumulates balanced Up/Down positions for merge arbitrage"
    }

    async fn initialize(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!(
            assets = ?self.config.assets,
            timeframes = ?self.config.timeframes,
            poll_interval_secs = self.config.poll_interval_secs,
            "Initializing Market Merger strategy"
        );

        // Initial market fetch
        let markets = self.fetch_matching_markets(ctx).await?;
        let eligible = self.filter_eligible_markets(markets.clone());
        let added = self.add_new_markets(eligible.clone());

        info!(
            total_matching = markets.len(),
            eligible_count = eligible.len(),
            added_to_tracking = added,
            "Initial market discovery completed"
        );

        Ok(())
    }

    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("Starting Market Merger strategy main loop");

        let poll_interval = StdDuration::from_secs_f64(self.config.poll_interval_secs);

        while ctx.is_running() {
            // 1. Fetch new markets from database
            match self.fetch_matching_markets(ctx).await {
                Ok(markets) => {
                    let eligible = self.filter_eligible_markets(markets);
                    let new_count = self.add_new_markets(eligible);

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

            // 2. Check which markets need accumulators spawned
            let markets_to_spawn = self.check_markets_for_accumulation();
            if !markets_to_spawn.is_empty() {
                info!(
                    "Spawning accumulators for {} markets",
                    markets_to_spawn.len()
                );
                self.spawn_accumulators(markets_to_spawn, ctx).await;
            }

            // 3. Cleanup completed accumulator tasks
            self.cleanup_accumulator_tasks();

            // 4. Cleanup ended markets
            self.cleanup_ended_markets();

            // 5. Log status periodically
            if !self.accumulator_tasks.is_empty() {
                debug!(
                    "Active accumulators: {}, Active markets: {}",
                    self.accumulator_tasks.len(),
                    self.active_markets.len()
                );
            }

            // 6. Wait for next iteration (interruptible by shutdown)
            ctx.shutdown.interruptible_sleep(poll_interval).await;
        }

        info!("Market Merger strategy loop ended (shutdown requested)");
        Ok(())
    }

    async fn stop(&mut self) -> StrategyResult<()> {
        info!(
            total_tracked = self.tracked_market_ids.len(),
            active_markets = self.active_markets.len(),
            active_accumulators = self.accumulator_tasks.len(),
            "Stopping Market Merger strategy"
        );

        // Wait for all accumulator tasks to complete (they will stop due to shutdown flag)
        if !self.accumulator_tasks.is_empty() {
            info!(
                count = self.accumulator_tasks.len(),
                "Waiting for accumulators to shut down"
            );

            for (market_id, handle) in self.accumulator_tasks.drain() {
                match handle.await {
                    Ok(()) => debug!(market_id = %market_id, "Accumulator task completed"),
                    Err(e) => {
                        warn!(market_id = %market_id, error = %e, "Accumulator task failed to join")
                    }
                }
            }

            info!("All accumulators shut down");
        }

        Ok(())
    }
}
