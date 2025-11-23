use crate::types::ApiCredentials;
use ethers::prelude::*;
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

        // Build signature message: timestamp + method + path + body
        let message = format!("{}{}{}{}", timestamp, method, path, body);

        // Compute HMAC-SHA256
        let mut mac = HmacSha256::new_from_slice(api_key.secret.as_bytes())
            .map_err(|e| AuthError::HmacError(e.to_string()))?;

        mac.update(message.as_bytes());

        let result = mac.finalize();
        let signature_bytes = result.into_bytes();

        Ok(hex::encode(signature_bytes))
    }

    /// Build L1 authentication headers
    pub async fn l1_headers(&self, timestamp: u64, nonce: u64) -> Result<HashMap<String, String>> {
        let signature = self.sign_l1_message(timestamp, nonce).await?;

        let mut headers = HashMap::new();
        headers.insert(
            "POLY_ADDRESS".to_string(),
            format!("{:?}", self.wallet_address),
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
        headers.insert(
            "POLY_ADDRESS".to_string(),
            format!("{:?}", self.wallet_address),
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

        // Set mock API credentials
        auth.set_api_key(ApiCredentials {
            key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            passphrase: "test_pass".to_string(),
        });

        let timestamp = 1234567890;
        let signature = auth.sign_l2_request(timestamp, "GET", "/markets", "");

        assert!(signature.is_ok());
    }
}
