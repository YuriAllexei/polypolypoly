use crate::error::Result;
use crate::parser::WsMessage;
use async_trait::async_trait;

/// Trait for handling application state
///
/// Implement this trait to define how your application state
/// should be updated in response to WebSocket messages or events.
///
/// The state is managed in a dedicated task and accessed via
/// message passing for lock-free operation.
#[async_trait]
pub trait StateHandler: Send + Sync + 'static {
    /// Handle a message and potentially update state
    ///
    /// This method is called after a message has been parsed.
    /// You can use this to update your application state based
    /// on the message content.
    ///
    /// # Arguments
    /// * `message` - The parsed WebSocket message
    ///
    /// # Returns
    /// * `Ok(())` - State updated successfully
    /// * `Err(HyperSocketError)` - State update failed
    async fn handle_message(&mut self, message: &WsMessage) -> Result<()>;

    /// Handle connection state change
    ///
    /// This method is called whenever the connection state changes
    /// (connected, disconnected, reconnecting, etc.)
    ///
    /// # Arguments
    /// * `connected` - true if connected, false if disconnected
    async fn handle_connection_change(&mut self, connected: bool) -> Result<()>;

    /// Get a snapshot of the current state
    ///
    /// This method allows you to retrieve information from your state
    /// without modifying it. The returned value should be a lightweight
    /// representation of the state.
    async fn snapshot(&self) -> Result<String>;
}

/// A no-op state handler that doesn't maintain any state
pub struct NoOpState;

#[async_trait]
impl StateHandler for NoOpState {
    async fn handle_message(&mut self, _message: &WsMessage) -> Result<()> {
        Ok(())
    }

    async fn handle_connection_change(&mut self, _connected: bool) -> Result<()> {
        Ok(())
    }

    async fn snapshot(&self) -> Result<String> {
        Ok("NoOpState".to_string())
    }
}
