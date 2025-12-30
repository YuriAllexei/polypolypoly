//! High-level trading client for Polymarket
//!
//! Provides a simplified API for order placement by encapsulating
//! authentication, credential management, and order building.
//!
//! # Example
//!
//! ```rust,ignore
//! use polymarket::infrastructure::client::clob::TradingClient;
//!
//! // Initialize once (loads credentials from env)
//! let client = TradingClient::from_env().await?;
//!
//! // Place orders with minimal parameters
//! let result = client.buy("token_id", 0.50, 10.0).await?;
//! let result = client.sell("token_id", 0.60, 5.0).await?;
//!
//! // Or with more control
//! let result = client
//!     .order("token_id")
//!     .price(0.50)
//!     .size(10.0)
//!     .buy()
//!     .fok()
//!     .execute()
//!     .await?;
//! ```

use super::super::auth::PolymarketAuth;
use super::order_builder::OrderBuilder;
use super::rest::{RestClient, RestError};
use super::types::{
    ApiCredentials, AssetType, BalanceAllowance, BalanceAllowanceParams, CancelResponse, OpenOrder,
    OpenOrderParams, OrderPlacementResponse, OrderType, Side, Trade, TradeParams,
};
use super::POLYGON_CHAIN_ID;
use dashmap::DashMap;
use ethers::types::Address;
use std::env;
use thiserror::Error;
use tracing::{debug, info, warn};

const DEFAULT_CLOB_URL: &str = "https://clob.polymarket.com";

#[derive(Error, Debug)]
pub enum TradingError {
    #[error("Environment variable '{0}' not set")]
    EnvVarMissing(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("REST API error: {0}")]
    RestError(#[from] RestError),

    #[error("Auth error: {0}")]
    AuthError(#[from] super::super::auth::AuthError),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

pub type Result<T> = std::result::Result<T, TradingError>;

/// Result of a batch order placement with partitioned success/failure responses.
#[derive(Debug, Clone)]
pub struct BatchOrderResult {
    pub succeeded: Vec<(String, OrderPlacementResponse)>,
    pub failed: Vec<(String, OrderPlacementResponse)>,
}

impl BatchOrderResult {
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn any_failed(&self) -> bool {
        !self.failed.is_empty()
    }

    pub fn all_failed(&self) -> bool {
        self.succeeded.is_empty()
    }

    pub fn success_count(&self) -> usize {
        self.succeeded.len()
    }

    pub fn failure_count(&self) -> usize {
        self.failed.len()
    }

    pub fn order_ids(&self) -> Vec<String> {
        self.succeeded
            .iter()
            .filter_map(|(_, r)| r.order_id.clone())
            .collect()
    }

    pub fn error_messages(&self) -> Vec<(String, String)> {
        self.failed
            .iter()
            .map(|(token_id, r)| {
                (
                    token_id.clone(),
                    r.error_msg.clone().unwrap_or_else(|| "Unknown error".to_string()),
                )
            })
            .collect()
    }

    /// Get the statuses of successful orders
    pub fn statuses(&self) -> Vec<(&str, Option<&str>)> {
        self.succeeded
            .iter()
            .map(|(token_id, r)| (token_id.as_str(), r.status.as_deref()))
            .collect()
    }
}

/// High-level trading client for Polymarket
///
/// Encapsulates all the complexity of authentication, credential management,
/// and order building into a simple API.
pub struct TradingClient {
    auth: PolymarketAuth,
    rest: RestClient,
    signer_addr: Address,
    proxy_addr: Option<Address>,
    neg_risk_cache: DashMap<String, bool>,
}

impl TradingClient {
    /// Create a new trading client from environment variables
    ///
    /// Required env vars:
    /// - `PRIVATE_KEY`: Ethereum private key (0x prefixed)
    ///
    /// Optional env vars:
    /// - `PROXY_WALLET`: Polymarket proxy wallet (if different from signer)
    /// - `API_KEY`, `API_SECRET`, `API_PASSPHRASE`: Pre-existing API credentials
    /// - `CLOB_URL`: Custom CLOB endpoint (defaults to mainnet)
    pub async fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();

        let private_key = env::var("PRIVATE_KEY")
            .map_err(|_| TradingError::EnvVarMissing("PRIVATE_KEY".to_string()))?;

        let proxy_wallet = env::var("PROXY_WALLET").ok();
        let clob_url = env::var("CLOB_URL").unwrap_or_else(|_| DEFAULT_CLOB_URL.to_string());

        let api_key = env::var("API_KEY").ok();
        let api_secret = env::var("API_SECRET").ok();
        let api_passphrase = env::var("API_PASSPHRASE").ok();

        let existing_creds = match (api_key, api_secret, api_passphrase) {
            (Some(key), Some(secret), Some(passphrase)) => Some(ApiCredentials {
                key,
                secret,
                passphrase,
            }),
            _ => None,
        };

        Self::new(&private_key, proxy_wallet.as_deref(), &clob_url, existing_creds).await
    }

    /// Create a new trading client with explicit parameters
    ///
    /// # Arguments
    /// - `private_key`: Ethereum private key (with or without 0x prefix)
    /// - `proxy_wallet`: Optional proxy wallet address (if None, uses signer as maker)
    /// - `clob_url`: CLOB API endpoint
    /// - `existing_creds`: Optional pre-existing API credentials
    pub async fn new(
        private_key: &str,
        proxy_wallet: Option<&str>,
        clob_url: &str,
        existing_creds: Option<ApiCredentials>,
    ) -> Result<Self> {
        let mut auth = PolymarketAuth::new(private_key, POLYGON_CHAIN_ID)?;
        let signer_addr = auth.address();

        let proxy_addr = match proxy_wallet {
            Some(addr) => Some(
                addr.parse::<Address>()
                    .map_err(|_| TradingError::InvalidAddress(addr.to_string()))?,
            ),
            None => None,
        };

        let rest = RestClient::new(clob_url);

        // Set up API credentials
        if let Some(creds) = existing_creds {
            info!("Using provided API credentials");
            auth.set_api_key(creds);
        } else {
            info!("Deriving API credentials from private key...");
            let creds = rest.get_or_create_api_creds(&auth).await?;
            auth.set_api_key(creds);
            info!("API credentials obtained successfully");
        }

        // Verify HTTP connectivity to CLOB API
        info!("Verifying HTTP connectivity to CLOB API...");
        match rest.health_check().await {
            Ok(_) => info!("✅ CLOB API connectivity verified"),
            Err(e) => {
                warn!("⚠️  CLOB API connectivity check failed: {}", e);
                warn!("⚠️  Order placement may fail due to network issues");
            }
        }

        debug!(
            "TradingClient initialized: signer={:?}, proxy={:?}",
            signer_addr, proxy_addr
        );

        Ok(Self {
            auth,
            rest,
            signer_addr,
            proxy_addr,
            neg_risk_cache: DashMap::new(),
        })
    }

    /// Get the signer address
    pub fn signer_address(&self) -> Address {
        self.signer_addr
    }

    /// Get the maker address (proxy if set, otherwise signer)
    pub fn maker_address(&self) -> Address {
        self.proxy_addr.unwrap_or(self.signer_addr)
    }

    /// Get neg_risk status for a token.
    ///
    /// Most markets (including Up or Down) are NOT neg_risk.
    /// The HTTP endpoint to fetch this is unreliable from Docker containers,
    /// so we default to false which is correct for the vast majority of markets.
    ///
    /// For neg_risk markets, you would need to explicitly specify neg_risk=true
    /// when placing orders (future enhancement: get from Gamma API data).
    fn get_neg_risk(&self, token_id: &str) -> bool {
        if let Some(neg_risk) = self.neg_risk_cache.get(token_id) {
            return *neg_risk;
        }

        // Default to false - most markets are not neg_risk
        // Up or Down markets specifically are NOT neg_risk
        let neg_risk = false;

        self.neg_risk_cache.insert(token_id.to_string(), neg_risk);
        neg_risk
    }

    /// Create an OrderBuilder for the given token
    ///
    /// Always uses Gnosis Safe signature type (signature_type=2) which is
    /// required for browser wallet users with proxy wallets.
    fn order_builder(&self, token_id: &str) -> OrderBuilder {
        let neg_risk = self.get_neg_risk(token_id);
        let maker_addr = self.maker_address();

        // Always use Gnosis Safe signature type
        OrderBuilder::new_gnosis_safe(
            self.signer_addr,
            maker_addr,
            POLYGON_CHAIN_ID,
            neg_risk,
        )
    }

    /// Place a buy order (GTC by default)
    ///
    /// # Arguments
    /// - `token_id`: The token to buy
    /// - `price`: Price per token (0.01 to 0.99)
    /// - `size`: Number of tokens to buy
    pub async fn buy(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        self.place_order(token_id, price, size, Side::Buy, OrderType::GTC)
            .await
    }

    /// Place a sell order (GTC by default)
    ///
    /// # Arguments
    /// - `token_id`: The token to sell
    /// - `price`: Price per token (0.01 to 0.99)
    /// - `size`: Number of tokens to sell
    pub async fn sell(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        self.place_order(token_id, price, size, Side::Sell, OrderType::GTC)
            .await
    }

    /// Place a buy order with FOK (Fill Or Kill)
    pub async fn buy_fok(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        self.place_order(token_id, price, size, Side::Buy, OrderType::FOK)
            .await
    }

    /// Place a sell order with FOK (Fill Or Kill)
    pub async fn sell_fok(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        self.place_order(token_id, price, size, Side::Sell, OrderType::FOK)
            .await
    }

    /// Place an order with full control over parameters
    pub async fn place_order(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
        side: Side,
        order_type: OrderType,
    ) -> Result<OrderPlacementResponse> {
        self.place_order_with_fee(token_id, price, size, side, order_type, None)
            .await
    }

    /// Place an order with custom fee rate
    pub async fn place_order_with_fee(
        &self,
        token_id: &str,
        price: f64,
        size: f64,
        side: Side,
        order_type: OrderType,
        fee_rate_bps: Option<u64>,
    ) -> Result<OrderPlacementResponse> {
        // Validate inputs
        if price <= 0.0 || price >= 1.0 {
            return Err(TradingError::InvalidParameter(format!(
                "Price must be between 0 and 1 (exclusive), got: {}",
                price
            )));
        }
        if size <= 0.0 {
            return Err(TradingError::InvalidParameter(format!(
                "Size must be positive, got: {}",
                size
            )));
        }

        let order_builder = self.order_builder(token_id);

        let result = self
            .rest
            .place_signed_order(
                &self.auth,
                &order_builder,
                token_id,
                price,
                size,
                side,
                order_type,
                fee_rate_bps,
            )
            .await?;

        Ok(result)
    }

    /// Place multiple orders in a single batch (max 15)
    pub async fn place_batch_orders(
        &self,
        orders: Vec<(String, f64, f64, Side, OrderType)>,
        fee_rate_bps: Option<u64>,
    ) -> Result<BatchOrderResult> {
        if orders.is_empty() {
            return Ok(BatchOrderResult {
                succeeded: Vec::new(),
                failed: Vec::new(),
            });
        }

        if orders.len() > 15 {
            return Err(TradingError::InvalidParameter(
                "Maximum 15 orders per batch".to_string(),
            ));
        }

        for (token_id, price, size, _, _) in &orders {
            if *price <= 0.0 || *price >= 1.0 {
                return Err(TradingError::InvalidParameter(format!(
                    "Price must be between 0 and 1 (exclusive), got: {} for token {}",
                    price, token_id
                )));
            }
            if *size <= 0.0 {
                return Err(TradingError::InvalidParameter(format!(
                    "Size must be positive, got: {} for token {}",
                    size, token_id
                )));
            }
        }

        let token_ids: Vec<String> = orders.iter().map(|(id, _, _, _, _)| id.clone()).collect();
        let order_builder = self.order_builder(&orders[0].0);

        let responses = self
            .rest
            .place_batch_orders(&self.auth, &order_builder, orders, fee_rate_bps)
            .await
            .map_err(TradingError::from)?;

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for (token_id, response) in token_ids.into_iter().zip(responses.into_iter()) {
            if response.success {
                succeeded.push((token_id, response));
            } else {
                failed.push((token_id, response));
            }
        }

        Ok(BatchOrderResult { succeeded, failed })
    }

    /// Place a market buy order at best ask
    pub async fn market_buy(
        &self,
        token_id: &str,
        amount_usd: f64,
    ) -> Result<OrderPlacementResponse> {
        let order_builder = self.order_builder(token_id);
        let result = self
            .rest
            .place_signed_market_buy(&self.auth, &order_builder, token_id, amount_usd)
            .await?;
        Ok(result)
    }

    /// Place a market sell order at best bid
    pub async fn market_sell(
        &self,
        token_id: &str,
        size: f64,
    ) -> Result<OrderPlacementResponse> {
        let order_builder = self.order_builder(token_id);
        let result = self
            .rest
            .place_signed_market_sell(&self.auth, &order_builder, token_id, size)
            .await?;
        Ok(result)
    }

    /// Get access to the underlying REST client for advanced operations
    pub fn rest(&self) -> &RestClient {
        &self.rest
    }

    /// Get access to the auth manager
    pub fn auth(&self) -> &PolymarketAuth {
        &self.auth
    }

    /// Start building an order with the fluent API
    pub fn order(&self, token_id: &str) -> OrderRequest<'_> {
        OrderRequest::new(self, token_id.to_string())
    }

    /// Cancel a single order by ID
    pub async fn cancel_order(&self, order_id: &str) -> Result<CancelResponse> {
        self.rest
            .cancel_order(&self.auth, order_id)
            .await
            .map_err(TradingError::from)
    }

    /// Cancel multiple orders by ID
    pub async fn cancel_orders(&self, order_ids: &[String]) -> Result<CancelResponse> {
        self.rest
            .cancel_orders(&self.auth, order_ids)
            .await
            .map_err(TradingError::from)
    }

    /// Cancel all open orders
    pub async fn cancel_all(&self) -> Result<CancelResponse> {
        self.rest
            .cancel_all_orders(&self.auth)
            .await
            .map_err(TradingError::from)
    }

    /// Cancel orders for a specific market or asset
    pub async fn cancel_market_orders(
        &self,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<CancelResponse> {
        self.rest
            .cancel_market_orders(&self.auth, market, asset_id)
            .await
            .map_err(TradingError::from)
    }

    /// Get all open orders
    pub async fn get_orders(&self, params: Option<&OpenOrderParams>) -> Result<Vec<OpenOrder>> {
        self.rest
            .get_all_orders(&self.auth, params)
            .await
            .map_err(TradingError::from)
    }

    /// Get a single order by ID
    pub async fn get_order(&self, order_id: &str) -> Result<OpenOrder> {
        self.rest
            .get_order(&self.auth, order_id)
            .await
            .map_err(TradingError::from)
    }

    /// Get all trades
    pub async fn get_trades(&self, params: Option<&TradeParams>) -> Result<Vec<Trade>> {
        self.rest
            .get_all_trades(&self.auth, params)
            .await
            .map_err(TradingError::from)
    }

    /// Get balance and allowance
    pub async fn get_balance_allowance(
        &self,
        params: Option<&BalanceAllowanceParams>,
    ) -> Result<BalanceAllowance> {
        self.rest
            .get_balance_allowance(&self.auth, params)
            .await
            .map_err(TradingError::from)
    }

    /// Get USDC balance as a human-readable float
    ///
    /// Fetches the collateral balance and converts from raw units (6 decimals) to USD.
    pub async fn get_usd_balance(&self) -> Result<f64> {
        let params = BalanceAllowanceParams {
            asset_type: Some(AssetType::Collateral),
            token_id: None,
            signature_type: Some(2), // POLY_GNOSIS_SAFE for proxy wallets
        };
        let balance = self.get_balance_allowance(Some(&params)).await?;

        // Parse balance string and divide by 1_000_000 (USDC has 6 decimals)
        let raw_balance: f64 = balance.balance.parse().unwrap_or(0.0);
        Ok(raw_balance / 1_000_000.0)
    }
}

/// Fluent order builder for more complex order configurations
pub struct OrderRequest<'a> {
    client: &'a TradingClient,
    token_id: String,
    price: Option<f64>,
    size: Option<f64>,
    side: Option<Side>,
    order_type: OrderType,
    fee_rate_bps: Option<u64>,
}

impl<'a> OrderRequest<'a> {
    fn new(client: &'a TradingClient, token_id: String) -> Self {
        Self {
            client,
            token_id,
            price: None,
            size: None,
            side: None,
            order_type: OrderType::GTC,
            fee_rate_bps: None,
        }
    }

    /// Set the price
    pub fn price(mut self, price: f64) -> Self {
        self.price = Some(price);
        self
    }

    /// Set the size
    pub fn size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    /// Set as buy order
    pub fn buy(mut self) -> Self {
        self.side = Some(Side::Buy);
        self
    }

    /// Set as sell order
    pub fn sell(mut self) -> Self {
        self.side = Some(Side::Sell);
        self
    }

    /// Set order type to GTC (Good Till Cancel)
    pub fn gtc(mut self) -> Self {
        self.order_type = OrderType::GTC;
        self
    }

    /// Set order type to FOK (Fill Or Kill)
    pub fn fok(mut self) -> Self {
        self.order_type = OrderType::FOK;
        self
    }

    /// Set order type to GTD (Good Till Date)
    pub fn gtd(mut self) -> Self {
        self.order_type = OrderType::GTD;
        self
    }

    /// Set order type to FAK (Fill And Kill)
    pub fn fak(mut self) -> Self {
        self.order_type = OrderType::FAK;
        self
    }

    /// Set custom fee rate in basis points
    pub fn fee_bps(mut self, bps: u64) -> Self {
        self.fee_rate_bps = Some(bps);
        self
    }

    /// Execute the order
    pub async fn execute(self) -> Result<OrderPlacementResponse> {
        let price = self
            .price
            .ok_or_else(|| TradingError::InvalidParameter("Price not set".to_string()))?;
        let size = self
            .size
            .ok_or_else(|| TradingError::InvalidParameter("Size not set".to_string()))?;
        let side = self
            .side
            .ok_or_else(|| TradingError::InvalidParameter("Side not set (use .buy() or .sell())".to_string()))?;

        self.client
            .place_order_with_fee(&self.token_id, price, size, side, self.order_type, self.fee_rate_bps)
            .await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_order_request_builder() {
        // Just test the builder pattern compiles correctly
        // Actual execution requires network
    }
}
