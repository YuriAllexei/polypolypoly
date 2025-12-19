//! WebSocket client for Sports Live Data tracking
//!
//! Connects to Polymarket's sports API WebSocket to receive real-time
//! game updates including scores, periods, and game status.

use super::types::{
    FetchedGames, FullTimeEvent, MarketsByGame, SportsLiveData, SportsLiveDataMessage, SportsRoute,
};
use crate::infrastructure::MarketDatabase;
use anyhow::Result;
use crossbeam_channel::Sender;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// WebSocket URL for Polymarket sports live data
const SPORTS_WS_URL: &str = "wss://sports-api.polymarket.com/ws";

// =============================================================================
// Router - Parses WebSocket messages
// =============================================================================

/// Router for parsing sports WebSocket messages
pub struct SportsLiveDataRouter;

impl SportsLiveDataRouter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SportsLiveDataRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MessageRouter for SportsLiveDataRouter {
    type Message = SportsLiveDataMessage;
    type RouteKey = SportsRoute;

    async fn parse(&self, message: WsMessage) -> hypersockets::Result<Self::Message> {
        let text = match message.as_text() {
            Some(t) => t,
            None => return Ok(SportsLiveDataMessage::Unknown("Binary data".to_string())),
        };

        // Try to parse as game update
        match serde_json::from_str::<SportsLiveData>(text) {
            Ok(data) => Ok(SportsLiveDataMessage::GameUpdate(data)),
            Err(e) => {
                debug!("[Sports WS] Failed to parse message: {} - {}", e, text);
                Ok(SportsLiveDataMessage::Unknown(text.to_string()))
            }
        }
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        SportsRoute::All
    }
}

// =============================================================================
// Handler - Logs game updates
// =============================================================================

/// Handler that logs sports live data messages
pub struct SportsLiveDataHandler {
    message_count: u64,
}

impl SportsLiveDataHandler {
    pub fn new() -> Self {
        Self { message_count: 0 }
    }
}

impl Default for SportsLiveDataHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageHandler<SportsLiveDataMessage> for SportsLiveDataHandler {
    fn handle(&mut self, message: SportsLiveDataMessage) -> hypersockets::Result<()> {
        self.message_count += 1;

        match message {
            SportsLiveDataMessage::GameUpdate(data) => {
                info!(
                    "[Sports WS] Game #{}: {} | {} vs {} | Score: {} | Period: {} | Status: {:?} | Live: {} | Ended: {}",
                    data.game_id,
                    data.league_abbreviation,
                    data.home_team.as_deref().unwrap_or("?"),
                    data.away_team.as_deref().unwrap_or("?"),
                    data.score,
                    data.period,
                    data.status,
                    data.live,
                    data.ended
                );
            }
            SportsLiveDataMessage::Unknown(text) => {
                debug!("[Sports WS] Unknown message: {}", text);
            }
        }

        Ok(())
    }
}

// =============================================================================
// State Handler - Updates shared state with game data
// =============================================================================

/// Handler that updates shared state with game updates
/// Used by strategies that need to access game data from the main loop
pub struct SportsLiveDataStateHandler {
    /// Shared set of game_ids for which markets have been fetched
    fetched_games: FetchedGames,
    /// Shared cache of markets per game_id
    markets_cache: MarketsByGame,
    /// Database access for fetching markets
    database: Arc<MarketDatabase>,
    /// Tokio runtime handle for blocking async calls
    runtime_handle: tokio::runtime::Handle,
    /// Optional channel to forward Full Time events to main loop
    ft_tx: Option<Sender<FullTimeEvent>>,
    /// Track which games we've already sent FT events for (prevent duplicates)
    ft_sent: HashSet<i64>,
}

impl SportsLiveDataStateHandler {
    pub fn new(
        fetched_games: FetchedGames,
        markets_cache: MarketsByGame,
        database: Arc<MarketDatabase>,
        runtime_handle: tokio::runtime::Handle,
        ft_tx: Option<Sender<FullTimeEvent>>,
    ) -> Self {
        Self {
            fetched_games,
            markets_cache,
            database,
            runtime_handle,
            ft_tx,
            ft_sent: HashSet::new(),
        }
    }
}

impl MessageHandler<SportsLiveDataMessage> for SportsLiveDataStateHandler {
    fn handle(&mut self, message: SportsLiveDataMessage) -> hypersockets::Result<()> {
        if let SportsLiveDataMessage::GameUpdate(data) = message {
            let game_id = data.game_id;

            // Skip games with no league abbreviation
            if data.league_abbreviation.is_empty() {
                debug!(
                    game_id = game_id,
                    "Skipping game with empty league abbreviation"
                );
                return Ok(());
            }

            // First time seeing this game? Fetch markets from DB
            if !self.fetched_games.contains(&game_id) {
                // Use blocking runtime to call async DB method
                let db = Arc::clone(&self.database);
                let markets = self
                    .runtime_handle
                    .block_on(async move { db.get_markets_by_game_id(game_id).await });

                match markets {
                    Ok(m) => {
                        debug!(
                            game_id = game_id,
                            count = m.len(),
                            "Fetched markets for game"
                        );
                        self.markets_cache.insert(game_id, m);
                    }
                    Err(e) => {
                        warn!(game_id = game_id, error = %e, "Failed to fetch markets");
                    }
                }
                self.fetched_games.insert(game_id);
            }

            // Check for Full Time and send event if not already sent
            let is_ft = data.period == "FT" || data.period == "VFT";
            if is_ft && !self.ft_sent.contains(&game_id) {
                info!(
                    game_id = game_id,
                    league = %data.league_abbreviation,
                    home = data.home_team.as_deref().unwrap_or("?"),
                    away = data.away_team.as_deref().unwrap_or("?"),
                    score = %data.score,
                    period = %data.period,
                    status = data.status.as_deref().unwrap_or("?"),
                    "Game reached Full Time/Virtually Full Time"
                );

                if let Some(ref tx) = self.ft_tx {
                    let event = FullTimeEvent {
                        game_id,
                        league: data.league_abbreviation.clone(),
                        home_team: data.home_team.clone(),
                        away_team: data.away_team.clone(),
                        final_score: data.score.clone(),
                        period: data.period.clone(),
                        status: data.status.clone(),
                    };
                    let _ = tx.send(event); // Non-blocking, ignore if receiver dropped
                }
                self.ft_sent.insert(game_id);
            }

            // debug!(
            //     game_id = game_id,
            //     league = %data.league_abbreviation,
            //     home = data.home_team.as_deref().unwrap_or("?"),
            //     away = data.away_team.as_deref().unwrap_or("?"),
            //     score = %data.score,
            //     period = %data.period,
            //     status = data.status.as_deref().unwrap_or("?"),
            //     live = data.live,
            //     "Game update"
            // );
        }
        Ok(())
    }
}

// =============================================================================
// WebSocket Client Builder
// =============================================================================

/// Build the sports live data WebSocket client (logging handler).
///
/// Each WebSocket client uses a local shutdown flag because hypersockets
/// sets the flag to false during `client.shutdown()`.
async fn build_sports_ws_client(
) -> Result<WebSocketClient<SportsLiveDataRouter, SportsLiveDataMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SportsLiveDataRouter::new();
    let handler = SportsLiveDataHandler::new();

    let client = WebSocketClientBuilder::new()
        .url(SPORTS_WS_URL)
        .router(router, move |routing| {
            routing.handler(SportsRoute::All, handler)
        })
        // No heartbeat needed for sports WS
        // No subscription needed for sports WS
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

/// Build the sports live data WebSocket client with shared state handler.
///
/// Fetches markets from DB on first game update and caches them.
///
/// # Arguments
/// * `fetched_games` - Shared set tracking which games have had markets fetched
/// * `markets_cache` - Shared cache of markets per game_id
/// * `database` - Database access for fetching markets
/// * `runtime_handle` - Tokio runtime handle for blocking async calls in sync handler
/// * `ft_tx` - Optional channel sender for forwarding Full Time events to main loop
async fn build_sports_ws_client_with_state(
    fetched_games: FetchedGames,
    markets_cache: MarketsByGame,
    database: Arc<MarketDatabase>,
    runtime_handle: tokio::runtime::Handle,
    ft_tx: Option<Sender<FullTimeEvent>>,
) -> Result<WebSocketClient<SportsLiveDataRouter, SportsLiveDataMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SportsLiveDataRouter::new();
    let handler = SportsLiveDataStateHandler::new(
        fetched_games,
        markets_cache,
        database,
        runtime_handle,
        ft_tx,
    );

    let client = WebSocketClientBuilder::new()
        .url(SPORTS_WS_URL)
        .router(router, move |routing| {
            routing.handler(SportsRoute::All, handler)
        })
        .shutdown_flag(local_shutdown_flag)
        .build()
        .await?;

    Ok(client)
}

// =============================================================================
// Main Tracking Loop
// =============================================================================

/// Handle a WebSocket client event
fn handle_client_event(event: ClientEvent) -> bool {
    match event {
        ClientEvent::Connected => {
            info!("[Sports WS] WebSocket connected");
            true
        }
        ClientEvent::Disconnected => {
            warn!("[Sports WS] WebSocket disconnected");
            false
        }
        ClientEvent::Reconnecting(attempt) => {
            warn!("[Sports WS] Reconnecting (attempt {})", attempt);
            true
        }
        ClientEvent::Error(err) => {
            warn!("[Sports WS] Error: {}", err);
            true
        }
    }
}

/// Spawn the sports live data tracker.
///
/// Connects to the sports WebSocket and logs incoming game updates.
/// Runs until the shutdown flag is set to false.
pub async fn spawn_sports_live_data_tracker(shutdown_flag: Arc<AtomicBool>) -> Result<()> {
    info!("════════════════════════════════════════════════════════════════");
    info!("  STARTING SPORTS LIVE DATA TRACKER");
    info!("════════════════════════════════════════════════════════════════");
    info!("  URL: {}", SPORTS_WS_URL);
    info!("════════════════════════════════════════════════════════════════");

    let client = build_sports_ws_client().await?;
    info!("[Sports WS] Connected and listening for game updates");

    // Main tracking loop
    loop {
        // Check shutdown flag first (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[Sports WS] Shutdown signal received");
            break;
        }

        // Handle WebSocket events
        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event) {
                    break;
                }
            }
            None => {
                // No event available, sleep briefly before checking again
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[Sports WS] Closing connection");
    if let Err(e) = client.shutdown().await {
        warn!("[Sports WS] Error during shutdown: {}", e);
    }
    info!("[Sports WS] Tracker stopped");
    Ok(())
}

/// Spawn the sports live data tracker that updates shared state.
///
/// Connects to the sports WebSocket, fetches markets from DB on first game update,
/// caches them, and sends FullTimeEvent when games reach FT/VFT.
/// Runs until the shutdown flag is set to false.
///
/// # Arguments
/// * `shutdown_flag` - Flag to signal shutdown
/// * `fetched_games` - Shared set tracking which games have had markets fetched
/// * `markets_cache` - Shared cache of markets per game_id
/// * `database` - Database access for fetching markets
/// * `runtime_handle` - Tokio runtime handle for blocking async calls in sync handler
/// * `ft_tx` - Optional channel sender for forwarding Full Time events to main loop
pub async fn spawn_sports_tracker_with_state(
    shutdown_flag: Arc<AtomicBool>,
    fetched_games: FetchedGames,
    markets_cache: MarketsByGame,
    database: Arc<MarketDatabase>,
    runtime_handle: tokio::runtime::Handle,
    ft_tx: Option<Sender<FullTimeEvent>>,
) -> Result<()> {
    info!("════════════════════════════════════════════════════════════════");
    info!("  STARTING SPORTS LIVE DATA TRACKER (State Mode)");
    info!("════════════════════════════════════════════════════════════════");
    info!("  URL: {}", SPORTS_WS_URL);
    info!("════════════════════════════════════════════════════════════════");

    let client = build_sports_ws_client_with_state(
        fetched_games,
        markets_cache,
        database,
        runtime_handle,
        ft_tx,
    )
    .await?;
    info!("[Sports WS] Connected and updating shared state");

    // Main tracking loop
    loop {
        // Check shutdown flag first (highest priority)
        if !shutdown_flag.load(Ordering::Acquire) {
            info!("[Sports WS] Shutdown signal received");
            break;
        }

        // Handle WebSocket events
        match client.try_recv_event() {
            Some(event) => {
                if !handle_client_event(event) {
                    break;
                }
            }
            None => {
                // No event available, sleep briefly before checking again
                sleep(Duration::from_millis(10)).await;
            }
        }
    }

    info!("[Sports WS] Closing connection");
    if let Err(e) = client.shutdown().await {
        warn!("[Sports WS] Error during shutdown: {}", e);
    }
    info!("[Sports WS] Tracker stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sports_route_equality() {
        let route1 = SportsRoute::All;
        let route2 = SportsRoute::All;
        assert_eq!(route1, route2);
    }

    #[test]
    fn test_parse_sports_live_data() {
        let json = r#"{
            "gameId": 70414,
            "score": "43-41",
            "elapsed": "10:08",
            "period": "Q2",
            "live": true,
            "ended": false,
            "leagueAbbreviation": "cbb",
            "homeTeam": "LOYMRY",
            "awayTeam": "UCSD",
            "status": "InProgress"
        }"#;

        let data: SportsLiveData = serde_json::from_str(json).unwrap();
        assert_eq!(data.game_id, 70414);
        assert_eq!(data.score, "43-41");
        assert_eq!(data.period, "Q2");
        assert!(data.live);
        assert!(!data.ended);
        assert_eq!(data.league_abbreviation, "cbb");
        assert_eq!(data.home_team, Some("LOYMRY".to_string()));
        assert_eq!(data.away_team, Some("UCSD".to_string()));
        assert_eq!(data.status, Some("InProgress".to_string()));
    }
}
