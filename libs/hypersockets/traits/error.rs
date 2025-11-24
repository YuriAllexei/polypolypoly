use thiserror::Error;

/// Main error type for hypersockets
#[derive(Error, Debug)]
pub enum HyperSocketError {
    /// WebSocket connection error
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// Connection closed unexpectedly
    #[error("Connection closed: {0}")]
    ConnectionClosed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Message parsing error
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Channel send error
    #[error("Channel send error: {0}")]
    ChannelSend(String),

    /// Channel receive error
    #[error("Channel receive error: {0}")]
    ChannelReceive(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Reconnection failed
    #[error("Reconnection failed after {attempts} attempts: {reason}")]
    ReconnectionFailed { attempts: usize, reason: String },

    /// Timeout error
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Invalid state transition
    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    /// Generic error
    #[error("Error: {0}")]
    Other(String),
}

/// Result type for hypersockets operations
pub type Result<T> = std::result::Result<T, HyperSocketError>;
