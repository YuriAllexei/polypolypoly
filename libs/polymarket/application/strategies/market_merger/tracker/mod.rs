//! Tracker module for the Market Merger strategy

mod market_tracker;
mod quote_manager;

pub use market_tracker::{run_accumulator, AccumulatorContext};
pub use quote_manager::QuoteManager;
