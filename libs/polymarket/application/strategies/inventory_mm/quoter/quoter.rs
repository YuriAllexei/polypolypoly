//! Per-market Quoter that runs its own tick loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use tracing::{info, warn, debug, error};

use super::context::{QuoterContext, MarketInfo};
use super::orderbook_ws::{QuoterWsConfig, QuoterWsClient, build_quoter_ws_client, wait_for_snapshot};
use crate::application::strategies::inventory_mm::components::{
    solve, Merger, MergerConfig, InFlightTracker, OpenOrderInfo, ExecutorError,
};
use crate::application::strategies::inventory_mm::types::{
    SolverInput, SolverOutput, SolverConfig, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
};
use crate::infrastructure::{SharedOrderbooks, UserOrderStatus as OrderStatus};

enum TickResult {
    Continue,
    ExecutorDead,
}

pub struct Quoter {
    market: MarketInfo,
    config: SolverConfig,
    tick_interval_ms: u64,
    snapshot_timeout_secs: u64,
    merge_cooldown_secs: u64,
    orderbooks: SharedOrderbooks,
    in_flight_tracker: InFlightTracker,
    merger: Merger,
    merge_pending_until: Option<Instant>,
    ctx: QuoterContext,
}

impl Quoter {
    pub fn new(
        market: MarketInfo,
        config: SolverConfig,
        merger_config: MergerConfig,
        tick_interval_ms: u64,
        snapshot_timeout_secs: u64,
        merge_cooldown_secs: u64,
        ctx: QuoterContext,
    ) -> Self {
        Self {
            market,
            config,
            tick_interval_ms,
            snapshot_timeout_secs,
            merge_cooldown_secs,
            orderbooks: Arc::new(RwLock::new(HashMap::new())),
            in_flight_tracker: InFlightTracker::with_default_ttl(),
            merger: Merger::new(merger_config),
            merge_pending_until: None,
            ctx,
        }
    }

    /// Get the market info.
    pub fn market(&self) -> &MarketInfo {
        &self.market
    }

    /// Get a reference to the orderbooks (for WebSocket updates).
    pub fn orderbooks(&self) -> &SharedOrderbooks {
        &self.orderbooks
    }

    /// Main run loop - call from spawned task.
    /// Runs until shutdown or market expired.
    pub async fn run(mut self) {
        let market_desc = self.market.short_desc();
        info!("[Quoter:{}] Starting", market_desc);

        // 1. Start orderbook WebSocket for (up_token_id, down_token_id)
        let ws_config = QuoterWsConfig::new(
            self.market.market_id.clone(),
            self.market.up_token_id.clone(),
            self.market.down_token_id.clone(),
        );

        let ws_client = match build_quoter_ws_client(&ws_config, Arc::clone(&self.orderbooks)).await {
            Ok(client) => client,
            Err(e) => {
                error!("[Quoter:{}] Failed to connect WebSocket: {}", market_desc, e);
                return;
            }
        };

        info!("[Quoter:{}] WebSocket connected", market_desc);

        // 2. Wait for initial orderbook snapshot
        let snapshot_timeout = Duration::from_secs(self.snapshot_timeout_secs);
        if !wait_for_snapshot(&ws_client, &self.ctx.shutdown_flag, &self.market.market_id, snapshot_timeout).await {
            error!("[Quoter:{}] Failed to receive orderbook snapshot", market_desc);
            self.cleanup(Some(ws_client)).await;
            return;
        }

        info!("[Quoter:{}] Orderbook snapshot received, starting tick loop", market_desc);

        let tick_duration = Duration::from_millis(self.tick_interval_ms);

        // Main tick loop
        while self.ctx.is_running() && !self.market.is_expired() {
            let tick_start = Instant::now();

            // Build input from shared state
            let input = self.extract_input();

            // Run tick
            match self.tick(&input) {
                (Some(output), TickResult::Continue) => {
                    // Output was sent to executor in tick()
                    debug!(
                        "[Quoter:{}] Tick: {} cancels, {} limits, {} takers",
                        market_desc,
                        output.cancellations.len(),
                        output.limit_orders.len(),
                        output.taker_orders.len()
                    );
                }
                (_, TickResult::ExecutorDead) => {
                    error!("[Quoter:{}] Executor channel closed, exiting", market_desc);
                    break;
                }
                (None, TickResult::Continue) => {}
            }

            // Sleep for remaining tick interval
            let elapsed = tick_start.elapsed();
            if elapsed < tick_duration {
                tokio::time::sleep(tick_duration - elapsed).await;
            }
        }

        // Cleanup on exit
        self.cleanup(Some(ws_client)).await;

        info!("[Quoter:{}] Stopped", market_desc);
    }

    /// Extract SolverInput from shared state.
    fn extract_input(&self) -> SolverInput {
        // 1. Extract open orders from OMS
        let (up_orders, down_orders) = {
            let oms = self.ctx.order_state.read();

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

            (
                extract_orders(&self.market.up_token_id),
                extract_orders(&self.market.down_token_id),
            )
        };

        // 2. Extract inventory from position tracker
        let inventory = {
            let tracker = self.ctx.position_tracker.read();
            let up_pos = tracker.get_position(&self.market.up_token_id);
            let down_pos = tracker.get_position(&self.market.down_token_id);

            InventorySnapshot {
                up_size: up_pos.map(|p| p.size).unwrap_or(0.0),
                up_avg_price: up_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
                down_size: down_pos.map(|p| p.size).unwrap_or(0.0),
                down_avg_price: down_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
            }
        };

        // 3. Extract orderbook snapshots
        let (up_orderbook, down_orderbook) = {
            let obs = self.orderbooks.read();

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

            (
                extract_ob(&self.market.up_token_id, &up_orders),
                extract_ob(&self.market.down_token_id, &down_orders),
            )
        };

        SolverInput {
            up_token_id: self.market.up_token_id.clone(),
            down_token_id: self.market.down_token_id.clone(),
            up_orders,
            down_orders,
            inventory,
            up_orderbook,
            down_orderbook,
            config: self.config.clone(),
        }
    }

    fn tick(&mut self, input: &SolverInput) -> (Option<SolverOutput>, TickResult) {
        let open_orders: Vec<OpenOrderInfo> = input.up_orders.bids.iter()
            .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.up_token_id.clone(), o.price))
            .chain(input.down_orders.bids.iter()
                .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.down_token_id.clone(), o.price)))
            .collect();
        self.in_flight_tracker.cleanup_from_orders(&open_orders);

        let mut output = solve(input);
        output.cancellations.retain(|oid| self.in_flight_tracker.should_cancel(oid));
        output.limit_orders.retain(|o| self.in_flight_tracker.should_place(&o.token_id, o.price));

        if output.has_actions() {
            if let Err(e) = self.ctx.executor.execute(output.clone()) {
                if matches!(e, ExecutorError::ChannelClosed) {
                    return (None, TickResult::ExecutorDead);
                }
                warn!("[Quoter:{}] Failed to send to executor: {}", self.market.short_desc(), e);
                for oid in &output.cancellations {
                    self.in_flight_tracker.cancel_failed(oid);
                }
                for order in &output.limit_orders {
                    self.in_flight_tracker.placement_failed(&order.token_id, order.price);
                }
            }
        }

        let decision = self.merger.check_merge(&input.inventory);
        if decision.should_merge {
            let now = Instant::now();
            let merge_allowed = self.merge_pending_until
                .map(|deadline| now >= deadline)
                .unwrap_or(true);

            if merge_allowed {
                info!(
                    "[Quoter:{}] Merge opportunity: {} pairs for ${:.4} profit",
                    self.market.short_desc(), decision.pairs_to_merge, decision.expected_profit
                );
                match self.ctx.executor.merge(
                    self.market.condition_id.clone(),
                    decision.pairs_to_merge,
                ) {
                    Ok(()) => {
                        self.merge_pending_until = Some(now + Duration::from_secs(self.merge_cooldown_secs));
                    }
                    Err(ExecutorError::ChannelClosed) => {
                        return (Some(output), TickResult::ExecutorDead);
                    }
                    Err(e) => {
                        warn!("[Quoter:{}] Merge failed: {}", self.market.short_desc(), e);
                    }
                }
            } else {
                debug!(
                    "[Quoter:{}] Merge skipped, cooldown active: {} pairs",
                    self.market.short_desc(), decision.pairs_to_merge
                );
            }
        }

        (Some(output), TickResult::Continue)
    }

    async fn cleanup(&mut self, ws_client: Option<QuoterWsClient>) {
        let market_desc = self.market.short_desc();
        info!("[Quoter:{}] Cleaning up", market_desc);

        if let Err(e) = self.ctx.executor.cancel_token_orders(self.market.up_token_id.clone()) {
            warn!("[Quoter:{}] Failed to cancel UP orders: {}", market_desc, e);
        }
        if let Err(e) = self.ctx.executor.cancel_token_orders(self.market.down_token_id.clone()) {
            warn!("[Quoter:{}] Failed to cancel DOWN orders: {}", market_desc, e);
        }

        let input = self.extract_input();
        let decision = self.merger.check_merge(&input.inventory);
        if decision.should_merge {
            info!(
                "[Quoter:{}] Final merge: {} pairs for ${:.4} profit",
                market_desc, decision.pairs_to_merge, decision.expected_profit
            );
            if let Err(e) = self.ctx.executor.merge(
                self.market.condition_id.clone(),
                decision.pairs_to_merge,
            ) {
                warn!("[Quoter:{}] Final merge failed: {}", market_desc, e);
            }
        }

        if let Some(client) = ws_client {
            if let Err(e) = client.shutdown().await {
                warn!("[Quoter:{}] Failed to shutdown WebSocket: {}", market_desc, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crossbeam_channel::unbounded;
    use crate::application::strategies::inventory_mm::components::QuoterExecutorHandle;
    use crate::infrastructure::{OrderStateStore, PositionTracker};

    fn make_test_context() -> QuoterContext {
        let (tx, _rx) = unbounded();
        let executor = QuoterExecutorHandle::from_sender(tx);

        QuoterContext::new(
            executor,
            Arc::new(RwLock::new(OrderStateStore::new())),
            Arc::new(RwLock::new(PositionTracker::new())),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }

    fn make_test_market() -> MarketInfo {
        MarketInfo::new(
            "market-123".to_string(),
            "condition-123".to_string(),
            "up-token-123".to_string(),
            "down-token-123".to_string(),
            Utc::now() + chrono::Duration::hours(1),
            "BTC".to_string(),
            "15m".to_string(),
        )
    }

    #[test]
    fn test_quoter_creation() {
        let ctx = make_test_context();
        let market = make_test_market();
        let quoter = Quoter::new(
            market.clone(),
            SolverConfig::default(),
            MergerConfig::default(),
            100,
            30,
            120,
            ctx,
        );

        assert_eq!(quoter.market().market_id, "market-123");
        assert_eq!(quoter.market().symbol, "BTC");
    }
}
