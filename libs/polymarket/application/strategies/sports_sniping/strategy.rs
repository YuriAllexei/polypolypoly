//! Sports Sniping Strategy Implementation
//!
//! Monitors sports markets using real-time game data from the Polymarket
//! sports WebSocket API to identify and execute sniping opportunities.

use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::infrastructure::config::SportsSnipingConfig;
use crate::infrastructure::{spawn_sports_tracker_with_state, IgnoredGames, SharedSportsLiveData};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info};

/// Sports Sniping strategy implementation
///
/// This strategy:
/// 1. Connects to the sports live data WebSocket
/// 2. Matches incoming game updates to markets by game_id
/// 3. Analyzes game state to identify trading opportunities
/// 4. Places orders when conditions are met
pub struct SportsSnipingStrategy {
    config: SportsSnipingConfig,
    /// Shared state containing live game data (league -> game_id -> SportsLiveData)
    shared_state: Option<SharedSportsLiveData>,
    /// Set of ignored game IDs (finished games seen on first message)
    ignored_games: Option<IgnoredGames>,
    /// Handle to the WebSocket tracker task
    ws_task: Option<JoinHandle<()>>,
}

impl SportsSnipingStrategy {
    /// Create a new Sports Sniping strategy instance
    pub fn new(config: SportsSnipingConfig) -> Self {
        Self {
            config,
            shared_state: None,
            ignored_games: None,
            ws_task: None,
        }
    }
}

#[async_trait]
impl Strategy for SportsSnipingStrategy {
    fn name(&self) -> &str {
        "sports_sniping"
    }

    fn description(&self) -> &str {
        "Snipes sports markets using live game data from Polymarket sports WebSocket"
    }

    async fn initialize(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!(
            poll_interval_secs = self.config.poll_interval_secs,
            enabled = self.config.enabled,
            "Initializing Sports Sniping strategy"
        );

        // 1. Create shared state for game data (league -> game_id -> data)
        let shared_state: SharedSportsLiveData = Arc::new(DashMap::new());
        let ignored_games: IgnoredGames = Arc::new(DashSet::new());
        self.shared_state = Some(shared_state.clone());
        self.ignored_games = Some(ignored_games.clone());

        // 2. Spawn WebSocket tracker task
        let shutdown_flag = ctx.shutdown.flag();
        let task = tokio::spawn(async move {
            if let Err(e) = spawn_sports_tracker_with_state(shutdown_flag, shared_state, ignored_games).await {
                error!("Sports WS tracker error: {}", e);
            }
        });
        self.ws_task = Some(task);

        info!("Sports WebSocket tracker spawned");
        Ok(())
    }

    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()> {
        info!("Starting Sports Sniping strategy main loop");

        let shared_state = self
            .shared_state
            .as_ref()
            .expect("Strategy must be initialized before start");
        let ignored_games = self
            .ignored_games
            .as_ref()
            .expect("Strategy must be initialized before start");

        // Print state every 5 seconds
        let print_interval = Duration::from_secs(5);

        while ctx.is_running() {
            // Print state summary
            info!("═══════════════════════════════════════════════════");
            info!("  SPORTS LIVE DATA STATE");
            info!("═══════════════════════════════════════════════════");

            let ignored_count = ignored_games.len();
            info!("Ignored games: {}", ignored_count);

            // Count total games across all leagues
            let total_games: usize = shared_state.iter().map(|entry| entry.value().len()).sum();
            let league_count = shared_state.len();
            info!("Tracking {} games across {} leagues", total_games, league_count);

            // Iterate over leagues and their games
            for league_entry in shared_state.iter() {
                let league = league_entry.key();
                let games = league_entry.value();
                info!("League [{}]: {} games", league, games.len());

                for game_entry in games.iter() {
                    let game_id = *game_entry.key();
                    let data = game_entry.value();
                    info!(
                        "  Game {}: {} vs {} | Score: {} | Period: {} | Live: {}",
                        game_id,
                        data.home_team.as_deref().unwrap_or("?"),
                        data.away_team.as_deref().unwrap_or("?"),
                        data.score,
                        data.period,
                        data.live
                    );
                }
            }
            info!("═══════════════════════════════════════════════════");

            // Sleep before next iteration
            ctx.shutdown.interruptible_sleep(print_interval).await;
        }

        info!("Sports Sniping strategy loop ended (shutdown requested)");
        Ok(())
    }

    async fn stop(&mut self) -> StrategyResult<()> {
        info!("Stopping Sports Sniping strategy");

        // WebSocket task will stop when shutdown flag is set
        // Await the task to ensure clean shutdown
        if let Some(task) = self.ws_task.take() {
            info!("Waiting for WebSocket tracker to stop...");
            let _ = task.await;
            info!("WebSocket tracker stopped");
        }

        Ok(())
    }
}
