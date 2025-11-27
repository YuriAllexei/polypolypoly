use anyhow::Result;
use polymarket::application::{init_logging, to_sniper_market, SniperApp, ConfigService};
use polymarket_arb_bot::bin_common::{load_config_from_env, ConfigType};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    let config_path = load_config_from_env(ConfigType::Sniper);
    let config = ConfigService::load_sniper_config(config_path.to_str().unwrap())?;
    config.log();

    let mut app = SniperApp::new(
        &config.database.url,
        300,
        config.probability,
    ).await?;

    let mut logged_market_ids = HashSet::new();
    let mut iteration = 0u64;

    print_banner("Market Sniper (Continuous Mode)");

    while app.shutdown.is_running() {
        iteration += 1;
        let markets = app
            .database
            .get_markets_expiring_soon(config.delta_t_seconds)
            .await?;
        let mut new_markets_found = 0;

        for market in markets {
            if logged_market_ids.insert(market.id.clone()) {
                new_markets_found += 1;

                if let Ok(sniper_market) = to_sniper_market(&market) {
                    sniper_market.log(iteration);

                    if sniper_market.can_spawn_tracker() {
                        spawn_tracker_for_market(&sniper_market, &app).await;
                    }
                }
            }
        }

        handle_heartbeat(&mut app, new_markets_found, iteration, logged_market_ids.len());

        app.shutdown
            .interruptible_sleep(Duration::from_secs_f64(config.loop_interval_secs))
            .await;
    }

    print_shutdown("Market Sniper", Some(&format!(
        "Total unique markets tracked: {}",
        logged_market_ids.len()
    )));

    Ok(())
}

async fn spawn_tracker_for_market(
    sniper_market: &polymarket::domain::SniperMarket,
    app: &SniperApp,
) {
    let market = sniper_market.clone();
    let flag = app.shutdown.flag();
    let tracker = app.tracker.clone();
    let event_id = app
        .database
        .get_market_event_id(&sniper_market.id)
        .await
        .ok()
        .flatten();

    tokio::spawn(async move {
        if let Err(e) = tracker.track_market(&market, flag, event_id).await {
            warn!("[WS {}] Tracker stopped: {}", market.id, e);
        }
    });
}

fn handle_heartbeat(
    app: &mut SniperApp,
    new_markets_found: usize,
    iteration: u64,
    total_markets: usize,
) {
    if new_markets_found == 0 && app.heartbeat.should_beat() {
        info!(
            "Heartbeat (Iteration #{}): No new markets in 5 minutes (tracking {} markets)",
            iteration, total_markets
        );
        app.heartbeat.beat();
    } else if new_markets_found > 0 {
        app.heartbeat.reset();
    }
}

fn print_banner(name: &str) {
    info!("");
    info!("========================================");
    info!("Starting {}", name);
    info!("Press Ctrl+C to stop");
    info!("========================================");
    info!("");
}

fn print_shutdown(name: &str, stats: Option<&str>) {
    info!("");
    info!("========================================");
    info!("{} stopped gracefully", name);
    if let Some(stats) = stats {
        info!("{}", stats);
    }
    info!("========================================");
}
