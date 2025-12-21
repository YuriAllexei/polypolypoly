use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::infrastructure::client::TradingClient;
use crate::infrastructure::config::SportsSnipingConfig;
use crate::infrastructure::{
    spawn_sports_tracker_with_state, BalanceManager, FetchedGames, FullTimeEvent, MarketsByGame,
};
use super::tracker::run_sports_market_tracker;
use async_trait::async_trait;
use crossbeam_channel::{unbounded, Receiver};
use dashmap::DashSet;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::task::JoinHandle;
use tracing::{error, info};

/// Sports Sniping strategy implementation
///
/// This strategy:
/// 1. Connects to the sports live data WebSocket
/// 2. Handler fetches and caches markets on first game message
/// 3. Receives FullTimeEvent when game reaches FT -> uses cached markets to react
pub struct SportsSnipingStrategy {
    config: SportsSnipingConfig,
    /// Cached markets per game_id (shared with handler, populated by handler)
    markets_cache: MarketsByGame,
    /// Shared set of game_ids for which we've fetched markets
    fetched_games: FetchedGames,
    /// Games already processed for FT (prevent duplicate handling)
    processed_games: DashSet<i64>,
    /// Receiver for Full Time events
    ft_rx: Option<Receiver<FullTimeEvent>>,
    /// Handle to the WebSocket tracker task
    ws_task: Option<JoinHandle<()>>,
    /// Trading client for order placement
    trading: Option<Arc<TradingClient>>,
    /// Balance manager for reading current balance
    balance_manager: Option<Arc<RwLock<BalanceManager>>>,
}

impl SportsSnipingStrategy {
    /// Create a new Sports Sniping strategy instance
    pub fn new(config: SportsSnipingConfig) -> Self {
        Self {
            config,
            markets_cache: Arc::new(dashmap::DashMap::new()),
            fetched_games: Arc::new(DashSet::new()),
            processed_games: DashSet::new(),
            ft_rx: None,
            ws_task: None,
            trading: None,
            balance_manager: None,
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
            order_pct = self.config.order_pct_of_collateral,
            bid_threshold = self.config.bid_threshold,
            "Initializing Sports Sniping strategy"
        );

        // Store trading client and balance manager for order placement
        self.trading = Some(Arc::clone(&ctx.trading));
        self.balance_manager = Some(Arc::clone(&ctx.balance_manager));

        // Create channel for FT events
        let (ft_tx, ft_rx) = unbounded::<FullTimeEvent>();
        self.ft_rx = Some(ft_rx);

        // Clone Arc references for the spawned task
        let shutdown_flag = ctx.shutdown.flag();
        let fetched_games = Arc::clone(&self.fetched_games);
        let markets_cache = Arc::clone(&self.markets_cache);
        let database = Arc::clone(&ctx.database);
        let runtime_handle = tokio::runtime::Handle::current();

        let task = tokio::spawn(async move {
            if let Err(e) = spawn_sports_tracker_with_state(
                shutdown_flag,
                fetched_games,
                markets_cache,
                database,
                runtime_handle,
                Some(ft_tx),
            )
            .await
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

        let poll_interval = StdDuration::from_millis(10);

        while ctx.is_running() {
            // Handle FT events - spawn market trackers
            if let Some(ref rx) = self.ft_rx {
                while let Ok(event) = rx.try_recv() {
                    // Skip if already processed
                    if self.processed_games.contains(&event.game_id) {
                        continue;
                    }

                    info!("═══════════════════════════════════════════════════");
                    info!("  🏁 GAME REACHED FULL TIME");
                    info!("═══════════════════════════════════════════════════");
                    info!(
                        "  Game {}: {} vs {} | Score: {} | Period: {} | Status: {}",
                        event.game_id,
                        event.home_team.as_deref().unwrap_or("?"),
                        event.away_team.as_deref().unwrap_or("?"),
                        event.final_score,
                        event.period,
                        event.status.as_deref().unwrap_or("?")
                    );

                    // Get cached markets for this game and spawn trackers
                    if let Some(markets) = self.markets_cache.get(&event.game_id) {
                        if markets.is_empty() {
                            info!("  No markets for this game");
                        } else {
                            info!("  Spawning {} market trackers for this game", markets.len());

                            for market in markets.value().iter() {
                                let market_clone = market.clone();
                                let event_clone = event.clone();
                                let shutdown_flag = ctx.shutdown.flag();
                                let trading = Arc::clone(self.trading.as_ref().unwrap());
                                let balance_manager =
                                    Arc::clone(self.balance_manager.as_ref().unwrap());
                                let order_pct = self.config.order_pct_of_collateral;
                                let bid_threshold = self.config.bid_threshold;

                                // Spawn a tracker task for each market
                                tokio::spawn(async move {
                                    if let Err(e) = run_sports_market_tracker(
                                        market_clone,
                                        event_clone,
                                        shutdown_flag,
                                        trading,
                                        balance_manager,
                                        order_pct,
                                        bid_threshold,
                                    )
                                    .await
                                    {
                                        error!("Sports market tracker error: {}", e);
                                    }
                                });
                            }
                        }
                    } else {
                        info!("  Markets not yet fetched for this game");
                    }
                    info!("═══════════════════════════════════════════════════");

                    // Mark as processed
                    self.processed_games.insert(event.game_id);
                }
            }

            // Brief sleep before next iteration
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
