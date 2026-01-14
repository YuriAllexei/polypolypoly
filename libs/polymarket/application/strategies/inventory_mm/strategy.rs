//! Inventory MM Strategy - multi-market orchestration.
//!
//! Each quoter spawns its own executor thread for order execution,
//! ensuring markets are independent and don't block each other.

use std::collections::{HashMap, HashSet};
use std::time::Duration;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tracing::{info, warn, debug, error};

use super::config::InventoryMMConfig;
use super::quoter::{Quoter, QuoterContext, MarketInfo};
use super::types::{
    SolverInput, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
};
use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult, StrategyError};
use crate::application::strategies::up_or_down::{CryptoAsset, Timeframe};
use crate::application::strategies::up_or_down::services::get_price_to_beat;
use crate::infrastructure::{
    spawn_order_reconciliation_task, spawn_position_reconciliation_task, ReconciliationConfig,
    SharedOrderbooks, SharedOrderState, SharedPositionTracker, UserOrderStatus as OrderStatus,
    spawn_oracle_trackers, SharedOraclePrices,
};

/// Maximum markets to fetch per category from DB
const MAX_MARKETS_PER_CATEGORY: i64 = 5;

/// Main strategy - implements Strategy trait.
/// Manages multiple quoters, one per market.
///
/// Each quoter has its own executor thread for order execution.
/// This ensures markets are independent and don't block each other.
pub struct InventoryMMStrategy {
    config: InventoryMMConfig,

    // Per-quoter task management
    quoter_tasks: HashMap<String, JoinHandle<()>>,  // market_id -> task
    tracked_markets: HashSet<String>,                // avoid duplicate spawns

    // Reconciliation task handles
    reconciliation_handle: Option<JoinHandle<()>>,
    order_reconciliation_handle: Option<JoinHandle<()>>,

    // Oracle prices (ChainLink for 15-min markets)
    oracle_prices: Option<SharedOraclePrices>,
}

impl InventoryMMStrategy {
    /// Create a new strategy instance.
    pub fn new(config: InventoryMMConfig) -> Self {
        Self {
            config,
            quoter_tasks: HashMap::new(),
            tracked_markets: HashSet::new(),
            reconciliation_handle: None,
            order_reconciliation_handle: None,
            oracle_prices: None,
        }
    }

    /// Spawn a quoter for a market.
    fn spawn_quoter(&mut self, market: MarketInfo, ctx: QuoterContext) {
        let market_id = market.market_id.clone();
        let market_desc = market.short_desc();

        info!("[InventoryMM] Spawning quoter for {}", market_desc);

        let solver_config = self.config.solver.clone();
        let merger_config = self.config.merger.clone();
        let taker_config = self.config.taker.clone();
        let tick_interval_ms = self.config.tick_interval_ms;
        let snapshot_timeout_secs = self.config.snapshot_timeout_secs;
        let merge_cooldown_secs = self.config.merge_cooldown_secs;
        let data_logging_config = self.config.data_logging.clone();

        // Register the token pair for this market (enables merge detection)
        ctx.position_tracker.write().register_token_pair(
            &market.up_token_id,
            &market.down_token_id,
            &market.condition_id,
        );
        debug!(
            "[InventoryMM] Registered token pair for market {}: {} <-> {}",
            market.market_id, market.up_token_id, market.down_token_id
        );

        let handle = tokio::spawn(async move {
            let quoter = Quoter::new(
                market,
                solver_config,
                merger_config,
                taker_config,
                tick_interval_ms,
                snapshot_timeout_secs,
                merge_cooldown_secs,
                ctx,
                data_logging_config,
            );
            quoter.run().await;
        });

        self.quoter_tasks.insert(market_id.clone(), handle);
        self.tracked_markets.insert(market_id);
    }

    /// Cleanup finished quoter tasks.
    fn cleanup_finished_quoters(&mut self) {
        let finished: Vec<String> = self.quoter_tasks
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(id, _)| id.clone())
            .collect();

        for market_id in finished {
            if let Some(_handle) = self.quoter_tasks.remove(&market_id) {
                // Task finished (we already checked is_finished)
                // Note: we don't await here since task is already done
                info!("[InventoryMM] Quoter {} finished", market_id);
            }
            self.tracked_markets.remove(&market_id);
        }
    }

    /// Fetch markets from DB using optimized sliding window query.
    async fn fetch_markets(&self, ctx: &StrategyContext) -> StrategyResult<Vec<MarketInfo>> {
        let markets = ctx.database.get_sliding_window_markets(MAX_MARKETS_PER_CATEGORY).await?;

        let mut result = Vec::new();
        // Pre-populate counts with already-tracked markets
        let mut counts: HashMap<(String, String), usize> = HashMap::new();

        for market in markets {
            // Parse tags to get symbol/timeframe for counting
            let tags_str = match &market.tags {
                Some(t) => t.as_str(),
                None => continue,
            };
            let tags: Vec<serde_json::Value> = match serde_json::from_str(tags_str) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let (symbol, timeframe) = match extract_symbol_timeframe(&tags) {
                Some(st) => st,
                None => continue,
            };

            // Skip if already tracked, but COUNT it toward the category limit
            if self.tracked_markets.contains(&market.id) {
                let key = (symbol.to_uppercase(), timeframe.to_uppercase());
                *counts.entry(key).or_insert(0) += 1;
                continue;
            }

            // Parse end_date
            let end_date = match DateTime::parse_from_rfc3339(&market.end_date) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };

            // Parse token_ids
            let token_ids: Vec<String> = match serde_json::from_str(&market.token_ids) {
                Ok(ids) => ids,
                Err(_) => continue,
            };
            if token_ids.len() < 2 {
                continue;
            }

            // Parse outcomes to correctly map UP/DOWN tokens
            // CRITICAL: Polymarket does NOT guarantee token order - must check outcomes!
            let outcomes: Vec<String> = match market.parse_outcomes() {
                Ok(o) => o,
                Err(_) => continue,
            };
            if outcomes.len() < 2 {
                continue;
            }

            // Find which index corresponds to "Up" outcome (case insensitive)
            let up_idx = match outcomes.iter().position(|o| o.eq_ignore_ascii_case("up")) {
                Some(idx) => idx,
                None => {
                    warn!(
                        "[InventoryMM] Market {} has no 'Up' outcome, skipping. outcomes: {:?}",
                        market.id, outcomes
                    );
                    continue;
                }
            };

            // Verify "Down" exists at the other index
            let down_idx = if up_idx == 0 { 1 } else { 0 };
            if !outcomes[down_idx].eq_ignore_ascii_case("down") {
                warn!(
                    "[InventoryMM] Market {} unexpected outcome at idx {}: '{}', expected 'Down'. Skipping.",
                    market.id, down_idx, outcomes[down_idx]
                );
                continue;
            }

            let up_token_id = token_ids[up_idx].clone();
            let down_token_id = token_ids[down_idx].clone();

            info!(
                "[InventoryMM] Token mapping for {}: outcomes={:?}, UP={} (idx {}), DOWN={} (idx {})",
                market.id,
                outcomes,
                &up_token_id[..8.min(up_token_id.len())],
                up_idx,
                &down_token_id[..8.min(down_token_id.len())],
                down_idx
            );

            // Get condition_id (required for merging)
            let condition_id = match &market.condition_id {
                Some(cid) if !cid.is_empty() => cid.clone(),
                _ => continue,
            };

            // Check if this (symbol, timeframe) is in our config
            if !self.config.is_symbol_enabled(&symbol) || !self.config.is_timeframe_enabled(&timeframe) {
                continue;
            }

            // Apply per-category count limit from config
            let key = (symbol.to_uppercase(), timeframe.to_uppercase());
            let count = counts.entry(key.clone()).or_insert(0);
            let max_count = self.config.get_count(&symbol, &timeframe).unwrap_or(3);
            if *count >= max_count {
                debug!(
                    "[InventoryMM] Skipping {} {} ({}): already have {}/{} quoters",
                    symbol, timeframe, market.id, *count, max_count
                );
                continue;
            }
            *count += 1;

            // Fetch price_to_beat (threshold) from Polymarket API
            // This is the opening price used to determine UP/DOWN resolution
            // CRITICAL: Without threshold, oracle adjustment doesn't work - skip market if fetch fails
            let threshold = match self.fetch_threshold(&symbol, &timeframe, &market).await {
                Ok(price) => {
                    info!(
                        "[InventoryMM] Fetched threshold for {} {}: ${}",
                        symbol, timeframe, price
                    );
                    price
                }
                Err(e) => {
                    error!(
                        "[InventoryMM] Failed to fetch threshold for {} {}: {}. SKIPPING MARKET - oracle adjustment requires threshold.",
                        symbol, timeframe, e
                    );
                    continue; // Skip this market - can't run strategy without threshold
                }
            };

            result.push(MarketInfo::new(
                market.id.clone(),
                condition_id,
                up_token_id,
                down_token_id,
                end_date,
                symbol,
                timeframe,
                threshold,
            ));
        }

        Ok(result)
    }

    /// Fetch the price_to_beat (threshold) for a market from Polymarket API.
    /// This is the opening price used to determine UP/DOWN resolution.
    /// Retries up to 3 times with exponential backoff.
    async fn fetch_threshold(
        &self,
        symbol: &str,
        timeframe: &str,
        market: &crate::domain::DbMarket,
    ) -> anyhow::Result<f64> {
        // Parse symbol to CryptoAsset
        let crypto_asset = match symbol.to_uppercase().as_str() {
            "BTC" => CryptoAsset::Bitcoin,
            "ETH" => CryptoAsset::Ethereum,
            "SOL" => CryptoAsset::Solana,
            "XRP" => CryptoAsset::Xrp,
            _ => return Err(anyhow::anyhow!("Unknown crypto asset: {}", symbol)),
        };

        // Parse timeframe to Timeframe enum
        let tf = match timeframe.to_uppercase().as_str() {
            "15M" => Timeframe::FifteenMin,
            "1H" | "1HR" => Timeframe::OneHour,
            "4H" | "4HR" => Timeframe::FourHour,
            "DAILY" | "1D" => Timeframe::Daily,
            _ => return Err(anyhow::anyhow!("Unknown timeframe: {}", timeframe)),
        };

        // Retry up to 5 times with longer delays for new markets
        // New markets may have openPrice=null for ~30+ seconds after start
        let mut last_error = None;
        for attempt in 1..=5 {
            match get_price_to_beat(tf, crypto_asset, market).await {
                Ok(price) => return Ok(price),
                Err(e) => {
                    warn!(
                        "[InventoryMM] Threshold fetch attempt {}/5 failed for {} {}: {}",
                        attempt, symbol, timeframe, e
                    );
                    last_error = Some(e);
                    if attempt < 5 {
                        // Longer delays: 3s, 5s, 7s, 9s (~24s total wait)
                        // This gives time for new markets to record opening price
                        let delay = std::time::Duration::from_secs(1 + attempt as u64 * 2);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error fetching threshold")))
    }
}

#[async_trait]
impl Strategy for InventoryMMStrategy {
    fn name(&self) -> &str {
        "inventory_mm"
    }

    fn description(&self) -> &str {
        "Inventory-balanced market making for Up/Down markets"
    }

    async fn initialize(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("[InventoryMM] Initializing strategy");

        // NOTE: Each quoter now spawns its own executor thread for order execution.
        // This ensures markets are independent and don't block each other.

        // Fetch and hydrate existing positions from REST API
        // Use maker_address (proxy wallet) - that's where positions are held
        info!("[InventoryMM] Fetching initial positions from REST API...");
        match ctx.trading.rest().get_positions(ctx.trading.maker_address()).await {
            Ok(positions) => {
                let mut tracker = ctx.position_tracker.write();
                let mut hydrated = 0;
                for pos in &positions {
                    // Parse size (API returns string)
                    if let Ok(size) = pos.size.parse::<f64>() {
                        if size.abs() > 0.001 {
                            // Only hydrate non-zero positions
                            // Note: REST API doesn't provide avg_price, set to 0
                            tracker.hydrate_position(&pos.asset_id, size, 0.0);
                            debug!("[InventoryMM] Hydrated position: {} = {}", pos.asset_id, size);
                            hydrated += 1;
                        }
                    }
                }
                info!("[InventoryMM] Hydrated {} positions (of {} total)", hydrated, positions.len());
            }
            Err(e) => {
                warn!("[InventoryMM] Failed to fetch initial positions: {}. Positions will build from fills.", e);
            }
        }

        // Spawn position reconciliation task (REST API as authoritative source)
        // Guard against duplicate spawns if initialize() is called multiple times
        if self.reconciliation_handle.is_none() {
            let reconciliation_config = ReconciliationConfig::with_interval(1); // 1 second interval
            self.reconciliation_handle = spawn_position_reconciliation_task(
                ctx.shutdown_flag.clone(),
                ctx.position_tracker.clone(),
                ctx.trading.clone(),
                reconciliation_config,
            );
        } else {
            warn!("[InventoryMM] Reconciliation task already running, skipping spawn");
        }

        // Spawn order reconciliation task (REST API as authoritative source)
        if self.order_reconciliation_handle.is_none() {
            let order_reconciliation_config = ReconciliationConfig::with_interval(1); // 1 second interval
            self.order_reconciliation_handle = spawn_order_reconciliation_task(
                ctx.shutdown_flag.clone(),
                ctx.order_state.clone(),
                ctx.trading.clone(),
                order_reconciliation_config,
            );
        } else {
            warn!("[InventoryMM] Order reconciliation task already running, skipping spawn");
        }

        // Spawn oracle price trackers (ChainLink + Binance)
        if self.oracle_prices.is_none() {
            info!("[InventoryMM] Starting oracle price trackers");
            match spawn_oracle_trackers(ctx.shutdown_flag.clone()).await {
                Ok(oracle_prices) => {
                    self.oracle_prices = Some(oracle_prices);
                    info!("[InventoryMM] Oracle price trackers started successfully");
                }
                Err(e) => {
                    warn!("[InventoryMM] Failed to start oracle trackers: {}. Quoting will use neutral oracle.", e);
                }
            }
        } else {
            warn!("[InventoryMM] Oracle trackers already running, skipping spawn");
        }

        info!("[InventoryMM] Strategy initialized");
        Ok(())
    }

    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("[InventoryMM] Starting strategy");

        // Require oracle prices for 4-layer quoting
        let oracle_prices = self.oracle_prices.clone()
            .ok_or_else(|| StrategyError::Config("Oracle prices not initialized. Call initialize() first.".to_string()))?;

        // Build shared context for quoters
        // NOTE: Each quoter will spawn its own executor thread
        let quoter_ctx = QuoterContext::new(
            ctx.trading.clone(),
            ctx.order_state.clone(),
            ctx.position_tracker.clone(),
            ctx.shutdown_flag.clone(),
            oracle_prices,
        );

        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);

        while ctx.is_running() {
            // 1. Fetch markets from DB
            match self.fetch_markets(ctx).await {
                Ok(markets) => {
                    // 2. Spawn quoters for new markets
                    for market in markets {
                        self.spawn_quoter(market, quoter_ctx.clone());
                    }
                }
                Err(e) => {
                    warn!("[InventoryMM] Failed to fetch markets: {}", e);
                }
            }

            // 3. Cleanup finished quoters
            self.cleanup_finished_quoters();

            // 4. Log status
            debug!(
                "[InventoryMM] Active quoters: {}, tracked markets: {}",
                self.quoter_tasks.len(),
                self.tracked_markets.len()
            );

            // 5. Sleep before next poll (interruptible by shutdown)
            ctx.shutdown.interruptible_sleep(poll_interval).await;
        }

        info!("[InventoryMM] Strategy main loop exited");
        Ok(())
    }

    async fn stop(&mut self) -> StrategyResult<()> {
        info!("[InventoryMM] Stopping strategy");

        // Abort the reconciliation tasks first
        if let Some(handle) = self.reconciliation_handle.take() {
            info!("[InventoryMM] Aborting position reconciliation task");
            handle.abort();
        }
        if let Some(handle) = self.order_reconciliation_handle.take() {
            info!("[InventoryMM] Aborting order reconciliation task");
            handle.abort();
        }

        // Wait for all quoter tasks to finish (they check shutdown flag)
        // Each quoter will shutdown its own executor during cleanup
        for (market_id, handle) in self.quoter_tasks.drain() {
            info!("[InventoryMM] Waiting for quoter {} to finish", market_id);
            if let Err(e) = handle.await {
                warn!("[InventoryMM] Quoter {} panicked: {:?}", market_id, e);
            }
        }
        self.tracked_markets.clear();

        info!("[InventoryMM] Strategy stopped");
        Ok(())
    }
}

/// Extract symbol and timeframe from market tags.
/// Returns (symbol, timeframe) if found.
fn extract_symbol_timeframe(tags: &[serde_json::Value]) -> Option<(String, String)> {
    let mut symbol: Option<String> = None;
    let mut timeframe: Option<String> = None;

    for tag in tags {
        let label = tag.get("label")?.as_str()?;

        // Check for crypto symbols
        match label {
            "Bitcoin" => symbol = Some("BTC".to_string()),
            "Ethereum" => symbol = Some("ETH".to_string()),
            "Solana" => symbol = Some("SOL".to_string()),
            "XRP" => symbol = Some("XRP".to_string()),
            _ => {}
        }

        // Check for timeframes
        match label {
            "15M" | "15m" => timeframe = Some("15M".to_string()),
            "1H" | "1hr" | "1HR" => timeframe = Some("1H".to_string()),
            "4H" | "4hr" | "4HR" => timeframe = Some("4H".to_string()),
            "Daily" | "DAILY" => timeframe = Some("Daily".to_string()),
            _ => {}
        }
    }

    match (symbol, timeframe) {
        (Some(s), Some(t)) => Some((s, t)),
        _ => None,
    }
}

/// Extract SolverInput from shared state.
///
/// Acquires read locks in order: OMS → PositionTracker → Orderbooks (prevent deadlocks).
/// This function is kept for backwards compatibility and testing.
pub fn extract_solver_input(
    config: &InventoryMMConfig,
    up_token_id: &str,
    down_token_id: &str,
    orderbooks: &SharedOrderbooks,
    order_state: &SharedOrderState,
    position_tracker: &SharedPositionTracker,
) -> SolverInput {
    // 1. Extract open orders from OMS
    let (up_orders, down_orders) = {
        let oms = order_state.read();

        let extract_orders = |token_id: &str| -> OrderSnapshot {
            let bids: Vec<OpenOrder> = oms.get_bids(token_id)
                .iter()
                .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
                .map(|o| OpenOrder::new(
                    o.order_id.clone(),
                    o.price,
                    o.original_size,
                    o.original_size - o.size_matched,
                ))
                .collect();

            OrderSnapshot { bids, asks: vec![] }
        };

        (extract_orders(up_token_id), extract_orders(down_token_id))
    };

    // 2. Extract inventory from position tracker
    let inventory = {
        let tracker = position_tracker.read();
        let up_pos = tracker.get_position(up_token_id);
        let down_pos = tracker.get_position(down_token_id);

        InventorySnapshot {
            up_size: up_pos.map(|p| p.size).unwrap_or(0.0),
            up_avg_price: up_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
            down_size: down_pos.map(|p| p.size).unwrap_or(0.0),
            down_avg_price: down_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
        }
    };

    // 3. Extract orderbook snapshots
    let (up_orderbook, down_orderbook) = {
        let obs = orderbooks.read();

        let extract_ob = |token_id: &str, our_orders: &OrderSnapshot| -> OrderbookSnapshot {
            match obs.get(token_id) {
                Some(ob) => {
                    let best_bid = ob.best_bid();
                    let best_ask = ob.best_ask();

                    let best_bid_is_ours = best_bid
                        .map(|(price, _)| our_orders.bids.iter().any(|o| (o.price - price).abs() < 1e-6))
                        .unwrap_or(false);

                    let best_ask_is_ours = best_ask
                        .map(|(price, _)| our_orders.asks.iter().any(|o| (o.price - price).abs() < 1e-6))
                        .unwrap_or(false);

                    OrderbookSnapshot { best_bid, best_ask, best_bid_is_ours, best_ask_is_ours }
                }
                None => OrderbookSnapshot::default(),
            }
        };

        (extract_ob(up_token_id, &up_orders), extract_ob(down_token_id, &down_orders))
    };

    SolverInput {
        up_token_id: up_token_id.to_string(),
        down_token_id: down_token_id.to_string(),
        up_orders,
        down_orders,
        inventory,
        up_orderbook,
        down_orderbook,
        config: config.solver.clone(),
        oracle_distance_pct: 0.0,      // Default neutral for testing
        minutes_to_resolution: 7.5,    // Default mid-market for testing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use parking_lot::RwLock;
    use crate::infrastructure::{OrderStateStore, PositionTracker};

    #[test]
    fn test_extract_solver_input() {
        let config = InventoryMMConfig::default();

        let orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let order_state = Arc::new(RwLock::new(OrderStateStore::new()));
        let position_tracker = Arc::new(RwLock::new(PositionTracker::new()));

        let input = extract_solver_input(
            &config,
            "up_token",
            "down_token",
            &orderbooks,
            &order_state,
            &position_tracker,
        );

        assert_eq!(input.up_token_id, "up_token");
        assert_eq!(input.down_token_id, "down_token");
        assert!(input.up_orders.bids.is_empty());
        assert!(input.down_orders.bids.is_empty());
    }

    #[test]
    fn test_extract_symbol_timeframe() {
        let tags: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"label": "Bitcoin"},
            {"label": "15M"},
            {"label": "Up or Down"}
        ]"#).unwrap();

        let (symbol, timeframe) = extract_symbol_timeframe(&tags).unwrap();
        assert_eq!(symbol, "BTC");
        assert_eq!(timeframe, "15M");
    }

    #[test]
    fn test_extract_symbol_timeframe_missing() {
        let tags: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"label": "Up or Down"}
        ]"#).unwrap();

        assert!(extract_symbol_timeframe(&tags).is_none());
    }

    #[test]
    fn test_strategy_creation() {
        let config = InventoryMMConfig::default();
        let strategy = InventoryMMStrategy::new(config);
        assert_eq!(strategy.name(), "inventory_mm");
    }
}
