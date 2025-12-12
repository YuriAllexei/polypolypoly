use anyhow::Result;
use polymarket::application::{init_logging_with_level, ConfigService, EventSyncApp};
use polymarket_arb_bot::bin_common::{load_config_from_env, ConfigType};
use std::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Load config first (before logging is initialized)
    let config_path = load_config_from_env(ConfigType::Events);
    let config = ConfigService::load_events_config(config_path.to_str().unwrap())?;

    // Initialize logging with configured level
    init_logging_with_level(&config.log_level);
    config.log();

    let mut app = EventSyncApp::new(
        &config.database.url,
        &config.gamma_api_url,
        300,
        config.closed,
    )
    .await?;

    let sync_interval = config.sync_interval_secs;

    print_banner("Polymarket Events Syncer", sync_interval);

    // Run sync loop
    while app.shutdown.is_running() {
        match app.sync_all_events().await {
            Ok(count) => {
                if count > 0 {
                    info!("Successfully synced {} events", count);
                    app.heartbeat.reset();
                } else if app.heartbeat.should_beat() {
                    info!("Heartbeat: No new events in last 5 minutes");
                    app.heartbeat.beat();
                }
            }
            Err(e) => {
                tracing::error!("Error during sync: {}", e);
            }
        }

        app.shutdown
            .interruptible_sleep(Duration::from_secs(sync_interval))
            .await;
    }

    print_shutdown("Event syncer");
    Ok(())
}

fn print_banner(name: &str, interval_secs: u64) {
    info!("");
    info!("========================================");
    info!("Starting {}", name);
    info!("Sync interval: {}s", interval_secs);
    info!("Press Ctrl+C to stop");
    info!("========================================");
    info!("");
}

fn print_shutdown(name: &str) {
    info!("");
    info!("========================================");
    info!("{} stopped gracefully", name);
    info!("========================================");
}
