//! Solver - pure function that takes inputs and returns actions.

mod core;
mod quotes;
mod diff;

#[cfg(test)]
mod test_solver_visual;

#[cfg(test)]
mod test_profitability_cap;

pub use core::solve;
pub use quotes::calculate_quotes;
pub use diff::diff_orders;
