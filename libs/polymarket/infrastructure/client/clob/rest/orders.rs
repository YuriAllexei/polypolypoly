//! Order placement methods for RestClient

use super::super::super::auth::PolymarketAuth;
use super::super::helpers::{extract_api_error, parse_json, with_headers};
use super::super::order_builder::{build_batch_order_payload, build_order_payload, OrderBuilder, SignedOrder};
use super::super::types::*;
use super::{RestClient, Result, RestError};
use serde_json::json;
use std::time::Instant;
use tracing::{debug, info, error};

impl RestClient {
    /// Place a limit order
    pub async fn place_order(
        &self,
        auth: &PolymarketAuth,
        order_args: &OrderArgs,
        order_type: OrderType,
    ) -> Result<OrderResponse> {
        let url = format!("{}/order", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Placing {:?} order for token {}", order_type, order_args.token_id);

        let body_json = json!({
            "order": order_args,
            "orderType": order_type,
        });
        let body = serde_json::to_string(&body_json)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        let headers = auth.l2_headers(timestamp, "POST", "/order", &body)?;
        let req = with_headers(
            self.client().post(&url).header("Content-Type", "application/json"),
            headers,
        );
        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to place order").await);
        }

        parse_json(response).await
    }

    /// Place a market order (buy/sell by amount)
    pub async fn place_market_order(
        &self,
        auth: &PolymarketAuth,
        market_order: &MarketOrderArgs,
        order_type: OrderType,
    ) -> Result<OrderResponse> {
        debug!(
            "Placing market {:?} order for {} USD",
            market_order.side, market_order.amount
        );

        let orderbook = self.get_orderbook(&market_order.token_id).await?;

        let (price, size) = match market_order.side {
            Side::Buy => {
                let best_ask = orderbook
                    .asks
                    .first()
                    .ok_or_else(|| RestError::ApiError("No asks available".to_string()))?;
                let price = best_ask.price_f64();
                let size = market_order.amount / price;
                (price, size)
            }
            Side::Sell => {
                let best_bid = orderbook
                    .bids
                    .first()
                    .ok_or_else(|| RestError::ApiError("No bids available".to_string()))?;
                let price = best_bid.price_f64();
                let size = market_order.amount / price;
                (price, size)
            }
        };

        let order_args = OrderArgs {
            token_id: market_order.token_id.clone(),
            price,
            size,
            side: market_order.side,
            fee_rate_bps: None,
            nonce: None,
            expiration: None,
        };

        self.place_order(auth, &order_args, order_type).await
    }

    /// Place a signed order using EIP-712 signing
    pub async fn place_signed_order(
        &self,
        auth: &PolymarketAuth,
        order_builder: &OrderBuilder,
        token_id: &str,
        price: f64,
        size: f64,
        side: Side,
        order_type: OrderType,
        fee_rate_bps: Option<u64>,
    ) -> Result<OrderPlacementResponse> {
        let timestamp = PolymarketAuth::current_timestamp();
        let nonce = 0u64;

        info!(
            "Building signed order: token={}, price={}, size={}, side={:?}",
            token_id, price, size, side
        );

        let signed_order = order_builder
            .build_signed_order(auth, token_id, price, size, side, nonce, fee_rate_bps, None)
            .map_err(|e| RestError::ApiError(format!("Failed to build order: {}", e)))?;

        self.submit_signed_order(auth, &signed_order, order_type, timestamp)
            .await
    }

    /// Submit a pre-built signed order to the exchange
    /// Uses a dedicated OS thread to completely isolate from tokio runtime
    pub async fn submit_signed_order(
        &self,
        auth: &PolymarketAuth,
        signed_order: &SignedOrder,
        order_type: OrderType,
        timestamp: u64,
    ) -> Result<OrderPlacementResponse> {
        let url = format!("{}/order", self.base_url);

        let api_key = auth
            .api_key()
            .ok_or_else(|| RestError::ApiError("API key not set".to_string()))?;

        let payload = build_order_payload(signed_order, &api_key.key, order_type);
        let body = serde_json::to_string(&payload)
            .map_err(|e| RestError::ApiError(format!("Failed to serialize order: {}", e)))?;

        let headers = auth.l2_headers(timestamp, "POST", "/order", &body)?;

        info!("📤 SENDING ORDER REQUEST");
        info!("   URL: {}", url);
        info!("   Body length: {} bytes", body.len());
        debug!("   Full body: {}", body);

        let start = Instant::now();

        // Use a completely separate OS thread with oneshot channel
        // This isolates the HTTP request from the tokio runtime entirely
        let (tx, rx) = tokio::sync::oneshot::channel();

        let headers_clone = headers.clone();

        std::thread::spawn(move || {
            info!("⏳ [Dedicated thread] Starting HTTP request...");
            let thread_start = Instant::now();

            // Use ureq (blocking HTTP client) in dedicated thread
            let result = (|| -> std::result::Result<OrderPlacementResponse, String> {
                let mut request = ureq::post(&url)
                    .set("Content-Type", "application/json")
                    .set("User-Agent", "rs_clob_client")
                    .set("Accept", "*/*");

                for (key, value) in &headers_clone {
                    request = request.set(key, value);
                }

                info!("⏳ [Dedicated thread] Sending request...");

                let response = request
                    .timeout(std::time::Duration::from_secs(15))
                    .send_string(&body)
                    .map_err(|e| format!("HTTP request failed: {}", e))?;

                let status = response.status();
                let response_body = response.into_string()
                    .map_err(|e| format!("Failed to read response: {}", e))?;

                info!("📥 [Dedicated thread] Got response: status={}, body_len={}", status, response_body.len());

                if status == 200 || status == 201 {
                    serde_json::from_str(&response_body)
                        .map_err(|e| format!("Failed to parse response: {} - body: {}", e, response_body))
                } else {
                    Err(format!("Order failed with status {}: {}", status, response_body))
                }
            })();

            info!("📥 [Dedicated thread] HTTP completed in {:?}", thread_start.elapsed());

            // Send result back to async context
            let _ = tx.send(result);
        });

        // Wait for the dedicated thread to complete
        let result = rx.await
            .map_err(|_| RestError::ApiError("Thread channel closed".to_string()))?;

        let elapsed = start.elapsed();

        match result {
            Ok(response) => {
                info!("✅ Order request successful in {:?}", elapsed);
                Ok(response)
            }
            Err(e) => {
                error!("❌ Order request failed after {:?}: {}", elapsed, e);
                Err(RestError::ApiError(e))
            }
        }
    }

    /// Submit multiple pre-built signed orders to the exchange
    pub async fn submit_batch_orders(
        &self,
        auth: &PolymarketAuth,
        signed_orders: &[(SignedOrder, OrderType)],
        timestamp: u64,
    ) -> Result<Vec<OrderPlacementResponse>> {
        let url = format!("{}/orders", self.base_url);

        let api_key = auth
            .api_key()
            .ok_or_else(|| RestError::ApiError("API key not set".to_string()))?;

        let payload = build_batch_order_payload(signed_orders, &api_key.key);
        let body = serde_json::to_string(&payload)
            .map_err(|e| RestError::ApiError(format!("Failed to serialize orders: {}", e)))?;

        let headers = auth.l2_headers(timestamp, "POST", "/orders", &body)?;

        info!("📤 SENDING BATCH ORDER REQUEST ({} orders)", signed_orders.len());
        info!("   URL: {}", url);
        info!("   Body length: {} bytes", body.len());
        debug!("   Full body: {}", body);

        let start = Instant::now();

        let (tx, rx) = tokio::sync::oneshot::channel();
        let headers_clone = headers.clone();

        std::thread::spawn(move || {
            info!("⏳ [Dedicated thread] Starting batch HTTP request...");
            let thread_start = Instant::now();

            let result = (|| -> std::result::Result<Vec<OrderPlacementResponse>, String> {
                let mut request = ureq::post(&url)
                    .set("Content-Type", "application/json")
                    .set("User-Agent", "rs_clob_client")
                    .set("Accept", "*/*");

                for (key, value) in &headers_clone {
                    request = request.set(key, value);
                }

                info!("⏳ [Dedicated thread] Sending batch request...");

                let response = request
                    .timeout(std::time::Duration::from_secs(30))
                    .send_string(&body)
                    .map_err(|e| format!("HTTP request failed: {}", e))?;

                let status = response.status();
                let response_body = response.into_string()
                    .map_err(|e| format!("Failed to read response: {}", e))?;

                info!("📥 [Dedicated thread] Got response: status={}, body_len={}", status, response_body.len());

                if status == 200 || status == 201 {
                    serde_json::from_str(&response_body)
                        .map_err(|e| format!("Failed to parse response: {} - body: {}", e, response_body))
                } else {
                    Err(format!("Batch order failed with status {}: {}", status, response_body))
                }
            })();

            info!("📥 [Dedicated thread] Batch HTTP completed in {:?}", thread_start.elapsed());
            let _ = tx.send(result);
        });

        let result = rx.await
            .map_err(|_| RestError::ApiError("Thread channel closed".to_string()))?;

        let elapsed = start.elapsed();

        match result {
            Ok(responses) => {
                info!("✅ Batch order request successful in {:?} ({} orders)", elapsed, responses.len());
                Ok(responses)
            }
            Err(e) => {
                error!("❌ Batch order request failed after {:?}: {}", elapsed, e);
                Err(RestError::ApiError(e))
            }
        }
    }

    /// Place multiple orders in a batch (max 15)
    pub async fn place_batch_orders(
        &self,
        auth: &PolymarketAuth,
        order_builder: &OrderBuilder,
        orders: Vec<(String, f64, f64, Side, OrderType)>,
        fee_rate_bps: Option<u64>,
    ) -> Result<Vec<OrderPlacementResponse>> {
        if orders.is_empty() {
            return Ok(Vec::new());
        }

        if orders.len() > 15 {
            return Err(RestError::ApiError(
                "Maximum 15 orders per batch".to_string(),
            ));
        }

        let timestamp = PolymarketAuth::current_timestamp();

        let mut nonce = self.get_nonce(auth).await?;

        info!("Building batch of {} orders, starting nonce={}", orders.len(), nonce);

        let mut signed_orders: Vec<(SignedOrder, OrderType)> = Vec::with_capacity(orders.len());

        for (token_id, price, size, side, order_type) in orders {
            let signed_order = order_builder
                .build_signed_order(
                    auth,
                    &token_id,
                    price,
                    size,
                    side,
                    nonce,
                    fee_rate_bps,
                    None,
                )
                .map_err(|e| RestError::ApiError(format!("Failed to build order: {}", e)))?;

            signed_orders.push((signed_order, order_type));
            nonce += 1;
        }

        self.submit_batch_orders(auth, &signed_orders, timestamp).await
    }

    /// Convenience method: Place a market buy order with proper signing
    pub async fn place_signed_market_buy(
        &self,
        auth: &PolymarketAuth,
        order_builder: &OrderBuilder,
        token_id: &str,
        amount_usd: f64,
    ) -> Result<OrderPlacementResponse> {
        let orderbook = self.get_orderbook(token_id).await?;
        let best_ask = orderbook
            .asks
            .first()
            .ok_or_else(|| RestError::ApiError("No asks available".to_string()))?;

        let price = best_ask.price_f64();
        let size = amount_usd / price;

        self.place_signed_order(
            auth,
            order_builder,
            token_id,
            price,
            size,
            Side::Buy,
            OrderType::FOK,
            None,
        )
        .await
    }

    /// Convenience method: Place a market sell order with proper signing
    pub async fn place_signed_market_sell(
        &self,
        auth: &PolymarketAuth,
        order_builder: &OrderBuilder,
        token_id: &str,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        let orderbook = self.get_orderbook(token_id).await?;
        let best_bid = orderbook
            .bids
            .first()
            .ok_or_else(|| RestError::ApiError("No bids available".to_string()))?;

        let price = best_bid.price_f64();

        self.place_signed_order(
            auth,
            order_builder,
            token_id,
            price,
            size,
            Side::Sell,
            OrderType::FOK,
            None,
        )
        .await
    }
}
