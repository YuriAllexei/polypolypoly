use anyhow::Result;
use polymarket::application::{init_logging, EventSyncApp, ConfigService};
use polymarket_arb_bot::bin_common::{load_config_from_env, ConfigType};
use std::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    
    let config_path = load_config_from_env(ConfigType::Bot);
    let config = ConfigService::load_bot_config(config_path.to_str().unwrap())?;

    let mut app = EventSyncApp::new(
        &config.database.url,
        &config.gamma_api.base_url,
        300,
    )
    .await?;

    const SYNC_INTERVAL_SECS: u64 = 60;

    print_banner("Polymarket Events Syncer", SYNC_INTERVAL_SECS);

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
            .interruptible_sleep(Duration::from_secs(SYNC_INTERVAL_SECS))
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
