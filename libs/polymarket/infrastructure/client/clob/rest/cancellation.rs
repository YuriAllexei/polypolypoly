//! Order cancellation methods for RestClient

use super::super::super::auth::PolymarketAuth;
use super::super::helpers::{extract_api_error, parse_json, with_headers};
use super::super::types::CancelResponse;
use super::{RestClient, RestError, Result};
use serde_json::json;
use std::collections::HashMap;
use tracing::debug;

impl RestClient {
    /// Cancel a single order by ID
    pub async fn cancel_order(
        &self,
        auth: &PolymarketAuth,
        order_id: &str,
    ) -> Result<CancelResponse> {
        let url = format!("{}/order", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Canceling order {}", order_id);

        let body_json = json!({ "orderID": order_id });
        let body = serde_json::to_string(&body_json)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "DELETE", "/order", &body)?;
        let req = with_headers(
            self.client()
                .delete(&url)
                .header("Content-Type", "application/json"),
            headers,
        );
        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to cancel order").await);
        }

        parse_json(response).await
    }

    /// Cancel multiple orders by ID
    pub async fn cancel_orders(
        &self,
        auth: &PolymarketAuth,
        order_ids: &[String],
    ) -> Result<CancelResponse> {
        if order_ids.is_empty() {
            return Ok(CancelResponse {
                canceled: Vec::new(),
                not_canceled: HashMap::new(),
            });
        }

        let url = format!("{}/orders", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Canceling {} orders", order_ids.len());

        let body = serde_json::to_string(order_ids)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "DELETE", "/orders", &body)?;
        let req = with_headers(
            self.client()
                .delete(&url)
                .header("Content-Type", "application/json"),
            headers,
        );
        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to cancel orders").await);
        }

        parse_json(response).await
    }

    /// Cancel all open orders
    pub async fn cancel_all_orders(&self, auth: &PolymarketAuth) -> Result<CancelResponse> {
        let url = format!("{}/cancel-all", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Canceling all orders");

        let headers = auth.l2_headers(timestamp, "DELETE", "/cancel-all", "")?;
        let req = with_headers(self.client().delete(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to cancel all orders").await);
        }

        parse_json(response).await
    }

    /// Cancel orders for a specific market or asset
    pub async fn cancel_market_orders(
        &self,
        auth: &PolymarketAuth,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<CancelResponse> {
        let url = format!("{}/cancel-market-orders", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Canceling market orders: market={:?}, asset_id={:?}", market, asset_id);

        let body_json = json!({
            "market": market.unwrap_or(""),
            "asset_id": asset_id.unwrap_or("")
        });
        let body = serde_json::to_string(&body_json)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "DELETE", "/cancel-market-orders", &body)?;
        let req = with_headers(
            self.client()
                .delete(&url)
                .header("Content-Type", "application/json"),
            headers,
        );
        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to cancel market orders").await);
        }

        parse_json(response).await
    }
}
