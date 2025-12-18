//! Sports Live Data types
//!
//! Data structures for parsing sports game updates from the Polymarket sports WebSocket.

use dashmap::{DashMap, DashSet};
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::DbMarket;

/// Shared sports live data state organized by league
/// Structure: league_abbreviation -> (game_id -> SportsLiveData)
/// Uses DashMap for lock-free concurrent access from WS handler and strategy main loop
pub type SharedSportsLiveData = Arc<DashMap<String, DashMap<i64, SportsLiveData>>>;

pub type MarketsByGame = Arc<DashMap<i64, Vec<DbMarket>>>;

/// Set of game_ids to ignore (finished games seen on first message)
pub type IgnoredGames = Arc<DashSet<i64>>;

/// Set of game_ids for which we've already fetched markets
pub type FetchedGames = Arc<DashSet<i64>>;

/// Sports game live data from Polymarket sports WebSocket
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SportsLiveData {
    pub game_id: i64,
    pub score: String,
    #[serde(default)]
    pub elapsed: String,
    pub period: String,
    pub live: bool,
    pub ended: bool,
    #[serde(default)]
    pub finished_timestamp: Option<String>,
    #[serde(default)]
    pub league_abbreviation: String,
    #[serde(default)]
    pub home_team: Option<String>,
    #[serde(default)]
    pub away_team: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Message types from sports WebSocket
#[derive(Debug)]
pub enum SportsLiveDataMessage {
    GameUpdate(SportsLiveData),
    Unknown(String),
}

/// Event sent when a game reaches Full Time (FT or VFT)
/// Forwarded via channel from WebSocket handler to strategy main loop
#[derive(Debug, Clone)]
pub struct FullTimeEvent {
    pub game_id: i64,
    pub league: String,
    pub home_team: Option<String>,
    pub away_team: Option<String>,
    pub final_score: String,
    pub period: String,
    pub status: Option<String>,
}

/// Event sent when a new game is first seen (triggers market fetch)
#[derive(Debug, Clone)]
pub struct NewGameEvent {
    pub game_id: i64,
    pub league: String,
}

/// Route key for sports messages (single route for now)
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum SportsRoute {
    All,
}
