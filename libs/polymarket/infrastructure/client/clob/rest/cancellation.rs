//! Order cancellation methods for RestClient
//! Uses dedicated OS threads to isolate HTTP requests from tokio runtime

use super::super::super::auth::PolymarketAuth;
use super::super::types::CancelResponse;
use super::{RestClient, RestError, Result};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, error, info};

impl RestClient {
    /// Cancel a single order by ID using dedicated thread
    pub async fn cancel_order(
        &self,
        auth: &PolymarketAuth,
        order_id: &str,
    ) -> Result<CancelResponse> {
        let url = format!("{}/order", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("🗑️ Canceling order {}", order_id);

        let body_json = json!({ "orderID": order_id });
        let body = serde_json::to_string(&body_json)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "DELETE", "/order", &body)?;

        self.send_delete_request(url, headers, body).await
    }

    /// Cancel multiple orders by ID using dedicated thread
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

        debug!("🗑️ Canceling {} orders", order_ids.len());

        let body = serde_json::to_string(order_ids)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "DELETE", "/orders", &body)?;

        self.send_delete_request(url, headers, body).await
    }

    /// Cancel all open orders using dedicated thread
    pub async fn cancel_all_orders(&self, auth: &PolymarketAuth) -> Result<CancelResponse> {
        let url = format!("{}/cancel-all", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("🗑️ Canceling all orders");

        let headers = auth.l2_headers(timestamp, "DELETE", "/cancel-all", "")?;

        self.send_delete_request(url, headers, String::new()).await
    }

    /// Cancel orders for a specific market or asset using dedicated thread
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

        self.send_delete_request(url, headers, body).await
    }

    /// Send DELETE request using dedicated OS thread (isolated from tokio runtime)
    async fn send_delete_request(
        &self,
        url: String,
        headers: HashMap<String, String>,
        body: String,
    ) -> Result<CancelResponse> {
        let start = Instant::now();

        let (tx, rx) = tokio::sync::oneshot::channel();

        std::thread::spawn(move || {
            debug!("⏳ [Cancel thread] Starting DELETE request to {}", url);
            let thread_start = Instant::now();

            let result = (|| -> std::result::Result<CancelResponse, String> {
                let mut request = ureq::request("DELETE", &url)
                    .set("Content-Type", "application/json")
                    .set("User-Agent", "rs_clob_client")
                    .set("Accept", "*/*");

                for (key, value) in &headers {
                    request = request.set(key, value);
                }

                let response = if body.is_empty() {
                    request
                        .timeout(Duration::from_secs(15))
                        .call()
                        .map_err(|e| format!("DELETE request failed: {}", e))?
                } else {
                    request
                        .timeout(Duration::from_secs(15))
                        .send_string(&body)
                        .map_err(|e| format!("DELETE request failed: {}", e))?
                };

                let status = response.status();
                let response_body = response.into_string()
                    .map_err(|e| format!("Failed to read response: {}", e))?;

                debug!("📥 [Cancel thread] Got response: status={}, body_len={}", status, response_body.len());

                if status == 200 || status == 201 {
                    serde_json::from_str(&response_body)
                        .map_err(|e| format!("Failed to parse response: {} - body: {}", e, response_body))
                } else {
                    Err(format!("Cancel failed with status {}: {}", status, response_body))
                }
            })();

            debug!("📥 [Cancel thread] DELETE completed in {:?}", thread_start.elapsed());

            let _ = tx.send(result);
        });

        let result = rx.await
            .map_err(|_| RestError::ApiError("Cancel thread channel closed".to_string()))?;

        let elapsed = start.elapsed();

        match result {
            Ok(response) => {
                debug!("✅ Cancel request successful in {:?}", elapsed);
                Ok(response)
            }
            Err(e) => {
                error!("❌ Cancel request failed after {:?}: {}", elapsed, e);
                Err(RestError::ApiError(e))
            }
        }
    }
}
