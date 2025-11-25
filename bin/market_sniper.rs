use anyhow::Result;
use polymarket::client::clob::spawn_market_tracker;
use polymarket::config::SniperConfig;
use polymarket::database::MarketDatabase;
use polymarket::sniper::SniperMarket;
use polymarket::utils::{init_tracing, Heartbeat, ShutdownManager};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    info!("Starting Market Sniper (Continuous Mode)");

    let config = SniperConfig::load("config/sniper_config.yaml")?;
    config.log();

    let db = Arc::new(MarketDatabase::new(&config.database.path).await?);
    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();
    let probability_threshold = config.probability;

    let mut logged_market_ids = HashSet::new();
    let mut iteration = 0u64;
    let mut heartbeat = Heartbeat::new(300);

    info!("");
    info!("========================================");
    info!("Starting continuous monitoring");
    info!("Press Ctrl+C to stop");
    info!("========================================");
    info!("");

    while shutdown.is_running() {
        iteration += 1;
        let markets = db.get_markets_expiring_soon(config.delta_t_seconds).await?;
        let mut new_markets_found = 0;

        for market in markets {
            if logged_market_ids.insert(market.id.clone()) {
                new_markets_found += 1;

                if let Ok(sniper_market) = SniperMarket::from_db_market(&market) {
                    sniper_market.log(iteration);

                    if sniper_market.can_spawn_tracker() {
                        let id = sniper_market.id.clone();
                        let tokens = sniper_market.token_ids.clone();
                        let outcomes = sniper_market.outcomes.clone();
                        let res_time = sniper_market.resolution_time_str.clone();
                        let flag = shutdown.flag();
                        let db_clone = Arc::clone(&db);
                        let event_id = db.get_market_event_id(&sniper_market.id).await.ok().flatten();

                        tokio::spawn(async move {
                            if let Err(e) = spawn_market_tracker(
                                id.clone(),
                                tokens,
                                outcomes,
                                res_time,
                                flag,
                                db_clone,
                                probability_threshold,
                                event_id,
                            ).await {
                                warn!("[WS {}] Tracker stopped: {}", id, e);
                            }
                        });
                    }
                }
            }
        }

        if new_markets_found == 0 && heartbeat.should_beat() {
            info!("Heartbeat (Iteration #{}): No new markets in 5 minutes (tracking {} markets)",
                  iteration, logged_market_ids.len());
            heartbeat.beat();
        } else if new_markets_found > 0 {
            heartbeat.reset();
        }

        shutdown.interruptible_sleep(Duration::from_secs_f64(config.loop_interval_secs)).await;
    }

    info!("");
    info!("========================================");
    info!("Market Sniper stopped gracefully");
    info!("Total unique markets tracked: {}", logged_market_ids.len());
    info!("========================================");

    Ok(())
}
