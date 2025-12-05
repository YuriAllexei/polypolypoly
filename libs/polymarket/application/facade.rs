//! Application Facade
//!
//! Public API for binaries (presentation layer).
//! Provides simplified access to application use cases.

use super::{
    EventSyncService,
    sniper::MarketTrackerService,
};
use crate::domain::SniperMarket;
use crate::infrastructure::{
    Heartbeat, MarketDatabase, ShutdownManager, init_tracing, init_tracing_with_level,
    database::{DbEvent, DbMarket},
};
use tracing::{debug, info};
use std::sync::Arc;

// Note: Configuration and tracking services are now in application::sniper module

/// Application facade for market sniper use case
pub struct SniperApp {
    pub database: Arc<MarketDatabase>,
    pub shutdown: ShutdownManager,
    pub heartbeat: Heartbeat,
    pub tracker: MarketTrackerService,
}

impl SniperApp {
    /// Initialize sniper application
    pub async fn new(database_url: &str, heartbeat_interval: u64, probability_threshold: f64) -> anyhow::Result<Self> {
        let database = Arc::new(MarketDatabase::new(database_url).await?);
        let shutdown = ShutdownManager::new();
        let tracker = MarketTrackerService::new(Arc::clone(&database), probability_threshold);
        shutdown.spawn_signal_handler();
        let heartbeat = Heartbeat::new(heartbeat_interval);

        Ok(Self {
            database,
            shutdown,
            heartbeat,
            tracker,
        })
    }

    /// Check if app is still running
    pub fn is_running(&self) -> bool {
        self.shutdown.is_running()
    }

    /// Check if heartbeat should log
    pub fn should_heartbeat(&self) -> bool {
        self.heartbeat.should_beat()
    }

    /// Record heartbeat
    pub fn beat(&mut self) {
        self.heartbeat.beat();
    }

    /// Get markets expiring soon
    pub async fn get_expiring_markets(
        &self,
        delta_seconds: f64,
    ) -> anyhow::Result<Vec<crate::domain::models::DbMarket>> {
        Ok(self.database.get_markets_expiring_soon(delta_seconds).await?)
    }
}

/// Application facade for event sync use case
pub struct EventSyncApp {
    pub sync_service: EventSyncService,
    pub shutdown: ShutdownManager,
    pub heartbeat: Heartbeat,
}

impl EventSyncApp {
    /// Initialize event sync application
    pub async fn new(
        database_url: &str,
        api_base_url: &str,
        heartbeat_secs: u64,
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
            let url = format!(
                "{}/events?closed=false&limit={}&offset={}&ascending=true",
                self.sync_service.api_base_url, LIMIT, offset
            );

            debug!(page = page, offset = offset, limit = LIMIT, "Fetching events page");

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

                // Convert and collect event
                let db_event = crate::application::sync::EventSyncService::event_to_db_event(event);
                db_events.push(db_event);

                // Process markets for this event
                if let Some(markets) = &event.markets {
                    for market in markets {
                        if let Ok(db_market) =
                            crate::application::sync::EventSyncService::market_to_db_market(market)
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
            let events_upserted = self.sync_service.database.batch_upsert_events(&db_events).await?;
            debug!(count = events_upserted, "Batch upserted events");

            // Batch upsert all markets
            let markets_upserted = self.sync_service.database.batch_upsert_markets(&db_markets).await?;
            debug!(count = markets_upserted, "Batch upserted markets");

            // Batch link events to markets
            let links_created = self.sync_service.database.batch_link_event_markets(&event_market_links).await?;
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
