//! Solver - pure function that takes inputs and returns actions.
//!
//! Implements 4-layer quoting framework (O'Hara Market Microstructure):
//! - Layer 1: Oracle-adjusted offset
//! - Layer 2: Adverse selection (Glosten-Milgrom)
//! - Layer 3: Inventory skew
//! - Layer 4: Edge check

mod core;
mod quotes;
mod diff;

pub use core::solve;
pub use quotes::calculate_quotes;
pub use diff::diff_orders;
