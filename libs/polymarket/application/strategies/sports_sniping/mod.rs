//! Sports Sniping Strategy
//!
//! Monitors sports markets using real-time game data from the Polymarket
//! sports WebSocket to identify sniping opportunities.

mod services;
mod strategy;
mod tracker;
mod types;

pub use strategy::SportsSnipingStrategy;
