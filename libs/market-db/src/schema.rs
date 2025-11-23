use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SchemaError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, SchemaError>;

/// Database schema version
pub const SCHEMA_VERSION: i32 = 2;

/// Initialize database schema
pub async fn initialize_schema(pool: &SqlitePool) -> Result<()> {
    // Create markets table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS markets (
            id TEXT PRIMARY KEY,
            condition_id TEXT UNIQUE NOT NULL,
            question TEXT NOT NULL,
            slug TEXT,
            start_date TEXT NOT NULL,
            end_date TEXT NOT NULL,
            resolution_time TEXT NOT NULL,
            active BOOLEAN NOT NULL DEFAULT 1,
            closed BOOLEAN NOT NULL DEFAULT 0,
            archived BOOLEAN NOT NULL DEFAULT 0,
            market_type TEXT,
            category TEXT,
            liquidity TEXT,
            volume TEXT,
            outcomes TEXT,
            token_ids TEXT,
            last_updated TEXT NOT NULL,
            created_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Create indexes for markets
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_markets_resolution ON markets(resolution_time)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_markets_active ON markets(active, closed)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_markets_updated ON markets(last_updated)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_markets_condition ON markets(condition_id)")
        .execute(pool)
        .await?;

    // Create LLM cache table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS llm_cache (
            question TEXT PRIMARY KEY,
            market_id TEXT NOT NULL,
            compatible BOOLEAN NOT NULL,
            checked_at TEXT NOT NULL,
            resolution_time TEXT NOT NULL,
            FOREIGN KEY (market_id) REFERENCES markets(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Create indexes for LLM cache
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_llm_cache_compatible ON llm_cache(compatible)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_llm_cache_checked ON llm_cache(checked_at)")
        .execute(pool)
        .await?;

    // Create events table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            ticker TEXT,
            slug TEXT,
            title TEXT NOT NULL,
            description TEXT,
            start_date TEXT,
            end_date TEXT,
            active BOOLEAN NOT NULL DEFAULT 1,
            closed BOOLEAN NOT NULL DEFAULT 0,
            archived BOOLEAN NOT NULL DEFAULT 0,
            featured BOOLEAN NOT NULL DEFAULT 0,
            restricted BOOLEAN NOT NULL DEFAULT 0,
            liquidity TEXT,
            volume TEXT,
            volume_24hr TEXT,
            volume_1wk TEXT,
            volume_1mo TEXT,
            volume_1yr TEXT,
            open_interest TEXT,
            image TEXT,
            icon TEXT,
            category TEXT,
            competitive TEXT,
            comment_count INTEGER DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_synced TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Create indexes for events
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_closed ON events(closed)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_active ON events(active, closed)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_end_date ON events(end_date)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_category ON events(category)")
        .execute(pool)
        .await?;

    // Create event_markets relationship table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS event_markets (
            event_id TEXT NOT NULL,
            market_id TEXT NOT NULL,
            PRIMARY KEY (event_id, market_id),
            FOREIGN KEY (event_id) REFERENCES events(id),
            FOREIGN KEY (market_id) REFERENCES markets(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_event_markets_event ON event_markets(event_id)")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_event_markets_market ON event_markets(market_id)")
        .execute(pool)
        .await?;

    // Create schema version table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Insert current schema version
    sqlx::query(
        "INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (?, datetime('now'))",
    )
    .bind(SCHEMA_VERSION)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get current schema version
pub async fn get_schema_version(pool: &SqlitePool) -> Result<Option<i32>> {
    let row = sqlx::query_as::<_, (i32,)>("SELECT version FROM schema_version ORDER BY version DESC LIMIT 1")
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|(version,)| version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_schema_initialization() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();

        initialize_schema(&pool).await.unwrap();

        let version = get_schema_version(&pool).await.unwrap();
        assert_eq!(version, Some(SCHEMA_VERSION));
    }
}
