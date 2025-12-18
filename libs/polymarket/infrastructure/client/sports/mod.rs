//! Sports Live Data WebSocket client
//!
//! Connects to the Polymarket sports API WebSocket to receive real-time
//! game updates including scores, periods, and game status.

mod sports_ws;
mod types;

pub use sports_ws::{spawn_sports_live_data_tracker, spawn_sports_tracker_with_state};
pub use types::{
    FetchedGames, FullTimeEvent, IgnoredGames, MarketsByGame, NewGameEvent, SharedSportsLiveData,
    SportsLiveData, SportsLiveDataMessage, SportsRoute,
};
