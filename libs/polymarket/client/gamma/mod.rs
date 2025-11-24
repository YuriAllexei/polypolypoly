//! Gamma API client and types
//!
//! The Gamma API provides market data and event information.

pub mod client;
pub mod types;

pub use client::GammaClient;

// Re-export types with Gamma prefix for backward compatibility
pub use types::Event as GammaEvent;
pub use types::Market as GammaMarket;
pub use types::Tag as GammaTag;
pub use types::Events;

// Re-export GammaFilters
pub use types::GammaFilters;
