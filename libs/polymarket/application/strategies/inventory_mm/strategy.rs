//! Inventory MM Strategy - multi-market orchestration.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tracing::{info, warn, debug};

use super::config::InventoryMMConfig;
use super::components::{Executor, ExecutorHandle};
use super::quoter::{Quoter, QuoterContext, MarketInfo};
use super::types::{
    SolverInput, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
};
use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult, StrategyError};
use crate::infrastructure::{
    SharedOrderbooks, SharedOrderState, SharedPositionTracker, UserOrderStatus as OrderStatus,
};

/// Maximum markets to fetch per category from DB
const MAX_MARKETS_PER_CATEGORY: i64 = 5;

/// Main strategy - implements Strategy trait.
/// Manages multiple quoters, one per market.
pub struct InventoryMMStrategy {
    config: InventoryMMConfig,

    // Owned by strategy (for shutdown)
    executor_handle: Option<ExecutorHandle>,

    // Per-quoter task management
    quoter_tasks: HashMap<String, JoinHandle<()>>,  // market_id -> task
    tracked_markets: HashSet<String>,                // avoid duplicate spawns
}

impl InventoryMMStrategy {
    /// Create a new strategy instance.
    pub fn new(config: InventoryMMConfig) -> Self {
        Self {
            config,
            executor_handle: None,
            quoter_tasks: HashMap::new(),
            tracked_markets: HashSet::new(),
        }
    }

    /// Spawn a quoter for a market.
    fn spawn_quoter(&mut self, market: MarketInfo, ctx: QuoterContext) {
        let market_id = market.market_id.clone();
        let market_desc = market.short_desc();

        info!("[InventoryMM] Spawning quoter for {}", market_desc);

        let solver_config = self.config.solver.clone();
        let merger_config = self.config.merger.clone();
        let tick_interval_ms = self.config.tick_interval_ms;
        let snapshot_timeout_secs = self.config.snapshot_timeout_secs;
        let merge_cooldown_secs = self.config.merge_cooldown_secs;

        let handle = tokio::spawn(async move {
            let quoter = Quoter::new(
                market,
                solver_config,
                merger_config,
                tick_interval_ms,
                snapshot_timeout_secs,
                merge_cooldown_secs,
                ctx,
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
        let mut counts: HashMap<(String, String), usize> = HashMap::new();

        for market in markets {
            // Skip if already tracked
            if self.tracked_markets.contains(&market.id) {
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

            // Get condition_id (required for merging)
            let condition_id = match &market.condition_id {
                Some(cid) if !cid.is_empty() => cid.clone(),
                _ => continue,
            };

            // Extract symbol and timeframe from tags
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

            // Check if this (symbol, timeframe) is in our config
            if !self.config.is_symbol_enabled(&symbol) || !self.config.is_timeframe_enabled(&timeframe) {
                continue;
            }

            // Apply per-category count limit from config
            let key = (symbol.to_uppercase(), timeframe.to_uppercase());
            let count = counts.entry(key.clone()).or_insert(0);
            let max_count = self.config.get_count(&symbol, &timeframe).unwrap_or(3);
            if *count >= max_count {
                continue;
            }
            *count += 1;

            result.push(MarketInfo::new(
                market.id.clone(),
                condition_id,
                token_ids[0].clone(),
                token_ids[1].clone(),
                end_date,
                symbol,
                timeframe,
            ));
        }

        Ok(result)
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

        // Spawn executor with TradingClient
        let executor = Executor::spawn(ctx.trading.clone());
        self.executor_handle = Some(executor);

        info!("[InventoryMM] Strategy initialized");
        Ok(())
    }

    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("[InventoryMM] Starting strategy");

        // Build shared context for quoters
        let executor_handle = self.executor_handle.as_ref()
            .ok_or_else(|| StrategyError::Config("Executor not initialized".to_string()))?;

        let quoter_ctx = QuoterContext::new(
            executor_handle.quoter_handle(),
            ctx.order_state.clone(),
            ctx.position_tracker.clone(),
            ctx.shutdown_flag.clone(),
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

        // 1. Wait for all quoter tasks to finish (they check shutdown flag)
        for (market_id, handle) in self.quoter_tasks.drain() {
            info!("[InventoryMM] Waiting for quoter {} to finish", market_id);
            if let Err(e) = handle.await {
                warn!("[InventoryMM] Quoter {} panicked: {:?}", market_id, e);
            }
        }
        self.tracked_markets.clear();

        // 2. Shutdown executor
        if let Some(executor) = self.executor_handle.take() {
            if let Err(e) = executor.shutdown() {
                warn!("[InventoryMM] Error shutting down executor: {}", e);
            }
        }

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
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
