use super::clob::types::ApiCredentials;
use base64::{engine::general_purpose::URL_SAFE, Engine};
use ethers::prelude::*;
use ethers::types::{H256, Signature};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Failed to sign message: {0}")]
    SigningError(String),

    #[error("Invalid private key")]
    InvalidPrivateKey,

    #[error("HMAC error: {0}")]
    HmacError(String),
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// Polymarket authentication manager
pub struct PolymarketAuth {
    wallet: LocalWallet,
    wallet_address: Address,
    chain_id: u64,
    api_key: Option<ApiCredentials>,
}

impl PolymarketAuth {
    /// Create new auth manager from private key
    pub fn new(private_key: &str, chain_id: u64) -> Result<Self> {
        // Remove 0x prefix if present
        let key = private_key.trim_start_matches("0x");

        let wallet = key
            .parse::<LocalWallet>()
            .map_err(|_| AuthError::InvalidPrivateKey)?
            .with_chain_id(chain_id);

        let wallet_address = wallet.address();

        Ok(Self {
            wallet,
            wallet_address,
            chain_id,
            api_key: None,
        })
    }

    /// Get wallet address
    pub fn address(&self) -> Address {
        self.wallet_address
    }

    /// Set API credentials (L2 auth)
    pub fn set_api_key(&mut self, credentials: ApiCredentials) {
        self.api_key = Some(credentials);
    }

    /// Get current API key
    pub fn api_key(&self) -> Option<&ApiCredentials> {
        self.api_key.as_ref()
    }

    /// Generate L1 EIP-712 signature for authentication
    pub async fn sign_l1_message(&self, timestamp: u64, nonce: u64) -> Result<String> {
        // Build message string for signing
        let message = format!(
            "This message attests that I control the given wallet\nAddress: {:?}\nTimestamp: {}\nNonce: {}",
            self.wallet_address, timestamp, nonce
        );

        // Sign the message
        let signature = self.wallet
            .sign_message(message.as_bytes())
            .await
            .map_err(|e| AuthError::SigningError(e.to_string()))?;

        Ok(format!("0x{}", hex::encode(signature.to_vec())))
    }

    /// Generate L2 HMAC signature for API requests
    ///
    /// The signature is computed as:
    /// 1. Base64-decode the API secret
    /// 2. Build message: timestamp + method + path + body
    /// 3. HMAC-SHA256 sign with decoded secret
    /// 4. Base64-encode the signature
    pub fn sign_l2_request(
        &self,
        timestamp: u64,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<String> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AuthError::SigningError("No API key set".to_string()))?;

        // Base64 decode the secret (URL-safe base64)
        let secret_bytes = URL_SAFE
            .decode(&api_key.secret)
            .map_err(|e| AuthError::HmacError(format!("Failed to decode secret: {}", e)))?;

        // Build signature message: timestamp + method + path + body
        let message = format!("{}{}{}{}", timestamp, method, path, body);

        // Compute HMAC-SHA256
        let mut mac = HmacSha256::new_from_slice(&secret_bytes)
            .map_err(|e| AuthError::HmacError(e.to_string()))?;

        mac.update(message.as_bytes());

        let result = mac.finalize();
        let signature_bytes = result.into_bytes();

        // Base64 encode the signature (URL-safe)
        Ok(URL_SAFE.encode(signature_bytes))
    }

    /// Build L1 authentication headers
    pub async fn l1_headers(&self, timestamp: u64, nonce: u64) -> Result<HashMap<String, String>> {
        let signature = self.sign_l1_message(timestamp, nonce).await?;

        let mut headers = HashMap::new();
        // Use checksummed address to match Python client
        headers.insert(
            "POLY_ADDRESS".to_string(),
            ethers::utils::to_checksum(&self.wallet_address, None),
        );
        headers.insert("POLY_SIGNATURE".to_string(), signature);
        headers.insert("POLY_TIMESTAMP".to_string(), timestamp.to_string());
        headers.insert("POLY_NONCE".to_string(), nonce.to_string());

        Ok(headers)
    }

    /// Build L2 authentication headers for API requests
    pub fn l2_headers(
        &self,
        timestamp: u64,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<HashMap<String, String>> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| AuthError::SigningError("No API key set".to_string()))?;

        let signature = self.sign_l2_request(timestamp, method, path, body)?;

        let mut headers = HashMap::new();
        // Use checksummed address (ethers to_checksum) to match Python client
        headers.insert(
            "POLY_ADDRESS".to_string(),
            ethers::utils::to_checksum(&self.wallet_address, None),
        );
        headers.insert("POLY_SIGNATURE".to_string(), signature);
        headers.insert("POLY_TIMESTAMP".to_string(), timestamp.to_string());
        headers.insert("POLY_API_KEY".to_string(), api_key.key.clone());
        headers.insert("POLY_PASSPHRASE".to_string(), api_key.passphrase.clone());

        Ok(headers)
    }

    /// Get current Unix timestamp in seconds
    pub fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs()
    }

    /// Get the wallet reference (for order building)
    pub fn wallet(&self) -> &LocalWallet {
        &self.wallet
    }

    /// Get the chain ID
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Sign a raw message hash (for EIP-712 signing)
    ///
    /// This signs the 32-byte hash directly without any prefix.
    /// Used for EIP-712 typed data signing where the hash is already computed.
    pub fn sign_hash(&self, hash: H256) -> Result<Signature> {
        self.wallet
            .sign_hash(hash)
            .map_err(|e| AuthError::SigningError(e.to_string()))
    }

    /// Sign a raw message hash and return as hex string
    pub fn sign_hash_hex(&self, hash: H256) -> Result<String> {
        let signature = self.sign_hash(hash)?;
        Ok(format!("0x{}", hex::encode(signature.to_vec())))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_creation() {
        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let auth = PolymarketAuth::new(private_key, 137);
        assert!(auth.is_ok());
    }

    #[test]
    fn test_invalid_private_key() {
        let private_key = "invalid";
        let auth = PolymarketAuth::new(private_key, 137);
        assert!(auth.is_err());
    }

    #[tokio::test]
    async fn test_l1_signature() {
        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let auth = PolymarketAuth::new(private_key, 137).unwrap();

        let timestamp = 1234567890;
        let nonce = 0;

        let signature = auth.sign_l1_message(timestamp, nonce).await;
        assert!(signature.is_ok());
        assert!(signature.unwrap().starts_with("0x"));
    }

    #[test]
    fn test_l2_signature() {
        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let mut auth = PolymarketAuth::new(private_key, 137).unwrap();

        // Set mock API credentials with valid base64 secret
        // "dGVzdF9zZWNyZXRfMTIzNDU2" is base64 for "test_secret_123456"
        auth.set_api_key(ApiCredentials {
            key: "test_key".to_string(),
            secret: "dGVzdF9zZWNyZXRfMTIzNDU2".to_string(),
            passphrase: "test_pass".to_string(),
        });

        let timestamp = 1234567890;
        let signature = auth.sign_l2_request(timestamp, "GET", "/markets", "");

        assert!(signature.is_ok());
    }
}
