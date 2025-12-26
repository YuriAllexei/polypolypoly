//! In-flight order tracking to prevent duplicate commands.

pub mod tracker;

pub use tracker::{InFlightTracker, OpenOrderInfo, price_to_key};
