pub mod models;
pub mod schema;

use chrono::{DateTime, Duration, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder};
use thiserror::Error;
use tracing::{debug, info};

// Re-export main types
pub use models::{DbEvent, DbMarket, MarketFilters, SyncStats};
pub use schema::{get_schema_version, initialize_schema};

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
    pool: PgPool,
}

impl MarketDatabase {
    /// Create new database connection and initialize schema
    pub async fn new(db_url: &str) -> Result<Self> {
        info!("Connecting to database: {}", db_url);

        // Connect to database
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(db_url)
            .await?;

        // Initialize schema
        schema::initialize_schema(&pool).await?;

        info!("Database initialized successfully");

        Ok(Self { pool })
    }

    // ==================== MARKET OPERATIONS ====================

    /// Insert a single market (or replace if exists)
    pub async fn upsert_market(&self, market: DbMarket) -> Result<()> {
        debug!(
            market_id = %market.id,
            question = %market.question,
            "Upserting market"
        );
        sqlx::query(
            r#"
            INSERT INTO markets (
                id, condition_id, question, description, slug, start_date, end_date, resolution_time,
                active, closed, archived, market_type, category, liquidity, volume,
                outcomes, token_ids, tags, last_updated, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ON CONFLICT (id) DO UPDATE SET
                condition_id = EXCLUDED.condition_id,
                question = EXCLUDED.question,
                description = EXCLUDED.description,
                slug = EXCLUDED.slug,
                start_date = EXCLUDED.start_date,
                end_date = EXCLUDED.end_date,
                resolution_time = EXCLUDED.resolution_time,
                active = EXCLUDED.active,
                closed = EXCLUDED.closed,
                archived = EXCLUDED.archived,
                market_type = EXCLUDED.market_type,
                category = EXCLUDED.category,
                liquidity = EXCLUDED.liquidity,
                volume = EXCLUDED.volume,
                outcomes = EXCLUDED.outcomes,
                token_ids = EXCLUDED.token_ids,
                tags = EXCLUDED.tags,
                last_updated = EXCLUDED.last_updated
            "#,
        )
        .bind(&market.id)
        .bind(&market.condition_id)
        .bind(&market.question)
        .bind(&market.description)
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
        .bind(&market.tags)
        .bind(&market.last_updated)
        .bind(&market.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Batch insert markets (legacy - uses individual upserts)
    pub async fn insert_markets(&self, markets: Vec<DbMarket>) -> Result<usize> {
        let mut count = 0;

        for market in markets {
            self.upsert_market(market).await?;
            count += 1;
        }

        debug!("Inserted {} markets", count);
        Ok(count)
    }

    /// Batch upsert multiple markets efficiently using multi-value INSERT
    /// Returns the number of markets upserted
    pub async fn batch_upsert_markets(&self, markets: &[DbMarket]) -> Result<usize> {
        if markets.is_empty() {
            return Ok(0);
        }

        // PostgreSQL has a limit on parameters, so we batch in chunks
        const BATCH_SIZE: usize = 100;
        let mut total_upserted = 0;

        for chunk in markets.chunks(BATCH_SIZE) {
            let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
                r#"INSERT INTO markets (
                    id, condition_id, question, description, slug, start_date, end_date, resolution_time,
                    active, closed, archived, market_type, category, liquidity, volume,
                    outcomes, token_ids, tags, last_updated, created_at
                ) "#,
            );

            query_builder.push_values(chunk, |mut b, market| {
                b.push_bind(&market.id)
                    .push_bind(&market.condition_id)
                    .push_bind(&market.question)
                    .push_bind(&market.description)
                    .push_bind(&market.slug)
                    .push_bind(&market.start_date)
                    .push_bind(&market.end_date)
                    .push_bind(&market.resolution_time)
                    .push_bind(market.active)
                    .push_bind(market.closed)
                    .push_bind(market.archived)
                    .push_bind(&market.market_type)
                    .push_bind(&market.category)
                    .push_bind(&market.liquidity)
                    .push_bind(&market.volume)
                    .push_bind(&market.outcomes)
                    .push_bind(&market.token_ids)
                    .push_bind(&market.tags)
                    .push_bind(&market.last_updated)
                    .push_bind(&market.created_at);
            });

            query_builder.push(
                r#" ON CONFLICT (id) DO UPDATE SET
                    condition_id = EXCLUDED.condition_id,
                    question = EXCLUDED.question,
                    description = EXCLUDED.description,
                    slug = EXCLUDED.slug,
                    start_date = EXCLUDED.start_date,
                    end_date = EXCLUDED.end_date,
                    resolution_time = EXCLUDED.resolution_time,
                    active = EXCLUDED.active,
                    closed = EXCLUDED.closed,
                    archived = EXCLUDED.archived,
                    market_type = EXCLUDED.market_type,
                    category = EXCLUDED.category,
                    liquidity = EXCLUDED.liquidity,
                    volume = EXCLUDED.volume,
                    outcomes = EXCLUDED.outcomes,
                    token_ids = EXCLUDED.token_ids,
                    tags = EXCLUDED.tags,
                    last_updated = EXCLUDED.last_updated"#,
            );

            let query = query_builder.build();
            query.execute(&self.pool).await?;
            total_upserted += chunk.len();
        }

        Ok(total_upserted)
    }

    /// Get all active markets
    pub async fn get_active_markets(&self) -> Result<Vec<DbMarket>> {
        let markets = sqlx::query_as::<_, DbMarket>(
            "SELECT * FROM markets WHERE active = true AND closed = false ORDER BY resolution_time ASC",
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
            WHERE active = true
            AND closed = false
            AND resolution_time > $1
            AND resolution_time <= $2
            ORDER BY resolution_time ASC
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(cutoff.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get markets expiring within the next X seconds (supports fractional seconds)
    pub async fn get_markets_expiring_soon(&self, within_seconds: f64) -> Result<Vec<DbMarket>> {
        let now = Utc::now();
        let cutoff = now + Duration::milliseconds((within_seconds * 1000.0) as i64);

        let markets = sqlx::query_as::<_, DbMarket>(
            r#"
            SELECT * FROM markets
            WHERE active = true
            AND closed = false
            AND resolution_time > $1
            AND resolution_time <= $2
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
            "SELECT * FROM markets WHERE last_updated > $1 ORDER BY last_updated DESC",
        )
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get market by ID
    pub async fn get_market(&self, id: &str) -> Result<DbMarket> {
        let market = sqlx::query_as::<_, DbMarket>("SELECT * FROM markets WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DatabaseError::MarketNotFound(id.to_string()))?;

        Ok(market)
    }

    /// Get market by condition ID
    pub async fn get_market_by_condition(&self, condition_id: &str) -> Result<DbMarket> {
        let market = sqlx::query_as::<_, DbMarket>("SELECT * FROM markets WHERE condition_id = $1")
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
            "SELECT COUNT(*) FROM markets WHERE active = true AND closed = false",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Delete resolved markets older than cutoff date
    pub async fn cleanup_resolved(&self, before: DateTime<Utc>) -> Result<u64> {
        let result =
            sqlx::query("DELETE FROM markets WHERE closed = true AND resolution_time < $1")
                .bind(before.to_rfc3339())
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected())
    }

    // ==================== EVENT OPERATIONS ====================

    /// Insert or update an event
    pub async fn upsert_event(&self, event: DbEvent) -> Result<()> {
        debug!(
            event_id = %event.id,
            title = %event.title,
            "Upserting event"
        );
        sqlx::query(
            r#"
            INSERT INTO events (
                id, ticker, slug, title, description, start_date, end_date,
                active, closed, archived, featured, restricted,
                liquidity, volume, volume_24hr, volume_1wk, volume_1mo, volume_1yr,
                open_interest, image, icon, category, competitive, tags, comment_count,
                created_at, updated_at, last_synced
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28)
            ON CONFLICT (id) DO UPDATE SET
                ticker = EXCLUDED.ticker,
                slug = EXCLUDED.slug,
                title = EXCLUDED.title,
                description = EXCLUDED.description,
                start_date = EXCLUDED.start_date,
                end_date = EXCLUDED.end_date,
                active = EXCLUDED.active,
                closed = EXCLUDED.closed,
                archived = EXCLUDED.archived,
                featured = EXCLUDED.featured,
                restricted = EXCLUDED.restricted,
                liquidity = EXCLUDED.liquidity,
                volume = EXCLUDED.volume,
                volume_24hr = EXCLUDED.volume_24hr,
                volume_1wk = EXCLUDED.volume_1wk,
                volume_1mo = EXCLUDED.volume_1mo,
                volume_1yr = EXCLUDED.volume_1yr,
                open_interest = EXCLUDED.open_interest,
                image = EXCLUDED.image,
                icon = EXCLUDED.icon,
                category = EXCLUDED.category,
                competitive = EXCLUDED.competitive,
                tags = EXCLUDED.tags,
                comment_count = EXCLUDED.comment_count,
                updated_at = EXCLUDED.updated_at,
                last_synced = EXCLUDED.last_synced
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
        .bind(&event.tags)
        .bind(event.comment_count)
        .bind(&event.created_at)
        .bind(&event.updated_at)
        .bind(&event.last_synced)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Batch upsert multiple events efficiently using multi-value INSERT
    /// Returns the number of events upserted
    pub async fn batch_upsert_events(&self, events: &[DbEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }

        // PostgreSQL has a limit on parameters, so we batch in chunks
        // Events have 27 columns, so use smaller batch size
        const BATCH_SIZE: usize = 50;
        let mut total_upserted = 0;

        for chunk in events.chunks(BATCH_SIZE) {
            let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
                r#"INSERT INTO events (
                    id, ticker, slug, title, description, start_date, end_date,
                    active, closed, archived, featured, restricted,
                    liquidity, volume, volume_24hr, volume_1wk, volume_1mo, volume_1yr,
                    open_interest, image, icon, category, competitive, tags, comment_count,
                    created_at, updated_at, last_synced
                ) "#,
            );

            query_builder.push_values(chunk, |mut b, event| {
                b.push_bind(&event.id)
                    .push_bind(&event.ticker)
                    .push_bind(&event.slug)
                    .push_bind(&event.title)
                    .push_bind(&event.description)
                    .push_bind(&event.start_date)
                    .push_bind(&event.end_date)
                    .push_bind(event.active)
                    .push_bind(event.closed)
                    .push_bind(event.archived)
                    .push_bind(event.featured)
                    .push_bind(event.restricted)
                    .push_bind(&event.liquidity)
                    .push_bind(&event.volume)
                    .push_bind(&event.volume_24hr)
                    .push_bind(&event.volume_1wk)
                    .push_bind(&event.volume_1mo)
                    .push_bind(&event.volume_1yr)
                    .push_bind(&event.open_interest)
                    .push_bind(&event.image)
                    .push_bind(&event.icon)
                    .push_bind(&event.category)
                    .push_bind(&event.competitive)
                    .push_bind(&event.tags)
                    .push_bind(event.comment_count)
                    .push_bind(&event.created_at)
                    .push_bind(&event.updated_at)
                    .push_bind(&event.last_synced);
            });

            query_builder.push(
                r#" ON CONFLICT (id) DO UPDATE SET
                    ticker = EXCLUDED.ticker,
                    slug = EXCLUDED.slug,
                    title = EXCLUDED.title,
                    description = EXCLUDED.description,
                    start_date = EXCLUDED.start_date,
                    end_date = EXCLUDED.end_date,
                    active = EXCLUDED.active,
                    closed = EXCLUDED.closed,
                    archived = EXCLUDED.archived,
                    featured = EXCLUDED.featured,
                    restricted = EXCLUDED.restricted,
                    liquidity = EXCLUDED.liquidity,
                    volume = EXCLUDED.volume,
                    volume_24hr = EXCLUDED.volume_24hr,
                    volume_1wk = EXCLUDED.volume_1wk,
                    volume_1mo = EXCLUDED.volume_1mo,
                    volume_1yr = EXCLUDED.volume_1yr,
                    open_interest = EXCLUDED.open_interest,
                    image = EXCLUDED.image,
                    icon = EXCLUDED.icon,
                    category = EXCLUDED.category,
                    competitive = EXCLUDED.competitive,
                    tags = EXCLUDED.tags,
                    comment_count = EXCLUDED.comment_count,
                    updated_at = EXCLUDED.updated_at,
                    last_synced = EXCLUDED.last_synced"#,
            );

            let query = query_builder.build();
            query.execute(&self.pool).await?;
            total_upserted += chunk.len();
        }

        Ok(total_upserted)
    }

    /// Link an event to its markets
    /// Silently skips markets that don't exist (FK constraint violations)
    pub async fn link_event_markets(&self, event_id: &str, market_ids: &[String]) -> Result<()> {
        for market_id in market_ids {
            let result = sqlx::query("INSERT INTO event_markets (event_id, market_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(event_id)
                .bind(market_id)
                .execute(&self.pool)
                .await;

            // Silently skip FK constraint violations (market doesn't exist yet)
            if let Err(e) = result {
                let err_str = e.to_string();
                if err_str.contains("foreign key constraint")
                    || err_str.contains("violates foreign key")
                {
                    debug!("Skipping link for non-existent market: {}", market_id);
                    continue;
                }
                return Err(e.into());
            }
        }

        Ok(())
    }

    /// Batch link events to their markets efficiently
    /// Takes a slice of (event_id, market_id) tuples
    /// Silently ignores FK constraint violations
    pub async fn batch_link_event_markets(&self, links: &[(String, String)]) -> Result<usize> {
        if links.is_empty() {
            return Ok(0);
        }

        const BATCH_SIZE: usize = 500;
        let mut total_linked = 0;

        for chunk in links.chunks(BATCH_SIZE) {
            let mut query_builder: QueryBuilder<Postgres> =
                QueryBuilder::new("INSERT INTO event_markets (event_id, market_id) ");

            query_builder.push_values(chunk, |mut b, (event_id, market_id)| {
                b.push_bind(event_id).push_bind(market_id);
            });

            query_builder.push(" ON CONFLICT DO NOTHING");

            let query = query_builder.build();
            let result = query.execute(&self.pool).await;

            // Silently handle FK constraint violations
            match result {
                Ok(r) => total_linked += r.rows_affected() as usize,
                Err(e) => {
                    let err_str = e.to_string();
                    if !err_str.contains("foreign key constraint")
                        && !err_str.contains("violates foreign key")
                    {
                        return Err(e.into());
                    }
                    // FK violation - some links failed, but that's expected
                }
            }
        }

        Ok(total_linked)
    }

    /// Check if event exists in database
    pub async fn event_exists(&self, event_id: &str) -> Result<bool> {
        let result = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0 > 0)
    }

    /// Get event by ID
    pub async fn get_event(&self, event_id: &str) -> Result<DbEvent> {
        let event = sqlx::query_as::<_, DbEvent>("SELECT * FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DatabaseError::EventNotFound(event_id.to_string()))?;

        Ok(event)
    }

    /// Get all active (non-closed) events
    pub async fn get_active_events(&self) -> Result<Vec<DbEvent>> {
        let events = sqlx::query_as::<_, DbEvent>(
            "SELECT * FROM events WHERE closed = false ORDER BY end_date ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    /// Get events by category
    pub async fn get_events_by_category(&self, category: &str) -> Result<Vec<DbEvent>> {
        let events = sqlx::query_as::<_, DbEvent>(
            "SELECT * FROM events WHERE category = $1 AND closed = false ORDER BY end_date ASC",
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    /// Get events by tag labels (matches events that have ALL specified tags)
    pub async fn get_events_by_tags(&self, tag_labels: &[&str]) -> Result<Vec<DbEvent>> {
        if tag_labels.is_empty() {
            return Ok(vec![]);
        }

        // Build placeholders: $1, $2, $3, ...
        let placeholders: Vec<String> = (1..=tag_labels.len()).map(|i| format!("${}", i)).collect();
        let placeholders_str = placeholders.join(", ");

        let query = format!(
            r#"
            SELECT e.*
            FROM events e
            WHERE e.closed = false
              AND e.tags IS NOT NULL
              AND (SELECT COUNT(DISTINCT tag->>'label')
                   FROM jsonb_array_elements(e.tags::jsonb) AS tag
                   WHERE tag->>'label' IN ({})) = ${}
            ORDER BY e.end_date ASC
            "#,
            placeholders_str,
            tag_labels.len() + 1
        );

        let mut query_builder = sqlx::query_as::<_, DbEvent>(&query);
        for label in tag_labels {
            query_builder = query_builder.bind(*label);
        }
        query_builder = query_builder.bind(tag_labels.len() as i64);

        let events = query_builder.fetch_all(&self.pool).await?;
        Ok(events)
    }

    /// Get markets by tag labels (matches markets that have ALL specified tags)
    pub async fn get_markets_by_tags(&self, tag_labels: &[&str]) -> Result<Vec<DbMarket>> {
        if tag_labels.is_empty() {
            return Ok(vec![]);
        }

        // Build placeholders: $1, $2, $3, ...
        let placeholders: Vec<String> = (1..=tag_labels.len()).map(|i| format!("${}", i)).collect();
        let placeholders_str = placeholders.join(", ");

        let query = format!(
            r#"
            SELECT m.*
            FROM markets m
            WHERE m.closed = false
              AND m.tags IS NOT NULL
              AND (SELECT COUNT(DISTINCT tag->>'label')
                   FROM jsonb_array_elements(m.tags::jsonb) AS tag
                   WHERE tag->>'label' IN ({})) = ${}
            ORDER BY m.end_date ASC
            "#,
            placeholders_str,
            tag_labels.len() + 1
        );

        let mut query_builder = sqlx::query_as::<_, DbMarket>(&query);
        for label in tag_labels {
            query_builder = query_builder.bind(*label);
        }
        query_builder = query_builder.bind(tag_labels.len() as i64);

        let markets = query_builder.fetch_all(&self.pool).await?;
        Ok(markets)
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
        let (count,) =
            sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM events WHERE closed = false")
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
            WHERE em.event_id = $1
            "#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(markets)
    }

    /// Get event ID for a specific market (reverse lookup)
    pub async fn get_market_event_id(&self, market_id: &str) -> Result<Option<String>> {
        let result = sqlx::query_as::<_, (String,)>(
            "SELECT event_id FROM event_markets WHERE market_id = $1",
        )
        .bind(market_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|(event_id,)| event_id))
    }

    // ==================== UTILITY ====================

    /// Get database pool reference
    pub fn pool(&self) -> &PgPool {
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

    // Tests require a running Postgres instance.
    // To run tests, set DATABASE_URL and ensure the database exists.
    // For now, these tests are disabled in the Docker build environment.
}
