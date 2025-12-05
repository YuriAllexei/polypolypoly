//! Logging initialization

use tracing_subscriber::EnvFilter;

/// Initialize tracing with standard configuration (defaults to info level)
pub fn init_tracing() {
    init_tracing_with_level("info");
}

/// Initialize tracing with a specific log level
///
/// The level can be: error, warn, info, debug, trace
/// RUST_LOG environment variable can override the configured level
pub fn init_tracing_with_level(level: &str) {
    // Build filter: use RUST_LOG if set, otherwise use the provided level
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            // Default filter for our crates at the specified level
            // sqlx=warn silences the verbose query logs at debug level
            EnvFilter::new(format!(
                "sqlx=warn,polymarket={level},polymarket_arb_bot={level},hypersockets={level},{level}",
                level = level
            ))
        });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)  // Show module path for context
        .with_thread_ids(false)
        .with_line_number(false)
        .init();
}
