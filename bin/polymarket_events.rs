use anyhow::Result;
use chrono::Utc;
use polymarket::client::gamma::types::{Event, Market};
use polymarket::database::{DbEvent, DbMarket, MarketDatabase};
use reqwest::Client;
use serde_json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber;

/// Convert Event to DbEvent
fn event_to_db_event(event: &Event) -> DbEvent {
    let now = Utc::now().to_rfc3339();

    DbEvent {
        id: event.id.clone().unwrap_or_default(),
        ticker: event.ticker.clone(),
        slug: event.slug.clone(),
        title: event.title.clone().unwrap_or_default(),
        description: event.description.clone(),
        start_date: event.start_date.clone(),
        end_date: event.end_date.clone(),
        active: event.active.unwrap_or(false),
        closed: event.closed.unwrap_or(false),
        archived: event.archived.unwrap_or(false),
        featured: event.featured.unwrap_or(false),
        restricted: event.restricted.unwrap_or(false),
        liquidity: event.liquidity.map(|v| v.to_string()),
        volume: event.volume.map(|v| v.to_string()),
        volume_24hr: event.volume24_hr.map(|v| v.to_string()),
        volume_1wk: event.volume1_wk.map(|v| v.to_string()),
        volume_1mo: event.volume1_mo.map(|v| v.to_string()),
        volume_1yr: event.volume1_yr.map(|v| v.to_string()),
        open_interest: event.open_interest.map(|v| v.to_string()),
        image: event.image.clone(),
        icon: event.icon.clone(),
        category: None, // Not in the new Event struct
        competitive: event.competitive.map(|v| v.to_string()),
        comment_count: event.comment_count.unwrap_or(0),
        created_at: event.created_at.clone().unwrap_or_else(|| now.clone()),
        updated_at: event.updated_at.clone().unwrap_or_else(|| now.clone()),
        last_synced: now,
    }
}

/// Convert Market to DbMarket
fn market_to_db_market(market: &Market) -> Result<DbMarket> {
    let now = Utc::now().to_rfc3339();

    // Serialize outcomes to JSON string
    // The outcomes field is Option<Outcomes> which is an enum, need to handle it differently
    let outcomes_json = market
        .outcomes
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());

    // clob_token_ids is Option<String> in the new struct
    let token_ids_json = market.clob_token_ids.clone().unwrap_or_else(|| "[]".to_string());

    Ok(DbMarket {
        id: market.id.clone().unwrap_or_default(),
        condition_id: market.condition_id.clone().unwrap_or_default(),
        question: market.question.clone().unwrap_or_default(),
        slug: market.slug.clone(),
        start_date: market.start_date.clone().unwrap_or_else(|| now.clone()),
        end_date: market.end_date.clone().unwrap_or_else(|| now.clone()),
        resolution_time: market.end_date.clone().unwrap_or_else(|| now.clone()),
        active: market.active.unwrap_or(false),
        closed: market.closed.unwrap_or(false),
        archived: market.archived.unwrap_or(false),
        market_type: None, // Not a simple field in new struct
        category: None, // Not in new struct
        liquidity: market.liquidity.clone(),
        volume: market.volume.clone(),
        outcomes: outcomes_json,
        token_ids: token_ids_json,
        last_updated: now.clone(),
        created_at: market.created_at.clone().unwrap_or(now),
    })
}

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
    let config = polymarket::config::BotConfig::load("config.yaml")?;

    // Initialize database
    info!("Initializing database: {}", config.database.path);
    let db = MarketDatabase::new(&config.database.path).await?;

    // Initialize HTTP client
    let http_client = Client::new();
    let api_base_url = &config.gamma_api.base_url;

    const LIMIT: usize = 500;
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

    let mut cycle_count = 0;

    // Main sync loop - runs forever
    loop {
        // Check for shutdown signal
        if shutdown.load(Ordering::SeqCst) {
            info!("Shutdown signal received. Exiting...");
            break;
        }
        cycle_count += 1;
        info!("");
        info!("========================================");
        info!("Starting sync cycle #{}", cycle_count);
        info!("========================================");

        // Statistics for this cycle
        let mut total_fetched = 0;
        let mut total_new = 0;
        let mut offset = 0;

        let start_time = Instant::now();

        info!("Fetching events from Polymarket API (closed=false)...");

        loop {
            info!("Fetching page: offset={}, limit={}", offset, LIMIT);

            // Build API URL
            let url = format!(
                "{}/events?closed=false&limit={}&offset={}&ascending=true",
                api_base_url, LIMIT, offset
            );

            // Fetch events page
            let events: Vec<Event> = match http_client.get(&url).send().await {
                Ok(response) => {
                    // Get response text first for debugging
                    let text = match response.text().await {
                        Ok(text) => text,
                        Err(e) => {
                            error!("Failed to read response text: {}", e);
                            break;
                        }
                    };

                    // Try to parse as JSON
                    match serde_json::from_str(&text) {
                        Ok(events) => events,
                        Err(e) => {
                            error!("Failed to parse events JSON: {}", e);
                            // Save first 1000 chars of response for debugging
                            let preview = if text.len() > 1000 {
                                &text[..1000]
                            } else {
                                &text
                            };
                            error!("Response preview: {}", preview);
                            break;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to fetch events from API: {}", e);
                    break;
                }
            };

            let page_count = events.len();
            total_fetched += page_count;

            info!("Received {} events in this page", page_count);

            // Process each event
            for event in events {
                // Skip events without an ID
                let event_id = match &event.id {
                    Some(id) => id,
                    None => {
                        warn!("Event without ID, skipping");
                        continue;
                    }
                };

                // Check if event exists
                let exists = db.event_exists(event_id).await?;

                if exists {
                    info!("Event {} already exists, skipping", event_id);
                    continue;
                }

                info!(
                    "Processing new event: {} - {}",
                    event_id,
                    event.title.as_ref().unwrap_or(&"no title".to_string())
                );

                // Convert to DbEvent
                let db_event = event_to_db_event(&event);

                // Save event to database
                match db.upsert_event(db_event).await {
                    Ok(_) => {
                        total_new += 1;

                        // Process and save associated markets if they exist
                        if let Some(markets) = &event.markets {
                            let market_ids: Vec<String> = markets
                                .iter()
                                .filter_map(|m| m.id.clone())
                                .collect();

                            if !market_ids.is_empty() {
                                info!(
                                    "Processing {} markets for event {}",
                                    market_ids.len(),
                                    event.id.as_ref().unwrap_or(&"unknown".to_string())
                                );

                                // Save each market
                                for market in markets {
                                    match market_to_db_market(market) {
                                        Ok(db_market) => {
                                            if let Err(e) = db.upsert_market(db_market).await {
                                                warn!(
                                                    "Failed to save market {} for event {}: {}",
                                                    market.id.as_ref().unwrap_or(&"unknown".to_string()),
                                                    event.id.as_ref().unwrap_or(&"unknown".to_string()),
                                                    e
                                                );
                                            } else {
                                                info!(
                                                    "  Saved market {} - {}",
                                                    market.id.as_ref().unwrap_or(&"unknown".to_string()),
                                                    market.question.as_ref().unwrap_or(&"no question".to_string())
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to convert market {} for event {}: {}",
                                                market.id.as_ref().unwrap_or(&"unknown".to_string()),
                                                event.id.as_ref().unwrap_or(&"unknown".to_string()),
                                                e
                                            );
                                        }
                                    }
                                }

                                // Link markets to event
                                if let Some(event_id) = &event.id {
                                    if let Err(e) = db.link_event_markets(event_id, &market_ids).await {
                                        warn!("Failed to link markets for event {}: {}", event_id, e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to save event {}: {}",
                            event.id.as_ref().unwrap_or(&"unknown".to_string()),
                            e
                        );
                    }
                }
            }

            // Check if we've reached the end
            if page_count < LIMIT {
                info!("Reached end of pagination (got {} < {})", page_count, LIMIT);
                break;
            }

            // Increment offset
            offset += LIMIT;

            // Rate limiting: 100ms delay between requests
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        let duration = start_time.elapsed();

        // Print cycle statistics
        info!("====================================");
        info!("Sync Cycle #{} Complete!", cycle_count);
        info!("====================================");
        info!("Total events fetched: {}", total_fetched);
        info!("New events saved: {}", total_new);
        info!(
            "Events skipped (already exist): {}",
            total_fetched - total_new
        );
        info!("Duration: {:.2}s", duration.as_secs_f64());
        info!("====================================");

        // Show database statistics
        let total_events = db.event_count().await?;
        let active_events = db.active_event_count().await?;

        info!("Database Statistics:");
        info!("  Total events in DB: {}", total_events);
        info!("  Active events: {}", active_events);
        info!("====================================");

        // Sleep before next cycle (check for shutdown every second)
        info!(
            "Sleeping for {} seconds before next cycle...",
            SYNC_INTERVAL_SECS
        );
        for _ in 0..SYNC_INTERVAL_SECS {
            if shutdown.load(Ordering::SeqCst) {
                info!("Shutdown signal received during sleep. Exiting...");
                return Ok(());
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    // Clean shutdown
    info!("Event syncer stopped gracefully");
    Ok(())
}
