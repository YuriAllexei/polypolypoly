//! Market tracking components for the Up or Down strategy.

mod market_tracker;
mod orderbook_checker;
mod risk_manager;

pub use market_tracker::run_market_tracker;
pub use orderbook_checker::{calculate_dynamic_threshold, check_all_orderbooks};
pub use risk_manager::{check_risk, guardian_check, place_order, upgrade_order_on_tick_change};
