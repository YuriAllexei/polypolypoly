//! Data query methods for RestClient

use super::super::super::auth::PolymarketAuth;
use super::super::helpers::{extract_api_error, parse_json, with_headers};
use super::super::types::*;
use super::{RestClient, Result};

const END_CURSOR: &str = "LTE=";
const START_CURSOR: &str = "MA==";

impl RestClient {
    /// Fetch open orders (single page)
    pub async fn get_orders(
        &self,
        auth: &PolymarketAuth,
        params: Option<&OpenOrderParams>,
        next_cursor: Option<&str>,
    ) -> Result<PaginatedResponse<OpenOrder>> {
        let mut query_parts = Vec::new();

        if let Some(p) = params {
            if let Some(ref id) = p.id {
                query_parts.push(format!("id={}", id));
            }
            if let Some(ref market) = p.market {
                query_parts.push(format!("market={}", market));
            }
            if let Some(ref asset_id) = p.asset_id {
                query_parts.push(format!("asset_id={}", asset_id));
            }
        }

        let cursor = next_cursor.unwrap_or(START_CURSOR);
        query_parts.push(format!("next_cursor={}", cursor));

        let query_string = query_parts.join("&");
        let url = format!("{}/data/orders?{}", self.base_url, query_string);
        let timestamp = PolymarketAuth::current_timestamp();

        let headers = auth.l2_headers(timestamp, "GET", "/data/orders", "")?;
        let req = with_headers(self.client.get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch orders").await);
        }

        parse_json(response).await
    }

    /// Fetch all open orders (auto-pagination)
    pub async fn get_all_orders(
        &self,
        auth: &PolymarketAuth,
        params: Option<&OpenOrderParams>,
    ) -> Result<Vec<OpenOrder>> {
        let mut all_orders = Vec::new();
        let mut cursor = Some(START_CURSOR.to_string());

        while let Some(ref cur) = cursor {
            let response = self.get_orders(auth, params, Some(cur)).await?;
            all_orders.extend(response.data);

            if response.next_cursor == END_CURSOR {
                cursor = None;
            } else {
                cursor = Some(response.next_cursor);
            }
        }

        Ok(all_orders)
    }

    /// Fetch a single order by ID
    pub async fn get_order(&self, auth: &PolymarketAuth, order_id: &str) -> Result<OpenOrder> {
        let path = format!("/data/order/{}", order_id);
        let url = format!("{}{}", self.base_url, path);
        let timestamp = PolymarketAuth::current_timestamp();

        let headers = auth.l2_headers(timestamp, "GET", &path, "")?;
        let req = with_headers(self.client.get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch order").await);
        }

        parse_json(response).await
    }

    /// Fetch trades (single page)
    pub async fn get_trades(
        &self,
        auth: &PolymarketAuth,
        params: Option<&TradeParams>,
        next_cursor: Option<&str>,
    ) -> Result<PaginatedResponse<Trade>> {
        let mut query_parts = Vec::new();

        if let Some(p) = params {
            if let Some(ref id) = p.id {
                query_parts.push(format!("id={}", id));
            }
            if let Some(ref maker_address) = p.maker_address {
                query_parts.push(format!("maker_address={}", maker_address));
            }
            if let Some(ref market) = p.market {
                query_parts.push(format!("market={}", market));
            }
            if let Some(ref asset_id) = p.asset_id {
                query_parts.push(format!("asset_id={}", asset_id));
            }
            if let Some(before) = p.before {
                query_parts.push(format!("before={}", before));
            }
            if let Some(after) = p.after {
                query_parts.push(format!("after={}", after));
            }
        }

        let cursor = next_cursor.unwrap_or(START_CURSOR);
        query_parts.push(format!("next_cursor={}", cursor));

        let query_string = query_parts.join("&");
        let url = format!("{}/data/trades?{}", self.base_url, query_string);
        let timestamp = PolymarketAuth::current_timestamp();

        let headers = auth.l2_headers(timestamp, "GET", "/data/trades", "")?;
        let req = with_headers(self.client.get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch trades").await);
        }

        parse_json(response).await
    }

    /// Fetch all trades (auto-pagination)
    pub async fn get_all_trades(
        &self,
        auth: &PolymarketAuth,
        params: Option<&TradeParams>,
    ) -> Result<Vec<Trade>> {
        let mut all_trades = Vec::new();
        let mut cursor = Some(START_CURSOR.to_string());

        while let Some(ref cur) = cursor {
            let response = self.get_trades(auth, params, Some(cur)).await?;
            all_trades.extend(response.data);

            if response.next_cursor == END_CURSOR {
                cursor = None;
            } else {
                cursor = Some(response.next_cursor);
            }
        }

        Ok(all_trades)
    }

    /// Get balance and allowance
    pub async fn get_balance_allowance(
        &self,
        auth: &PolymarketAuth,
        params: Option<&BalanceAllowanceParams>,
    ) -> Result<BalanceAllowance> {
        let mut query_parts = Vec::new();

        if let Some(p) = params {
            if let Some(asset_type) = p.asset_type {
                let type_str = match asset_type {
                    AssetType::Collateral => "COLLATERAL",
                    AssetType::Conditional => "CONDITIONAL",
                };
                query_parts.push(format!("asset_type={}", type_str));
            }
            if let Some(ref token_id) = p.token_id {
                query_parts.push(format!("token_id={}", token_id));
            }
            if let Some(sig_type) = p.signature_type {
                query_parts.push(format!("signature_type={}", sig_type));
            }
        }

        let url = if query_parts.is_empty() {
            format!("{}/balance-allowance", self.base_url)
        } else {
            format!("{}/balance-allowance?{}", self.base_url, query_parts.join("&"))
        };
        let timestamp = PolymarketAuth::current_timestamp();

        let headers = auth.l2_headers(timestamp, "GET", "/balance-allowance", "")?;
        let req = with_headers(self.client.get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch balance/allowance").await);
        }

        parse_json(response).await
    }
}
