// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Events;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Events = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Events = Vec<Event>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restricted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_interest: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub competitive: Option<f64>,
    #[serde(rename = "volume24hr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume24_hr: Option<f64>,
    #[serde(rename = "volume1wk")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_wk: Option<f64>,
    #[serde(rename = "volume1mo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_mo: Option<f64>,
    #[serde(rename = "volume1yr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_yr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_order_book: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_clob: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markets: Option<Vec<Market>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<Tag>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyom: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_all_outcomes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_market_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_neg_risk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatically_active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk_augmented: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_deployment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploying: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<Series>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gmp_chart_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<SortBy>,
    #[serde(rename = "negRiskMarketID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk_market_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploying_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub election_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured_order: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_week: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_creators: Option<Vec<EventCreator>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCreator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcomes: Option<Outcomes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_prices: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_maker_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured: Option<bool>,
    #[serde(rename = "submitted_by")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restricted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_item_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_item_threshold: Option<String>,
    #[serde(rename = "questionID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_order_book: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_price_min_tick_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_min_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_num: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_num: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_reviewed_dates: Option<bool>,
    #[serde(rename = "volume24hr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume24_hr: Option<f64>,
    #[serde(rename = "volume1wk")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_wk: Option<f64>,
    #[serde(rename = "volume1mo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_mo: Option<f64>,
    #[serde(rename = "volume1yr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_yr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clob_token_ids: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uma_bond: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uma_reward: Option<String>,
    #[serde(rename = "volume24hrClob")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume24_hr_clob: Option<f64>,
    #[serde(rename = "volume1wkClob")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_wk_clob: Option<f64>,
    #[serde(rename = "volume1moClob")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_mo_clob: Option<f64>,
    #[serde(rename = "volume1yrClob")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_yr_clob: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_clob: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_clob: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_liveness: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepting_orders: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk: Option<bool>,
    #[serde(rename = "negRiskRequestID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepting_orders_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyom: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub competitive: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pager_duty_notification_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewards_min_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewards_max_spread: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_week_price_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_month_price_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_trade_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_bid: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_ask: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatically_active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_book_on_start: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_activation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk_other: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uma_resolution_statuses: Option<UmaResolutionStatuses>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_deployment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploying: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploying_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rfq_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holding_rewards_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fees_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clob_rewards: Option<Vec<ClobReward>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_day_price_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_hour_price_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_gmp_series: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_gmp_outcome: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uma_end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uma_resolution_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatically_resolved: Option<bool>,
    #[serde(rename = "volume24hrAmm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume24_hr_amm: Option<f64>,
    #[serde(rename = "volume1wkAmm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_wk_amm: Option<f64>,
    #[serde(rename = "volume1moAmm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_mo_amm: Option<f64>,
    #[serde(rename = "volume1yrAmm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume1_yr_amm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_amm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity_amm: Option<f64>,
    #[serde(rename = "negRiskMarketID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neg_risk_market_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_year_price_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds_delay: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sports_market_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_start_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClobReward {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewards_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewards_daily_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
}


// Outcomes can be any JSON value (array of strings, etc.)
pub type Outcomes = serde_json::Value;

// UmaResolutionStatuses can be any JSON value (array of status strings)
pub type UmaResolutionStatuses = serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Recurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restricted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquidity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub competitive: Option<String>,
    #[serde(rename = "volume24hr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume24_hr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
}

// Recurrence can be any string value (annual, daily, monthly, weekly, hourly, etc.)
pub type Recurrence = String;


// SortBy can be any string value (price, ascending, etc.)
pub type SortBy = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_show: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_hide: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_carousel: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<f64>,
}

/// Filters for querying Gamma API
#[derive(Debug, Clone, Default)]
pub struct GammaFilters {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub start_date_min: Option<chrono::DateTime<chrono::Utc>>,
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
            params.push(("start_date_min".to_string(), start_date_min.to_rfc3339()));
        }

        params
    }
}
