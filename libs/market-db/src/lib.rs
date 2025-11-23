pub mod models;
pub mod schema;
pub mod sync;

use chrono::{DateTime, Duration, Utc};
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;
use thiserror::Error;
use tracing::{debug, info};

// Re-export main types
pub use models::{DbEvent, DbLLMCache, DbMarket, MarketFilters, SyncStats};
pub use schema::{get_schema_version, initialize_schema};
pub use sync::MarketSyncService;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Database connection error: {0}")]
    ConnectionError(#[from] sqlx::Error),

    #[error("Schema error: {0}")]
    SchemaError(#[from] schema::SchemaError),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Market not found: {0}")]
    MarketNotFound(String),

    #[error("Event not found: {0}")]
    EventNotFound(String),
}

pub type Result<T> = std::result::Result<T, DatabaseError>;

/// Market database manager
pub struct MarketDatabase {
    pool: SqlitePool,
}

impl MarketDatabase {
    /// Create new database connection and initialize schema
    pub async fn new(db_path: &str) -> Result<Self> {
        info!("Connecting to database: {}", db_path);

        // Create connection options
        let options = SqliteConnectOptions::from_str(db_path)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        // Connect to database
        let pool = SqlitePool::connect_with(options).await?;

        // Initialize schema
        schema::initialize_schema(&pool).await?;

        info!("Database initialized successfully");

        Ok(Self { pool })
    }

    // ==================== MARKET OPERATIONS ====================

    /// Insert a single market (or replace if exists)
    pub async fn upsert_market(&self, market: DbMarket) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO markets (
                id, condition_id, question, slug, start_date, end_date, resolution_time,
                active, closed, archived, market_type, category, liquidity, volume,
                outcomes, token_ids, last_updated, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&market.id)
        .bind(&market.condition_id)
        .bind(&market.question)
        .bind(&market.slug)
        .bind(&market.start_date)
        .bind(&market.end_date)
        .bind(&market.resolution_time)
        .bind(market.active)
        .bind(market.closed)
        .bind(market.archived)
        .bind(&market.market_type)
        .bind(&market.category)
        .bind(&market.liquidity)
        .bind(&market.volume)
        .bind(&market.outcomes)
        .bind(&market.token_ids)
        .bind(&market.last_updated)
        .bind(&market.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Batch insert markets
    pub async fn insert_markets(&self, markets: Vec<DbMarket>) -> Result<usize> {
        let mut count = 0;

        for market in markets {
            self.upsert_market(market).await?;
            count += 1;
        }

        debug!("Inserted {} markets", count);
        Ok(count)
    }

    /// Get all active markets
    pub async fn get_active_markets(&self) -> Result<Vec<DbMarket>> {
        let markets = sqlx::query_as::<_, DbMarket>(
            "SELECT * FROM markets WHERE active = 1 AND closed = 0 ORDER BY resolution_time ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get markets resolving within the next X hours
    pub async fn get_upcoming_markets(&self, within_hours: u64) -> Result<Vec<DbMarket>> {
        let now = Utc::now();
        let cutoff = now + Duration::hours(within_hours as i64);

        let markets = sqlx::query_as::<_, DbMarket>(
            r#"
            SELECT * FROM markets
            WHERE active = 1
            AND closed = 0
            AND resolution_time > ?
            AND resolution_time <= ?
            ORDER BY resolution_time ASC
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(cutoff.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get markets updated since timestamp
    pub async fn get_updated_since(&self, since: DateTime<Utc>) -> Result<Vec<DbMarket>> {
        let markets = sqlx::query_as::<_, DbMarket>(
            "SELECT * FROM markets WHERE last_updated > ? ORDER BY last_updated DESC",
        )
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get market by ID
    pub async fn get_market(&self, id: &str) -> Result<DbMarket> {
        let market = sqlx::query_as::<_, DbMarket>("SELECT * FROM markets WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DatabaseError::MarketNotFound(id.to_string()))?;

        Ok(market)
    }

    /// Get market by condition ID
    pub async fn get_market_by_condition(&self, condition_id: &str) -> Result<DbMarket> {
        let market =
            sqlx::query_as::<_, DbMarket>("SELECT * FROM markets WHERE condition_id = ?")
                .bind(condition_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| DatabaseError::MarketNotFound(condition_id.to_string()))?;

        Ok(market)
    }

    /// Query markets with filters
    pub async fn query_markets(&self, filters: MarketFilters) -> Result<Vec<DbMarket>> {
        let (where_clause, params) = filters.build_where_clause();

        let query = format!(
            "SELECT * FROM markets {} ORDER BY resolution_time ASC",
            where_clause
        );

        let mut query_builder = sqlx::query_as::<_, DbMarket>(&query);

        for param in params {
            query_builder = query_builder.bind(param);
        }

        let markets = query_builder.fetch_all(&self.pool).await?;

        Ok(markets)
    }

    /// Get total number of markets
    pub async fn market_count(&self) -> Result<i64> {
        let (count,) = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM markets")
            .fetch_one(&self.pool)
            .await?;

        Ok(count)
    }

    /// Get number of active markets
    pub async fn active_market_count(&self) -> Result<i64> {
        let (count,) = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM markets WHERE active = 1 AND closed = 0",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Delete resolved markets older than cutoff date
    pub async fn cleanup_resolved(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM markets WHERE closed = 1 AND resolution_time < ?",
        )
        .bind(before.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // ==================== LLM CACHE OPERATIONS ====================

    /// Get LLM cache entry by question
    pub async fn get_llm_cache(&self, question: &str) -> Result<Option<DbLLMCache>> {
        let cache = sqlx::query_as::<_, DbLLMCache>("SELECT * FROM llm_cache WHERE question = ?")
            .bind(question)
            .fetch_optional(&self.pool)
            .await?;

        Ok(cache)
    }

    /// Insert LLM cache entry
    pub async fn insert_llm_cache(&self, entry: DbLLMCache) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO llm_cache (question, market_id, compatible, checked_at, resolution_time)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&entry.question)
        .bind(&entry.market_id)
        .bind(entry.compatible)
        .bind(&entry.checked_at)
        .bind(&entry.resolution_time)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get all compatible markets from LLM cache
    pub async fn get_compatible_markets(&self) -> Result<Vec<String>> {
        let ids: Vec<(String,)> = sqlx::query_as(
            "SELECT market_id FROM llm_cache WHERE compatible = 1",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(ids.into_iter().map(|(id,)| id).collect())
    }

    /// Get LLM cache statistics
    pub async fn llm_cache_stats(&self) -> Result<(i64, i64, i64)> {
        let (total,) = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM llm_cache")
            .fetch_one(&self.pool)
            .await?;

        let (compatible,) = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM llm_cache WHERE compatible = 1",
        )
        .fetch_one(&self.pool)
        .await?;

        let incompatible = total - compatible;

        Ok((total, compatible, incompatible))
    }

    /// Cleanup old LLM cache entries
    pub async fn cleanup_llm_cache(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM llm_cache WHERE checked_at < ?")
            .bind(before.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ==================== EVENT OPERATIONS ====================

    /// Insert or update an event
    pub async fn upsert_event(&self, event: DbEvent) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO events (
                id, ticker, slug, title, description, start_date, end_date,
                active, closed, archived, featured, restricted,
                liquidity, volume, volume_24hr, volume_1wk, volume_1mo, volume_1yr,
                open_interest, image, icon, category, competitive, comment_count,
                created_at, updated_at, last_synced
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.id)
        .bind(&event.ticker)
        .bind(&event.slug)
        .bind(&event.title)
        .bind(&event.description)
        .bind(&event.start_date)
        .bind(&event.end_date)
        .bind(event.active)
        .bind(event.closed)
        .bind(event.archived)
        .bind(event.featured)
        .bind(event.restricted)
        .bind(&event.liquidity)
        .bind(&event.volume)
        .bind(&event.volume_24hr)
        .bind(&event.volume_1wk)
        .bind(&event.volume_1mo)
        .bind(&event.volume_1yr)
        .bind(&event.open_interest)
        .bind(&event.image)
        .bind(&event.icon)
        .bind(&event.category)
        .bind(&event.competitive)
        .bind(event.comment_count)
        .bind(&event.created_at)
        .bind(&event.updated_at)
        .bind(&event.last_synced)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Link an event to its markets
    pub async fn link_event_markets(&self, event_id: &str, market_ids: &[String]) -> Result<()> {
        for market_id in market_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO event_markets (event_id, market_id) VALUES (?, ?)"
            )
            .bind(event_id)
            .bind(market_id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Check if event exists in database
    pub async fn event_exists(&self, event_id: &str) -> Result<bool> {
        let result = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM events WHERE id = ?")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0 > 0)
    }

    /// Get event by ID
    pub async fn get_event(&self, event_id: &str) -> Result<DbEvent> {
        let event = sqlx::query_as::<_, DbEvent>("SELECT * FROM events WHERE id = ?")
            .bind(event_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DatabaseError::EventNotFound(event_id.to_string()))?;

        Ok(event)
    }

    /// Get all active (non-closed) events
    pub async fn get_active_events(&self) -> Result<Vec<DbEvent>> {
        let events = sqlx::query_as::<_, DbEvent>(
            "SELECT * FROM events WHERE closed = 0 ORDER BY end_date ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    /// Get events by category
    pub async fn get_events_by_category(&self, category: &str) -> Result<Vec<DbEvent>> {
        let events = sqlx::query_as::<_, DbEvent>(
            "SELECT * FROM events WHERE category = ? AND closed = 0 ORDER BY end_date ASC",
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    /// Get total event count
    pub async fn event_count(&self) -> Result<i64> {
        let (count,) = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM events")
            .fetch_one(&self.pool)
            .await?;

        Ok(count)
    }

    /// Get active event count
    pub async fn active_event_count(&self) -> Result<i64> {
        let (count,) = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM events WHERE closed = 0",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Get markets for a specific event
    pub async fn get_event_markets(&self, event_id: &str) -> Result<Vec<DbMarket>> {
        let markets = sqlx::query_as::<_, DbMarket>(
            r#"
            SELECT m.* FROM markets m
            INNER JOIN event_markets em ON m.id = em.market_id
            WHERE em.event_id = ?
            "#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    // ==================== UTILITY ====================

    /// Get database pool reference
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close database connection
    pub async fn close(self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_db() -> MarketDatabase {
        MarketDatabase::new(":memory:").await.unwrap()
    }

    fn create_test_market(id: &str) -> DbMarket {
        DbMarket {
            id: id.to_string(),
            condition_id: format!("0x{}", id),
            question: format!("Test market {}", id),
            slug: Some(format!("test-{}", id)),
            start_date: Utc::now().to_rfc3339(),
            end_date: (Utc::now() + Duration::hours(24)).to_rfc3339(),
            resolution_time: (Utc::now() + Duration::hours(24)).to_rfc3339(),
            active: true,
            closed: false,
            archived: false,
            market_type: Some("binary".to_string()),
            category: Some("crypto".to_string()),
            liquidity: Some("1000".to_string()),
            volume: Some("500".to_string()),
            outcomes: r#"["Yes","No"]"#.to_string(),
            token_ids: r#"["0x1","0x2"]"#.to_string(),
            last_updated: Utc::now().to_rfc3339(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_upsert_and_get_market() {
        let db = create_test_db().await;
        let market = create_test_market("test1");

        db.upsert_market(market.clone()).await.unwrap();

        let retrieved = db.get_market("test1").await.unwrap();
        assert_eq!(retrieved.question, market.question);
    }

    #[tokio::test]
    async fn test_get_active_markets() {
        let db = create_test_db().await;

        db.upsert_market(create_test_market("active1")).await.unwrap();
        db.upsert_market(create_test_market("active2")).await.unwrap();

        let active_markets = db.get_active_markets().await.unwrap();
        assert_eq!(active_markets.len(), 2);
    }

    #[tokio::test]
    async fn test_llm_cache() {
        let db = create_test_db().await;

        let cache_entry = DbLLMCache {
            question: "Test question?".to_string(),
            market_id: "market1".to_string(),
            compatible: true,
            checked_at: Utc::now().to_rfc3339(),
            resolution_time: (Utc::now() + Duration::hours(1)).to_rfc3339(),
        };

        db.insert_llm_cache(cache_entry.clone()).await.unwrap();

        let retrieved = db.get_llm_cache("Test question?").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().compatible, true);
    }
}
