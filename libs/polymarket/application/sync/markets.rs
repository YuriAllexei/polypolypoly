use crate::domain::models::{DbMarket, SyncStats};
use crate::infrastructure::database::{DatabaseError, MarketDatabase, Result};
use crate::infrastructure::client::{GammaClient, GammaMarket};
use chrono::Utc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Market synchronization service
pub struct MarketSyncService {
    gamma_client: Arc<GammaClient>,
    database: Arc<MarketDatabase>,
    last_sync: Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
}

impl MarketSyncService {
    /// Create new sync service
    pub fn new(gamma_client: Arc<GammaClient>, database: Arc<MarketDatabase>) -> Self {
        Self {
            gamma_client,
            database,
            last_sync: Arc::new(RwLock::new(None)),
        }
    }

    /// Initial full sync on startup - fetches ALL active markets
    pub async fn initial_sync(&self) -> Result<SyncStats> {
        let start = Instant::now();
        info!("🔄 Starting initial market sync...");

        // Fetch ALL active markets from Gamma API with pagination
        let gamma_markets = self
            .gamma_client
            .get_all_active_markets()
            .await
            .map_err(|e| DatabaseError::ConnectionError(sqlx::Error::Protocol(e.to_string())))?;

        info!("   Fetched {} markets from Gamma API", gamma_markets.len());

        // Convert to DB format and insert
        let mut inserted = 0;
        for gamma_market in &gamma_markets {
            match Self::convert_gamma_to_db(gamma_market) {
                Ok(db_market) => {
                    if let Err(e) = self.database.upsert_market(db_market).await {
                        warn!("Failed to insert market {}: {}", gamma_market.id.as_ref().unwrap_or(&"unknown".to_string()), e);
                    } else {
                        inserted += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to convert market {}: {}", gamma_market.id.as_ref().unwrap_or(&"unknown".to_string()), e);
                }
            }
        }

        // Update last sync timestamp
        *self.last_sync.write().await = Some(Utc::now());

        let duration = start.elapsed();
        info!("✅ Initial sync complete: {} markets in {:?}", inserted, duration);

        Ok(SyncStats {
            markets_fetched: gamma_markets.len(),
            markets_inserted: inserted,
            markets_updated: 0,
            duration,
        })
    }

    /// Incremental sync - fetches only new markets since last sync
    pub async fn incremental_sync(&self) -> Result<SyncStats> {
        let start = Instant::now();
        debug!("Starting incremental market sync...");

        let last_sync = *self.last_sync.read().await;

        let new_markets = if let Some(last_sync_time) = last_sync {
            // Fetch only markets created/updated since last sync
            debug!("Fetching markets since {}", last_sync_time);
            self.gamma_client
                .get_new_markets(last_sync_time)
                .await
                .map_err(|e| DatabaseError::ConnectionError(sqlx::Error::Protocol(e.to_string())))?
        } else {
            // No last sync time - do full sync
            warn!("No last sync time found, performing full sync");
            return self.initial_sync().await;
        };

        debug!("Fetched {} new markets", new_markets.len());

        // Update database
        let mut inserted = 0;
        let mut updated = 0;

        for gamma_market in &new_markets {
            match Self::convert_gamma_to_db(gamma_market) {
                Ok(db_market) => {
                    // Check if market exists
                    let exists = self.database.get_market(&db_market.id).await.is_ok();

                    if let Err(e) = self.database.upsert_market(db_market).await {
                        warn!("Failed to upsert market {}: {}", gamma_market.id.as_ref().unwrap_or(&"unknown".to_string()), e);
                    } else if exists {
                        updated += 1;
                    } else {
                        inserted += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to convert market {}: {}", gamma_market.id.as_ref().unwrap_or(&"unknown".to_string()), e);
                }
            }
        }

        // Update last sync timestamp
        *self.last_sync.write().await = Some(Utc::now());

        let duration = start.elapsed();

        if inserted > 0 || updated > 0 {
            info!(
                "Incremental sync: {} new, {} updated in {:?}",
                inserted, updated, duration
            );
        }

        Ok(SyncStats {
            markets_fetched: new_markets.len(),
            markets_inserted: inserted,
            markets_updated: updated,
            duration,
        })
    }

    /// Background sync loop - runs incremental sync at regular intervals
    pub async fn start_sync_loop(self: Arc<Self>, interval: Duration) {
        info!("Starting background sync loop (interval: {:?})", interval);

        loop {
            tokio::time::sleep(interval).await;

            match self.incremental_sync().await {
                Ok(stats) => {
                    if stats.markets_inserted > 0 || stats.markets_updated > 0 {
                        debug!(
                            "Sync: {} new, {} updated",
                            stats.markets_inserted, stats.markets_updated
                        );
                    }
                }
                Err(e) => {
                    warn!("Sync failed: {}", e);
                }
            }
        }
    }

    /// Convert Gamma API market to database format
    fn convert_gamma_to_db(gamma: &GammaMarket) -> std::result::Result<DbMarket, String> {
        let now = Utc::now().to_rfc3339();

        // Serialize outcomes to JSON
        let outcomes_json = if let Some(ref outcomes) = gamma.outcomes {
            serde_json::to_string(outcomes)
                .map_err(|e| format!("Failed to serialize outcomes: {}", e))?
        } else {
            "[]".to_string()
        };

        // clob_token_ids is already a string in the new schema
        let token_ids_json = gamma.clob_token_ids.clone().unwrap_or_else(|| "[]".to_string());

        Ok(DbMarket {
            id: gamma.id.clone().unwrap_or_default(),
            condition_id: gamma.condition_id.clone(),
            question: gamma.question.clone().unwrap_or_default(),
            description: None, // Markets synced directly from Gamma don't have parent event description
            slug: gamma.slug.clone(),
            start_date: gamma.start_date.clone().unwrap_or_else(|| now.clone()),
            end_date: gamma.end_date.clone().unwrap_or_else(|| now.clone()),
            resolution_time: gamma.end_date.clone().unwrap_or_else(|| now.clone()),
            active: gamma.active.unwrap_or(false),
            closed: gamma.closed.unwrap_or(false),
            archived: gamma.archived.unwrap_or(false),
            market_type: None, // Not present in new Market struct
            category: None, // Not present in new Market struct
            liquidity: gamma.liquidity.clone(),
            volume: gamma.volume.clone(),
            outcomes: outcomes_json,
            token_ids: token_ids_json,
            tags: None, // Markets synced directly from Gamma don't have tags
            last_updated: now.clone(),
            created_at: now,
            game_id: None, // Markets synced directly from Gamma don't have parent event game_id
        })
    }

    /// Get last sync time
    pub async fn last_sync_time(&self) -> Option<chrono::DateTime<Utc>> {
        *self.last_sync.read().await
    }
}
