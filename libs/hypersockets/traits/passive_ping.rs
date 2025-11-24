use crate::parser::WsMessage;

/// Trait for detecting and responding to passive ping messages
///
/// Some WebSocket servers send ping messages as regular data messages
/// (not WebSocket PING frames). This trait allows you to detect these
/// messages and provide an appropriate response.
///
/// # Important
/// Both detection (inbound) AND response (outbound) are required.
/// When a passive ping is detected, the response will ALWAYS be sent.
///
/// # Flow
/// ```text
/// Server ──["ping"]──> Client
///                       │
///                       ├─> is_ping() checks message
///                       │   └─> returns true
///                       │
///                       ├─> get_pong_response() gets response
///                       │   └─> MUST return a WsMessage
///                       │
/// Server <─["pong"]──── Client sends response immediately
/// ```
pub trait PassivePingDetector: Send + Sync {
    /// Check if a message is a passive ping from the server (INBOUND detection)
    ///
    /// This is called for EVERY received message to check if it's a passive ping.
    /// If this returns true, the message will NOT be sent to the parser.
    ///
    /// # Arguments
    /// * `message` - The received message to check
    ///
    /// # Returns
    /// * `true` - This message is a passive ping, will trigger response
    /// * `false` - This is a normal message, will be parsed normally
    fn is_ping(&self, message: &WsMessage) -> bool;

    /// Get the response message for a passive ping (OUTBOUND response)
    ///
    /// This method is called immediately when `is_ping` returns true.
    /// The returned message will be sent to the server automatically.
    ///
    /// # Returns
    /// The message to send as the pong response (REQUIRED, not optional)
    fn get_pong_response(&self) -> WsMessage;
}

/// A no-op passive ping detector that never detects pings
///
/// This is used as a placeholder when passive ping is not configured.
pub struct NoOpPassivePing;

impl PassivePingDetector for NoOpPassivePing {
    fn is_ping(&self, _message: &WsMessage) -> bool {
        false
    }

    fn get_pong_response(&self) -> WsMessage {
        // This should never be called since is_ping always returns false
        WsMessage::Text(String::new())
    }
}

/// Simple text-based passive ping detector
///
/// Detects pings based on exact text matching and responds with a configured message.
///
/// # Example
/// ```ignore
/// // Detect "ping" and respond with "pong"
/// let detector = TextPassivePing::new(
///     "ping",
///     WsMessage::Text("pong".to_string())
/// );
/// ```
pub struct TextPassivePing {
    ping_text: String,
    pong_response: WsMessage,
}

impl TextPassivePing {
    /// Create a new text-based passive ping detector
    ///
    /// Both inbound detection pattern AND outbound response are required.
    ///
    /// # Arguments
    /// * `ping_text` - The exact text to match for ping detection (INBOUND)
    /// * `pong_response` - The message to send as a response (OUTBOUND)
    pub fn new(ping_text: impl Into<String>, pong_response: WsMessage) -> Self {
        Self {
            ping_text: ping_text.into(),
            pong_response,
        }
    }
}

impl PassivePingDetector for TextPassivePing {
    fn is_ping(&self, message: &WsMessage) -> bool {
        message
            .as_text()
            .map(|text| text == self.ping_text)
            .unwrap_or(false)
    }

    fn get_pong_response(&self) -> WsMessage {
        self.pong_response.clone()
    }
}

/// JSON-based passive ping detector
///
/// Detects JSON messages with a specific field/value and responds with a JSON message.
///
/// # Example
/// ```ignore
/// // Detect {"type":"ping"} and respond with {"type":"pong"}
/// let detector = JsonPassivePing::new(
///     "type",
///     "ping",
///     WsMessage::Text(r#"{"type":"pong"}"#.to_string())
/// );
/// ```
pub struct JsonPassivePing {
    field_name: String,
    ping_value: String,
    pong_response: WsMessage,
}

impl JsonPassivePing {
    /// Create a new JSON-based passive ping detector
    ///
    /// # Arguments
    /// * `field_name` - The JSON field to check (e.g., "type", "event")
    /// * `ping_value` - The value that indicates a ping (e.g., "ping")
    /// * `pong_response` - The message to send as response (OUTBOUND)
    pub fn new(
        field_name: impl Into<String>,
        ping_value: impl Into<String>,
        pong_response: WsMessage,
    ) -> Self {
        Self {
            field_name: field_name.into(),
            ping_value: ping_value.into(),
            pong_response,
        }
    }
}

impl PassivePingDetector for JsonPassivePing {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if let Some(text) = message.as_text() {
            // Try to parse as JSON and check the field
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                if let Some(value) = json.get(&self.field_name) {
                    return value.as_str() == Some(&self.ping_value);
                }
            }
        }
        false
    }

    fn get_pong_response(&self) -> WsMessage {
        self.pong_response.clone()
    }
}
