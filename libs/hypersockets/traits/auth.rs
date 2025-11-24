use crate::error::Result;
use crate::parser::WsMessage;
use async_trait::async_trait;

/// Trait for providing authentication/authorization logic
///
/// Implement this trait to define how your WebSocket client
/// should authenticate with the server.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Get the authentication message to send after connection
    ///
    /// This method is called immediately after a successful WebSocket
    /// connection is established (or re-established after reconnection).
    ///
    /// # Returns
    /// * `Ok(Some(message))` - Send this message for authentication
    /// * `Ok(None)` - No authentication required
    /// * `Err(HyperSocketError)` - Authentication preparation failed
    async fn get_auth_message(&self) -> Result<Option<WsMessage>>;

    /// Validate authentication response from server
    ///
    /// This method is called after sending the auth message to check
    /// if authentication was successful.
    ///
    /// # Arguments
    /// * `response` - The server's response message
    ///
    /// # Returns
    /// * `Ok(true)` - Authentication successful
    /// * `Ok(false)` - Authentication failed
    /// * `Err(HyperSocketError)` - Error validating response
    async fn validate_auth_response(&self, response: &WsMessage) -> Result<bool>;
}

/// A no-op auth provider that doesn't require authentication
pub struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn get_auth_message(&self) -> Result<Option<WsMessage>> {
        Ok(None)
    }

    async fn validate_auth_response(&self, _response: &WsMessage) -> Result<bool> {
        Ok(true)
    }
}
