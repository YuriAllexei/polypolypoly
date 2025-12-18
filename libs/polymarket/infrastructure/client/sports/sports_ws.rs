//! WebSocket client for Sports Live Data tracking
//!
//! Connects to Polymarket's sports API WebSocket to receive real-time
//! game updates including scores, periods, and game status.

use super::types::{IgnoredGames, SharedSportsLiveData, SportsLiveData, SportsLiveDataMessage, SportsRoute};
use anyhow::Result;
use hypersockets::core::*;
use hypersockets::{MessageHandler, MessageRouter, WsMessage};
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
    shared_state: SharedSportsLiveData,
    ignored_games: IgnoredGames,
}

impl SportsLiveDataStateHandler {
    pub fn new(shared_state: SharedSportsLiveData, ignored_games: IgnoredGames) -> Self {
        Self { shared_state, ignored_games }
    }

    /// Check if game should be ignored based on status
    /// Games are ignored if they appear to be finished on first sight
    fn should_ignore(data: &SportsLiveData) -> bool {
        if data.ended {
            return true;
        }
        match data.status.as_deref() {
            Some("Final") | Some("final") | Some("finished") | Some("") => true,
            None => false,
            _ => false,
        }
    }
}

impl MessageHandler<SportsLiveDataMessage> for SportsLiveDataStateHandler {
    fn handle(&mut self, message: SportsLiveDataMessage) -> hypersockets::Result<()> {
        if let SportsLiveDataMessage::GameUpdate(data) = message {
            let game_id = data.game_id;

            // 1. Check if already ignored
            if self.ignored_games.contains(&game_id) {
                return Ok(());
            }

            // 2. Skip games with no league abbreviation
            if data.league_abbreviation.is_empty() {
                debug!(game_id = game_id, "Skipping game with empty league abbreviation");
                return Ok(());
            }

            // 3. Check if game exists in any league
            let exists = self.shared_state.iter().any(|league| league.value().contains_key(&game_id));

            // 4. If first time seeing this game and should ignore, add to ignore set
            if !exists && Self::should_ignore(&data) {
                self.ignored_games.insert(game_id);
                debug!(
                    game_id = game_id,
                    league = %data.league_abbreviation,
                    status = ?data.status,
                    ended = data.ended,
                    "Ignoring finished game"
                );
                return Ok(());
            }

            // 5. Add/update game in league-specific map
            let league = data.league_abbreviation.clone();
            debug!(
                game_id = game_id,
                league = %league,
                home = data.home_team.as_deref().unwrap_or("?"),
                away = data.away_team.as_deref().unwrap_or("?"),
                score = %data.score,
                period = %data.period,
                status = data.status.as_deref().unwrap_or("?"),
                live = data.live,
                "Game update"
            );
            self.shared_state
                .entry(league)
                .or_insert_with(dashmap::DashMap::new)
                .insert(game_id, data);
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
async fn build_sports_ws_client() -> Result<WebSocketClient<SportsLiveDataRouter, SportsLiveDataMessage>>
{
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
/// Updates the provided SharedSportsLiveData instead of logging.
/// Finished games on first sight are added to ignored_games and skipped.
async fn build_sports_ws_client_with_state(
    shared_state: SharedSportsLiveData,
    ignored_games: IgnoredGames,
) -> Result<WebSocketClient<SportsLiveDataRouter, SportsLiveDataMessage>> {
    let local_shutdown_flag = Arc::new(AtomicBool::new(true));

    let router = SportsLiveDataRouter::new();
    let handler = SportsLiveDataStateHandler::new(shared_state, ignored_games);

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
pub async fn spawn_sports_live_data_tracker(
    shutdown_flag: Arc<AtomicBool>,
) -> Result<()> {
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
/// Connects to the sports WebSocket and updates the provided SharedSportsLiveData
/// with incoming game updates. Games that are already finished on first sight
/// are added to ignored_games. Runs until the shutdown flag is set to false.
pub async fn spawn_sports_tracker_with_state(
    shutdown_flag: Arc<AtomicBool>,
    shared_state: SharedSportsLiveData,
    ignored_games: IgnoredGames,
) -> Result<()> {
    info!("════════════════════════════════════════════════════════════════");
    info!("  STARTING SPORTS LIVE DATA TRACKER (State Mode)");
    info!("════════════════════════════════════════════════════════════════");
    info!("  URL: {}", SPORTS_WS_URL);
    info!("════════════════════════════════════════════════════════════════");

    let client = build_sports_ws_client_with_state(shared_state, ignored_games).await?;
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
