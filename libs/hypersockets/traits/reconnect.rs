use std::time::Duration;

/// Trait for defining reconnection strategies
///
/// Implement this trait to control how the client should
/// behave when reconnecting after a disconnection.
pub trait ReconnectionStrategy: Send + Sync {
    /// Get the delay before the next reconnection attempt
    ///
    /// # Arguments
    /// * `attempt` - The reconnection attempt number (0-indexed)
    ///
    /// # Returns
    /// * `Some(duration)` - Wait this long before reconnecting
    /// * `None` - Stop reconnecting
    fn next_delay(&self, attempt: usize) -> Option<Duration>;

    /// Reset the strategy state (called after successful connection)
    fn reset(&mut self);

    /// Check if we should continue reconnecting
    ///
    /// # Arguments
    /// * `attempt` - The current reconnection attempt number
    ///
    /// # Returns
    /// * `true` - Continue reconnecting
    /// * `false` - Stop reconnecting
    fn should_reconnect(&self, attempt: usize) -> bool;
}

/// Exponential backoff reconnection strategy
///
/// Delays between reconnection attempts grow exponentially:
/// initial_delay * 2^attempt, capped at max_delay
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    initial_delay: Duration,
    max_delay: Duration,
    max_attempts: Option<usize>,
}

impl ExponentialBackoff {
    /// Create a new exponential backoff strategy
    ///
    /// # Arguments
    /// * `initial_delay` - The initial delay before first reconnect
    /// * `max_delay` - The maximum delay between reconnects
    /// * `max_attempts` - Maximum number of attempts (None = unlimited)
    pub fn new(
        initial_delay: Duration,
        max_delay: Duration,
        max_attempts: Option<usize>,
    ) -> Self {
        Self {
            initial_delay,
            max_delay,
            max_attempts,
        }
    }
}

impl ReconnectionStrategy for ExponentialBackoff {
    fn next_delay(&self, attempt: usize) -> Option<Duration> {
        if !self.should_reconnect(attempt) {
            return None;
        }

        let delay = self.initial_delay.as_millis() as u64 * 2u64.pow(attempt as u32);
        let delay = Duration::from_millis(delay.min(self.max_delay.as_millis() as u64));
        Some(delay)
    }

    fn reset(&mut self) {
        // No state to reset for exponential backoff
    }

    fn should_reconnect(&self, attempt: usize) -> bool {
        self.max_attempts.map_or(true, |max| attempt < max)
    }
}

/// Fixed delay reconnection strategy
///
/// Always waits the same amount of time between reconnection attempts
#[derive(Debug, Clone)]
pub struct FixedDelay {
    delay: Duration,
    max_attempts: Option<usize>,
}

impl FixedDelay {
    /// Create a new fixed delay strategy
    ///
    /// # Arguments
    /// * `delay` - The fixed delay between reconnects
    /// * `max_attempts` - Maximum number of attempts (None = unlimited)
    pub fn new(delay: Duration, max_attempts: Option<usize>) -> Self {
        Self { delay, max_attempts }
    }
}

impl ReconnectionStrategy for FixedDelay {
    fn next_delay(&self, attempt: usize) -> Option<Duration> {
        if !self.should_reconnect(attempt) {
            return None;
        }
        Some(self.delay)
    }

    fn reset(&mut self) {
        // No state to reset for fixed delay
    }

    fn should_reconnect(&self, attempt: usize) -> bool {
        self.max_attempts.map_or(true, |max| attempt < max)
    }
}

/// Never reconnect strategy
///
/// The client will not attempt to reconnect after disconnection
#[derive(Debug, Clone)]
pub struct NeverReconnect;

impl ReconnectionStrategy for NeverReconnect {
    fn next_delay(&self, _attempt: usize) -> Option<Duration> {
        None
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn should_reconnect(&self, _attempt: usize) -> bool {
        false
    }
}
