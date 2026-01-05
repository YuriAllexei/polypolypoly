//! Per-market Quoter that runs its own tick loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn, debug, error};

use super::context::{QuoterContext, MarketInfo};
use super::orderbook_ws::{QuoterWsConfig, QuoterWsClient, build_quoter_ws_client, wait_for_snapshot};
use crate::application::strategies::inventory_mm::components::{
    solve, Merger, MergerConfig, InFlightTracker, OpenOrderInfo, ExecutorError,
    TakerTask, TakerConfig, price_to_key,
};
use crate::application::strategies::inventory_mm::types::{
    SolverInput, SolverOutput, SolverConfig, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
};
use crate::infrastructure::{parse_timestamp_to_i64, SharedOrderbooks, UserOrderStatus as OrderStatus};

enum TickResult {
    Continue,
    ExecutorDead,
}

/// Count distinct price levels from a list of orders.
/// Uses price_to_key (price * 10000 rounded) to group by price level.
fn count_distinct_price_levels(orders: &[OpenOrder]) -> usize {
    use std::collections::HashSet;
    let price_keys: HashSet<i64> = orders
        .iter()
        .map(|o| price_to_key(o.price))
        .collect();
    price_keys.len()
}

/// Get distinct price levels from a list of orders.
fn get_price_levels(orders: &[OpenOrder]) -> std::collections::HashSet<i64> {
    orders
        .iter()
        .map(|o| price_to_key(o.price))
        .collect()
}

pub struct Quoter {
    market: MarketInfo,
    config: SolverConfig,
    taker_config: TakerConfig,
    tick_interval_ms: u64,
    snapshot_timeout_secs: u64,
    merge_cooldown_secs: u64,
    orderbooks: SharedOrderbooks,
    in_flight_tracker: InFlightTracker,
    merger: Merger,
    merge_pending_until: Option<Instant>,
    ctx: QuoterContext,
    /// Last logged delta (to reduce log spam)
    last_logged_delta: Option<f64>,
}

impl Quoter {
    pub fn new(
        market: MarketInfo,
        config: SolverConfig,
        merger_config: MergerConfig,
        taker_config: TakerConfig,
        tick_interval_ms: u64,
        snapshot_timeout_secs: u64,
        merge_cooldown_secs: u64,
        ctx: QuoterContext,
    ) -> Self {
        Self {
            market,
            config,
            taker_config,
            tick_interval_ms,
            snapshot_timeout_secs,
            merge_cooldown_secs,
            orderbooks: Arc::new(RwLock::new(HashMap::new())),
            in_flight_tracker: InFlightTracker::with_default_ttl(),
            merger: Merger::new(merger_config),
            merge_pending_until: None,
            ctx,
            last_logged_delta: None,
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
        info!("[Quoter:{}] Starting with {}ms tick interval", market_desc, self.tick_interval_ms);

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
            self.cleanup(Some(ws_client), None).await;
            return;
        }

        info!("[Quoter:{}] Orderbook snapshot received, starting tick loop", market_desc);

        // 3. Spawn TakerTask for immediate FOK execution
        let taker_handle: Option<JoinHandle<()>> = if self.taker_config.enabled {
            let taker_task = TakerTask::new(
                self.market.clone(),
                self.taker_config.clone(),
                Arc::clone(&self.ctx.trading),
                self.ctx.order_state.clone(),
                self.ctx.position_tracker.clone(),
                Arc::clone(&self.orderbooks),
                Arc::clone(&self.ctx.shutdown_flag),
            );
            info!("[Quoter:{}] Spawning TakerTask", market_desc);
            Some(tokio::spawn(async move {
                taker_task.run().await;
            }))
        } else {
            None
        };

        let tick_duration = Duration::from_millis(self.tick_interval_ms);

        // Main tick loop
        while self.ctx.is_running() && !self.market.is_expired() {
            // Cancel all orders if WebSocket disconnected (stale orderbook data)
            // Don't leave orders at potentially stale prices - cancel defensively
            if !ws_client.is_connected() {
                warn!("[Quoter:{}] WebSocket disconnected, cancelling all orders", market_desc);

                // Cancel all orders on both sides (defensive)
                if let Err(e) = self.ctx.executor.cancel_token_orders(self.market.up_token_id.clone()) {
                    warn!("[Quoter:{}] Failed to cancel UP orders: {}", market_desc, e);
                }
                if let Err(e) = self.ctx.executor.cancel_token_orders(self.market.down_token_id.clone()) {
                    warn!("[Quoter:{}] Failed to cancel DOWN orders: {}", market_desc, e);
                }

                tokio::time::sleep(tick_duration).await;
                continue;
            }

            let tick_start = Instant::now();

            // Build input from shared state
            let input = self.extract_input();

            // Log strategy state only when delta changes significantly (reduces spam)
            let delta = input.inventory.imbalance();
            let total_inv = input.inventory.up_size + input.inventory.down_size;
            let should_log = total_inv > 0.0 && self.last_logged_delta
                .map(|last| (delta - last).abs() >= 0.05)
                .unwrap_or(true);
            if should_log {
                info!(
                    "[Quoter:{}] delta={:.2}, inv=(UP:{:.1}@${:.2}, DOWN:{:.1}@${:.2})",
                    market_desc,
                    delta,
                    input.inventory.up_size,
                    input.inventory.up_avg_price,
                    input.inventory.down_size,
                    input.inventory.down_avg_price,
                );
                self.last_logged_delta = Some(delta);
            }

            // Run tick
            match self.tick(&input) {
                (Some(output), TickResult::Continue) => {
                    // Output was sent to executor in tick()
                    debug!(
                        "[Quoter:{}] Tick: {} cancels, {} limits",
                        market_desc,
                        output.cancellations.len(),
                        output.limit_orders.len()
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
        self.cleanup(Some(ws_client), taker_handle).await;

        info!("[Quoter:{}] Stopped", market_desc);
    }

    /// Extract SolverInput from shared state.
    ///
    /// NOTE: This reads from OMS, position tracker, and orderbooks with separate locks.
    /// There is a potential race condition where state could change between reads.
    /// Under normal operation, this is acceptable as the reads happen in quick succession
    /// (typically < 1ms total). For high-frequency scenarios, consider implementing
    /// a versioned snapshot system.
    fn extract_input(&self) -> SolverInput {
        // 1. Extract open orders from OMS
        let (up_orders, down_orders) = {
            let oms = self.ctx.order_state.read();

            let extract_orders = |token_id: &str| -> OrderSnapshot {
                let bids: Vec<OpenOrder> = oms.get_bids(token_id)
                    .iter()
                    .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
                    .map(|o| OpenOrder::with_created_at(
                        o.order_id.clone(),
                        o.price,
                        o.original_size,
                        o.original_size - o.size_matched,
                        parse_timestamp_to_i64(&o.created_at),
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
            let market_desc = self.market.short_desc();

            // Debug: Log what keys exist in the orderbooks map
            if obs.len() != 2 {
                warn!(
                    "[Quoter:{}] Orderbooks map has {} entries (expected 2). Keys: {:?}",
                    market_desc,
                    obs.len(),
                    obs.keys().map(|k| format!("{}...", &k[..16.min(k.len())])).collect::<Vec<_>>()
                );
            }

            let extract_ob = |token_id: &str, our_orders: &OrderSnapshot, side: &str| -> OrderbookSnapshot {
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

                        // Log if no ask data - this prevents quote generation!
                        if best_ask.is_none() {
                            warn!(
                                "[Quoter:{}] {} orderbook has NO ASK data (bid={:?}) - cannot generate quotes!",
                                market_desc, side, best_bid
                            );
                        }

                        OrderbookSnapshot { best_bid, best_ask, best_bid_is_ours, best_ask_is_ours }
                    }
                    None => {
                        // CRITICAL: Orderbook not found - this will prevent all quotes for this side!
                        warn!(
                            "[Quoter:{}] {} orderbook NOT FOUND (looking for {}..., have {:?}) - cannot generate quotes!",
                            market_desc,
                            side,
                            &token_id[..16.min(token_id.len())],
                            obs.keys().map(|k| format!("{}...", &k[..16.min(k.len())])).collect::<Vec<_>>()
                        );
                        OrderbookSnapshot::default()
                    }
                }
            };

            (
                extract_ob(&self.market.up_token_id, &up_orders, "UP"),
                extract_ob(&self.market.down_token_id, &down_orders, "DOWN"),
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
        // NUCLEAR CANCEL: If orders are severely out of sync, cancel ALL for the token.
        // This handles cases where individual cancellations aren't keeping up.
        // Note: We cap PRICE LEVELS to num_levels, but allow multiple orders per level.
        // Nuclear threshold is still based on total orders as a safety fallback.
        let total_orders = input.up_orders.bids.len() + input.down_orders.bids.len();
        let max_levels_total = self.config.num_levels * 2;  // 3 UP levels + 3 DOWN levels = 6
        let nuclear_threshold = max_levels_total * 3;  // 18 orders = something is very wrong

        if total_orders >= nuclear_threshold {
            error!(
                "[Quoter:{}] NUCLEAR CANCEL: {} orders detected (threshold {}), cancelling ALL",
                self.market.short_desc(), total_orders, nuclear_threshold
            );

            // Cancel all orders for both tokens
            let mut all_cancellations: Vec<String> = Vec::with_capacity(total_orders);
            for order in &input.up_orders.bids {
                all_cancellations.push(order.order_id.clone());
                self.in_flight_tracker.mark_cancel_pending(&order.order_id);
            }
            for order in &input.down_orders.bids {
                all_cancellations.push(order.order_id.clone());
                self.in_flight_tracker.mark_cancel_pending(&order.order_id);
            }

            // Return early with just cancellations, no new placements
            let output = SolverOutput {
                limit_orders: Vec::new(),
                cancellations: all_cancellations,
            };
            return (Some(output), TickResult::Continue);
        }

        let open_orders: Vec<OpenOrderInfo> = input.up_orders.bids.iter()
            .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.up_token_id.clone(), o.price))
            .chain(input.down_orders.bids.iter()
                .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.down_token_id.clone(), o.price)))
            .collect();
        self.in_flight_tracker.cleanup_from_orders(&open_orders);

        // Count open orders + pending placements per side for capacity check
        // This prevents order accumulation from race conditions where placements
        // are sent before OMS updates with confirmations
        let up_open = input.up_orders.bids.len();
        let down_open = input.down_orders.bids.len();
        let up_pending = self.in_flight_tracker.pending_placements_for_token(&input.up_token_id);
        let down_pending = self.in_flight_tracker.pending_placements_for_token(&input.down_token_id);

        // Count pending cancellations for orders we currently see as open
        // These orders are being cancelled, so don't count them toward capacity
        let up_pending_cancels = input.up_orders.bids.iter()
            .filter(|o| self.in_flight_tracker.is_cancel_pending(&o.order_id))
            .count();
        let down_pending_cancels = input.down_orders.bids.iter()
            .filter(|o| self.in_flight_tracker.is_cancel_pending(&o.order_id))
            .count();

        // Count distinct PRICE LEVELS (not total orders) for capacity checks
        // This allows multiple orders at the same price for FIFO preservation
        let up_price_levels = count_distinct_price_levels(&input.up_orders.bids);
        let down_price_levels = count_distinct_price_levels(&input.down_orders.bids);
        let max_price_levels = self.config.num_levels;

        let mut output = solve(input);

        // FIX: Don't filter cancellations - they are IDEMPOTENT and safe to retry.
        // The exchange will just return "order already cancelled" if we send twice.
        // Blocking cancels causes massive order accumulation when OMS confirmation is slow.
        // We still track them so is_cancel_pending() returns true for capacity calculations.
        //
        // CRITICAL FIX: Also clear pending placements for cancelled orders' price levels.
        // Without this, cancelled orders still count toward capacity until TTL expires,
        // blocking new placements at different (better) prices.
        for oid in &output.cancellations {
            // Mark as pending for tracking - this makes is_cancel_pending() return true
            // which reduces effective_open count and prevents repeated "excess orders" warnings
            self.in_flight_tracker.mark_cancel_pending(oid);

            // Clear pending placement for this order's price level
            // Look up the order in UP orders first, then DOWN orders
            if let Some(order) = input.up_orders.bids.iter().find(|o| &o.order_id == oid) {
                self.in_flight_tracker.placement_cancelled(&input.up_token_id, order.price);
            } else if let Some(order) = input.down_orders.bids.iter().find(|o| &o.order_id == oid) {
                self.in_flight_tracker.placement_cancelled(&input.down_token_id, order.price);
            }
        }

        // SAFETY CHECK: If we have excess PRICE LEVELS, cancel orders at extra levels.
        // This handles accumulation from previous bugs or race conditions.
        // We keep the `num_levels` price levels closest to best_ask (highest prices).
        let up_excess_levels = up_price_levels.saturating_sub(max_price_levels);
        let down_excess_levels = down_price_levels.saturating_sub(max_price_levels);

        if up_excess_levels > 0 {
            warn!(
                "[Quoter:{}] UP has {} excess price levels ({} levels, max {}), cancelling orders at lowest levels",
                self.market.short_desc(), up_excess_levels, up_price_levels, max_price_levels
            );
            // Get price levels, sort descending (highest first = closest to best_ask)
            let mut levels: Vec<i64> = get_price_levels(&input.up_orders.bids).into_iter().collect();
            levels.sort_by(|a, b| b.cmp(a));  // Descending
            // Keep top `max_price_levels`, cancel orders at the rest
            let levels_to_cancel: std::collections::HashSet<i64> = levels.into_iter().skip(max_price_levels).collect();
            for order in &input.up_orders.bids {
                let price_key = price_to_key(order.price);
                if levels_to_cancel.contains(&price_key) && !output.cancellations.contains(&order.order_id) {
                    output.cancellations.push(order.order_id.clone());
                    self.in_flight_tracker.mark_cancel_pending(&order.order_id);
                    self.in_flight_tracker.placement_cancelled(&input.up_token_id, order.price);
                }
            }
        }

        if down_excess_levels > 0 {
            warn!(
                "[Quoter:{}] DOWN has {} excess price levels ({} levels, max {}), cancelling orders at lowest levels",
                self.market.short_desc(), down_excess_levels, down_price_levels, max_price_levels
            );
            // Get price levels, sort descending (highest first = closest to best_ask)
            let mut levels: Vec<i64> = get_price_levels(&input.down_orders.bids).into_iter().collect();
            levels.sort_by(|a, b| b.cmp(a));  // Descending
            // Keep top `max_price_levels`, cancel orders at the rest
            let levels_to_cancel: std::collections::HashSet<i64> = levels.into_iter().skip(max_price_levels).collect();
            for order in &input.down_orders.bids {
                let price_key = price_to_key(order.price);
                if levels_to_cancel.contains(&price_key) && !output.cancellations.contains(&order.order_id) {
                    output.cancellations.push(order.order_id.clone());
                    self.in_flight_tracker.mark_cancel_pending(&order.order_id);
                    self.in_flight_tracker.placement_cancelled(&input.down_token_id, order.price);
                }
            }
        }

        // Filter placements: Block NEW price levels if at capacity.
        // CRITICAL FIX: Combine OMS levels with PENDING levels from InFlightTracker.
        // Previous bug: Only counted OMS levels, so when OMS was slow to update,
        // we'd place orders at new levels thinking we had capacity. This caused
        // 13+ price levels instead of max 3.
        let up_oms_levels = get_price_levels(&input.up_orders.bids);
        let down_oms_levels = get_price_levels(&input.down_orders.bids);
        let up_pending_levels = self.in_flight_tracker.pending_price_levels_for_token(&input.up_token_id);
        let down_pending_levels = self.in_flight_tracker.pending_price_levels_for_token(&input.down_token_id);

        // Combined levels = OMS + pending (union)
        let up_all_levels: std::collections::HashSet<i64> = up_oms_levels.union(&up_pending_levels).copied().collect();
        let down_all_levels: std::collections::HashSet<i64> = down_oms_levels.union(&down_pending_levels).copied().collect();

        let up_total_levels = up_all_levels.len();
        let down_total_levels = down_all_levels.len();

        // Count orders before filtering
        let up_orders_before = output.limit_orders.iter()
            .filter(|o| o.token_id == input.up_token_id).count();
        let down_orders_before = output.limit_orders.iter()
            .filter(|o| o.token_id == input.down_token_id).count();
        let down_ob_ask = input.down_orderbook.best_ask_price();
        let down_ob_bid = input.down_orderbook.best_bid_price();
        let delta_snapshot = input.inventory.imbalance();

        // CRITICAL SAFETY: Check if either side has ZERO inventory
        // If inventory is zero on a side, we MUST allow placements regardless of OMS state
        // because the OMS levels are likely ghost/stale orders
        let up_inventory_zero = input.inventory.up_size.abs() < 1.0;
        let down_inventory_zero = input.inventory.down_size.abs() < 1.0;

        output.limit_orders.retain(|o| {
            let price_key = price_to_key(o.price);
            let is_up_token = o.token_id == input.up_token_id;

            let (all_levels, total_level_count) = if is_up_token {
                (&up_all_levels, up_total_levels)
            } else {
                (&down_all_levels, down_total_levels)
            };

            // CRITICAL SAFETY BYPASS: If inventory on this side is ZERO, allow ALL placements
            // This prevents the disaster where OMS shows stale orders but actual inventory is 0
            let inventory_zero_bypass = if is_up_token { up_inventory_zero } else { down_inventory_zero };
            if inventory_zero_bypass {
                // Always allow placements when we have ZERO inventory on this side
                return self.in_flight_tracker.should_place(&o.token_id, o.price);
            }

            // If price level already exists (OMS or pending), allow for FIFO preservation
            let is_existing_level = all_levels.contains(&price_key);

            // If it's a NEW price level, only allow if under capacity
            if !is_existing_level && total_level_count >= max_price_levels {
                warn!(
                    "[Quoter] BLOCKED placement at {:.2} for {} - would exceed price level capacity ({}/{}) [OMS:{}, pending:{}]",
                    o.price,
                    &o.token_id[..8.min(o.token_id.len())],
                    total_level_count,
                    max_price_levels,
                    if is_up_token { up_oms_levels.len() } else { down_oms_levels.len() },
                    if is_up_token { up_pending_levels.len() } else { down_pending_levels.len() }
                );
                return false;
            }

            self.in_flight_tracker.should_place(&o.token_id, o.price)
        });

        // Log if DOWN orders were filtered out
        let up_orders_after = output.limit_orders.iter()
            .filter(|o| o.token_id == input.up_token_id).count();
        let down_orders_after = output.limit_orders.iter()
            .filter(|o| o.token_id == input.down_token_id).count();

        if down_orders_before != down_orders_after || up_orders_before != up_orders_after {
            info!(
                "[Quoter:{}] Filtered: UP {}->{}, DOWN {}->{}",
                self.market.short_desc(),
                up_orders_before, up_orders_after,
                down_orders_before, down_orders_after
            );
        }

        // Log if solver generated 0 DOWN orders
        if down_orders_before == 0 && delta_snapshot > 0.1 {
            warn!(
                "[Quoter:{}] Solver generated 0 DOWN orders with delta={:.2}! DOWN_ob: ask={:?}, bid={:?}",
                self.market.short_desc(),
                delta_snapshot,
                down_ob_ask,
                down_ob_bid
            );
        }

        // CRITICAL FIX: Check merge BEFORE sending orders to executor.
        // Previous bug: Merge was checked AFTER orders sent, causing race condition
        // where merge quantities could become stale during order execution.
        let decision = self.merger.check_merge(&input.inventory);
        if decision.should_merge {
            let now = Instant::now();
            let merge_allowed = self.merge_pending_until
                .map(|deadline| now >= deadline)
                .unwrap_or(true);

            if merge_allowed {
                info!(
                    "[Quoter:{}] Merge opportunity: {} pairs for ${:.4} profit - deferring orders",
                    self.market.short_desc(), decision.pairs_to_merge, decision.expected_profit
                );

                // When merge is possible, only send CANCELLATIONS (no new placements).
                // This prevents race between new orders and merge execution.
                let cancel_only_output = SolverOutput {
                    cancellations: output.cancellations.clone(),
                    limit_orders: Vec::new(),
                };

                if cancel_only_output.has_actions() {
                    if let Err(e) = self.ctx.executor.execute(cancel_only_output.clone()) {
                        if matches!(e, ExecutorError::ChannelClosed) {
                            return (None, TickResult::ExecutorDead);
                        }
                    }
                }

                // Execute the merge
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

                // Return cancel-only output (placements deferred to next tick after merge settles)
                return (Some(cancel_only_output), TickResult::Continue);
            } else {
                debug!(
                    "[Quoter:{}] Merge skipped, cooldown active: {} pairs",
                    self.market.short_desc(), decision.pairs_to_merge
                );
            }
        }

        // Normal path: no merge opportunity, send full output
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

        (Some(output), TickResult::Continue)
    }

    async fn cleanup(
        &mut self,
        ws_client: Option<QuoterWsClient>,
        taker_handle: Option<JoinHandle<()>>,
    ) {
        let market_desc = self.market.short_desc();
        info!("[Quoter:{}] Cleaning up", market_desc);

        // Abort TakerTask if running
        if let Some(handle) = taker_handle {
            handle.abort();
            info!("[Quoter:{}] Aborted TakerTask", market_desc);
        }

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
    // Note: Quoter tests are disabled because QuoterContext now requires Arc<TradingClient>
    // which needs real credentials to create. Run as integration tests instead.
    //
    // TODO: Add integration tests with mock TradingClient or refactor to allow testing
    // without real credentials.
}
