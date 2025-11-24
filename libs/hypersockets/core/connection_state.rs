use std::sync::atomic::{AtomicU8, Ordering};

/// Connection state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionState {
    /// Not connected
    Disconnected = 0,
    /// Currently connecting
    Connecting = 1,
    /// Connected and operational
    Connected = 2,
    /// Attempting to reconnect
    Reconnecting = 3,
    /// Shutting down
    ShuttingDown = 4,
}

impl From<u8> for ConnectionState {
    fn from(value: u8) -> Self {
        match value {
            0 => ConnectionState::Disconnected,
            1 => ConnectionState::Connecting,
            2 => ConnectionState::Connected,
            3 => ConnectionState::Reconnecting,
            4 => ConnectionState::ShuttingDown,
            _ => ConnectionState::Disconnected,
        }
    }
}

impl From<ConnectionState> for u8 {
    fn from(state: ConnectionState) -> Self {
        state as u8
    }
}

/// Lock-free connection state tracker
///
/// Uses atomic operations to track connection state without locks,
/// enabling high-performance state checks from multiple threads.
pub struct AtomicConnectionState {
    state: AtomicU8,
}

impl AtomicConnectionState {
    /// Create a new connection state tracker
    pub fn new(initial_state: ConnectionState) -> Self {
        Self {
            state: AtomicU8::new(initial_state.into()),
        }
    }

    /// Get the current connection state
    #[inline]
    pub fn get(&self) -> ConnectionState {
        self.state.load(Ordering::Acquire).into()
    }

    /// Set the connection state
    #[inline]
    pub fn set(&self, new_state: ConnectionState) {
        self.state.store(new_state.into(), Ordering::Release);
    }

    /// Compare and swap the connection state
    ///
    /// # Returns
    /// * `Ok(())` - State was successfully changed
    /// * `Err(current)` - State was not changed, returns current state
    #[inline]
    pub fn compare_exchange(
        &self,
        expected: ConnectionState,
        new: ConnectionState,
    ) -> std::result::Result<(), ConnectionState> {
        self.state
            .compare_exchange(
                expected.into(),
                new.into(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|actual| actual.into())
    }

    /// Check if currently connected
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.get() == ConnectionState::Connected
    }

    /// Check if disconnected
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        matches!(self.get(), ConnectionState::Disconnected)
    }

    /// Check if in any connecting state
    #[inline]
    pub fn is_connecting(&self) -> bool {
        matches!(
            self.get(),
            ConnectionState::Connecting | ConnectionState::Reconnecting
        )
    }

    /// Check if shutting down
    #[inline]
    pub fn is_shutting_down(&self) -> bool {
        self.get() == ConnectionState::ShuttingDown
    }
}

impl Default for AtomicConnectionState {
    fn default() -> Self {
        Self::new(ConnectionState::Disconnected)
    }
}

/// Lock-free metrics tracker
///
/// Tracks message counts and other metrics using atomic operations
pub struct AtomicMetrics {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    reconnect_count: AtomicU64,
}

use std::sync::atomic::AtomicU64;

impl AtomicMetrics {
    /// Create a new metrics tracker
    pub fn new() -> Self {
        Self {
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            reconnect_count: AtomicU64::new(0),
        }
    }

    /// Increment sent message counter
    #[inline]
    pub fn increment_sent(&self) {
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment received message counter
    #[inline]
    pub fn increment_received(&self) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment reconnection counter
    #[inline]
    pub fn increment_reconnects(&self) {
        self.reconnect_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get number of messages sent
    #[inline]
    pub fn messages_sent(&self) -> u64 {
        self.messages_sent.load(Ordering::Relaxed)
    }

    /// Get number of messages received
    #[inline]
    pub fn messages_received(&self) -> u64 {
        self.messages_received.load(Ordering::Relaxed)
    }

    /// Get number of reconnections
    #[inline]
    pub fn reconnect_count(&self) -> u64 {
        self.reconnect_count.load(Ordering::Relaxed)
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.messages_sent.store(0, Ordering::Relaxed);
        self.messages_received.store(0, Ordering::Relaxed);
        self.reconnect_count.store(0, Ordering::Relaxed);
    }
}

impl Default for AtomicMetrics {
    fn default() -> Self {
        Self::new()
    }
}
