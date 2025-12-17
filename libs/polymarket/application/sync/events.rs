use crate::domain::models::{DbEvent, DbMarket};
use crate::infrastructure::database::MarketDatabase;
use crate::infrastructure::client::gamma::types::{Event, Market};
use reqwest::Client;
use std::sync::Arc;
use std::time::Instant;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};
use chrono::Utc;

/// Event synchronization service
pub struct EventSyncService {
    pub database: Arc<MarketDatabase>,
    pub http_client: Client,
    pub api_base_url: String,
}

impl EventSyncService {
    /// Create new event sync service
    pub fn new(database: Arc<MarketDatabase>, api_base_url: String) -> Self {
        Self {
            database,
            http_client: Client::new(),
            api_base_url,
        }
    }

    /// Start the sync loop
    pub async fn start_sync_loop(self: Arc<Self>, interval_secs: u64, shutdown: Arc<AtomicBool>) {
        let mut cycle_count = 0;
        const LIMIT: usize = 500;

        info!(
            "Starting perpetual event sync (interval: {}s)",
            interval_secs
        );

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
            let mut offset = 0;

            let start_time = Instant::now();

            info!("Fetching events from Polymarket API (closed=false)...");

            loop {
                info!("Fetching page: offset={}, limit={}", offset, LIMIT);

                // Build API URL
                let url = format!(
                    "{}/events?closed=false&limit={}&offset={}&ascending=true",
                    self.api_base_url, LIMIT, offset
                );

                // Fetch events page
                let events: Vec<Event> = match self.http_client.get(&url).send().await {
                    Ok(response) => {
                        let text = match response.text().await {
                            Ok(text) => text,
                            Err(e) => {
                                error!("Failed to read response text: {}", e);
                                break;
                            }
                        };

                        match serde_json::from_str(&text) {
                            Ok(events) => events,
                            Err(e) => {
                                error!("Failed to parse events JSON: {}", e);
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
                    if let Err(e) = self.process_event(&event).await {
                        error!("Error processing event: {}", e);
                    } else {
                         // We could track new events here if process_event returned that info
                         // For now, simple increment if successful isn't quite accurate for "new"
                         // but we'll leave detailed stats for later refinement
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
            info!("Duration: {:.2}s", duration.as_secs_f64());
            info!("====================================");

            // Sleep before next cycle
            info!(
                "Sleeping for {} seconds before next cycle...",
                interval_secs
            );
            for _ in 0..interval_secs {
                if shutdown.load(Ordering::SeqCst) {
                    info!("Shutdown signal received during sleep. Exiting...");
                    return;
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }

    async fn process_event(&self, event: &Event) -> anyhow::Result<()> {
        // Skip events without an ID
        let event_id = match &event.id {
            Some(id) => id,
            None => return Ok(()),
        };

        // Check if event exists
        let exists = self.database.event_exists(event_id).await?;

        if exists {
            return Ok(());
        }

        info!(
            "Processing new event: {} - {}",
            event_id,
            event.title.as_ref().unwrap_or(&"no title".to_string())
        );

        // Serialize event tags to pass to markets
        let event_tags_json = event
            .tags
            .as_ref()
            .map(|tags| serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()));

        // Get event description to pass to markets
        let event_description = event.description.clone();

        // Get event game_id to pass to markets (sports events have this)
        let event_game_id = event.game_id.map(|v| v as i64);

        // Convert to DbEvent
        let db_event = Self::event_to_db_event(event);

        // Save event to database
        self.database.upsert_event(db_event).await?;

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
                    event_id
                );

                // Save each market (inheriting tags, description, and game_id from parent event)
                for market in markets {
                     let db_market = Self::market_to_db_market(market, event_tags_json.clone(), event_description.clone(), event_game_id)?;
                     if let Err(e) = self.database.upsert_market(db_market).await {
                         warn!("Failed to save market: {}", e);
                     }
                }

                // Link markets to event
                if let Err(e) = self.database.link_event_markets(event_id, &market_ids).await {
                    warn!("Failed to link markets for event {}: {}", event_id, e);
                }
            }
        }

        Ok(())
    }

    pub fn event_to_db_event(event: &Event) -> DbEvent {
        let now = Utc::now().to_rfc3339();

        let tags_json = event
            .tags
            .as_ref()
            .map(|tags| serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()));

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
            category: None,
            competitive: event.competitive.map(|v| v.to_string()),
            tags: tags_json,
            comment_count: event.comment_count.map(|v| v as i64).unwrap_or(0),
            created_at: event.created_at.clone().unwrap_or_else(|| now.clone()),
            updated_at: event.updated_at.clone().unwrap_or_else(|| now.clone()),
            last_synced: now,
            game_id: event.game_id.map(|v| v as i64),
        }
    }

    pub fn market_to_db_market(
        market: &Market,
        event_tags: Option<String>,
        event_description: Option<String>,
        event_game_id: Option<i64>,
    ) -> anyhow::Result<DbMarket> {
        let now = Utc::now().to_rfc3339();

        let outcomes_json = market
            .outcomes
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()))
            .unwrap_or_else(|| "[]".to_string());

        let token_ids_json = market.clob_token_ids.clone().unwrap_or_else(|| "[]".to_string());

        Ok(DbMarket {
            id: market.id.clone().unwrap_or_default(),
            condition_id: market.condition_id.clone(),
            question: market.question.clone().unwrap_or_default(),
            description: event_description,
            slug: market.slug.clone(),
            start_date: market.start_date.clone().unwrap_or_else(|| now.clone()),
            end_date: market.end_date.clone().unwrap_or_else(|| now.clone()),
            resolution_time: market.end_date.clone().unwrap_or_else(|| now.clone()),
            active: market.active.unwrap_or(false),
            closed: market.closed.unwrap_or(false),
            archived: market.archived.unwrap_or(false),
            market_type: None,
            category: None,
            liquidity: market.liquidity.clone(),
            volume: market.volume.clone(),
            outcomes: outcomes_json,
            token_ids: token_ids_json,
            tags: event_tags,
            last_updated: now.clone(),
            created_at: market.created_at.clone().unwrap_or(now),
            game_id: event_game_id,
        })
    }
}
