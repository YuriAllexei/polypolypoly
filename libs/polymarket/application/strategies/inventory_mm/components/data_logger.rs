//! Market data logger for backtesting.
//!
//! Logs tick data to CSV format compatible with model_tuning Python package.
//! CSV columns: timestamp,oracle_price,threshold,best_ask_up,best_bid_up,best_ask_down,best_bid_down,minutes_to_resolution

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use tracing::info;

use crate::application::strategies::inventory_mm::types::SolverInput;

/// Market tick data for logging (matches Python MarketTick).
#[derive(Debug, Clone)]
pub struct MarketTick {
    pub timestamp: DateTime<Utc>,
    pub oracle_price: f64,
    pub threshold: f64,
    pub best_ask_up: f64,
    pub best_bid_up: f64,
    pub best_ask_down: f64,
    pub best_bid_down: f64,
    pub minutes_to_resolution: f64,
}

impl MarketTick {
    /// Create from SolverInput and market info.
    pub fn from_input(
        input: &SolverInput,
        oracle_price: f64,
        threshold: f64,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            oracle_price,
            threshold,
            best_ask_up: input.up_orderbook.best_ask_price().unwrap_or(0.0),
            best_bid_up: input.up_orderbook.best_bid_price().unwrap_or(0.0),
            best_ask_down: input.down_orderbook.best_ask_price().unwrap_or(0.0),
            best_bid_down: input.down_orderbook.best_bid_price().unwrap_or(0.0),
            minutes_to_resolution: input.minutes_to_resolution,
        }
    }

    /// Format as CSV row.
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{:.2},{:.2},{:.4},{:.4},{:.4},{:.4},{:.2}",
            self.timestamp.to_rfc3339(),
            self.oracle_price,
            self.threshold,
            self.best_ask_up,
            self.best_bid_up,
            self.best_ask_down,
            self.best_bid_down,
            self.minutes_to_resolution,
        )
    }
}

/// CSV header for market tick data.
pub const CSV_HEADER: &str = "timestamp,oracle_price,threshold,best_ask_up,best_bid_up,best_ask_down,best_bid_down,minutes_to_resolution";

/// Market data logger that writes to CSV.
pub struct MarketDataLogger {
    writer: BufWriter<File>,
    file_path: PathBuf,
    tick_count: usize,
    flush_interval: usize,
}

impl MarketDataLogger {
    /// Create new logger, writing to specified directory.
    /// Filename: {symbol}_{timeframe}_{market_id_prefix}_{timestamp}.csv
    pub fn new(
        output_dir: &str,
        symbol: &str,
        timeframe: &str,
        market_id: &str,
    ) -> std::io::Result<Self> {
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(output_dir)?;

        // Generate filename
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let market_prefix = &market_id[..8.min(market_id.len())];
        let filename = format!("{}_{}_{}_{}.csv", symbol, timeframe, market_prefix, timestamp);
        let file_path = PathBuf::from(output_dir).join(&filename);

        // Open file and write header
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&file_path)?;

        let mut writer = BufWriter::new(file);
        writeln!(writer, "{}", CSV_HEADER)?;

        info!("MarketDataLogger: Writing to {}", file_path.display());

        Ok(Self {
            writer,
            file_path,
            tick_count: 0,
            flush_interval: 10, // Flush every 10 ticks
        })
    }

    /// Log a market tick.
    pub fn log_tick(&mut self, tick: &MarketTick) -> std::io::Result<()> {
        writeln!(self.writer, "{}", tick.to_csv_row())?;
        self.tick_count += 1;

        // Periodic flush to ensure data is written
        if self.tick_count % self.flush_interval == 0 {
            self.writer.flush()?;
        }

        Ok(())
    }

    /// Flush and close the logger.
    pub fn close(mut self) -> std::io::Result<()> {
        self.writer.flush()?;
        info!(
            "MarketDataLogger: Closed {} with {} ticks",
            self.file_path.display(),
            self.tick_count
        );
        Ok(())
    }

    /// Get the output file path.
    pub fn file_path(&self) -> &PathBuf {
        &self.file_path
    }

    /// Get tick count.
    pub fn tick_count(&self) -> usize {
        self.tick_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_csv_row_format() {
        let tick = MarketTick {
            timestamp: DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            oracle_price: 97500.0,
            threshold: 97000.0,
            best_ask_up: 0.52,
            best_bid_up: 0.48,
            best_ask_down: 0.53,
            best_bid_down: 0.47,
            minutes_to_resolution: 8.5,
        };

        let row = tick.to_csv_row();
        assert!(row.contains("97500.00"));
        assert!(row.contains("97000.00"));
        assert!(row.contains("0.5200"));
        assert!(row.contains("8.50"));
    }

    #[test]
    fn test_logger_creates_file() {
        let dir = tempdir().unwrap();
        let logger = MarketDataLogger::new(
            dir.path().to_str().unwrap(),
            "BTC",
            "15m",
            "0x1234567890abcdef",
        ).unwrap();

        assert!(logger.file_path().exists());
        assert_eq!(logger.tick_count(), 0);
    }

    #[test]
    fn test_logger_writes_ticks() {
        let dir = tempdir().unwrap();
        let mut logger = MarketDataLogger::new(
            dir.path().to_str().unwrap(),
            "ETH",
            "1hr",
            "0xabcdef1234567890",
        ).unwrap();

        let tick = MarketTick {
            timestamp: Utc::now(),
            oracle_price: 3500.0,
            threshold: 3450.0,
            best_ask_up: 0.55,
            best_bid_up: 0.50,
            best_ask_down: 0.50,
            best_bid_down: 0.45,
            minutes_to_resolution: 30.0,
        };

        logger.log_tick(&tick).unwrap();
        logger.log_tick(&tick).unwrap();
        assert_eq!(logger.tick_count(), 2);

        let path = logger.file_path().clone();
        logger.close().unwrap();

        // Read file and verify
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3); // Header + 2 ticks
        assert!(lines[0].starts_with("timestamp,"));
    }
}
