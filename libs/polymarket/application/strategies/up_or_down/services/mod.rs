//! Services for the Up or Down strategy.

mod logging;
mod price_service;

pub use logging::{
    log_market_ended, log_no_asks_started, log_order_failed, log_order_success, log_placing_order,
    log_risk_detected, log_threshold_exceeded,
};
pub use price_service::{
    get_market_oracle_age, get_oracle_price, get_price_to_beat, is_market_oracle_fresh,
};
