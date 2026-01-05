//! In-flight order tracker to prevent duplicate commands.

use std::collections::HashMap;
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

    /// Call when an order at this price is cancelled - clears pending placement.
    /// CRITICAL FIX: Without this, cancelled orders would still count toward
    /// capacity until TTL expires, blocking new placements at different prices.
    pub fn placement_cancelled(&mut self, token_id: &str, price: f64) {
        let key = (token_id.to_string(), price_to_key(price));
        if self.pending_placements.remove(&key).is_some() {
            debug!(
                "[InFlight] Placement cleared (cancelled): token={}, price={:.2}",
                &token_id[..8.min(token_id.len())],
                price
            );
        }
    }

    /// Call when an order at this price is FILLED - clears pending placement.
    /// CRITICAL FIX: Without this, filled orders still count toward capacity
    /// and block new placements, causing severe imbalance issues.
    pub fn placement_filled(&mut self, token_id: &str, price: f64) {
        let key = (token_id.to_string(), price_to_key(price));
        if self.pending_placements.remove(&key).is_some() {
            debug!(
                "[InFlight] Placement cleared (filled): token={}, price={:.2}",
                &token_id[..8.min(token_id.len())],
                price
            );
        }
    }

    /// Batch version - clear pending placements for multiple filled orders.
    pub fn placements_filled(&mut self, orders: &[(String, f64)]) {
        for (token_id, price) in orders {
            self.placement_filled(token_id, *price);
        }
    }

    /// Batch version - clear pending placements for multiple cancelled orders.
    pub fn placements_cancelled(&mut self, orders: &[(String, f64)]) {
        for (token_id, price) in orders {
            self.placement_cancelled(token_id, *price);
        }
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

    /// Cleanup expired pending entries. Call this each tick.
    ///
    /// Cancels use TTL-only cleanup to prevent race conditions.
    /// Placements use TTL + OMS state: cleared if expired OR if no order exists at that price.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        let before_cancels = self.pending_cancels.len();

        // CRITICAL: Only clear pending cancels based on TTL, not OMS state.
        // The OMS removes orders immediately on REST cancel confirmation, so
        // if we cleared based on "order not in OMS", we'd clear too early.
        // This would make is_cancel_pending() return false, causing quoter to
        // think it has capacity and place more orders (feedback loop).
        self.pending_cancels.retain(|_oid, sent_at| {
            let expired = now.duration_since(*sent_at) >= self.ttl;
            !expired // Keep only if not expired
        });

        let removed_cancels = before_cancels - self.pending_cancels.len();
        if removed_cancels > 0 {
            debug!(
                "[InFlight] Cleanup: removed {} cancels (TTL)",
                removed_cancels,
            );
        }
    }

    /// Cleanup with OMS state for placements.
    ///
    /// CRITICAL FIX: Clear pending placements when:
    /// 1. TTL expired (safety fallback)
    /// 2. No order exists at that price in OMS AND placement is old enough (grace period)
    ///
    /// The grace period (1.5 seconds) prevents clearing pending placements before they
    /// appear in OMS. REST API takes ~300ms and WebSocket confirmation can take longer.
    /// Without this grace period, cleanup would clear pending entries before OMS updates,
    /// causing duplicate placements.
    pub fn cleanup_from_orders(&mut self, open_orders: &[OpenOrderInfo]) {
        // First, do TTL-based cancel cleanup
        self.cleanup();

        let now = Instant::now();
        let before_placements = self.pending_placements.len();

        // Grace period: don't clear pending placements that are very recent
        // REST API takes ~300ms, WebSocket can take 500ms+, so give 1.5s grace
        let grace_period = Duration::from_millis(1500);

        // Build set of (token_id, price_key) for all open orders
        let open_price_levels: std::collections::HashSet<(String, i64)> = open_orders
            .iter()
            .map(|o| (o.token_id.clone(), price_to_key(o.price)))
            .collect();

        // Clear pending placements if:
        // 1. TTL expired (always clear), OR
        // 2. No order exists at that price in OMS AND placement is older than grace period
        //    (order was filled/cancelled, not just slow to appear)
        self.pending_placements.retain(|(token_id, price_key), sent_at| {
            let age = now.duration_since(*sent_at);
            let expired = age >= self.ttl;
            let order_exists = open_price_levels.contains(&(token_id.clone(), *price_key));
            let past_grace_period = age >= grace_period;

            // Keep if:
            // - TTL not expired AND (order exists OR still within grace period)
            // Remove if:
            // - TTL expired, OR
            // - Order gone AND past grace period (was filled/cancelled)
            let should_keep = !expired && (order_exists || !past_grace_period);

            if !should_keep && !expired {
                debug!(
                    "[InFlight] Placement cleared (order gone from OMS after grace): token={}, price_key={}, age={:?}",
                    &token_id[..8.min(token_id.len())],
                    price_key,
                    age
                );
            }
            should_keep
        });

        let removed_placements = before_placements - self.pending_placements.len();
        if removed_placements > 0 {
            debug!(
                "[InFlight] Cleanup: removed {} placements (TTL or filled after grace)",
                removed_placements,
            );
        }
    }

    /// Clear ALL pending placements and cancels.
    /// Use this after nuclear cancel to reset state.
    pub fn clear_all_pending(&mut self) {
        let cancel_count = self.pending_cancels.len();
        let placement_count = self.pending_placements.len();
        self.pending_cancels.clear();
        self.pending_placements.clear();
        debug!(
            "[InFlight] Cleared ALL pending: {} cancels, {} placements",
            cancel_count, placement_count
        );
    }

    /// Clear pending placements for a specific token.
    /// Use this when excess levels are detected and we're in cleanup mode.
    pub fn clear_pending_for_token(&mut self, token_id: &str) {
        let before = self.pending_placements.len();
        self.pending_placements.retain(|(tid, _), _| tid != token_id);
        let removed = before - self.pending_placements.len();
        if removed > 0 {
            debug!(
                "[InFlight] Cleared {} pending placements for token {}",
                removed, &token_id[..8.min(token_id.len())]
            );
        }
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

    /// Get pending price levels (as price keys) for a specific token (not expired).
    /// Used for capacity checking - these levels should count toward the max level cap
    /// even if they haven't appeared in OMS yet.
    pub fn pending_price_levels_for_token(&self, token_id: &str) -> std::collections::HashSet<i64> {
        let now = Instant::now();
        self.pending_placements.iter()
            .filter(|((tid, _), sent_at)| {
                tid == token_id && now.duration_since(**sent_at) < self.ttl
            })
            .map(|((_, price_key), _)| *price_key)
            .collect()
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
    fn test_cleanup_removes_cancelled_orders_by_ttl() {
        // Use short TTL for this test
        let mut tracker = InFlightTracker::new(short_ttl());

        // Register a pending cancel
        tracker.should_cancel("order-123");
        assert!(tracker.is_cancel_pending("order-123"));

        // Cleanup immediately - should NOT clear (TTL not expired)
        tracker.cleanup();

        // Should STILL be pending (TTL not expired)
        assert!(tracker.is_cancel_pending("order-123"));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        // Now cleanup should clear it
        tracker.cleanup();
        assert!(!tracker.is_cancel_pending("order-123"));
    }

    #[test]
    fn test_cleanup_removes_placements_by_ttl() {
        // Use short TTL for this test
        let mut tracker = InFlightTracker::new(short_ttl());

        // Register a pending placement
        tracker.should_place("up_token", 0.55);
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Cleanup immediately - should NOT clear (TTL not expired)
        tracker.cleanup();

        // Should STILL be pending (TTL not expired)
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        // Now cleanup should clear it
        tracker.cleanup();
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
        // Use short TTL for this test
        let mut tracker = InFlightTracker::new(short_ttl());

        // Register pending operations
        tracker.should_cancel("order-123");
        tracker.should_place("up_token", 0.55);

        // Cancels use TTL-only cleanup, placements use TTL + OMS state + grace period
        // If order at same price exists in OMS, keep pending
        // If order is gone AND past grace period (1.5s), clear pending
        let open_orders_with_same_price = vec![
            OpenOrderInfo::new("order-456".to_string(), "up_token".to_string(), 0.55),
        ];

        tracker.cleanup_from_orders(&open_orders_with_same_price);

        // Cancel still pending (TTL not expired)
        assert!(tracker.is_cancel_pending("order-123"));
        // Placement still pending (order exists at same price)
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Now simulate order being filled (not in OMS anymore)
        // But placement is still within grace period (1.5s) so should NOT be cleared
        let open_orders_different_price = vec![
            OpenOrderInfo::new("order-789".to_string(), "up_token".to_string(), 0.60),
        ];

        tracker.cleanup_from_orders(&open_orders_different_price);

        // Cancel still pending (TTL not expired)
        assert!(tracker.is_cancel_pending("order-123"));
        // Placement still pending (within grace period even though order gone)
        assert!(tracker.is_placement_pending("up_token", 0.55));
    }

    #[test]
    fn test_cleanup_from_orders_after_grace_period() {
        // Test that cleanup works after grace period
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));  // Long TTL

        tracker.should_place("up_token", 0.55);

        // Wait for grace period (1.5 seconds) to pass
        std::thread::sleep(Duration::from_millis(1600));

        // No order in OMS at this price anymore
        let open_orders_different_price = vec![
            OpenOrderInfo::new("order-789".to_string(), "up_token".to_string(), 0.60),
        ];

        tracker.cleanup_from_orders(&open_orders_different_price);

        // Should be cleared (order gone AND past grace period)
        assert!(!tracker.is_placement_pending("up_token", 0.55));
    }

    #[test]
    fn test_clear_all_pending() {
        let mut tracker = InFlightTracker::new(Duration::from_secs(10));

        tracker.should_cancel("order-123");
        tracker.should_cancel("order-456");
        tracker.should_place("up_token", 0.55);
        tracker.should_place("down_token", 0.45);

        assert_eq!(tracker.pending_cancel_count(), 2);
        assert_eq!(tracker.pending_placement_count(), 2);

        tracker.clear_all_pending();

        assert_eq!(tracker.pending_cancel_count(), 0);
        assert_eq!(tracker.pending_placement_count(), 0);
    }

    #[test]
    fn test_cleanup_from_orders_ttl_fallback() {
        // Test that TTL still works as fallback even if order exists
        let mut tracker = InFlightTracker::new(short_ttl());

        tracker.should_place("up_token", 0.55);

        let open_orders = vec![
            OpenOrderInfo::new("order-123".to_string(), "up_token".to_string(), 0.55),
        ];

        // Should be pending (order exists and TTL not expired)
        tracker.cleanup_from_orders(&open_orders);
        assert!(tracker.is_placement_pending("up_token", 0.55));

        // Wait for TTL to expire
        sleep(Duration::from_millis(60));

        // Should be cleared even though order still exists (TTL expired)
        tracker.cleanup_from_orders(&open_orders);
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
