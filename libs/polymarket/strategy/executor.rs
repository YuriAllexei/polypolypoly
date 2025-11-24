use super::monitor::MonitoredMarket;
use super::risk::{RiskConfig, RiskManager};
use crate::client::{
    clob::{types::*, OrderType, RestClient},
    PolymarketAuth,
};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("REST API error: {0}")]
    RestError(#[from] crate::client::clob::rest::RestError),

    #[error("Risk check failed: {0}")]
    RiskError(#[from] super::risk::RiskError),

    #[error("No profitable opportunity found")]
    NoOpportunity,

    #[error("Orderbook empty or invalid")]
    InvalidOrderbook,
}

pub type Result<T> = std::result::Result<T, ExecutorError>;

/// Executed trade information
#[derive(Debug, Clone)]
pub struct ExecutedTrade {
    pub market_id: String,
    pub token_id: String,
    pub side: Side,
    pub amount_usd: f64,
    pub price: f64,
    pub expected_profit: f64,
    pub order_id: String,
}

/// Trading configuration
#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub probability_threshold: f64,
    pub bet_amount_usd: f64,
}

/// Order executor
pub struct OrderExecutor {
    rest_client: Arc<RestClient>,
    auth: Arc<PolymarketAuth>,
    risk_manager: Arc<RiskManager>,
}

impl OrderExecutor {
    /// Create new order executor
    pub fn new(
        rest_client: Arc<RestClient>,
        auth: Arc<PolymarketAuth>,
        risk_manager: Arc<RiskManager>,
    ) -> Self {
        Self {
            rest_client,
            auth,
            risk_manager,
        }
    }

    /// Try to execute arbitrage trade on a market
    pub async fn try_execute(
        &self,
        market: &MonitoredMarket,
        config: &TradingConfig,
    ) -> Result<Option<ExecutedTrade>> {
        debug!("Analyzing market: {}", market.question);

        // Get orderbooks for all outcome tokens
        let mut best_opportunity: Option<(String, BestOpportunity)> = None;

        for token_id in &market.token_ids {
            match self.rest_client.get_orderbook(token_id).await {
                Ok(orderbook) => {
                    if let Some(opp) = orderbook.best_opportunity() {
                        debug!(
                            "Token {} - {} side at {:.4} ({:.2}%)",
                            token_id,
                            match opp.side {
                                Side::Buy => "BUY",
                                Side::Sell => "SELL",
                            },
                            opp.price,
                            opp.price * 100.0
                        );

                        // Keep track of best opportunity across all tokens
                        if best_opportunity.is_none()
                            || opp.price > best_opportunity.as_ref().unwrap().1.price
                        {
                            best_opportunity = Some((token_id.clone(), opp));
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to get orderbook for token {}: {}", token_id, e);
                    continue;
                }
            }
        }

        let (token_id, opportunity) = best_opportunity.ok_or(ExecutorError::NoOpportunity)?;

        // Check if price meets threshold
        if opportunity.price < config.probability_threshold {
            debug!(
                "Price {:.4} below threshold {:.4}, skipping",
                opportunity.price, config.probability_threshold
            );
            return Ok(None);
        }

        // Calculate expected profit
        let shares = config.bet_amount_usd / opportunity.price;
        let payout = shares * 1.0;  // Each share pays $1.00 on win
        let profit = payout - config.bet_amount_usd;
        let profit_cents = profit * 100.0;

        debug!(
            "Expected profit: ${:.2} ({:.0} cents) from {} shares @ ${:.4}",
            profit, profit_cents, shares, opportunity.price
        );

        // Check if profit meets minimum threshold
        if !self.risk_manager.is_profitable(profit_cents) {
            debug!("Profit below minimum threshold, skipping");
            return Ok(None);
        }

        // Check risk limits
        self.risk_manager.check_limits(config.bet_amount_usd)?;

        info!(
            "🎯 EXECUTING TRADE: {} - {:?} @ {:.4} for ${:.2} (expected profit: ${:.2})",
            market.question, opportunity.side, opportunity.price, config.bet_amount_usd, profit
        );

        // Create market order
        let market_order = MarketOrderArgs {
            token_id: token_id.clone(),
            amount: config.bet_amount_usd,
            side: opportunity.side,
        };

        // Execute order
        let response = self
            .rest_client
            .place_market_order(&self.auth, &market_order, OrderType::FOK)
            .await?;

        if response.success {
            // Record position
            self.risk_manager.add_position(config.bet_amount_usd);

            info!("✅ Order executed successfully. Order ID: {}", response.order_id);

            Ok(Some(ExecutedTrade {
                market_id: market.market_id.clone(),
                token_id,
                side: opportunity.side,
                amount_usd: config.bet_amount_usd,
                price: opportunity.price,
                expected_profit: profit,
                order_id: response.order_id,
            }))
        } else {
            warn!(
                "❌ Order failed: {}",
                response.error_msg.unwrap_or_else(|| "Unknown error".to_string())
            );
            Ok(None)
        }
    }

    /// Calculate actual profit from a position
    ///
    /// Call this after market resolves to record final P&L
    pub fn record_resolution(&self, trade: &ExecutedTrade, won: bool) {
        let pnl = if won {
            trade.expected_profit
        } else {
            -trade.amount_usd
        };

        info!(
            "{} Trade resolved: {} - P&L: ${:.2}",
            if won { "✅" } else { "❌" },
            trade.market_id,
            pnl
        );

        self.risk_manager.close_position(pnl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trading_config() {
        let config = TradingConfig {
            probability_threshold: 0.98,
            bet_amount_usd: 50.0,
        };

        assert_eq!(config.probability_threshold, 0.98);
        assert_eq!(config.bet_amount_usd, 50.0);
    }
}
