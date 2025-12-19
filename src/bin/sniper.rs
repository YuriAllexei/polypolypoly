//! Sniper Binary - Pluggable Strategy Runner
//!
//! Runs market monitoring strategies with graceful shutdown support.
//!
//! Usage:
//!   STRATEGY_NAME=up_or_down ./sniper   # Via environment variable (Docker)
//!   ./sniper up_or_down                 # Via CLI argument

use anyhow::{bail, Result};
use polymarket::application::{
    create_strategy, init_logging_with_level, BalanceManager, Strategy, StrategyContext, StrategyType,
};
use std::sync::RwLock;
use polymarket::infrastructure::client::clob::TradingClient;
use polymarket::infrastructure::config::StrategiesConfig;
use polymarket::infrastructure::database::MarketDatabase;
use polymarket::infrastructure::shutdown::ShutdownManager;
use polymarket_arb_bot::bin_common::{load_config_from_env, parse_args, ConfigType};
use std::sync::Arc;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Load config
    let config_path = load_config_from_env(ConfigType::Strategies);
    let config = StrategiesConfig::load(&config_path)?;

    // Initialize logging
    init_logging_with_level(&config.log_level);
    config.log();

    // Get database URL from environment
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL environment variable is required"))?;

    // Determine which strategy to run
    // Priority: STRATEGY_NAME env var > CLI arg
    let args = parse_args();
    let strategy_name = if let Ok(name) = std::env::var("STRATEGY_NAME") {
        info!("Strategy from STRATEGY_NAME env var: {}", name);
        name
    } else if let Some(name) = args.first() {
        info!("Strategy from CLI argument: {}", name);
        name.clone()
    } else {
        let available = StrategyType::available().join(", ");
        bail!(
            "No strategy specified. Use STRATEGY_NAME env var or CLI argument.\nAvailable strategies: {}",
            available
        );
    };

    // Parse strategy type
    let strategy_type = match StrategyType::from_str(&strategy_name) {
        Some(t) => t,
        None => {
            let available = StrategyType::available().join(", ");
            bail!(
                "Unknown strategy: '{}'. Available strategies: {}",
                strategy_name,
                available
            );
        }
    };

    // Create strategy
    let mut strategy: Box<dyn Strategy> = create_strategy(&strategy_type, &config);

    print_banner(strategy.name(), strategy.description());

    // Initialize infrastructure
    let shutdown = Arc::new(ShutdownManager::new());
    shutdown.spawn_signal_handler();
    let database = Arc::new(MarketDatabase::new(&database_url).await?);

    // Initialize trading client (loads credentials from env)
    info!("Initializing trading client...");
    let trading = Arc::new(TradingClient::from_env().await?);
    info!(
        "Trading client initialized: signer={:?}, maker={:?}",
        trading.signer_address(),
        trading.maker_address()
    );

    // Initialize balance manager with configured threshold
    info!("Initializing balance manager...");
    let mut balance_manager = BalanceManager::new(config.components.balance_manager.threshold);
    balance_manager
        .start(Arc::clone(&trading), shutdown.flag())
        .await?;
    let balance_manager = Arc::new(RwLock::new(balance_manager));

    // Create strategy context
    let ctx = StrategyContext::new(database, shutdown.clone(), trading, balance_manager.clone());

    // Run strategy lifecycle
    info!("Initializing strategy: {}", strategy.name());
    if let Err(e) = strategy.initialize(&ctx).await {
        error!("Strategy initialization failed: {}", e);
        return Err(e.into());
    }

    info!("Starting strategy: {}", strategy.name());
    if let Err(e) = strategy.start(&ctx).await {
        error!("Strategy execution failed: {}", e);
        // Still try to stop gracefully
    }

    info!("Stopping strategy: {}", strategy.name());
    if let Err(e) = strategy.stop().await {
        error!("Strategy stop failed: {}", e);
    }

    // Stop balance manager
    balance_manager.write().unwrap().stop().await;

    print_shutdown(strategy.name());
    Ok(())
}

fn print_banner(name: &str, description: &str) {
    info!("");
    info!("========================================");
    info!("Sniper - Strategy Runner");
    info!("  Strategy: {}", name);
    info!("  Description: {}", description);
    info!("  Press Ctrl+C to stop");
    info!("========================================");
    info!("");
}

fn print_shutdown(name: &str) {
    info!("");
    info!("========================================");
    info!("Strategy '{}' stopped gracefully", name);
    info!("========================================");
}
