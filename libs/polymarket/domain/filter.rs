//! Filter domain entities and errors
//!
//! Contains business entities and errors for market filtering

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ==================== ERRORS ====================

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Failed to read cache file: {0}")]
    ReadError(String),

    #[error("Failed to parse cache JSON: {0}")]
    ParseError(String),
}

impl CacheError {
    pub fn from_io_error(err: std::io::Error) -> Self {
        CacheError::ReadError(err.to_string())
    }

    pub fn from_json_error(err: serde_json::Error) -> Self {
        CacheError::ParseError(err.to_string())
    }
}

#[derive(Error, Debug)]
pub enum OllamaError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Failed to parse response: {0}")]
    ParseError(String),
}

impl From<reqwest::Error> for OllamaError {
    fn from(err: reqwest::Error) -> Self {
        OllamaError::RequestFailed(err.to_string())
    }
}

impl OllamaError {
    pub fn from_reqwest_error(err: reqwest::Error) -> Self {
        err.into()
    }
}

#[derive(Error, Debug)]
pub enum FilterError {
    #[error("Cache error: {0}")]
    CacheError(#[from] CacheError),

    #[error("Ollama error: {0}")]
    OllamaError(#[from] OllamaError),
}

// ==================== ENTITIES ====================

/// Market information for filtering
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub id: String,
    pub question: String,
    pub resolution_time: DateTime<Utc>,
}

/// Entry in the market cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Market condition ID
    pub market_id: String,

    /// Market question/title
    pub question: String,

    /// Whether LLM identified this as compatible
    pub compatible: bool,

    /// When this was last checked by LLM
    pub checked_at: DateTime<Utc>,

    /// When the market resolves
    pub resolution_time: DateTime<Utc>,
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total: usize,
    pub compatible: usize,
    pub incompatible: usize,
}
