use crate::application::strategies::traits::{Strategy, StrategyContext, StrategyResult};
use crate::domain::DbMarket;
use crate::infrastructure::config::SportsSnipingConfig;
use crate::infrastructure::{
    build_ws_client, spawn_sports_tracker_with_state, FetchedGames, FullTimeEvent,
    MarketTrackerConfig, MarketsByGame, SharedOrderbooks, SharedPrecisions,
};
use async_trait::async_trait;
use crossbeam_channel::{unbounded, Receiver};
use dashmap::DashSet;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration as StdDuration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{error, info, warn};

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
        }
    }
}

// =============================================================================
// Market Tracker Types and Functions
// =============================================================================

/// Result of analyzing orderbooks to determine the winning token
#[derive(Debug)]
struct WinnerAnalysis {
    token_id: String,
    outcome_name: String,
    best_bid: Option<(f64, f64)>, // (price, size)
    has_asks: bool,
    confidence: f64, // 0.0 - 1.0
}

/// Analyze orderbooks to find the likely winning token
///
/// Winner criteria:
/// - Highest bid price
/// - Preferably no asks (market makers pulled out)
/// - High confidence if bid > 0.90 and no asks
fn analyze_orderbooks_for_winner(
    orderbooks: &SharedOrderbooks,
    token_ids: &[String],
    outcomes: &[String],
) -> Option<WinnerAnalysis> {
    let obs = orderbooks.read().unwrap();
    let mut best_candidate: Option<WinnerAnalysis> = None;

    for (token_id, outcome) in token_ids.iter().zip(outcomes.iter()) {
        if let Some(ob) = obs.get(token_id) {
            let best_bid = ob.best_bid();
            let has_asks = !ob.asks.is_empty();

            // Winner criteria: highest bid price
            let is_better = match &best_candidate {
                None => true,
                Some(current) => match (best_bid, current.best_bid) {
                    (Some((price, _)), Some((curr_price, _))) => price > curr_price,
                    (Some(_), None) => true,
                    _ => false,
                },
            };

            if is_better {
                // Calculate confidence based on bid price and ask presence
                let confidence = match best_bid {
                    Some((price, _)) if !has_asks && price > 0.90 => 1.0,
                    Some((price, _)) if !has_asks && price > 0.70 => 0.8,
                    Some((price, _)) if price > 0.90 => 0.7,
                    Some(_) => 0.5,
                    None => 0.1,
                };

                best_candidate = Some(WinnerAnalysis {
                    token_id: token_id.clone(),
                    outcome_name: outcome.clone(),
                    best_bid,
                    has_asks,
                    confidence,
                });
            }
        }
    }

    best_candidate
}

/// Log the winning token analysis
fn log_winning_token(market: &DbMarket, event: &FullTimeEvent, winner: &Option<WinnerAnalysis>) {
    let market_url = market
        .slug
        .as_ref()
        .map(|s| format!("https://polymarket.com/event/{}", s))
        .unwrap_or_else(|| "N/A".to_string());

    info!("════════════════════════════════════════════════════════════════");
    info!("  🏆 WINNER ANALYSIS - GAME ENDED");
    info!("════════════════════════════════════════════════════════════════");
    info!(
        "  Game: {} vs {}",
        event.home_team.as_deref().unwrap_or("?"),
        event.away_team.as_deref().unwrap_or("?")
    );
    info!("  Final Score: {}", event.final_score);
    info!("  Market: {}", market.question);
    info!("  URL: {}", market_url);

    match winner {
        Some(w) => {
            info!("  ─────────────────────────────────────────────────────────────");
            info!("  Predicted Winner: {}", w.outcome_name);
            info!("  Token ID: {}", w.token_id);
            if let Some((price, size)) = w.best_bid {
                info!("  Best Bid: ${:.4} x {:.2}", price, size);
            } else {
                info!("  Best Bid: None");
            }
            info!("  Has Asks: {}", w.has_asks);
            info!("  Confidence: {:.0}%", w.confidence * 100.0);
        }
        None => {
            info!("  Could not determine winner from orderbooks");
        }
    }
    info!("════════════════════════════════════════════════════════════════");
}

/// Run a market tracker for a single sports market
///
/// Connects to the orderbook WebSocket, waits for snapshot,
/// analyzes orderbooks to find the winning token, and logs the result.
async fn run_sports_market_tracker(
    market: DbMarket,
    event: FullTimeEvent,
    shutdown_flag: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    // Parse market data
    let token_ids = market.parse_token_ids()?;
    let outcomes = market.parse_outcomes()?;

    if token_ids.len() < 2 {
        warn!(
            "[Sports Tracker] Market {} has fewer than 2 tokens, skipping",
            market.id
        );
        return Ok(());
    }

    info!(
        "[Sports Tracker] Starting tracker for market: {} ({})",
        market.question, market.id
    );

    // Build WebSocket configuration
    let ws_config = MarketTrackerConfig::new(
        market.id.clone(),
        market.question.clone(),
        market.slug.clone(),
        token_ids.clone(),
        outcomes.clone(),
        &market.end_date,
    )?;

    // Create shared orderbooks and precisions
    let orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));
    let precisions: SharedPrecisions = Arc::new(RwLock::new(HashMap::new()));
    let first_snapshot_received = Arc::new(AtomicBool::new(false));

    // Connect to WebSocket
    let client = match build_ws_client(
        &ws_config,
        Arc::clone(&orderbooks),
        Arc::clone(&precisions),
        None, // No tick_size_tx needed for now
        Arc::clone(&first_snapshot_received),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            error!(
                "[Sports Tracker] Failed to connect to WS for market {}: {}",
                market.id, e
            );
            return Err(e);
        }
    };

    info!(
        "[Sports Tracker] Connected to orderbook WS for market {}",
        market.id
    );

    // Wait for first snapshot (with timeout)
    let start = std::time::Instant::now();
    let snapshot_received = loop {
        if first_snapshot_received.load(Ordering::Acquire) {
            break true;
        }
        if start.elapsed() > StdDuration::from_secs(10) {
            error!(
                "[Sports Tracker] Timeout waiting for snapshot on market {}",
                market.id
            );
            break false;
        }
        if !shutdown_flag.load(Ordering::Acquire) {
            info!(
                "[Sports Tracker] Shutdown during snapshot wait for market {}",
                market.id
            );
            let _ = client.shutdown().await;
            return Ok(());
        }
        sleep(StdDuration::from_millis(50)).await;
    };

    if !snapshot_received {
        let _ = client.shutdown().await;
        return Ok(());
    }

    // Give a brief moment for orderbooks to populate fully
    sleep(StdDuration::from_millis(200)).await;

    // Analyze orderbooks to find winner
    let winner = analyze_orderbooks_for_winner(&orderbooks, &token_ids, &outcomes);

    // Log the result
    log_winning_token(&market, &event, &winner);

    // Cleanup
    if let Err(e) = client.shutdown().await {
        warn!(
            "[Sports Tracker] Error shutting down WS for market {}: {}",
            market.id, e
        );
    }

    info!(
        "[Sports Tracker] Tracker completed for market {}",
        market.id
    );
    Ok(())
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

                                // Spawn a tracker task for each market
                                tokio::spawn(async move {
                                    if let Err(e) = run_sports_market_tracker(
                                        market_clone,
                                        event_clone,
                                        shutdown_flag,
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
