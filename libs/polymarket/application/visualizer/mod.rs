//! MM Visualizer
//!
//! Terminal UI for visualizing orderbooks and orders in real-time.
//! Uses the same real-time WebSocket components as the strategy.

pub mod app;
pub mod state;
pub mod ui;

pub use app::App;
pub use state::{MarketInfo, VisualizerState};
