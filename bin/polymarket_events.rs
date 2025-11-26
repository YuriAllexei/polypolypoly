use anyhow::Result;
use polymarket::application::sync::EventSyncService;
use polymarket::database::MarketDatabase;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_line_number(false)
        .init();

    info!("Starting Polymarket Events Syncer");

    // Load configuration
    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.yaml".to_string());
    let config = polymarket::config::BotConfig::load(&config_path)?;

    // Initialize database
    info!("Initializing database: {}", config.database.url);
    let db = MarketDatabase::new(&config.database.url).await?;
    let db = Arc::new(db);

    // Initialize Sync Service
    let api_base_url = config.gamma_api.base_url.clone();
    let sync_service = Arc::new(EventSyncService::new(db.clone(), api_base_url));

    const SYNC_INTERVAL_SECS: u64 = 60; // Check for new events every 60 seconds

    info!(
        "Starting perpetual event sync (interval: {}s)",
        SYNC_INTERVAL_SECS
    );
    info!("Press Ctrl+C to stop");

    // Setup graceful shutdown
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("");
                info!("Received shutdown signal (Ctrl+C)");
                info!("Finishing current cycle and shutting down gracefully...");
                shutdown_clone.store(true, Ordering::SeqCst);
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    // Run sync loop
    sync_service.start_sync_loop(SYNC_INTERVAL_SECS, shutdown).await;

    info!("Event syncer stopped gracefully");
    Ok(())
}
