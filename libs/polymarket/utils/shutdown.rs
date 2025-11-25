//! Graceful shutdown management

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::sleep;
use tracing::info;

/// Manages graceful shutdown for long-running processes
pub struct ShutdownManager {
    flag: Arc<AtomicBool>,
}

impl ShutdownManager {
    /// Create a new shutdown manager with running state
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Spawn a Ctrl+C signal handler that triggers shutdown
    pub fn spawn_signal_handler(&self) {
        let flag = Arc::clone(&self.flag);
        tokio::spawn(async move {
            if signal::ctrl_c().await.is_ok() {
                info!("");
                info!("Received shutdown signal (Ctrl+C)");
                info!("Shutting down gracefully...");
                flag.store(false, Ordering::Release);
            }
        });
    }

    /// Check if the process should continue running
    pub fn is_running(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    /// Get a clone of the shutdown flag for passing to async tasks
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    /// Sleep for a duration, but wake early if shutdown is triggered
    pub async fn interruptible_sleep(&self, duration: Duration) {
        let check_interval = Duration::from_millis(50);
        let mut elapsed = Duration::ZERO;

        while elapsed < duration && self.is_running() {
            sleep(check_interval).await;
            elapsed += check_interval;
        }
    }
}

impl Default for ShutdownManager {
    fn default() -> Self {
        Self::new()
    }
}
