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
    Heartbeat, MarketDatabase, ShutdownManager, init_tracing,
};
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

    /// Sync all events from API
    pub async fn sync_all_events(&self) -> anyhow::Result<usize> {
        const LIMIT: usize = 500;
        let mut total_synced = 0;
        let mut offset = 0;

        loop {
            let url = format!(
                "{}/events?closed=false&limit={}&offset={}&ascending=true",
                self.sync_service.api_base_url, LIMIT, offset
            );

            let response = self.sync_service.http_client.get(&url).send().await?;
            let text = response.text().await?;
            let events: Vec<crate::infrastructure::client::gamma::types::Event> =
                serde_json::from_str(&text)?;

            if events.is_empty() {
                break;
            }

            for event in &events {
                if let Err(e) = self.sync_event(event).await {
                    tracing::warn!("Failed to sync event: {}", e);
                } else {
                    total_synced += 1;
                }
            }

            offset += LIMIT;

            if events.len() < LIMIT {
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Ok(total_synced)
    }

    async fn sync_event(
        &self,
        event: &crate::infrastructure::client::gamma::types::Event,
    ) -> anyhow::Result<()> {
        let event_id = match &event.id {
            Some(id) => id,
            None => return Ok(()),
        };

        if self.sync_service.database.event_exists(event_id).await? {
            return Ok(());
        }

        // Use EventSyncService conversion functions
        let db_event =
            crate::application::sync::EventSyncService::event_to_db_event(event);
        self.sync_service.database.upsert_event(db_event).await?;

        // Process markets
        if let Some(markets) = &event.markets {
            let market_ids: Vec<String> = markets
                .iter()
                .filter_map(|m| m.id.clone())
                .collect();

            for market in markets {
                if let Ok(db_market) =
                    crate::application::sync::EventSyncService::market_to_db_market(market)
                {
                    let _ = self.sync_service.database.upsert_market(db_market).await;
                }
            }

            if !market_ids.is_empty() {
                let _ = self
                    .sync_service
                    .database
                    .link_event_markets(event_id, &market_ids)
                    .await;
            }
        }

        Ok(())
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

/// Initialize tracing for binaries
pub fn init_logging() {
    init_tracing();
}

/// Helper to convert DB market to domain model
pub fn to_sniper_market(
    db_market: &crate::domain::models::DbMarket,
) -> anyhow::Result<SniperMarket> {
    Ok(SniperMarket::from_db_market(db_market)?)
}
