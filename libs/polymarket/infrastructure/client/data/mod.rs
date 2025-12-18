//! Data API client for Polymarket
//!
//! Provides access to user positions and related data.
//!
//! # Example
//!
//! ```rust,ignore
//! use polymarket::infrastructure::client::data::DataApiClient;
//!
//! let client = DataApiClient::new();
//!
//! // Get all positions for a user
//! let positions = client.get_all_positions("0x1234...", None).await?;
//!
//! // Get positions for specific market
//! let positions = client.get_positions_for_market(
//!     "0x1234...",
//!     &["condition_id".to_string()],
//! ).await?;
//! ```

mod client;
mod types;

pub use client::{DataApiClient, DataApiError, Result};
pub use types::{Position, PositionFilters, PositionSortBy, SortDirection};
