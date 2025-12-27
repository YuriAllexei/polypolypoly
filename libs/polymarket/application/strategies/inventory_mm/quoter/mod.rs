//! Per-market quoter module.
//!
//! Each quoter manages quoting for a single market, running its own
//! tick loop and maintaining per-market state (orderbooks, in-flight tracker, merger).

mod context;
mod orderbook_ws;
mod quoter;

pub use context::{QuoterContext, MarketInfo};
pub use orderbook_ws::{QuoterWsConfig, QuoterWsClient, build_quoter_ws_client, wait_for_snapshot};
pub use quoter::Quoter;
