use crate::{DatabaseError, MarketDatabase, Result, SyncStats};
use crate::models::DbMarket;
use chrono::Utc;
use polymarket_client::{GammaClient, GammaMarket};
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
                        warn!("Failed to insert market {}: {}", gamma_market.id, e);
                    } else {
                        inserted += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to convert market {}: {}", gamma_market.id, e);
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
                        warn!("Failed to upsert market {}: {}", gamma_market.id, e);
                    } else if exists {
                        updated += 1;
                    } else {
                        inserted += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to convert market {}: {}", gamma_market.id, e);
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

        // Serialize outcomes and token IDs to JSON
        let outcomes_json = serde_json::to_string(
            &gamma.outcomes.as_ref().unwrap_or(&vec![])
        )
        .map_err(|e| format!("Failed to serialize outcomes: {}", e))?;

        let token_ids_json = serde_json::to_string(
            &gamma.clob_token_ids.as_ref().unwrap_or(&vec![])
        )
        .map_err(|e| format!("Failed to serialize token IDs: {}", e))?;

        Ok(DbMarket {
            id: gamma.id.clone(),
            condition_id: gamma.condition_id.clone(),
            question: gamma.question.clone(),
            slug: gamma.slug.clone(),
            start_date: gamma.start_date.clone(),
            end_date: gamma.end_date.clone(),
            resolution_time: gamma.end_date.clone(), // Use end_date as resolution time
            active: gamma.active,
            closed: gamma.closed,
            archived: gamma.archived,
            market_type: gamma.market_type.clone(),
            category: gamma.category.clone(),
            liquidity: gamma.liquidity.clone(),
            volume: gamma.volume.clone(),
            outcomes: outcomes_json,
            token_ids: token_ids_json,
            last_updated: now.clone(),
            created_at: now,
        })
    }

    /// Get last sync time
    pub async fn last_sync_time(&self) -> Option<chrono::DateTime<Utc>> {
        *self.last_sync.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_gamma_to_db() {
        let gamma = GammaMarket {
            id: "123".to_string(),
            condition_id: "0xabc".to_string(),
            question: "Test?".to_string(),
            slug: Some("test".to_string()),
            start_date: "2025-01-01T00:00:00Z".to_string(),
            end_date: "2025-01-02T00:00:00Z".to_string(),
            active: true,
            closed: false,
            archived: false,
            market_type: Some("binary".to_string()),
            category: Some("crypto".to_string()),
            liquidity: Some("1000".to_string()),
            volume: Some("500".to_string()),
            volume_24hr: Some(100.0),
            outcomes: Some(vec!["Yes".to_string(), "No".to_string()]),
            clob_token_ids: Some(vec!["0x1".to_string(), "0x2".to_string()]),
            tags: vec![],
        };

        let db_market = MarketSyncService::convert_gamma_to_db(&gamma).unwrap();

        assert_eq!(db_market.id, "123");
        assert_eq!(db_market.question, "Test?");
        assert_eq!(db_market.active, true);
    }
}

