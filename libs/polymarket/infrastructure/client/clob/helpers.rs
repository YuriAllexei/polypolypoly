//! Shared HTTP helper functions for the CLOB client
//!
//! Provides common patterns for error handling, response validation,
//! and request building.

use reqwest::RequestBuilder;
use std::collections::HashMap;

use super::rest::RestError;

/// Extract error message from a failed API response
pub async fn extract_api_error(response: reqwest::Response, context: &str) -> RestError {
    let error_text = response
        .text()
        .await
        .unwrap_or_else(|_| "Unknown error".to_string());
    RestError::ApiError(format!("{}: {}", context, error_text))
}

/// Check if response is successful, returning the response or an error
pub async fn require_success(
    response: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response, RestError> {
    if !response.status().is_success() {
        return Err(extract_api_error(response, context).await);
    }
    Ok(response)
}

/// Add headers from a HashMap to a request builder
pub fn with_headers(
    req: RequestBuilder,
    headers: HashMap<String, String>,
) -> RequestBuilder {
    headers
        .into_iter()
        .fold(req, |r, (k, v)| r.header(k, v))
}

/// Deserialize JSON response with proper error handling
pub async fn parse_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, RestError> {
    response
        .json()
        .await
        .map_err(|e| RestError::DeserializeFailed(e.to_string()))
}
