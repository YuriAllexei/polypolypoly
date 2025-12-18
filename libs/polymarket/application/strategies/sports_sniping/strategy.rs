//! Sports Sniping Strategy Implementation
//!
//! Monitors sports markets using real-time game data from the Polymarket
//! sports WebSocket API to identify and execute sniping opportunities.

use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::domain::DbMarket;
use crate::infrastructure::config::SportsSnipingConfig;
use crate::infrastructure::{spawn_sports_tracker_with_state, IgnoredGames, SharedSportsLiveData};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

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
    /// Cached markets per game_id (fetched once when game first appears)
    game_markets: Arc<DashMap<i64, Vec<DbMarket>>>,
    /// Games that have reached Full Time (FT/VFT) - logged once
    ft_games: Arc<DashSet<i64>>,
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
            game_markets: Arc::new(DashMap::new()),
            ft_games: Arc::new(DashSet::new()),
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
            if let Err(e) =
                spawn_sports_tracker_with_state(shutdown_flag, shared_state, ignored_games).await
            {
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

        // Fast polling for speed - check every 10ms
        let poll_interval = Duration::from_millis(10);

        while ctx.is_running() {
            // Fetch markets for any new games
            // Collect game_ids to process (to avoid holding iterator while modifying)
            let mut game_ids_to_check: Vec<(String, i64)> = Vec::new();
            for league_entry in shared_state.iter() {
                let league = league_entry.key().clone();
                for game_entry in league_entry.value().iter() {
                    let game_id = *game_entry.key();
                    if !self.game_markets.contains_key(&game_id) {
                        game_ids_to_check.push((league.clone(), game_id));
                    }
                }
            }

            for (league, game_id) in game_ids_to_check {
                match ctx.database.get_markets_by_game_id(game_id).await {
                    Ok(markets) => {
                        if markets.is_empty() {
                            // No markets for this game - remove from tracking and ignore
                            debug!(
                                game_id = game_id,
                                league = %league,
                                "Game has no markets - removing from tracking and adding to ignore set"
                            );
                            // Remove from shared state
                            if let Some(league_games) = shared_state.get(&league) {
                                league_games.remove(&game_id);
                            }
                            // Add to ignore set
                            ignored_games.insert(game_id);
                        } else {
                            info!(
                                game_id = game_id,
                                market_count = markets.len(),
                                "Fetched markets for game"
                            );
                            self.game_markets.insert(game_id, markets);
                        }
                    }
                    Err(e) => {
                        warn!(game_id = game_id, error = %e, "Failed to fetch markets for game");
                        // Insert empty vec to avoid retrying
                        self.game_markets.insert(game_id, vec![]);
                    }
                }
            }

            // Check for games that reached Full Time
            for league_entry in shared_state.iter() {
                for game_entry in league_entry.value().iter() {
                    let game_id = *game_entry.key();
                    let data = game_entry.value();

                    // Check if period indicates Full Time and not already tracked
                    let is_ft = data.period == "FT" || data.period == "VFT";
                    if is_ft && !self.ft_games.contains(&game_id) {
                        // Log the FT event
                        info!("═══════════════════════════════════════════════════");
                        info!("  🏁 GAME REACHED FULL TIME");
                        info!("═══════════════════════════════════════════════════");
                        info!(
                            "  Game {}: {} vs {} | Score: {} | Period: {} | Status: {}",
                            game_id,
                            data.home_team.as_deref().unwrap_or("?"),
                            data.away_team.as_deref().unwrap_or("?"),
                            data.score,
                            data.period,
                            data.status.as_deref().unwrap_or("?")
                        );

                        // Log associated markets
                        if let Some(markets) = self.game_markets.get(&game_id) {
                            info!("  Markets for this game:");
                            for market in markets.value().iter() {
                                let market_url = market
                                    .slug
                                    .as_ref()
                                    .map(|s| format!("https://polymarket.com/event/{}", s))
                                    .unwrap_or_else(|| "N/A".to_string());
                                info!(
                                    "    - {} | Active: {} | End: {} | URL: {}",
                                    market.question, market.active, market.end_date, market_url
                                );
                            }
                        }
                        info!("═══════════════════════════════════════════════════");

                        // Add to ft_games to avoid repeated logging
                        self.ft_games.insert(game_id);
                    }
                }
            }

            // Sleep before next iteration
            ctx.shutdown.interruptible_sleep(poll_interval).await;
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
