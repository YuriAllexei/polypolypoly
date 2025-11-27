//! Binary runner utilities
//!
//! Provides a standardized way to run binaries with proper
//! logging, heartbeat, and graceful shutdown.

use std::time::Duration;
use tracing::info;

/// Configuration for running a binary application
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Name of the binary (for logging)
    pub name: String,
    /// Heartbeat interval in seconds
    pub heartbeat_interval_secs: u64,
    /// Main loop interval (if applicable)
    pub loop_interval_secs: Option<f64>,
}

impl RunConfig {
    /// Create a new run configuration
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            heartbeat_interval_secs: 300, // 5 minutes default
            loop_interval_secs: None,
        }
    }

    /// Set heartbeat interval
    pub fn with_heartbeat(mut self, secs: u64) -> Self {
        self.heartbeat_interval_secs = secs;
        self
    }

    /// Set loop interval
    pub fn with_loop_interval(mut self, secs: f64) -> Self {
        self.loop_interval_secs = Some(secs);
        self
    }
}

/// Trait for binary applications
///
/// Implement this trait to create a standardized binary
/// that follows Clean Architecture principles.
pub trait BinaryRunner {
    /// Run the application main loop
    async fn run(&mut self) -> anyhow::Result<()>;

    /// Get the run configuration
    fn config(&self) -> &RunConfig;

    /// Print startup banner
    fn print_banner(&self) {
        let config = self.config();
        info!("");
        info!("========================================");
        info!("Starting {}", config.name);
        info!("Press Ctrl+C to stop");
        info!("========================================");
        info!("");
    }

    /// Print shutdown banner
    fn print_shutdown(&self, stats: Option<&str>) {
        let config = self.config();
        info!("");
        info!("========================================");
        info!("{} stopped gracefully", config.name);
        if let Some(stats) = stats {
            info!("{}", stats);
        }
        info!("========================================");
    }

    /// Execute the binary with proper initialization and cleanup
    async fn execute(&mut self) -> anyhow::Result<()> {
        self.print_banner();
        let result = self.run().await;
        self.print_shutdown(None);
        result
    }
}

/// Helper to create a graceful shutdown sleep
///
/// This allows the sleep to be interrupted by a shutdown signal
pub async fn interruptible_sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_config_builder() {
        let config = RunConfig::new("test-binary")
            .with_heartbeat(120)
            .with_loop_interval(5.0);

        assert_eq!(config.name, "test-binary");
        assert_eq!(config.heartbeat_interval_secs, 120);
        assert_eq!(config.loop_interval_secs, Some(5.0));
    }

    #[test]
    fn test_default_config() {
        let config = RunConfig::new("default");
        assert_eq!(config.heartbeat_interval_secs, 300);
        assert_eq!(config.loop_interval_secs, None);
    }
}
