//! Order types and error definitions

use ethers::types::{Address, U256};
use ethers::utils::to_checksum;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::super::constants::*;

#[derive(Error, Debug)]
pub enum OrderBuilderError {
    #[error("Invalid price: {0}")]
    InvalidPrice(String),

    #[error("Invalid size: {0}")]
    InvalidSize(String),

    #[error("Invalid token ID: {0}")]
    InvalidTokenId(String),

    #[error("Failed to sign order: {0}")]
    SigningError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

pub type Result<T> = std::result::Result<T, OrderBuilderError>;

/// CTF Exchange Order matching the on-chain EIP-712 struct
///
/// Field order and types must match exactly:
/// ```solidity
/// struct Order {
///     uint256 salt;
///     address maker;
///     address signer;
///     address taker;
///     uint256 tokenId;
///     uint256 makerAmount;
///     uint256 takerAmount;
///     uint256 expiration;
///     uint256 nonce;
///     uint256 feeRateBps;
///     uint8 side;
///     uint8 signatureType;
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// Random salt for order uniqueness
    pub salt: U256,
    /// Funder address (proxy wallet for POLY_PROXY)
    pub maker: Address,
    /// Signing wallet address
    pub signer: Address,
    /// Taker address (operator for public orders)
    pub taker: Address,
    /// ERC1155 conditional token ID
    pub token_id: U256,
    /// Max amount maker will spend
    pub maker_amount: U256,
    /// Min amount taker must provide
    pub taker_amount: U256,
    /// Order expiration timestamp (0 = no expiration)
    pub expiration: U256,
    /// Nonce for on-chain cancellations
    pub nonce: U256,
    /// Fee rate in basis points
    pub fee_rate_bps: U256,
    /// Side: 0 = BUY, 1 = SELL
    pub side: u8,
    /// Signature type: 0 = EOA, 1 = POLY_PROXY
    pub signature_type: u8,
}

/// Signed order ready for API submission
#[derive(Debug, Clone)]
pub struct SignedOrder {
    pub order: Order,
    pub signature: String,
}

impl SignedOrder {
    /// Convert to JSON-serializable format for API
    ///
    /// Field formats match Polymarket API expectations (from py_order_utils):
    /// - salt: JSON number (integer, NOT string) - uses arbitrary_precision
    /// - maker/signer/taker: string (checksummed address)
    /// - tokenId: string
    /// - makerAmount/takerAmount: string
    /// - expiration/nonce/feeRateBps: string
    /// - side: string ("BUY" or "SELL")
    /// - signatureType: JSON number (integer)
    /// - signature: string (0x-prefixed hex)
    ///
    /// IMPORTANT: Field order must match Python client exactly:
    /// salt, maker, signer, taker, tokenId, makerAmount, takerAmount,
    /// expiration, nonce, feeRateBps, side, signatureType, signature
    pub fn to_api_json(&self) -> serde_json::Value {
        // Use Map with explicit insertion order (requires preserve_order feature)
        let mut map = serde_json::Map::new();

        // Salt must be a JSON number, not a string
        let salt_number = serde_json::Number::from_string_unchecked(self.order.salt.to_string());
        map.insert("salt".to_string(), serde_json::Value::Number(salt_number));
        map.insert("maker".to_string(), serde_json::Value::String(to_checksum(&self.order.maker, None)));
        map.insert("signer".to_string(), serde_json::Value::String(to_checksum(&self.order.signer, None)));
        map.insert("taker".to_string(), serde_json::Value::String(to_checksum(&self.order.taker, None)));
        map.insert("tokenId".to_string(), serde_json::Value::String(self.order.token_id.to_string()));
        map.insert("makerAmount".to_string(), serde_json::Value::String(self.order.maker_amount.to_string()));
        map.insert("takerAmount".to_string(), serde_json::Value::String(self.order.taker_amount.to_string()));
        map.insert("expiration".to_string(), serde_json::Value::String(self.order.expiration.to_string()));
        map.insert("nonce".to_string(), serde_json::Value::String(self.order.nonce.to_string()));
        map.insert("feeRateBps".to_string(), serde_json::Value::String(self.order.fee_rate_bps.to_string()));
        map.insert("side".to_string(), serde_json::Value::String(
            if self.order.side == SIDE_BUY { "BUY" } else { "SELL" }.to_string()
        ));
        map.insert("signatureType".to_string(), serde_json::Value::Number(self.order.signature_type.into()));
        map.insert("signature".to_string(), serde_json::Value::String(self.signature.clone()));

        serde_json::Value::Object(map)
    }
}
