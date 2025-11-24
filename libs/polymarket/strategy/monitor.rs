use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;
use tracing::{debug, info};

/// Information about a market being monitored
#[derive(Debug, Clone)]
pub struct MonitoredMarket {
    pub market_id: String,
    pub question: String,
    pub resolution_time: DateTime<Utc>,
    pub token_ids: Vec<String>,  // Outcome token IDs
}

/// Resolution monitor - tracks markets approaching resolution
pub struct ResolutionMonitor {
    /// Markets sorted by resolution time
    markets: BTreeMap<DateTime<Utc>, Vec<MonitoredMarket>>,

    /// Quick lookup by market ID
    market_index: std::collections::HashMap<String, DateTime<Utc>>,
}

impl ResolutionMonitor {
    /// Create new resolution monitor
    pub fn new() -> Self {
        Self {
            markets: BTreeMap::new(),
            market_index: std::collections::HashMap::new(),
        }
    }

    /// Add market to monitoring queue
    pub fn add_market(&mut self, market: MonitoredMarket) {
        debug!("Adding market to monitor: {} (resolves at {})", market.question, market.resolution_time);

        let resolution_time = market.resolution_time;
        let market_id = market.market_id.clone();

        // Add to time-sorted map
        self.markets
            .entry(resolution_time)
            .or_insert_with(Vec::new)
            .push(market);

        // Add to index for quick lookup
        self.market_index.insert(market_id, resolution_time);
    }

    /// Remove market from monitoring
    pub fn remove_market(&mut self, market_id: &str) {
        if let Some(resolution_time) = self.market_index.remove(market_id) {
            if let Some(markets) = self.markets.get_mut(&resolution_time) {
                markets.retain(|m| m.market_id != market_id);

                // Remove empty time slot
                if markets.is_empty() {
                    self.markets.remove(&resolution_time);
                }
            }

            debug!("Removed market from monitor: {}", market_id);
        }
    }

    /// Get markets resolving within the next `within_secs` seconds
    pub fn get_upcoming_markets(&self, within_secs: u64) -> Vec<MonitoredMarket> {
        let now = Utc::now();
        let cutoff = now + Duration::seconds(within_secs as i64);

        let mut upcoming = Vec::new();

        for (resolution_time, markets) in &self.markets {
            // Stop once we reach markets too far in the future
            if resolution_time > &cutoff {
                break;
            }

            // Only include markets that haven't resolved yet
            if resolution_time > &now {
                upcoming.extend(markets.clone());
            }
        }

        upcoming
    }

    /// Check if it's time to trade a specific market
    ///
    /// Returns true if current time is within `seconds_before` of resolution
    pub fn should_trade(&self, market_id: &str, seconds_before: u64) -> bool {
        if let Some(resolution_time) = self.market_index.get(market_id) {
            let now = Utc::now();
            let trade_time = *resolution_time - Duration::seconds(seconds_before as i64);

            // Time to trade if we're past the trade_time but before resolution
            now >= trade_time && now < *resolution_time
        } else {
            false
        }
    }

    /// Get market by ID
    pub fn get_market(&self, market_id: &str) -> Option<MonitoredMarket> {
        let resolution_time = self.market_index.get(market_id)?;
        let markets = self.markets.get(resolution_time)?;
        markets.iter().find(|m| m.market_id == market_id).cloned()
    }

    /// Get number of markets being monitored
    pub fn market_count(&self) -> usize {
        self.market_index.len()
    }

    /// Remove markets that have already resolved
    pub fn cleanup_resolved(&mut self) {
        let now = Utc::now();

        // Find all resolution times in the past
        let past_times: Vec<_> = self
            .markets
            .keys()
            .filter(|&&time| time < now)
            .cloned()
            .collect();

        // Remove them
        for time in past_times {
            if let Some(markets) = self.markets.remove(&time) {
                for market in markets {
                    self.market_index.remove(&market.market_id);
                }
            }
        }
    }

    /// Get next market to resolve
    pub fn next_market(&self) -> Option<(&DateTime<Utc>, &MonitoredMarket)> {
        let (time, markets) = self.markets.iter().next()?;
        let market = markets.first()?;
        Some((time, market))
    }

    /// Get all monitored markets
    pub fn all_markets(&self) -> Vec<MonitoredMarket> {
        self.markets
            .values()
            .flat_map(|markets| markets.clone())
            .collect()
    }
}

impl Default for ResolutionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_market(id: &str, hours_from_now: i64) -> MonitoredMarket {
        MonitoredMarket {
            market_id: id.to_string(),
            question: format!("Test market {}", id),
            resolution_time: Utc::now() + Duration::hours(hours_from_now),
            token_ids: vec!["0x123".to_string()],
        }
    }

    #[test]
    fn test_add_and_get_market() {
        let mut monitor = ResolutionMonitor::new();

        let market = create_test_market("0x1", 1);
        monitor.add_market(market.clone());

        assert_eq!(monitor.market_count(), 1);
        assert!(monitor.get_market("0x1").is_some());
    }

    #[test]
    fn test_upcoming_markets() {
        let mut monitor = ResolutionMonitor::new();

        // Add markets at different times
        monitor.add_market(create_test_market("0x1", 1));  // 1 hour from now
        monitor.add_market(create_test_market("0x2", 2));  // 2 hours from now
        monitor.add_market(create_test_market("0x3", 3));  // 3 hours from now

        // Get markets resolving within 2.5 hours
        let upcoming = monitor.get_upcoming_markets(2 * 3600 + 1800);  // 2.5 hours in seconds
        assert_eq!(upcoming.len(), 2);  // Should get first two markets
    }

    #[test]
    fn test_remove_market() {
        let mut monitor = ResolutionMonitor::new();

        monitor.add_market(create_test_market("0x1", 1));
        monitor.add_market(create_test_market("0x2", 2));

        assert_eq!(monitor.market_count(), 2);

        monitor.remove_market("0x1");
        assert_eq!(monitor.market_count(), 1);
        assert!(monitor.get_market("0x1").is_none());
        assert!(monitor.get_market("0x2").is_some());
    }

    #[test]
    fn test_should_trade() {
        let mut monitor = ResolutionMonitor::new();

        // Market resolves in 30 seconds
        let mut market = create_test_market("0x1", 0);
        market.resolution_time = Utc::now() + Duration::seconds(30);
        monitor.add_market(market);

        // Should trade if we're 10 seconds before resolution
        // Since market is 30s away, shouldn't trade yet
        assert!(!monitor.should_trade("0x1", 10));

        // But should trade if we check 40 seconds before
        assert!(monitor.should_trade("0x1", 40));
    }
}
