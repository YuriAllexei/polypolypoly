//! Polymarket Trading Bot - Main Library
//!
//! This crate provides the main library for the Polymarket trading bot,
//! following Clean Architecture principles.
//!
//! ## Architecture
//!
//! - **bin_common**: Common utilities for binary executables (CLI, runners)
//! - **polymarket**: Core business logic (re-exported from workspace)
//! - **hypersockets**: WebSocket library (re-exported from workspace)
//!
//! ## Usage in Binaries
//!
//! ```rust
//! use polymarket_arb_bot::bin_common::{load_config_from_env, ConfigType};
//! use polymarket_arb_bot::polymarket::application::SniperApp;
//! ```

// Re-export workspace libraries for convenience
pub use polymarket;
pub use hypersockets;

// Binary common utilities
pub mod bin_common {
    //! Common utilities for binary executables
    //!
    //! Provides shared functionality for the presentation layer (binaries)
    //! following Clean Architecture principles.

    pub mod cli;
    pub mod runner;

    pub use cli::{load_config_from_env, parse_args, ConfigType};
    pub use runner::{BinaryRunner, RunConfig};
}
