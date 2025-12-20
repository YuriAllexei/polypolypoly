//! PONG Detection Trait
//!
//! Provides a trait for detecting PONG responses in the WebSocket message stream.
//! Different servers may use different PONG formats (text "PONG", JSON, etc.),
//! so this trait allows customization.

use crate::traits::WsMessage;

/// Trait for detecting PONG responses in the message stream
///
/// Implement this trait to define how PONG messages are identified
/// for a specific WebSocket protocol.
///
/// # Example
///
/// ```rust,ignore
/// use hypersockets::traits::{PongDetector, WsMessage};
///
/// struct TextPongDetector;
///
/// impl PongDetector for TextPongDetector {
///     fn is_pong(&self, message: &WsMessage) -> bool {
///         if let WsMessage::Text(text) = message {
///             text == "PONG"
///         } else {
///             false
///         }
///     }
/// }
/// ```
pub trait PongDetector: Send + Sync {
    /// Check if the given message is a PONG response
    ///
    /// Returns true if the message is a PONG response that should be
    /// tracked by the PongTracker.
    fn is_pong(&self, message: &WsMessage) -> bool;
}

/// A simple text-based PONG detector
///
/// Detects PONG messages that are exactly the configured text string.
pub struct TextPongDetector {
    pong_text: String,
}

impl TextPongDetector {
    /// Create a new text PONG detector
    ///
    /// # Arguments
    /// * `pong_text` - The exact text that indicates a PONG response
    pub fn new(pong_text: impl Into<String>) -> Self {
        Self {
            pong_text: pong_text.into(),
        }
    }
}

impl PongDetector for TextPongDetector {
    fn is_pong(&self, message: &WsMessage) -> bool {
        if let WsMessage::Text(text) = message {
            text == &self.pong_text
        } else {
            false
        }
    }
}

/// No-op PONG detector that never detects PONGs
///
/// Use this when PONG detection is not needed.
pub struct NoOpPongDetector;

impl PongDetector for NoOpPongDetector {
    fn is_pong(&self, _message: &WsMessage) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_pong_detector() {
        let detector = TextPongDetector::new("PONG");

        assert!(detector.is_pong(&WsMessage::Text("PONG".to_string())));
        assert!(!detector.is_pong(&WsMessage::Text("pong".to_string())));
        assert!(!detector.is_pong(&WsMessage::Text("PING".to_string())));
        assert!(!detector.is_pong(&WsMessage::Binary(vec![1, 2, 3])));
    }

    #[test]
    fn test_noop_pong_detector() {
        let detector = NoOpPongDetector;

        assert!(!detector.is_pong(&WsMessage::Text("PONG".to_string())));
        assert!(!detector.is_pong(&WsMessage::Text("anything".to_string())));
    }
}
