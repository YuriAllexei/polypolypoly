//! TakerTask - separate process for immediate FOK order execution.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, debug, error};

/// Price comparison epsilon for floating point tolerance (1 basis point)
const PRICE_EPSILON: f64 = 1e-4;

use super::config::TakerConfig;
use crate::application::strategies::inventory_mm::quoter::context::MarketInfo;
use crate::infrastructure::{
    SharedOrderbooks, SharedOrderState, SharedPositionTracker,
    UserOrderStatus as OrderStatus,
};
use crate::infrastructure::client::clob::TradingClient;

/// TakerTask runs independently from the Quoter tick loop.
/// It monitors imbalance and executes FOK orders immediately when opportunities arise.
pub struct TakerTask {
    market: MarketInfo,
    config: TakerConfig,
    trading: Arc<TradingClient>,
    order_state: SharedOrderState,
    position_tracker: SharedPositionTracker,
    orderbooks: SharedOrderbooks,
    shutdown_flag: Arc<AtomicBool>,
    /// Tracks whether a FOK order is currently pending to prevent duplicate orders
    fok_pending: Arc<AtomicBool>,
}

impl TakerTask {
    /// Create a new TakerTask.
    pub fn new(
        market: MarketInfo,
        config: TakerConfig,
        trading: Arc<TradingClient>,
        order_state: SharedOrderState,
        position_tracker: SharedPositionTracker,
        orderbooks: SharedOrderbooks,
        shutdown_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            market,
            config,
            trading,
            order_state,
            position_tracker,
            orderbooks,
            shutdown_flag,
            fok_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Main run loop - call from spawned task.
    pub async fn run(self) {
        if !self.config.enabled {
            info!("[Taker:{}] Disabled, exiting", self.market.short_desc());
            return;
        }

        let market_desc = self.market.short_desc();
        info!("[Taker:{}] Starting", market_desc);

        let tick_duration = Duration::from_millis(self.config.tick_interval_ms);

        while self.shutdown_flag.load(Ordering::Acquire) {
            // Check for taker opportunity
            self.check_and_execute().await;

            // Sleep before next check
            tokio::time::sleep(tick_duration).await;
        }

        info!("[Taker:{}] Stopped", market_desc);
    }

    /// Check for taker opportunity and execute if found.
    async fn check_and_execute(&self) {
        if self.fok_pending.load(Ordering::Acquire) {
            return;
        }

        let (up_size, up_avg, down_size, down_avg) = {
            let tracker = self.position_tracker.read();
            let up = tracker.get_position(&self.market.up_token_id);
            let down = tracker.get_position(&self.market.down_token_id);
            (
                up.map(|p| p.size).unwrap_or(0.0),
                up.map(|p| p.avg_entry_price).unwrap_or(0.0),
                down.map(|p| p.size).unwrap_or(0.0),
                down.map(|p| p.avg_entry_price).unwrap_or(0.0),
            )
        };

        let total = up_size + down_size;
        if total < 1e-9 {
            return;
        }

        let delta = (up_size - down_size) / total;
        if delta.abs() < self.config.min_delta_threshold {
            return;
        }

        // Determine overweight/underweight sides
        let (overweight_token, underweight_token) = if delta > 0.0 {
            (&self.market.up_token_id, &self.market.down_token_id)
        } else {
            (&self.market.down_token_id, &self.market.up_token_id)
        };

        let imbalance_amount = (up_size - down_size).abs();

        // Get our orders for self-trade prevention
        let (our_overweight_bids, our_underweight_asks): (Vec<f64>, Vec<f64>) = {
            let oms = self.order_state.read();
            let bids = oms.get_bids(overweight_token)
                .iter()
                .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
                .map(|o| o.price)
                .collect();
            let asks = oms.get_asks(underweight_token)
                .iter()
                .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
                .map(|o| o.price)
                .collect();
            (bids, asks)
        };

        let mirror_prices: Vec<f64> = our_overweight_bids.iter().map(|p| 1.0 - p).collect();

        // Get best ask on underweight side
        let (ask_price, ask_size) = {
            let obs = self.orderbooks.read();
            match obs.get(underweight_token).and_then(|ob| ob.best_ask()) {
                Some((price, size)) => (price, size),
                None => return,
            }
        };

        // Self-trade prevention
        let ask_is_ours = our_underweight_asks.iter()
            .any(|p| (p - ask_price).abs() < PRICE_EPSILON);
        if ask_is_ours {
            return;
        }
        if mirror_prices.iter().any(|mp| (mp - ask_price).abs() < PRICE_EPSILON) {
            return;
        }

        let take_size = imbalance_amount
            .min(ask_size)
            .min(self.config.max_take_size);
        if take_size < self.config.min_take_size {
            return;
        }

        // Simulate avg cost after taking
        let new_underweight_avg = if delta > 0.0 {
            let old_cost = down_size * down_avg;
            let new_cost = take_size * ask_price;
            let new_size = down_size + take_size;
            if new_size > 0.0 { (old_cost + new_cost) / new_size } else { ask_price }
        } else {
            let old_cost = up_size * up_avg;
            let new_cost = take_size * ask_price;
            let new_size = up_size + take_size;
            if new_size > 0.0 { (old_cost + new_cost) / new_size } else { ask_price }
        };

        let combined_avg = if delta > 0.0 {
            up_avg + new_underweight_avg
        } else {
            new_underweight_avg + down_avg
        };

        if combined_avg >= self.config.max_combined_avg {
            debug!(
                "[Taker:{}] Skipping: combined_avg ${:.4} >= max ${:.4}",
                self.market.short_desc(),
                combined_avg,
                self.config.max_combined_avg
            );
            return;
        }

        // Execute FOK immediately
        self.fok_pending.store(true, Ordering::Release);

        let trading = Arc::clone(&self.trading);
        let token_id = underweight_token.clone();
        let market_desc = self.market.short_desc();
        let fok_pending = Arc::clone(&self.fok_pending);

        tokio::spawn(async move {
            let result = trading.buy_fok(&token_id, ask_price, take_size).await;
            fok_pending.store(false, Ordering::Release);

            match result {
                Ok(r) if r.status.as_deref() == Some("matched") => {
                    info!(
                        "[Taker:{}] Filled {} @ ${:.4} (combined_avg: ${:.4})",
                        market_desc, take_size, ask_price, combined_avg
                    );
                }
                Ok(r) => {
                    debug!("[Taker:{}] Not filled: {:?}", market_desc, r.status);
                }
                Err(e) => {
                    error!("[Taker:{}] FOK failed: {}", market_desc, e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require mocking TradingClient and shared state
    // Unit tests for logic can be added here

    #[test]
    fn test_delta_calculation() {
        // Test delta formula
        let up_size: f64 = 80.0;
        let down_size: f64 = 20.0;
        let total = up_size + down_size;
        let delta = (up_size - down_size) / total;
        assert!((delta - 0.6_f64).abs() < 1e-9);
    }

    #[test]
    fn test_mirror_price() {
        // Test mirror price calculation
        let bid_price: f64 = 0.55;
        let mirror = 1.0 - bid_price;
        assert!((mirror - 0.45_f64).abs() < 1e-9);
    }

    #[test]
    fn test_combined_avg() {
        // Test combined avg calculation
        let up_avg: f64 = 0.52;
        let down_avg: f64 = 0.46;
        let combined = up_avg + down_avg;
        assert!((combined - 0.98_f64).abs() < 1e-9);
        assert!(combined < 1.0); // Profitable
    }

    #[test]
    fn test_price_epsilon() {
        // Prices within PRICE_EPSILON should be considered equal
        let p1: f64 = 0.5500;
        let p2: f64 = 0.55005; // Difference = 0.00005 < 0.0001
        assert!((p1 - p2).abs() < super::PRICE_EPSILON);

        // Prices outside PRICE_EPSILON should be different
        let p3: f64 = 0.5502; // Difference = 0.0002 > 0.0001
        assert!((p1 - p3).abs() >= super::PRICE_EPSILON);
    }
}
