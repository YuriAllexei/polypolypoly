//! Heartbeat logging for long-running processes

use chrono::{DateTime, Utc};
use std::time::Duration;

/// Tracks heartbeat intervals for periodic status logging
pub struct Heartbeat {
    interval: Duration,
    last_beat: DateTime<Utc>,
}

impl Heartbeat {
    /// Create a new heartbeat with the given interval in seconds
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval: Duration::from_secs(interval_secs),
            last_beat: Utc::now(),
        }
    }

    /// Check if enough time has passed since the last beat
    pub fn should_beat(&self) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.last_beat);
        elapsed.to_std().unwrap_or_default() >= self.interval
    }

    /// Record a heartbeat at the current time
    pub fn beat(&mut self) {
        self.last_beat = Utc::now();
    }

    /// Reset the heartbeat timer (alias for beat)
    pub fn reset(&mut self) {
        self.beat();
    }
}
