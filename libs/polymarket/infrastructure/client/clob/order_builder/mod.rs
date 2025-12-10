//! Order Builder for Polymarket CTF Exchange
//!
//! Implements EIP-712 order signing for the Polymarket CLOB API.
//! Based on the Python implementation from:
//! - https://github.com/Polymarket/python-order-utils
//! - https://github.com/Polymarket/py-clob-client
//!
//! Split into focused modules:
//! - `types`: Order and SignedOrder structs, error types
//! - `encoding`: ABI encoding helpers
//! - `signing`: EIP-712 hash computation
//! - `payload`: API payload builders

mod encoding;
mod payload;
mod signing;
mod types;

pub use payload::{build_batch_order_payload, build_order_payload};
pub use types::{Order, OrderBuilderError, Result, SignedOrder};

use super::constants::*;
use super::types::Side;
use crate::infrastructure::client::auth::PolymarketAuth;
use ethers::types::{Address, H256, U256};
use rand::Rng;
use signing::compute_eip712_hash;
#[cfg(test)]
use signing::{compute_domain_separator, compute_struct_hash};

/// Builder for creating signed orders
pub struct OrderBuilder {
    /// Signer wallet address
    signer: Address,
    /// Maker/Funder address (proxy wallet for POLY_PROXY)
    maker: Address,
    /// Chain ID (137 for Polygon)
    chain_id: u64,
    /// Signature type (default: POLY_PROXY)
    signature_type: u8,
    /// Whether the market uses neg_risk exchange (affects EIP-712 domain)
    neg_risk: bool,
}

impl OrderBuilder {
    /// Create a new order builder
    ///
    /// For POLY_PROXY signature type:
    /// - `signer` is your signing wallet address
    /// - `maker` is your proxy wallet address (funder)
    ///
    /// For EOA signature type:
    /// - `signer` and `maker` should be the same address
    ///
    /// # Arguments
    /// * `neg_risk` - Whether the market uses neg_risk exchange (check via API)
    pub fn new(signer: Address, maker: Address, chain_id: u64, neg_risk: bool) -> Self {
        Self {
            signer,
            maker,
            chain_id,
            signature_type: SIGNATURE_TYPE_POLY_PROXY,
            neg_risk,
        }
    }

    /// Create a builder with EOA signature type
    pub fn new_eoa(address: Address, chain_id: u64, neg_risk: bool) -> Self {
        Self {
            signer: address,
            maker: address,
            chain_id,
            signature_type: SIGNATURE_TYPE_EOA,
            neg_risk,
        }
    }

    /// Set signature type
    pub fn with_signature_type(mut self, signature_type: u8) -> Self {
        self.signature_type = signature_type;
        self
    }

    /// Set neg_risk flag
    pub fn with_neg_risk(mut self, neg_risk: bool) -> Self {
        self.neg_risk = neg_risk;
        self
    }

    /// Build and sign an order
    ///
    /// # Arguments
    /// * `auth` - Authentication manager for signing
    /// * `token_id` - ERC1155 token ID (as string)
    /// * `price` - Price per token (0.0 to 1.0)
    /// * `size` - Number of tokens
    /// * `side` - BUY or SELL
    /// * `nonce` - Current nonce from exchange
    /// * `fee_rate_bps` - Fee rate in basis points (default: 0)
    /// * `expiration` - Expiration timestamp (0 = no expiration)
    pub fn build_signed_order(
        &self,
        auth: &PolymarketAuth,
        token_id: &str,
        price: f64,
        size: f64,
        side: Side,
        nonce: u64,
        fee_rate_bps: Option<u64>,
        expiration: Option<u64>,
    ) -> Result<SignedOrder> {
        // Validate inputs
        if price <= 0.0 || price >= 1.0 {
            return Err(OrderBuilderError::InvalidPrice(format!(
                "Price must be between 0 and 1, got: {}",
                price
            )));
        }
        if size <= 0.0 {
            return Err(OrderBuilderError::InvalidSize(format!(
                "Size must be positive, got: {}",
                size
            )));
        }

        // Parse token ID
        let token_id_u256 = U256::from_dec_str(token_id).map_err(|e| {
            OrderBuilderError::InvalidTokenId(format!("Failed to parse token ID: {}", e))
        })?;

        // Calculate amounts
        let (maker_amount, taker_amount) = self.calculate_amounts(price, size, side);

        // Generate random salt
        let salt = self.generate_salt();

        // Build order
        let order = Order {
            salt,
            maker: self.maker,
            signer: self.signer,
            taker: zero_address(), // ZERO_ADDRESS for public orders
            token_id: token_id_u256,
            maker_amount,
            taker_amount,
            expiration: U256::from(expiration.unwrap_or(0)),
            nonce: U256::from(nonce),
            fee_rate_bps: U256::from(fee_rate_bps.unwrap_or(0)),
            side: match side {
                Side::Buy => SIDE_BUY,
                Side::Sell => SIDE_SELL,
            },
            signature_type: self.signature_type,
        };

        // Sign the order
        let signature = self.sign_order(auth, &order)?;

        Ok(SignedOrder { order, signature })
    }

    /// Calculate maker and taker amounts based on side
    ///
    /// All amounts are in token decimals (6 decimal places).
    ///
    /// For BUY orders:
    /// - makerAmount = price * size (USDC to spend)
    /// - takerAmount = size (tokens to receive)
    ///
    /// For SELL orders:
    /// - makerAmount = size (tokens to sell)
    /// - takerAmount = price * size (USDC to receive)
    fn calculate_amounts(&self, price: f64, size: f64, side: Side) -> (U256, U256) {
        let size_scaled = (size * DECIMAL_MULTIPLIER as f64).round() as u128;
        let usdc_amount = (price * size * DECIMAL_MULTIPLIER as f64).round() as u128;

        match side {
            Side::Buy => (U256::from(usdc_amount), U256::from(size_scaled)),
            Side::Sell => (U256::from(size_scaled), U256::from(usdc_amount)),
        }
    }

    /// Generate a random salt matching Python's format
    ///
    /// Python uses: round(timestamp * random()) which produces a 32-bit-ish value
    /// We generate a similar small random number to stay compatible.
    fn generate_salt(&self) -> U256 {
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs_f64();

        let mut rng = rand::thread_rng();
        let random: f64 = rng.gen();

        // Match Python's generate_seed(): round(now * random())
        let salt = (now * random).round() as u64;
        U256::from(salt)
    }

    /// Sign an order using EIP-712
    fn sign_order(&self, auth: &PolymarketAuth, order: &Order) -> Result<String> {
        let message_hash = compute_eip712_hash(order, self.chain_id, self.neg_risk);

        auth.sign_hash_hex(H256::from(message_hash))
            .map_err(|e| OrderBuilderError::SigningError(e.to_string()))
    }

    // Expose internal methods for testing
    #[cfg(test)]
    pub fn compute_domain_separator(&self) -> [u8; 32] {
        compute_domain_separator(self.chain_id, self.neg_risk)
    }

    #[cfg(test)]
    pub fn compute_struct_hash(&self, order: &Order) -> [u8; 32] {
        compute_struct_hash(order)
    }

    #[cfg(test)]
    pub fn compute_eip712_hash(&self, order: &Order) -> [u8; 32] {
        compute_eip712_hash(order, self.chain_id, self.neg_risk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_calculation_buy() {
        let builder = OrderBuilder::new(
            Address::zero(),
            Address::zero(),
            POLYGON_CHAIN_ID,
            false,
        );

        // Buy 100 tokens at $0.50 each = $50 USDC
        let (maker_amount, taker_amount) = builder.calculate_amounts(0.5, 100.0, Side::Buy);

        // maker pays USDC: 0.5 * 100 * 1_000_000 = 50_000_000
        assert_eq!(maker_amount, U256::from(50_000_000u64));
        // taker provides tokens: 100 * 1_000_000 = 100_000_000
        assert_eq!(taker_amount, U256::from(100_000_000u64));
    }

    #[test]
    fn test_amount_calculation_sell() {
        let builder = OrderBuilder::new(
            Address::zero(),
            Address::zero(),
            POLYGON_CHAIN_ID,
            false,
        );

        // Sell 100 tokens at $0.50 each = $50 USDC
        let (maker_amount, taker_amount) = builder.calculate_amounts(0.5, 100.0, Side::Sell);

        // maker provides tokens: 100 * 1_000_000 = 100_000_000
        assert_eq!(maker_amount, U256::from(100_000_000u64));
        // taker pays USDC: 0.5 * 100 * 1_000_000 = 50_000_000
        assert_eq!(taker_amount, U256::from(50_000_000u64));
    }

    #[test]
    fn test_salt_generation_uniqueness() {
        let builder = OrderBuilder::new(
            Address::zero(),
            Address::zero(),
            POLYGON_CHAIN_ID,
            false,
        );

        let salt1 = builder.generate_salt();
        let salt2 = builder.generate_salt();

        // Salts should be different (extremely high probability)
        assert_ne!(salt1, salt2);
    }

    #[test]
    fn test_signature_matches_python() {
        // Expected signature from Python for the test order:
        // 0x069db5e77ee9b663b7c2d9bb388b156b314d42d39d3f968edcba9ebbd662b8856a116138dc95883183889d48d615b1f4ead5a35d18b439ab0a2b45b794744d151b

        // Private key that derives to 0x497284Cd581433f3C8224F07556a8d903113E0D3
        let private_key = "0x257091039adf0d3df1f3171508f7db838782ee9b4f6ad61054be773e7541d90a";

        let auth = PolymarketAuth::new(private_key, POLYGON_CHAIN_ID).unwrap();

        // Verify the address matches
        let maker = auth.address();
        assert_eq!(
            format!("{:?}", maker).to_lowercase(),
            "0x497284cd581433f3c8224f07556a8d903113e0d3",
            "Address mismatch"
        );

        let builder = OrderBuilder::new_eoa(maker, POLYGON_CHAIN_ID, false);

        let order = Order {
            salt: U256::from(12345u64),
            maker,
            signer: maker,
            taker: zero_address(),
            token_id: U256::from_dec_str("87681536460342357667165150330318852851476971055929009934844581402585803923513").unwrap(),
            maker_amount: U256::from(16400000u64),
            taker_amount: U256::from(40000000u64),
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::zero(),
            side: SIDE_BUY,
            signature_type: SIGNATURE_TYPE_EOA,
        };

        // Compute the hash
        let eip712_hash = builder.compute_eip712_hash(&order);
        let expected_hash = hex::decode("36ea8c22435f8c4a2804e77be5074f23f98101af0a339564693cd0b186ebda46").unwrap();
        assert_eq!(eip712_hash.to_vec(), expected_hash, "Hash mismatch");

        // Sign it
        let signature = auth.sign_hash_hex(H256::from(eip712_hash)).unwrap();

        // Expected signature from Python
        let expected_sig = "0x069db5e77ee9b663b7c2d9bb388b156b314d42d39d3f968edcba9ebbd662b8856a116138dc95883183889d48d615b1f4ead5a35d18b439ab0a2b45b794744d151b";

        assert_eq!(
            signature.to_lowercase(),
            expected_sig.to_lowercase(),
            "Signature mismatch.\nGot: {}\nExpected: {}",
            signature,
            expected_sig
        );
    }
}
