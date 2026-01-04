//! In-flight order tracker to prevent duplicate commands.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::debug;

/// Converts price to integer key for HashMap (avoids float comparison issues)
/// 0.7823 → 7823
///
/// Polymarket prices are always in range [0.0, 1.0].
/// This function clamps to valid range for safety.
pub fn price_to_key(price: f64) -> i64 {
    debug_assert!(
        price >= 0.0 && price <= 1.0,
        "Price {} outside valid Polymarket range [0, 1]",
        price
    );

    // Clamp to valid range for safety in release builds
    let clamped = price.clamp(0.0, 1.0);
    (clamped * 10000.0).round() as i64
}

/// Tracks in-flight order operations to prevent duplicate commands.
pub struct InFlightTracker {
    /// Pending cancellations: OID → sent timestamp
    pending_cancels: HashMap<String, Instant>,

    /// Pending placements: (token_id, price_key) → sent timestamp
    pending_placements: HashMap<(String, i64), Instant>,

    /// Time-to-live for pending entries
    ttl: Duration,
}

impl InFlightTracker {
    /// Create a new tracker with the specified TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending_cancels: HashMap::new(),
            pending_placements: HashMap::new(),
            ttl,
        }
    }

    /// Create with default TTL of 5 seconds.
    /// This gives enough time for WebSocket CANCELLATION messages to arrive
    /// before we retry. Too short = repeated cancels, too long = slow recovery.
    pub fn with_default_ttl() -> Self {
        Self::new(Duration::from_secs(5))
    }

    // =========================================================================
    // Cancellation Tracking
    // =========================================================================

    /// Check if we should send a cancel command for this order.
    /// Returns true if cancel should be sent.
    /// Automatically registers the cancel as pending if returning true.
    pub fn should_cancel(&mut self, order_id: &str) -> bool {
        if let Some(sent_at) = self.pending_cancels.get(order_id) {
            if sent_at.elapsed() < self.ttl {
                return false; // Still pending and not expired
            }
            // Expired - fall through to allow retry
        }

        // Register as pending and allow send
        self.pending_cancels.insert(order_id.to_string(), Instant::now());
        true
    }

    /// Call when a cancel command fails - immediately allows retry.
    pub fn cancel_failed(&mut self, order_id: &str) {
        self.pending_cancels.remove(order_id);
    }

    /// Call when a cancel command succeeds (REST API confirms).
    /// This immediately clears the pending cancel, avoiding the need to wait
    /// for cleanup() to see the order removed from OMS.
    ///
    /// IMPORTANT: Call this when REST API confirms cancellation, not when WebSocket
    /// sends the CANCELLATION message. This prevents race conditions where WebSocket
    /// is delayed but REST has already confirmed.
    pub fn cancel_confirmed(&mut self, order_id: &str) {
        if self.pending_cancels.remove(order_id).is_some() {
            debug!(
                "[InFlight] Cancel confirmed (REST): {}",
                &order_id[..16.min(order_id.len())]
            );
        }
    }

    /// Batch version of cancel_confirmed for multiple order IDs.
    pub fn cancels_confirmed(&mut self, order_ids: &[String]) {
        for order_id in order_ids {
            self.cancel_confirmed(order_id);
        }
    }

    /// Check if an order ID is currently pending cancellation.
    pub fn is_cancel_pending(&self, order_id: &str) -> bool {
        self.pending_cancels.get(order_id)
            .map(|sent_at| sent_at.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    /// Mark an order as pending cancellation without blocking logic.
    /// Use this when you want to track that a cancel was sent but don't want
    /// to prevent retries (cancels are idempotent on the exchange).
    pub fn mark_cancel_pending(&mut self, order_id: &str) {
        self.pending_cancels.insert(order_id.to_string(), Instant::now());
    }

    // =========================================================================
    // Placement Tracking
    // =========================================================================

    /// Check if we should send a placement command for this price level.
    /// Returns true if placement should be sent.
    /// Automatically registers the placement as pending if returning true.
    pub fn should_place(&mut self, token_id: &str, price: f64) -> bool {
        let key = (token_id.to_string(), price_to_key(price));

        if let Some(sent_at) = self.pending_placements.get(&key) {
            if sent_at.elapsed() < self.ttl {
                debug!(
                    "[InFlight] BLOCKED place: token={}, price={:.2} (pending for {:?})",
                    &token_id[..8.min(token_id.len())],
                    price,
                    sent_at.elapsed()
                );
                return false; // Still pending and not expired
            }
            debug!(
                "[InFlight] EXPIRED place: token={}, price={:.2} (was pending {:?})",
                &token_id[..8.min(token_id.len())],
                price,
                sent_at.elapsed()
            );
            // Expired - fall through to allow retry
        }

        // Register as pending and allow send
        debug!(
            "[InFlight] ALLOW place: token={}, price={:.2}",
            &token_id[..8.min(token_id.len())],
            price
        );
        self.pending_placements.insert(key, Instant::now());
        true
    }

    /// Call when a placement command fails - immediately allows retry.
    pub fn placement_failed(&mut self, token_id: &str, price: f64) {
        let key = (token_id.to_string(), price_to_key(price));
        self.pending_placements.remove(&key);
    }

    /// Check if a price level is currently pending placement.
    pub fn is_placement_pending(&self, token_id: &str, price: f64) -> bool {
        let key = (token_id.to_string(), price_to_key(price));
        self.pending_placements.get(&key)
            .map(|sent_at| sent_at.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    // =========================================================================
    // Cleanup
    // =========================================================================

    /// Cleanup based on current OMS state. Call this each tick.
    ///
    /// Parameters:
    /// - `open_order_ids`: Set of order IDs that are currently OPEN in OMS
    /// - `open_price_levels`: Set of (token_id, price_key) for OPEN orders in OMS
    pub fn cleanup(
        &mut self,
        open_order_ids: &HashSet<String>,
        open_price_levels: &HashSet<(String, i64)>,
    ) {
        let now = Instant::now();
        let before_cancels = self.pending_cancels.len();
        let before_placements = self.pending_placements.len();

        // Remove cancels if:
        // - Order no longer in OMS (cancel succeeded), OR
        // - Entry expired (allows retry)
        self.pending_cancels.retain(|oid, sent_at| {
            let still_in_oms = open_order_ids.contains(oid);
            let expired = now.duration_since(*sent_at) >= self.ttl;

            // Keep only if: still in OMS AND not expired
            still_in_oms && !expired
        });

        // Remove placements if:
        // - Price level IS in OMS (placement succeeded), OR
        // - Entry expired (allows retry)
        self.pending_placements.retain(|key, sent_at| {
            let in_oms = open_price_levels.contains(key);
            let expired = now.duration_since(*sent_at) >= self.ttl;

            // Keep only if: NOT in OMS AND not expired
            !in_oms && !expired
        });

        let removed_cancels = before_cancels - self.pending_cancels.len();
        let removed_placements = before_placements - self.pending_placements.len();
        if removed_cancels > 0 || removed_placements > 0 {
            debug!(
                "[InFlight] Cleanup: removed {} cancels, {} placements (OMS has {} price levels)",
                removed_cancels,
                removed_placements,
                open_price_levels.len()
            );
        }
    }

    /// Convenience method: Build cleanup sets from list of open orders.
    pub fn cleanup_from_orders(&mut self, open_orders: &[OpenOrderInfo]) {
        let open_order_ids: HashSet<String> = open_orders.iter()
            .map(|o| o.order_id.clone())
            .collect();

        let open_price_levels: HashSet<(String, i64)> = open_orders.iter()
            .map(|o| (o.token_id.clone(), price_to_key(o.price)))
            .collect();

        self.cleanup(&open_order_ids, &open_price_levels);
    }

    // =========================================================================
    // Stats (for logging/debugging)
    // =========================================================================

    pub fn pending_cancel_count(&self) -> usize {
        self.pending_cancels.len()
    }

    pub fn pending_placement_count(&self) -> usize {
        self.pending_placements.len()
    }

    /// Count pending placements for a specific token (not expired).
    /// Used for capacity checking to prevent order accumulation.
    pub fn pending_placements_for_token(&self, token_id: &str) -> usize {
        let now = Instant::now();
        self.pending_placements.iter()
            .filter(|((tid, _), sent_at)| {
                tid == token_id && now.duration_since(**sent_at) < self.ttl
            })
            .count()
    }
}

/// Minimal order info needed for cleanup.
#[derive(Debug, Clone)]
pub struct OpenOrderInfo {
    pub order_id: String,
    pub token_id: String,
    pub price: f64,
}

impl OpenOrderInfo {
    pub fn new(order_id: String, token_id: String, price: f64) -> Self {
        Self { order_id, token_id, price }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn short_ttl() -> Duration {
        Duration::from_millis(50)
    }

    #[test]
    fn test_duplicate_cancel_prevented() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // First call should allow cancel
        assert!(tracker.should_cancel("order-123"));

        // Second call within TTL should prevent duplicate
        assert!(!tracker.should_cancel("order-123"));

        // Different order should still be allowed
        assert!(tracker.should_cancel("order-456"));
    }

    #[test]
    fn test_duplicate_placement_prevented() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // First call should allow placement
        assert!(tracker.should_place("up_token", 0.55));

        // Second call within TTL should prevent duplicate
        assert!(!tracker.should_place("up_token", 0.55));

        // Different price should still be allowed
        assert!(tracker.should_place("up_token", 0.56));
    }

    #[test]
    fn test_cancel_retry_after_ttl_expires() {
        let mut tracker = InFlightTracker::new(short_ttl());

        // First call
        assert!(tracker.should_cancel("order-123"));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        // Should allow retry after TTL
        assert!(tracker.should_cancel("order-123"));
    }

    #[test]
    fn test_placement_retry_after_ttl_expires() {
        let mut tracker = InFlightTracker::new(short_ttl());

        // First call
        assert!(tracker.should_place("up_token", 0.55));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        // Should allow retry after TTL
        assert!(tracker.should_place("up_token", 0.55));
    }

    #[test]
    fn test_cleanup_removes_cancelled_orders() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // Register a pending cancel
        tracker.should_cancel("order-123");
        assert!(tracker.is_cancel_pending("order-123"));

        // Cleanup with order-123 NOT in OMS (cancel succeeded)
        let open_ids: HashSet<String> = HashSet::new();
        let open_levels: HashSet<(String, i64)> = HashSet::new();
        tracker.cleanup(&open_ids, &open_levels);

        // Should no longer be pending
        assert!(!tracker.is_cancel_pending("order-123"));
    }

    #[test]
    fn test_cleanup_removes_placed_orders() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // Register a pending placement
        tracker.should_place("up_token", 0.55);
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Cleanup with this price level IN OMS (placement succeeded)
        let open_ids: HashSet<String> = HashSet::new();
        let mut open_levels: HashSet<(String, i64)> = HashSet::new();
        open_levels.insert(("up_token".to_string(), price_to_key(0.55)));
        tracker.cleanup(&open_ids, &open_levels);

        // Should no longer be pending
        assert!(!tracker.is_placement_pending("up_token", 0.55));
    }

    #[test]
    fn test_token_id_isolation() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // Place for up_token at 0.55
        assert!(tracker.should_place("up_token", 0.55));

        // Should still allow placement for down_token at same price
        assert!(tracker.should_place("down_token", 0.55));

        // But duplicate on up_token should be blocked
        assert!(!tracker.should_place("up_token", 0.55));
    }

    #[test]
    fn test_cancel_failed_allows_retry() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // First call
        assert!(tracker.should_cancel("order-123"));
        assert!(tracker.is_cancel_pending("order-123"));

        // Simulate failure
        tracker.cancel_failed("order-123");

        // Should immediately allow retry
        assert!(!tracker.is_cancel_pending("order-123"));
        assert!(tracker.should_cancel("order-123"));
    }

    #[test]
    fn test_placement_failed_allows_retry() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // First call
        assert!(tracker.should_place("up_token", 0.55));
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Simulate failure
        tracker.placement_failed("up_token", 0.55);

        // Should immediately allow retry
        assert!(!tracker.is_placement_pending("up_token", 0.55));
        assert!(tracker.should_place("up_token", 0.55));
    }

    #[test]
    fn test_price_to_key() {
        // Normal cases
        assert_eq!(price_to_key(0.55), 5500);
        assert_eq!(price_to_key(0.7823), 7823);
        assert_eq!(price_to_key(0.01), 100);
        assert_eq!(price_to_key(1.0), 10000);
        assert_eq!(price_to_key(0.0), 0);

        // Rounding behavior
        assert_eq!(price_to_key(0.55005), 5501); // rounds up
        assert_eq!(price_to_key(0.55004), 5500); // rounds down
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore)]
    fn test_price_to_key_clamps_invalid() {
        // In release builds, invalid prices are clamped (no panic)
        // This test is ignored in debug mode since debug_assert fires
        // Negative price clamps to 0
        assert_eq!(price_to_key(-0.5), 0);

        // Price > 1.0 clamps to 10000
        assert_eq!(price_to_key(1.5), 10000);
        assert_eq!(price_to_key(100.0), 10000);
    }

    #[test]
    fn test_cleanup_from_orders() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        // Register pending operations
        tracker.should_cancel("order-123");
        tracker.should_place("up_token", 0.55);

        // Simulate: order-123 still exists, but placement at 0.55 succeeded
        let open_orders = vec![
            OpenOrderInfo::new("order-123".to_string(), "up_token".to_string(), 0.60),
            OpenOrderInfo::new("order-456".to_string(), "up_token".to_string(), 0.55),
        ];

        tracker.cleanup_from_orders(&open_orders);

        // Cancel should still be pending (order still in OMS)
        assert!(tracker.is_cancel_pending("order-123"));

        // Placement should be cleared (price level now in OMS)
        assert!(!tracker.is_placement_pending("up_token", 0.55));
    }

    #[test]
    fn test_stats() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        assert_eq!(tracker.pending_cancel_count(), 0);
        assert_eq!(tracker.pending_placement_count(), 0);

        tracker.should_cancel("order-1");
        tracker.should_cancel("order-2");
        tracker.should_place("up_token", 0.55);

        assert_eq!(tracker.pending_cancel_count(), 2);
        assert_eq!(tracker.pending_placement_count(), 1);
    }
}
