use async_trait::async_trait;
use std::collections::HashMap;

/// HTTP headers to send with WebSocket connection
pub type Headers = HashMap<String, String>;

/// Trait for providing HTTP headers dynamically
///
/// Implement this trait to define headers that should be sent
/// with the WebSocket connection request. This is called on every
/// connection/reconnection, allowing for dynamic header generation.
///
/// # Use Cases
/// - Authorization tokens that change
/// - Timestamps or nonces
/// - Session IDs
/// - API keys
/// - Custom authentication schemes
///
/// # Example
/// ```ignore
/// struct DynamicHeaders {
///     api_key: String,
/// }
///
/// #[async_trait::async_trait]
/// impl HeaderProvider for DynamicHeaders {
///     async fn get_headers(&self) -> Headers {
///         let mut headers = HashMap::new();
///
///         // Add API key
///         headers.insert("X-API-Key".to_string(), self.api_key.clone());
///
///         // Add fresh timestamp on every connection
///         let timestamp = chrono::Utc::now().timestamp_millis();
///         headers.insert("X-Timestamp".to_string(), timestamp.to_string());
///
///         // Add fresh nonce
///         headers.insert("X-Nonce".to_string(), uuid::Uuid::new_v4().to_string());
///
///         headers
///     }
/// }
/// ```
#[async_trait]
pub trait HeaderProvider: Send + Sync {
    /// Generate headers to send with the WebSocket connection
    ///
    /// This method is called every time a WebSocket connection is
    /// established (including reconnections), allowing you to generate
    /// fresh headers with timestamps, nonces, tokens, etc.
    ///
    /// # Returns
    /// A HashMap of header name -> header value pairs
    async fn get_headers(&self) -> Headers;
}

/// A no-op header provider that doesn't add any headers
pub struct NoHeaders;

#[async_trait]
impl HeaderProvider for NoHeaders {
    async fn get_headers(&self) -> Headers {
        HashMap::new()
    }
}
