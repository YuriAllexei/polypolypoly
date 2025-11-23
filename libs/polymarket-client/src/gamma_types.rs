use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Market from Gamma API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaMarket {
    pub id: String,

    #[serde(rename = "conditionId")]
    pub condition_id: String,

    pub question: String,

    pub slug: Option<String>,

    #[serde(rename = "startDate")]
    pub start_date: String,

    #[serde(rename = "endDate")]
    pub end_date: String,

    pub active: bool,
    pub closed: bool,
    pub archived: bool,

    #[serde(rename = "marketType")]
    pub market_type: Option<String>,

    pub category: Option<String>,

    pub liquidity: Option<String>,
    pub volume: Option<String>,

    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,

    pub outcomes: Option<Vec<String>>,

    #[serde(rename = "clobTokenIds")]
    pub clob_token_ids: Option<Vec<String>>,

    #[serde(default)]
    pub tags: Vec<GammaTag>,
}

impl GammaMarket {
    /// Parse end date as DateTime
    pub fn end_datetime(&self) -> Result<DateTime<Utc>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(&self.end_date).map(|dt| dt.with_timezone(&Utc))
    }

    /// Parse start date as DateTime
    pub fn start_datetime(&self) -> Result<DateTime<Utc>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(&self.start_date).map(|dt| dt.with_timezone(&Utc))
    }
}

/// Event from Gamma API (contains markets)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaEvent {
    pub id: String,

    pub ticker: Option<String>,

    pub slug: Option<String>,

    pub title: String,

    pub description: Option<String>,

    #[serde(rename = "resolutionSource")]
    pub resolution_source: Option<String>,

    #[serde(rename = "startDate")]
    pub start_date: Option<String>,

    #[serde(rename = "creationDate")]
    pub creation_date: Option<String>,

    #[serde(rename = "endDate")]
    pub end_date: Option<String>,

    pub image: Option<String>,

    pub icon: Option<String>,

    pub active: bool,
    pub closed: bool,
    pub archived: bool,

    pub new: Option<bool>,
    pub featured: Option<bool>,
    pub restricted: Option<bool>,

    pub liquidity: Option<f64>,
    pub volume: Option<f64>,

    #[serde(rename = "openInterest")]
    pub open_interest: Option<f64>,

    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,

    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,

    pub competitive: Option<f64>,

    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,

    #[serde(rename = "volume1wk")]
    pub volume_1wk: Option<f64>,

    #[serde(rename = "volume1mo")]
    pub volume_1mo: Option<f64>,

    #[serde(rename = "volume1yr")]
    pub volume_1yr: Option<f64>,

    #[serde(rename = "enableOrderBook")]
    pub enable_order_book: Option<bool>,

    #[serde(rename = "liquidityClob")]
    pub liquidity_clob: Option<f64>,

    #[serde(rename = "commentCount")]
    pub comment_count: Option<i64>,

    pub category: Option<String>,

    #[serde(default)]
    pub markets: Vec<GammaMarket>,

    #[serde(default)]
    pub tags: Vec<GammaTag>,
}

/// Tag/Category from Gamma API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaTag {
    pub id: String,
    pub label: String,
    pub slug: Option<String>,
}

/// Filters for querying Gamma API
#[derive(Debug, Clone, Default)]
pub struct GammaFilters {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub start_date_min: Option<DateTime<Utc>>,
    pub end_date_min: Option<DateTime<Utc>>,
    pub end_date_max: Option<DateTime<Utc>>,
    pub liquidity_min: Option<f64>,
    pub volume_min: Option<f64>,
    pub tag_id: Option<String>,
}

impl GammaFilters {
    /// Build query parameters for HTTP request
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(active) = self.active {
            params.push(("active".to_string(), active.to_string()));
        }

        if let Some(closed) = self.closed {
            params.push(("closed".to_string(), closed.to_string()));
        }

        if let Some(archived) = self.archived {
            params.push(("archived".to_string(), archived.to_string()));
        }

        if let Some(start_date_min) = self.start_date_min {
            params.push((
                "start_date_min".to_string(),
                start_date_min.to_rfc3339(),
            ));
        }

        if let Some(end_date_min) = self.end_date_min {
            params.push(("end_date_min".to_string(), end_date_min.to_rfc3339()));
        }

        if let Some(end_date_max) = self.end_date_max {
            params.push(("end_date_max".to_string(), end_date_max.to_rfc3339()));
        }

        if let Some(liquidity_min) = self.liquidity_min {
            params.push((
                "liquidity_num_min".to_string(),
                liquidity_min.to_string(),
            ));
        }

        if let Some(volume_min) = self.volume_min {
            params.push(("volume_num_min".to_string(), volume_min.to_string()));
        }

        if let Some(ref tag_id) = self.tag_id {
            params.push(("tag_id".to_string(), tag_id.clone()));
        }

        params
    }
}
