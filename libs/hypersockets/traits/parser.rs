use crate::error::Result;
use async_trait::async_trait;

/// Type alias for WebSocket messages
/// Can be Text or Binary data
#[derive(Debug, Clone)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

impl WsMessage {
    /// Get the message as text, if it is text
    pub fn as_text(&self) -> Option<&str> {
        match self {
            WsMessage::Text(s) => Some(s),
            WsMessage::Binary(_) => None,
        }
    }

    /// Get the message as binary, if it is binary
    pub fn as_binary(&self) -> Option<&[u8]> {
        match self {
            WsMessage::Text(_) => None,
            WsMessage::Binary(b) => Some(b),
        }
    }

    /// Check if message is text
    pub fn is_text(&self) -> bool {
        matches!(self, WsMessage::Text(_))
    }

    /// Check if message is binary
    pub fn is_binary(&self) -> bool {
        matches!(self, WsMessage::Binary(_))
    }
}

/// Trait for parsing WebSocket messages
///
/// Implement this trait to define custom message parsing logic
/// that will be executed on each received message in parallel.
#[async_trait]
pub trait MessageParser: Send + Sync {
    /// Parse a received WebSocket message
    ///
    /// This method is called in a dedicated task for each message,
    /// enabling parallel processing of messages.
    ///
    /// # Arguments
    /// * `message` - The WebSocket message to parse
    ///
    /// # Returns
    /// * `Ok(())` - Message parsed successfully
    /// * `Err(HyperSocketError)` - Parsing failed
    async fn parse(&self, message: WsMessage) -> Result<()>;
}

/// A no-op parser that does nothing
/// Useful for testing or when you don't need message parsing
pub struct NoOpParser;

#[async_trait]
impl MessageParser for NoOpParser {
    async fn parse(&self, _message: WsMessage) -> Result<()> {
        Ok(())
    }
}
