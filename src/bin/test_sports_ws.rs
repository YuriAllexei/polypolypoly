//! Test binary for Sports Live Data WebSocket
//!
//! Connects to the Polymarket sports API WebSocket and logs incoming
//! game updates for testing purposes.

use anyhow::Result;
use polymarket::infrastructure::{init_tracing, spawn_sports_live_data_tracker, ShutdownManager};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let shutdown = ShutdownManager::new();
    shutdown.spawn_signal_handler();

    println!("Starting Sports Live Data WebSocket tracker...");
    println!("Press Ctrl+C to stop\n");

    spawn_sports_live_data_tracker(shutdown.flag()).await?;

    println!("Shutdown complete");
    Ok(())
}
