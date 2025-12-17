//! Application Facade
//!
//! Public API for binaries (presentation layer).
//! Provides simplified access to application use cases.

use super::EventSyncService;
use crate::domain::SniperMarket;
use crate::infrastructure::{
    database::{DbEvent, DbMarket},
    init_tracing, init_tracing_with_level, Heartbeat, MarketDatabase, ShutdownManager,
};
use std::sync::Arc;
use tracing::{debug, info};

// Note: Configuration and tracking services are now in application::sniper module

/// Application facade for event sync use case.
///
/// Provides a high-level interface for syncing events and markets from the
/// Polymarket Gamma API to the local database. Supports batch operations
/// for efficient bulk syncing.
pub struct EventSyncApp {
    pub sync_service: EventSyncService,
    pub shutdown: ShutdownManager,
    pub heartbeat: Heartbeat,
    /// Whether to fetch closed events (true = fetch all, false = only non-closed)
    pub closed: bool,
}

impl EventSyncApp {
    /// Initialize event sync application
    pub async fn new(
        database_url: &str,
        api_base_url: &str,
        heartbeat_secs: u64,
        closed: bool,
    ) -> anyhow::Result<Self> {
        let database = Arc::new(MarketDatabase::new(database_url).await?);
        let sync_service = EventSyncService::new(database.clone(), api_base_url.to_string());

        let shutdown = ShutdownManager::new();
        shutdown.spawn_signal_handler();
        let heartbeat = Heartbeat::new(heartbeat_secs);

        Ok(Self {
            sync_service,
            shutdown,
            heartbeat,
            closed,
        })
    }

    /// Check if app is still running
    pub fn is_running(&self) -> bool {
        self.shutdown.is_running()
    }

    /// Sync all events from API using batch operations
    pub async fn sync_all_events(&self) -> anyhow::Result<usize> {
        const LIMIT: usize = 500;
        let mut total_events = 0;
        let mut total_markets = 0;
        let mut offset = 0;
        let mut page = 1;

        info!("Starting event sync from Gamma API");

        loop {
            // If closed=false in config, add closed=false param to fetch only non-closed
            // If closed=true in config, omit closed param to fetch all events
            let url = if self.closed {
                format!(
                    "{}/events?limit={}&offset={}&ascending=true",
                    self.sync_service.api_base_url, LIMIT, offset
                )
            } else {
                format!(
                    "{}/events?closed=false&limit={}&offset={}&ascending=true",
                    self.sync_service.api_base_url, LIMIT, offset
                )
            };

            debug!(
                page = page,
                offset = offset,
                limit = LIMIT,
                "Fetching events page"
            );

            let response = self.sync_service.http_client.get(&url).send().await?;
            let text = response.text().await?;
            let api_events: Vec<crate::infrastructure::client::gamma::types::Event> =
                serde_json::from_str(&text)?;

            if api_events.is_empty() {
                debug!("No more events to fetch");
                break;
            }

            let events_in_page = api_events.len();
            debug!(count = events_in_page, "Received events in page");

            // Collect all data for batch operations
            let mut db_events: Vec<DbEvent> = Vec::with_capacity(events_in_page);
            let mut db_markets: Vec<DbMarket> = Vec::new();
            let mut event_market_links: Vec<(String, String)> = Vec::new();

            for event in &api_events {
                let event_id = match &event.id {
                    Some(id) => id.clone(),
                    None => continue,
                };

                let event_title = event.title.as_deref().unwrap_or("Unknown");
                debug!(event_id = %event_id, title = %event_title, "Processing event");

                // Serialize event tags to pass to markets
                let event_tags_json = event
                    .tags
                    .as_ref()
                    .map(|tags| serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()));

                // Get event description to pass to markets
                let event_description = event.description.clone();

                // Get event game_id to pass to markets (sports events have this)
                let event_game_id = event.game_id.map(|v| v as i64);

                // Convert and collect event
                let db_event = crate::application::sync::EventSyncService::event_to_db_event(event);
                db_events.push(db_event);

                // Process markets for this event (inheriting tags, description, and game_id from parent event)
                if let Some(markets) = &event.markets {
                    for market in markets {
                        if let Ok(db_market) =
                            crate::application::sync::EventSyncService::market_to_db_market(
                                market,
                                event_tags_json.clone(),
                                event_description.clone(),
                                event_game_id,
                            )
                        {
                            debug!(
                                market_id = %db_market.id,
                                question = %db_market.question,
                                event_id = %event_id,
                                "Processing market"
                            );

                            // Collect link
                            event_market_links.push((event_id.clone(), db_market.id.clone()));
                            db_markets.push(db_market);
                        }
                    }
                }
            }

            // Batch upsert all events
            let events_upserted = self
                .sync_service
                .database
                .batch_upsert_events(&db_events)
                .await?;
            debug!(count = events_upserted, "Batch upserted events");

            // Batch upsert all markets
            let markets_upserted = self
                .sync_service
                .database
                .batch_upsert_markets(&db_markets)
                .await?;
            debug!(count = markets_upserted, "Batch upserted markets");

            // Batch link events to markets
            let links_created = self
                .sync_service
                .database
                .batch_link_event_markets(&event_market_links)
                .await?;
            debug!(count = links_created, "Batch linked event-markets");

            total_events += events_upserted;
            total_markets += markets_upserted;

            info!(
                page = page,
                events = events_upserted,
                markets = markets_upserted,
                "Completed page sync"
            );

            offset += LIMIT;
            page += 1;

            if api_events.len() < LIMIT {
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        info!(
            total_events = total_events,
            total_markets = total_markets,
            pages_fetched = page,
            "Event sync completed"
        );

        Ok(total_events)
    }

    /// Check if heartbeat should log
    pub fn should_heartbeat(&self) -> bool {
        self.heartbeat.should_beat()
    }

    /// Record heartbeat
    pub fn beat(&mut self) {
        self.heartbeat.beat();
    }
}

/// Initialize tracing for binaries with default (info) level
pub fn init_logging() {
    init_tracing();
}

/// Initialize tracing for binaries with a specific log level
pub fn init_logging_with_level(level: &str) {
    init_tracing_with_level(level);
}

/// Helper to convert DB market to domain model
pub fn to_sniper_market(
    db_market: &crate::domain::models::DbMarket,
) -> anyhow::Result<SniperMarket> {
    Ok(SniperMarket::from_db_market(db_market)?)
}
