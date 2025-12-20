//! PONG Response Tracker
//!
//! Tracks PONG responses to detect dead/zombie WebSocket connections.
//! A connection is considered unhealthy if no PONG is received within
//! the configured timeout after a PING was sent.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Tracks PONG responses to detect dead connections
///
/// Uses atomic operations for lock-free access from multiple tasks.
/// The tracker stores timestamps as milliseconds since an internal epoch
/// to allow atomic u64 operations.
pub struct PongTracker {
    /// Epoch time when tracking started (for converting Instant to u64)
    epoch: Instant,
    /// Last PING sent (ms since epoch)
    last_ping_sent_ms: AtomicU64,
    /// Last PONG received (ms since epoch)
    last_pong_received_ms: AtomicU64,
    /// Timeout threshold - if no PONG within this duration after PING, connection is unhealthy
    timeout: Duration,
}

impl PongTracker {
    /// Create a new PONG tracker with the specified timeout
    ///
    /// The timeout determines how long to wait for a PONG after sending a PING
    /// before considering the connection dead.
    ///
    /// # Arguments
    /// * `timeout` - Duration to wait for PONG after PING (recommended: 3x heartbeat interval)
    pub fn new(timeout: Duration) -> Self {
        Self {
            epoch: Instant::now(),
            last_ping_sent_ms: AtomicU64::new(0),
            last_pong_received_ms: AtomicU64::new(0),
            timeout,
        }
    }

    /// Record that a PING was just sent
    ///
    /// Call this immediately after sending a PING message.
    pub fn record_ping_sent(&self) {
        let ms = self.epoch.elapsed().as_millis() as u64;
        self.last_ping_sent_ms.store(ms, Ordering::Release);
    }

    /// Record that a PONG was just received
    ///
    /// Call this when a PONG message is detected in the message stream.
    pub fn record_pong_received(&self) {
        let ms = self.epoch.elapsed().as_millis() as u64;
        self.last_pong_received_ms.store(ms, Ordering::Release);
    }

    /// Check if the connection appears healthy
    ///
    /// Returns true if:
    /// - No PING has been sent yet (nothing to check)
    /// - A PONG was received after the last PING
    /// - The timeout hasn't elapsed since the last PING
    ///
    /// Returns false if a PING was sent but no PONG received within the timeout.
    pub fn is_healthy(&self) -> bool {
        let ping_ms = self.last_ping_sent_ms.load(Ordering::Acquire);
        let pong_ms = self.last_pong_received_ms.load(Ordering::Acquire);

        // No pings sent yet = healthy (nothing to check)
        if ping_ms == 0 {
            return true;
        }

        // PONG received after last PING = healthy
        if pong_ms >= ping_ms {
            return true;
        }

        // Check if timeout exceeded since last PING
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        let since_ping_ms = now_ms.saturating_sub(ping_ms);
        since_ping_ms < self.timeout.as_millis() as u64
    }

    /// Get time since last PONG was received
    ///
    /// Returns None if no PONG has ever been received.
    pub fn time_since_last_pong(&self) -> Option<Duration> {
        let pong_ms = self.last_pong_received_ms.load(Ordering::Acquire);
        if pong_ms == 0 {
            return None;
        }
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        Some(Duration::from_millis(now_ms.saturating_sub(pong_ms)))
    }

    /// Get time since last PING was sent
    ///
    /// Returns None if no PING has ever been sent.
    pub fn time_since_last_ping(&self) -> Option<Duration> {
        let ping_ms = self.last_ping_sent_ms.load(Ordering::Acquire);
        if ping_ms == 0 {
            return None;
        }
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        Some(Duration::from_millis(now_ms.saturating_sub(ping_ms)))
    }

    /// Reset the tracker state
    ///
    /// Call this when reconnecting to start fresh.
    pub fn reset(&self) {
        self.last_ping_sent_ms.store(0, Ordering::Release);
        self.last_pong_received_ms.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_healthy_before_first_ping() {
        let tracker = PongTracker::new(Duration::from_secs(15));
        assert!(tracker.is_healthy());
    }

    #[test]
    fn test_healthy_after_pong() {
        let tracker = PongTracker::new(Duration::from_secs(15));
        tracker.record_ping_sent();
        tracker.record_pong_received();
        assert!(tracker.is_healthy());
    }

    #[test]
    fn test_healthy_within_timeout() {
        let tracker = PongTracker::new(Duration::from_millis(100));
        tracker.record_ping_sent();
        // No PONG yet, but within timeout
        assert!(tracker.is_healthy());
    }

    #[test]
    fn test_unhealthy_after_timeout() {
        let tracker = PongTracker::new(Duration::from_millis(50));
        tracker.record_ping_sent();
        sleep(Duration::from_millis(60));
        // Timeout exceeded without PONG
        assert!(!tracker.is_healthy());
    }

    #[test]
    fn test_reset() {
        let tracker = PongTracker::new(Duration::from_millis(50));
        tracker.record_ping_sent();
        sleep(Duration::from_millis(60));
        assert!(!tracker.is_healthy());

        tracker.reset();
        assert!(tracker.is_healthy());
    }

    #[test]
    fn test_time_since_last_pong() {
        let tracker = PongTracker::new(Duration::from_secs(15));
        assert!(tracker.time_since_last_pong().is_none());

        tracker.record_pong_received();
        sleep(Duration::from_millis(10));

        let elapsed = tracker.time_since_last_pong().unwrap();
        assert!(elapsed >= Duration::from_millis(10));
    }
}
