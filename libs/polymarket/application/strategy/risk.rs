use crate::domain::strategy::{DailyStats, RiskConfig, RiskError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

pub type Result<T> = std::result::Result<T, RiskError>;

/// Risk manager - enforces trading limits
pub struct RiskManager {
    config: RiskConfig,

    /// Number of currently open positions
    concurrent_positions: AtomicUsize,

    /// Daily profit/loss
    daily_pnl: Arc<RwLock<f64>>,

    /// Whether trading is halted
    halted: AtomicBool,

    /// Total trades executed today
    trades_today: AtomicUsize,

    /// Winning trades today
    wins_today: AtomicUsize,
}

impl RiskManager {
    /// Create new risk manager
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config,
            concurrent_positions: AtomicUsize::new(0),
            daily_pnl: Arc::new(RwLock::new(0.0)),
            halted: AtomicBool::new(false),
            trades_today: AtomicUsize::new(0),
            wins_today: AtomicUsize::new(0),
        }
    }

    /// Check if a new trade is within risk limits
    pub fn check_limits(&self, amount: f64) -> Result<()> {
        // Check if trading is halted
        if self.is_halted() {
            return Err(RiskError::TradingHalted);
        }

        // Check max bet per market
        if amount > self.config.max_bet_per_market {
            return Err(RiskError::MaxBetExceeded(
                amount,
                self.config.max_bet_per_market,
            ));
        }

        // Check concurrent positions
        let current_positions = self.concurrent_positions.load(Ordering::Relaxed);
        if current_positions >= self.config.max_concurrent_positions {
            return Err(RiskError::MaxPositionsReached(
                self.config.max_concurrent_positions,
            ));
        }

        // Check daily loss limit
        if let Ok(pnl) = self.daily_pnl.read() {
            if *pnl < -self.config.daily_loss_limit {
                self.halt();
                return Err(RiskError::DailyLossLimitReached(self.config.daily_loss_limit));
            }
        }

        Ok(())
    }

    /// Record a new position being opened
    pub fn add_position(&self, amount: f64) {
        let new_count = self.concurrent_positions.fetch_add(1, Ordering::Relaxed) + 1;
        info!("Position opened (${:.2}). Open positions: {}", amount, new_count);
    }

    /// Record a position being closed with its P&L
    pub fn close_position(&self, pnl: f64) {
        // Decrease concurrent positions
        let remaining = self.concurrent_positions.fetch_sub(1, Ordering::Relaxed) - 1;

        // Update daily P&L
        if let Ok(mut daily_pnl) = self.daily_pnl.write() {
            *daily_pnl += pnl;

            // Update trade stats
            let total_trades = self.trades_today.fetch_add(1, Ordering::Relaxed) + 1;
            let wins = if pnl > 0.0 {
                self.wins_today.fetch_add(1, Ordering::Relaxed) + 1
            } else {
                self.wins_today.load(Ordering::Relaxed)
            };

            info!(
                "Position closed. P&L: ${:.2} | Daily P&L: ${:.2} | Win rate: {}/{} ({:.1}%) | Open: {}",
                pnl,
                *daily_pnl,
                wins,
                total_trades,
                (wins as f64 / total_trades as f64) * 100.0,
                remaining
            );

            // Check if we hit loss limit
            if *daily_pnl < -self.config.daily_loss_limit {
                warn!("Daily loss limit reached: -${:.2}", daily_pnl.abs());
                self.halt();
            }
        }
    }

    /// Halt trading
    pub fn halt(&self) {
        self.halted.store(true, Ordering::Release);
        warn!("🛑 TRADING HALTED due to risk limits");
    }

    /// Resume trading
    pub fn resume(&self) {
        self.halted.store(false, Ordering::Release);
        info!("✅ Trading resumed");
    }

    /// Check if trading is halted
    pub fn is_halted(&self) -> bool {
        self.halted.load(Ordering::Acquire)
    }

    /// Get current number of open positions
    pub fn open_positions(&self) -> usize {
        self.concurrent_positions.load(Ordering::Relaxed)
    }

    /// Get daily P&L
    pub fn daily_pnl(&self) -> f64 {
        self.daily_pnl.read().map(|pnl| *pnl).unwrap_or(0.0)
    }

    /// Get daily trade statistics
    pub fn daily_stats(&self) -> DailyStats {
        let total = self.trades_today.load(Ordering::Relaxed);
        let wins = self.wins_today.load(Ordering::Relaxed);
        let pnl = self.daily_pnl();

        DailyStats::new(total, wins, pnl)
    }

    /// Reset daily counters (call at start of new day)
    pub fn reset_daily(&self) {
        if let Ok(mut pnl) = self.daily_pnl.write() {
            *pnl = 0.0;
        }
        self.trades_today.store(0, Ordering::Relaxed);
        self.wins_today.store(0, Ordering::Relaxed);
        self.resume();  // Resume trading for new day

        info!("📊 Daily stats reset for new trading day");
    }

    /// Check if profit meets minimum threshold
    pub fn is_profitable(&self, expected_profit_cents: f64) -> bool {
        expected_profit_cents >= self.config.min_profit_cents
    }
}

// DailyStats is now defined in domain::strategy

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_concurrent_positions: 5,
            max_bet_per_market: 100.0,
            daily_loss_limit: 500.0,
            min_profit_cents: 50.0,
        }
    }

    #[test]
    fn test_check_limits_ok() {
        let risk = RiskManager::new(test_config());
        assert!(risk.check_limits(50.0).is_ok());
    }

    #[test]
    fn test_max_bet_exceeded() {
        let risk = RiskManager::new(test_config());
        assert!(risk.check_limits(150.0).is_err());
    }

    #[test]
    fn test_max_positions() {
        let risk = RiskManager::new(test_config());

        // Add 5 positions (max)
        for _ in 0..5 {
            risk.add_position(50.0);
        }

        // 6th should fail
        assert!(risk.check_limits(50.0).is_err());
    }

    #[test]
    fn test_daily_loss_limit() {
        let risk = RiskManager::new(test_config());

        // Simulate losing trades
        risk.close_position(-200.0);
        risk.close_position(-200.0);
        risk.close_position(-200.0);

        // Should be halted now (lost 600, limit is 500)
        assert!(risk.is_halted());
        assert!(risk.check_limits(50.0).is_err());
    }

    #[test]
    fn test_profit_threshold() {
        let risk = RiskManager::new(test_config());

        assert!(risk.is_profitable(60.0));  // Above threshold
        assert!(!risk.is_profitable(40.0)); // Below threshold
    }

    #[test]
    fn test_daily_reset() {
        let risk = RiskManager::new(test_config());

        risk.close_position(-100.0);
        assert_eq!(risk.daily_pnl(), -100.0);

        risk.reset_daily();
        assert_eq!(risk.daily_pnl(), 0.0);
        assert!(!risk.is_halted());
    }
}
