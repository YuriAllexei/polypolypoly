//! Logging initialization

use tracing_subscriber;

/// Initialize tracing with standard configuration
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_line_number(false)
        .init();
}
