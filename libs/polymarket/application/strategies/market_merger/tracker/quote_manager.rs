//! Quote management for the Market Merger strategy
//!
//! Handles placing, updating, and canceling bid orders.

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::application::strategies::market_merger::types::{BidInfo, MarketContext, MarketState, Quote, QuoteLadder};
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::{OrderStateStore, OrderType, Side};
use tokio::sync::RwLock;

/// Manages quote placement and updates
pub struct QuoteManager {
    trading: Arc<TradingClient>,
    #[allow(dead_code)] // Reserved for future STP integration
    order_state: Arc<RwLock<OrderStateStore>>,
    /// Price tolerance for considering a bid "same" (no update needed)
    price_tolerance: f64,
    /// Maximum age before a bid is considered stale (seconds)
    max_bid_age_secs: u64,
}

impl QuoteManager {
    /// Create a new quote manager
    pub fn new(
        trading: Arc<TradingClient>,
        order_state: Arc<RwLock<OrderStateStore>>,
    ) -> Self {
        Self {
            trading,
            order_state,
            price_tolerance: 0.005,
            max_bid_age_secs: 30,
        }
    }

    /// Update bid ladder - cancel stale bids, place new ones
    pub async fn update_bids(
        &self,
        ctx: &MarketContext,
        state: &mut MarketState,
        ladder: &QuoteLadder,
    ) -> anyhow::Result<()> {
        // 1. Find bids to cancel (price changed significantly or stale)
        let to_cancel = self.find_stale_bids(state, ladder);

        // 2. Cancel stale bids
        if !to_cancel.is_empty() {
            debug!("Canceling {} stale bids", to_cancel.len());
            if let Err(e) = self.trading.cancel_orders(&to_cancel).await {
                warn!("Failed to cancel some orders: {}", e);
            }
            self.remove_from_state(state, &to_cancel);
        }

        // 3. Find bids to place (new levels or replaced)
        let to_place = self.find_bids_to_place(state, ladder);

        if to_place.is_empty() {
            return Ok(());
        }

        // 4. Place new bids (one at a time to get order IDs)
        for quote in &to_place {
            match self.place_single_bid(ctx, quote).await {
                Ok(order_id) => {
                    let bid_info = BidInfo::new(
                        order_id,
                        quote.price,
                        quote.size,
                        quote.level,
                    );

                    // Add to state
                    let is_up = quote.token_id == ctx.up_token_id;
                    if is_up {
                        state.up_bids.insert(quote.level, bid_info);
                    } else {
                        state.down_bids.insert(quote.level, bid_info);
                    }

                    debug!(
                        "Placed {} bid L{}: ${:.3} x {:.1}",
                        if is_up { "Up" } else { "Down" },
                        quote.level,
                        quote.price,
                        quote.size
                    );
                }
                Err(e) => {
                    warn!("Failed to place bid: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Cancel all active bids
    pub async fn cancel_all(&self, state: &mut MarketState) -> anyhow::Result<()> {
        let all_ids: Vec<String> = state
            .up_bids
            .values()
            .chain(state.down_bids.values())
            .map(|b| b.order_id.clone())
            .collect();

        if !all_ids.is_empty() {
            info!("Canceling all {} bids", all_ids.len());
            if let Err(e) = self.trading.cancel_orders(&all_ids).await {
                warn!("Failed to cancel some orders: {}", e);
            }
        }

        state.clear_bids();
        Ok(())
    }

    /// Find bids that need to be canceled (price changed or stale)
    fn find_stale_bids(&self, state: &MarketState, ladder: &QuoteLadder) -> Vec<String> {
        let mut stale = Vec::new();

        // Check Up bids
        for (level, bid) in &state.up_bids {
            let new_bid = ladder.up_bids.iter().find(|b| b.level == *level);
            let should_cancel = self.should_cancel_bid(bid, new_bid);
            if should_cancel {
                stale.push(bid.order_id.clone());
            }
        }

        // Check Down bids
        for (level, bid) in &state.down_bids {
            let new_bid = ladder.down_bids.iter().find(|b| b.level == *level);
            let should_cancel = self.should_cancel_bid(bid, new_bid);
            if should_cancel {
                stale.push(bid.order_id.clone());
            }
        }

        stale
    }

    /// Check if an existing bid should be canceled
    fn should_cancel_bid(&self, existing: &BidInfo, new: Option<&Quote>) -> bool {
        // No new bid at this level - cancel existing
        if new.is_none() {
            return true;
        }

        let new = new.unwrap();

        // Price changed significantly
        let price_diff = (existing.price - new.price).abs();
        if price_diff > self.price_tolerance {
            return true;
        }

        // Bid is stale
        if existing.is_stale(self.max_bid_age_secs) {
            return true;
        }

        false
    }

    /// Find quotes that need to be placed (no existing bid at that level)
    fn find_bids_to_place<'a>(&self, state: &MarketState, ladder: &'a QuoteLadder) -> Vec<&'a Quote> {
        let mut to_place = Vec::new();

        // Check Up bids
        for quote in &ladder.up_bids {
            if !state.up_bids.contains_key(&quote.level) && quote.size > 0.0 {
                to_place.push(quote);
            }
        }

        // Check Down bids
        for quote in &ladder.down_bids {
            if !state.down_bids.contains_key(&quote.level) && quote.size > 0.0 {
                to_place.push(quote);
            }
        }

        to_place
    }

    /// Remove canceled orders from state
    fn remove_from_state(&self, state: &mut MarketState, order_ids: &[String]) {
        state.up_bids.retain(|_, bid| !order_ids.contains(&bid.order_id));
        state.down_bids.retain(|_, bid| !order_ids.contains(&bid.order_id));
    }

    /// Place a single bid order
    async fn place_single_bid(&self, _ctx: &MarketContext, quote: &Quote) -> anyhow::Result<String> {
        // Place GTC (good till cancel) order for maker bids
        let result = self
            .trading
            .place_order(
                &quote.token_id,
                quote.price,
                quote.size,
                Side::Buy,
                OrderType::GTC,
            )
            .await?;

        // Return order ID, or error if not provided
        result
            .order_id
            .ok_or_else(|| anyhow::anyhow!("Order placed but no order_id returned"))
    }
}
