use anyhow::Result;
use chrono::Utc;
use polymarket::client::clob::RestClient;
use polymarket::config::SniperConfig;
use polymarket::database::MarketDatabase;
use serde_json;
use std::collections::HashSet;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_line_number(false)
        .init();

    info!("Starting Market Sniper (Continuous Mode)");

    // Load configuration
    let config = SniperConfig::load("config/sniper_config.yaml")?;

    info!("Configuration loaded:");
    info!("  Probability threshold: {}", config.probability);
    info!("  Time window: {} seconds", config.delta_t_seconds);
    info!("  Loop interval: {} seconds", config.loop_interval_secs);
    info!("  Database path: {}", config.database.path);

    // Initialize database
    info!("Connecting to database: {}", config.database.path);
    let db = MarketDatabase::new(&config.database.path).await?;
    info!("Database connected successfully");

    // Initialize CLOB client for orderbook fetching
    let clob_client = RestClient::new("https://clob.polymarket.com");

    // HashSet to track logged market IDs
    let mut logged_market_ids: HashSet<String> = HashSet::new();
    let mut iteration = 0;
    let mut last_heartbeat = Utc::now();

    info!("");
    info!("========================================");
    info!("Starting continuous monitoring");
    info!("Press Ctrl+C to stop");
    info!("========================================");
    info!("");

    // Continuous monitoring loop with graceful shutdown
    loop {
        iteration += 1;

        // Fetch markets expiring soon
        let markets = db.get_markets_expiring_soon(config.delta_t_seconds).await?;

        // Filter for new markets only
        let mut new_markets_found = 0;

        for market in markets {
            // Only process markets we haven't logged before
            if !logged_market_ids.contains(&market.id) {
                new_markets_found += 1;

                // Calculate resolution time and time until resolution
                let resolution_time = market.resolution_datetime()
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|_| "Invalid date".to_string());

                let now = Utc::now();
                let time_until_resolution = market.resolution_datetime()
                    .ok()
                    .map(|dt| {
                        let duration = dt.signed_duration_since(now);
                        if duration.num_seconds() > 0 {
                            format!("{} seconds", duration.num_seconds())
                        } else {
                            "Expired".to_string()
                        }
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                // Parse token IDs
                let token_ids = market.parse_token_ids()
                    .unwrap_or_else(|_| vec![]);

                // Parse outcomes - it's double-encoded, so parse twice
                let outcomes: Vec<String> = serde_json::from_str::<String>(&market.outcomes)
                    .ok()
                    .and_then(|inner_string| serde_json::from_str(&inner_string).ok())
                    .unwrap_or_else(|| vec![]);

                // Fetch event information for this market
                let event_info = match db.get_market_event_id(&market.id).await {
                    Ok(Some(event_id)) => {
                        // Fetch event details
                        db.get_event(&event_id).await.ok()
                    }
                    _ => None,
                };

                // Log the new market
                info!("========================================");
                info!("NEW MARKET FOUND (Iteration #{})", iteration);
                info!("========================================");
                info!("  ID: {}", market.id);
                info!("  Question: {}", market.question);
                info!("  Resolution Time: {}", resolution_time);
                info!("  Time Until Resolution: {}", time_until_resolution);
                info!("  Active: {}", market.active);
                info!("  Closed: {}", market.closed);

                if let Some(liquidity) = &market.liquidity {
                    info!("  Liquidity: {}", liquidity);
                }

                if let Some(volume) = &market.volume {
                    info!("  Volume: {}", volume);
                }

                // Display event information if available
                if let Some(event) = event_info {
                    info!("");
                    info!("  Related Event:");
                    info!("    Event ID: {}", event.id);
                    info!("    Title: {}", event.title);

                    if let Some(ticker) = &event.ticker {
                        info!("    Ticker: {}", ticker);
                    }

                    if let Some(description) = &event.description {
                        // Truncate description if too long
                        let desc = if description.len() > 200 {
                            format!("{}...", &description[..200])
                        } else {
                            description.clone()
                        };
                        info!("    Description: {}", desc);
                    }

                    if let Some(category) = &event.category {
                        info!("    Category: {}", category);
                    }

                    if let Some(volume) = &event.volume {
                        info!("    Event Volume: {}", volume);
                    }
                }

                // Display outcomes and token IDs paired together
                info!("");
                if !outcomes.is_empty() && !token_ids.is_empty() {
                    info!("  Outcomes:");
                    for (idx, outcome) in outcomes.iter().enumerate() {
                        if let Some(token_id) = token_ids.get(idx) {
                            info!("    [{}] {} -> Token ID: {}", idx, outcome, token_id);

                            // Fetch orderbook for this token
                            match clob_client.get_orderbook(token_id).await {
                                Ok(orderbook) => {
                                    if let Some(best_ask) = orderbook.best_ask() {
                                        let price = best_ask.price_f64();
                                        let size = best_ask.size_f64();
                                        let sweep_cost = orderbook.total_ask_sweep_cost();
                                        info!("        Best Ask: ${:.2} ({:.2} shares available)", price, size);
                                        info!("        Total Cost to Sweep Asks: ${:.2}", sweep_cost);
                                    } else {
                                        info!("        Best Ask: N/A (no asks available)");
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to fetch orderbook for token {}: {}", token_id, e);
                                }
                            }
                        } else {
                            info!("    [{}] {}", idx, outcome);
                        }
                    }
                } else if !outcomes.is_empty() {
                    info!("  Outcomes: {:?}", outcomes);
                } else if !token_ids.is_empty() {
                    info!("  Token IDs: {:?}", token_ids);
                }

                info!("========================================");
                info!("");

                // Mark this market as logged
                logged_market_ids.insert(market.id.clone());
            }
        }

        // Log heartbeat every 5 minutes if no new markets found
        if new_markets_found == 0 {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(last_heartbeat).num_seconds();

            if elapsed >= 300 {  // 5 minutes = 300 seconds
                info!("Heartbeat (Iteration #{}): No new markets found in last 5 minutes (tracking {} markets)", iteration, logged_market_ids.len());
                last_heartbeat = now;
            }
        } else {
            // Reset heartbeat timer when markets are found
            last_heartbeat = Utc::now();
        }

        // Sleep before next iteration, or break on Ctrl+C
        tokio::select! {
            _ = sleep(Duration::from_secs_f64(config.loop_interval_secs)) => {
                // Sleep completed normally, continue to next iteration
            }
            _ = signal::ctrl_c() => {
                // Ctrl+C received, break out of loop
                info!("");
                info!("Received shutdown signal (Ctrl+C)");
                break;
            }
        }
    }

    info!("");
    info!("========================================");
    info!("Market Sniper stopped gracefully");
    info!("Total unique markets tracked: {}", logged_market_ids.len());
    info!("========================================");

    Ok(())
}
