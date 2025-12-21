//! Market metadata types for the Up or Down strategy.
//!
//! Contains enums for oracle sources, crypto assets, and timeframes,
//! plus strategy-wide constants.

use chrono::Duration;
use crate::infrastructure::OracleType;

/// Required tags for Up or Down markets
pub const REQUIRED_TAGS: &[&str] = &["Up or Down", "Crypto Prices", "Recurring", "Crypto"];

/// Staleness threshold for orderbook data in seconds
pub const STALENESS_THRESHOLD_SECS: f64 = 60.0;

/// Maximum WebSocket reconnection attempts before giving up
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// Seconds before market end when we bypass all risk checks and threshold waits
pub const FINAL_SECONDS_BYPASS: f64 = 5.0;

/// Safety BPS threshold for guardian check - cancels if oracle is within this
/// distance of price_to_beat. Never bypassed, runs until market ends.
pub const GUARDIAN_SAFETY_BPS: f64 = 2.0;

// =============================================================================
// Oracle Source
// =============================================================================

/// Oracle source detected from market description
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OracleSource {
    Binance,
    ChainLink,
    Unknown,
}

impl OracleSource {
    /// Detect oracle source from market description text
    pub fn from_description(description: &Option<String>) -> Self {
        match description {
            Some(desc) => {
                if desc.contains("www.binance.com") {
                    OracleSource::Binance
                } else if desc.contains("data.chain.link") {
                    OracleSource::ChainLink
                } else {
                    OracleSource::Unknown
                }
            }
            None => OracleSource::Unknown,
        }
    }

    /// Convert to infrastructure OracleType for price lookups
    pub fn to_oracle_type(&self) -> Option<OracleType> {
        match self {
            OracleSource::Binance => Some(OracleType::Binance),
            OracleSource::ChainLink => Some(OracleType::ChainLink),
            OracleSource::Unknown => None,
        }
    }
}

impl std::fmt::Display for OracleSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleSource::Binance => write!(f, "Binance"),
            OracleSource::ChainLink => write!(f, "ChainLink"),
            OracleSource::Unknown => write!(f, "Unknown"),
        }
    }
}

// =============================================================================
// Crypto Asset
// =============================================================================

/// Cryptocurrency tracked by the market
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CryptoAsset {
    Bitcoin,
    Ethereum,
    Solana,
    Xrp,
    Unknown,
}

impl CryptoAsset {
    /// Detect crypto asset from market tags
    pub fn from_tags(tags: &serde_json::Value) -> Self {
        if let serde_json::Value::Array(arr) = tags {
            for tag in arr {
                if let Some(label) = tag.get("label").and_then(|l| l.as_str()) {
                    match label {
                        "Bitcoin" => return CryptoAsset::Bitcoin,
                        "Ethereum" => return CryptoAsset::Ethereum,
                        "Solana" => return CryptoAsset::Solana,
                        "XRP" => return CryptoAsset::Xrp,
                        _ => {}
                    }
                }
            }
        }
        CryptoAsset::Unknown
    }

    /// Get the symbol used for oracle price lookup (e.g., "BTC", "ETH")
    pub fn oracle_symbol(&self) -> Option<&'static str> {
        match self {
            CryptoAsset::Bitcoin => Some("BTC"),
            CryptoAsset::Ethereum => Some("ETH"),
            CryptoAsset::Solana => Some("SOL"),
            CryptoAsset::Xrp => Some("XRP"),
            CryptoAsset::Unknown => None,
        }
    }
}

impl std::fmt::Display for CryptoAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoAsset::Bitcoin => write!(f, "Bitcoin (BTC)"),
            CryptoAsset::Ethereum => write!(f, "Ethereum (ETH)"),
            CryptoAsset::Solana => write!(f, "Solana (SOL)"),
            CryptoAsset::Xrp => write!(f, "XRP"),
            CryptoAsset::Unknown => write!(f, "Unknown"),
        }
    }
}

// =============================================================================
// Timeframe
// =============================================================================

/// Timeframe of the market
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timeframe {
    FiveMin,    // 5M - NOT officially live, should be skipped
    FifteenMin, // 15M
    OneHour,    // 1H
    FourHour,   // 4H
    Daily,
    Unknown,
}

impl Timeframe {
    /// Detect timeframe from market tags
    pub fn from_tags(tags: &serde_json::Value) -> Self {
        if let serde_json::Value::Array(arr) = tags {
            for tag in arr {
                if let Some(label) = tag.get("label").and_then(|l| l.as_str()) {
                    match label {
                        "5M" => return Timeframe::FiveMin,
                        "15M" => return Timeframe::FifteenMin,
                        "1H" => return Timeframe::OneHour,
                        "4H" => return Timeframe::FourHour,
                        "Daily" => return Timeframe::Daily,
                        _ => {}
                    }
                }
            }
        }
        Timeframe::Unknown
    }

    /// Check if this timeframe is officially supported/live
    /// 5M markets are not officially live yet and should be skipped
    pub fn is_supported(&self) -> bool {
        match self {
            Timeframe::FiveMin => false,  // Not officially live
            Timeframe::Unknown => false,  // Unknown timeframes should be skipped
            _ => true,
        }
    }

    /// Get the duration of this timeframe
    pub fn duration(&self) -> Option<Duration> {
        match self {
            Timeframe::FiveMin => Some(Duration::minutes(5)),
            Timeframe::FifteenMin => Some(Duration::minutes(15)),
            Timeframe::OneHour => Some(Duration::hours(1)),
            Timeframe::FourHour => Some(Duration::hours(4)),
            Timeframe::Daily => Some(Duration::days(1)),
            Timeframe::Unknown => None,
        }
    }

    /// Get the API variant string for Polymarket's crypto price API
    pub fn api_variant(&self) -> Option<&'static str> {
        match self {
            Timeframe::FiveMin => None, // Not supported
            Timeframe::FifteenMin => Some("fifteen"),
            Timeframe::OneHour => Some("hourly"),
            Timeframe::FourHour => Some("fourhour"),
            Timeframe::Daily => Some("daily"),
            Timeframe::Unknown => None,
        }
    }
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Timeframe::FiveMin => write!(f, "5M"),
            Timeframe::FifteenMin => write!(f, "15M"),
            Timeframe::OneHour => write!(f, "1H"),
            Timeframe::FourHour => write!(f, "4H"),
            Timeframe::Daily => write!(f, "Daily"),
            Timeframe::Unknown => write!(f, "Unknown"),
        }
    }
}
