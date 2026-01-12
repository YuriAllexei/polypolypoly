//! Quick test to verify ChainLink oracle connection
//!
//! Run with: cargo run --bin test-oracle

use polymarket::infrastructure::{spawn_oracle_trackers, OracleType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,polymarket=debug")
        .init();

    info!("════════════════════════════════════════════════════════════════");
    info!("ChainLink Oracle Connection Test");
    info!("════════════════════════════════════════════════════════════════");

    // Check env vars
    let client_id = std::env::var("CHAINLINK_CLIENT_ID");
    let secret = std::env::var("STREAMS_SECRET");

    match (&client_id, &secret) {
        (Ok(_), Ok(_)) => info!("✅ Environment variables are set"),
        _ => {
            warn!("❌ Missing environment variables:");
            if client_id.is_err() {
                warn!("   - CHAINLINK_CLIENT_ID not set");
            }
            if secret.is_err() {
                warn!("   - STREAMS_SECRET not set");
            }
            return Ok(());
        }
    }

    // Create shutdown flag
    let shutdown_flag = Arc::new(AtomicBool::new(true));
    let shutdown_clone = Arc::clone(&shutdown_flag);

    // Spawn oracle trackers
    info!("Spawning oracle trackers...");
    let prices = spawn_oracle_trackers(shutdown_flag).await?;

    info!("Waiting for price updates (10 seconds)...");
    info!("");

    // Poll for prices every second for 10 seconds
    for i in 1..=10 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let price_manager = prices.read();

        // Check ChainLink prices
        let btc_cl = price_manager.get_price(OracleType::ChainLink, "BTC");
        let eth_cl = price_manager.get_price(OracleType::ChainLink, "ETH");
        let sol_cl = price_manager.get_price(OracleType::ChainLink, "SOL");

        // Check Binance prices
        let btc_bn = price_manager.get_price(OracleType::Binance, "BTC");
        let eth_bn = price_manager.get_price(OracleType::Binance, "ETH");

        info!("─── Second {} ───", i);

        if let Some(p) = btc_cl {
            info!("  ChainLink BTC: ${:.2} (age: {:.1}s)", p.value, p.received_at.elapsed().as_secs_f64());
        }
        if let Some(p) = eth_cl {
            info!("  ChainLink ETH: ${:.2} (age: {:.1}s)", p.value, p.received_at.elapsed().as_secs_f64());
        }
        if let Some(p) = sol_cl {
            info!("  ChainLink SOL: ${:.2} (age: {:.1}s)", p.value, p.received_at.elapsed().as_secs_f64());
        }
        if let Some(p) = btc_bn {
            info!("  Binance   BTC: ${:.2} (age: {:.1}s)", p.value, p.received_at.elapsed().as_secs_f64());
        }
        if let Some(p) = eth_bn {
            info!("  Binance   ETH: ${:.2} (age: {:.1}s)", p.value, p.received_at.elapsed().as_secs_f64());
        }

        if btc_cl.is_none() && eth_cl.is_none() && btc_bn.is_none() {
            info!("  (no prices yet...)");
        }
    }

    info!("");
    info!("════════════════════════════════════════════════════════════════");
    info!("Test complete. Shutting down...");
    info!("════════════════════════════════════════════════════════════════");

    // Trigger shutdown
    shutdown_clone.store(false, Ordering::Release);
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(())
}
