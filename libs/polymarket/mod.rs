//! Polymarket Trading Platform
//!
//! Clean Architecture implementation for Polymarket prediction market trading.
//! Organized in three layers:
//! - Domain: Business entities and rules (framework-agnostic)
//! - Infrastructure: External services (database, API clients, config)
//! - Application: Use cases and business workflows

pub mod application;
pub mod domain;
pub mod infrastructure;
