use polymarket::strategy::{ExecutedTrade, MonitoredMarket, OrderExecutor, ResolutionMonitor, RiskConfig, RiskManager, TradingConfig};
use polymarket::config::BotConfig;
use chrono::{Duration, Utc};
use polymarket::filter::{LLMFilter, MarketInfo};
use polymarket::database::{MarketDatabase, MarketSyncService};
use polymarket::client::{GammaClient, PolymarketAuth};
use polymarket::client::clob::RestClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time;
use tracing::{debug, error, info, warn};
use serde_json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    info!("╔═══════════════════════════════════════════════════════════╗");
    info!("║       Polymarket Resolution Arbitrage Bot v0.1.0          ║");
    info!("╚═══════════════════════════════════════════════════════════╝");
    info!("");

    // Load configuration
    info!("📝 Loading configuration...");
    let config = BotConfig::load("config.yaml")?;
    info!("✅ Configuration loaded");

    // Initialize database
    info!("💾 Initializing database...");
    let database = Arc::new(MarketDatabase::new(&config.database.path).await?);
    let db_market_count = database.market_count().await?;
    let db_active_count = database.active_market_count().await?;
    info!("   Database: {} total markets ({} active)", db_market_count, db_active_count);

    // Initialize Gamma API client
    info!("🌐 Initializing Gamma API client...");
    let gamma_client = Arc::new(GammaClient::new(&config.gamma_api.base_url));
    info!("   Endpoint: {}", config.gamma_api.base_url);

    // Initialize market sync service
    info!("🔄 Setting up market synchronization...");
    let sync_service = Arc::new(MarketSyncService::new(
        Arc::clone(&gamma_client),
        Arc::clone(&database),
    ));

    // Perform initial sync if configured
    if config.gamma_api.initial_sync_on_startup {
        info!("🔄 Starting initial market sync (this may take a moment)...");
        let sync_stats = sync_service.initial_sync().await?;
        info!("✅ Initial sync complete:");
        info!("   Fetched: {} markets", sync_stats.markets_fetched);
        info!("   Inserted: {} markets", sync_stats.markets_inserted);
        info!("   Duration: {:?}", sync_stats.duration);
    }

    // Start background sync loop
    let sync_service_clone = Arc::clone(&sync_service);
    let sync_interval = std::time::Duration::from_secs(config.gamma_api.sync_interval_secs);
    tokio::spawn(async move {
        sync_service_clone.start_sync_loop(sync_interval).await;
    });
    info!("✅ Background sync started (interval: {}s)", config.gamma_api.sync_interval_secs);

    // Initialize Polymarket authentication
    info!("🔑 Initializing authentication...");
    let mut auth = PolymarketAuth::new(&config.private_key, config.polymarket.chain_id)?;
    info!("   Wallet: {}", config.wallet_address);

    // Create REST client
    let rest_client = Arc::new(RestClient::new(&config.polymarket.clob_url));

    // Get or create API credentials
    info!("🔐 Setting up API credentials...");
    let api_creds = rest_client.get_or_create_api_creds(&auth).await?;
    auth.set_api_key(api_creds);
    info!("✅ API credentials ready");

    let auth = Arc::new(auth);

    // Initialize LLM filter
    info!("🤖 Initializing LLM filter...");
    let mut llm_filter = LLMFilter::new(
        &config.llm.cache_file,
        &config.llm.endpoint,
        &config.llm.model,
        &config.llm.prompt,
    )?;

    // Check LLM health
    match llm_filter.health_check().await {
        Ok(true) => info!("✅ LLM ({}:) is ready", config.llm.model),
        Ok(false) => {
            error!("❌ LLM model {} not found. Please pull it first:", config.llm.model);
            error!("   docker exec -it polymarket-ollama ollama pull {}", config.llm.model);
            return Err(anyhow::anyhow!("LLM model not available"));
        }
        Err(e) => {
            error!("❌ Failed to connect to Ollama: {}", e);
            error!("   Make sure Ollama is running: docker-compose up -d");
            return Err(anyhow::anyhow!("Ollama not available"));
        }
    }

    let cache_stats = llm_filter.cache_stats();
    info!("   Cache: {} entries ({} compatible, {} incompatible)",
        cache_stats.total, cache_stats.compatible, cache_stats.incompatible);

    // Initialize risk manager
    info!("🛡️  Initializing risk manager...");
    let risk_config = RiskConfig {
        max_concurrent_positions: config.risk.max_concurrent_positions,
        max_bet_per_market: config.risk.max_bet_per_market,
        daily_loss_limit: config.risk.daily_loss_limit,
        min_profit_cents: config.risk.min_profit_cents,
    };
    let risk_manager = Arc::new(RiskManager::new(risk_config));
    info!("   Max positions: {}", config.risk.max_concurrent_positions);
    info!("   Max bet: ${:.2}", config.risk.max_bet_per_market);
    info!("   Daily loss limit: ${:.2}", config.risk.daily_loss_limit);
    info!("   Min profit: {:.0}¢", config.risk.min_profit_cents);

    // Initialize order executor
    let executor = OrderExecutor::new(
        Arc::clone(&rest_client),
        Arc::clone(&auth),
        Arc::clone(&risk_manager),
    );

    // Initialize resolution monitor
    let mut monitor = ResolutionMonitor::new();

    // Trading configuration
    let trading_config = TradingConfig {
        probability_threshold: config.trading.probability_threshold,
        bet_amount_usd: config.trading.bet_amount_usd,
    };

    info!("   Probability threshold: {:.1}%", config.trading.probability_threshold * 100.0);
    info!("   Bet amount: ${:.2}", config.trading.bet_amount_usd);
    info!("   Trade window: {}s before resolution", config.trading.seconds_before_resolution);

    // Track executed trades
    let mut active_trades: HashMap<String, ExecutedTrade> = HashMap::new();

    info!("");
    info!("🚀 Bot started! Scanning for opportunities...");
    info!("");

    // Main loop
    let mut last_scan = Utc::now() - Duration::seconds(999);
    let mut last_cleanup = Utc::now();

    loop {
        let now = Utc::now();

        // 1. Market Scanner (every poll_interval_secs)
        if (now - last_scan).num_seconds() >= config.scanner.poll_interval_secs as i64 {
            last_scan = now;

            info!("🔍 Scanning database for markets...");

            // Query markets from database instead of API
            let within_hours = (config.scanner.min_resolution_window_mins / 60) as u64;
            match database.get_upcoming_markets(within_hours).await {
                Ok(db_markets) => {
                    info!("   Found {} markets resolving within {} minutes",
                        db_markets.len(),
                        config.scanner.min_resolution_window_mins
                    );

                    if !db_markets.is_empty() {
                        // Filter markets not yet checked with LLM
                        let mut markets_to_check = Vec::new();

                        for db_market in &db_markets {
                            // Check if already in LLM cache
                            match database.get_llm_cache(&db_market.question).await {
                                Ok(Some(cache_entry)) => {
                                    // Already checked by LLM
                                    if cache_entry.compatible {
                                        debug!("Market already verified as compatible: {}", db_market.question);

                                        // Parse outcomes and token_ids from JSON
                                        if let Ok(token_ids) = serde_json::from_str::<Vec<String>>(&db_market.token_ids) {
                                            let monitored = MonitoredMarket {
                                                market_id: db_market.id.clone(),
                                                question: db_market.question.clone(),
                                                resolution_time: chrono::DateTime::parse_from_rfc3339(&db_market.resolution_time)
                                                    .ok()
                                                    .map(|dt| dt.with_timezone(&Utc))
                                                    .unwrap_or_else(Utc::now),
                                                token_ids,
                                            };
                                            monitor.add_market(monitored);
                                        }
                                    }
                                }
                                Ok(None) => {
                                    // Not checked yet - add to LLM queue
                                    markets_to_check.push(MarketInfo {
                                        id: db_market.id.clone(),
                                        question: db_market.question.clone(),
                                        resolution_time: chrono::DateTime::parse_from_rfc3339(&db_market.resolution_time)
                                            .ok()
                                            .map(|dt| dt.with_timezone(&Utc))
                                            .unwrap_or_else(Utc::now),
                                    });
                                }
                                Err(e) => {
                                    warn!("Failed to check LLM cache: {}", e);
                                }
                            }
                        }

                        // Filter new markets with LLM
                        if !markets_to_check.is_empty() {
                            info!("   Checking {} new markets with LLM...", markets_to_check.len());

                            match llm_filter.filter_markets(markets_to_check.clone()).await {
                                Ok(compatible_markets) => {
                                    info!("   LLM identified {} compatible markets", compatible_markets.len());

                                    // Cache results and add to monitor
                                    for market_info in &compatible_markets {
                                        // Find full market data
                                        if let Some(db_market) = db_markets.iter().find(|m| m.id == market_info.id) {
                                            // Store in LLM cache
                                            let cache_entry = polymarket::database::DbLLMCache {
                                                question: db_market.question.clone(),
                                                market_id: db_market.id.clone(),
                                                compatible: true,
                                                checked_at: Utc::now().to_rfc3339(),
                                                resolution_time: db_market.resolution_time.clone(),
                                            };

                                            if let Err(e) = database.insert_llm_cache(cache_entry).await {
                                                warn!("Failed to cache LLM result: {}", e);
                                            }

                                            // Parse token IDs and add to monitor
                                            if let Ok(token_ids) = serde_json::from_str::<Vec<String>>(&db_market.token_ids) {
                                                let monitored = MonitoredMarket {
                                                    market_id: db_market.id.clone(),
                                                    question: db_market.question.clone(),
                                                    resolution_time: market_info.resolution_time,
                                                    token_ids,
                                                };
                                                monitor.add_market(monitored);
                                            }
                                        }
                                    }

                                    // Cache incompatible markets too
                                    for market_info in &markets_to_check {
                                        if !compatible_markets.iter().any(|m| m.id == market_info.id) {
                                            if let Some(db_market) = db_markets.iter().find(|m| m.id == market_info.id) {
                                                let cache_entry = polymarket::database::DbLLMCache {
                                                    question: db_market.question.clone(),
                                                    market_id: db_market.id.clone(),
                                                    compatible: false,
                                                    checked_at: Utc::now().to_rfc3339(),
                                                    resolution_time: db_market.resolution_time.clone(),
                                                };

                                                if let Err(e) = database.insert_llm_cache(cache_entry).await {
                                                    warn!("Failed to cache LLM result: {}", e);
                                                }
                                            }
                                        }
                                    }

                                    info!("   Total markets being monitored: {}", monitor.market_count());
                                }
                                Err(e) => {
                                    warn!("Failed to filter markets with LLM: {}", e);
                                }
                            }
                        } else {
                            info!("   All markets already checked (using cache)");
                            info!("   Total markets being monitored: {}", monitor.market_count());
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to query database: {}", e);
                }
            }
        }

        // 2. Check for trading opportunities
        let upcoming = monitor.get_upcoming_markets(config.trading.seconds_before_resolution + 5);

        for market in upcoming {
            // Check if it's time to trade
            if monitor.should_trade(&market.market_id, config.trading.seconds_before_resolution) {
                // Skip if we already have an active trade on this market
                if active_trades.contains_key(&market.market_id) {
                    continue;
                }

                info!("⏰ Trade window open for: {}", market.question);
                info!("   Resolves at: {}", market.resolution_time);

                // Try to execute
                match executor.try_execute(&market, &trading_config).await {
                    Ok(Some(trade)) => {
                        info!("🎉 Trade executed!");
                        active_trades.insert(market.market_id.clone(), trade);
                    }
                    Ok(None) => {
                        debug!("No profitable opportunity found");
                    }
                    Err(e) => {
                        warn!("Failed to execute trade: {}", e);
                    }
                }

                // Remove from monitor (already traded or skipped)
                monitor.remove_market(&market.market_id);
            }
        }

        // 3. Check for resolved markets and close positions
        // TODO: Implement position monitoring and resolution checking
        // For now, we'd need to poll positions and check if markets resolved

        // 4. Cleanup (every hour)
        if (now - last_cleanup).num_hours() >= 1 {
            last_cleanup = now;

            info!("🧹 Running cleanup...");

            // Remove resolved markets from monitor
            monitor.cleanup_resolved();

            // Cleanup old cache entries (>7 days)
            if let Err(e) = llm_filter.cleanup_cache(Duration::days(7)) {
                warn!("Failed to cleanup cache: {}", e);
            }

            let stats = risk_manager.daily_stats();
            info!("📊 Daily Stats:");
            info!("   Trades: {} ({} wins, {} losses, {:.1}% win rate)",
                stats.total_trades, stats.winning_trades, stats.losing_trades, stats.win_rate);
            info!("   P&L: ${:.2}", stats.total_pnl);
            info!("   Open positions: {}", risk_manager.open_positions());
        }

        // Sleep briefly before next iteration
        time::sleep(time::Duration::from_secs(1)).await;
    }
}
