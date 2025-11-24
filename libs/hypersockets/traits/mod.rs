//! # HyperSockets Traits
//!
//! Core traits and types for the HyperSockets WebSocket client library.
//!
//! This crate provides the fundamental abstractions used throughout
//! the HyperSockets ecosystem:
//!
//! - **MessageParser**: Parse incoming WebSocket messages
//! - **AuthProvider**: Handle authentication/authorization
//! - **ReconnectionStrategy**: Control reconnection behavior
//! - **StateHandler**: Manage application state
//! - **PassivePingDetector**: Detect and respond to passive pings
//!
//! ## Example
//!
//! ```rust,ignore
//! use hypersockets_traits::*;
//!
//! // Implement custom message parser
//! struct MyParser;
//!
//! #[async_trait]
//! impl MessageParser for MyParser {
//!     async fn parse(&self, message: WsMessage) -> Result<()> {
//!         // Your parsing logic here
//!         Ok(())
//!     }
//! }
//! ```

pub mod auth;
pub mod error;
pub mod headers;
pub mod parser;
pub mod passive_ping;
pub mod reconnect;
pub mod router;
pub mod state;

// Re-export commonly used types
pub use auth::{AuthProvider, NoAuth};
pub use error::{HyperSocketError, Result};
pub use headers::{HeaderProvider, Headers, NoHeaders};
pub use parser::{MessageParser, NoOpParser, WsMessage};
pub use passive_ping::{JsonPassivePing, NoOpPassivePing, PassivePingDetector, TextPassivePing};
pub use reconnect::{ExponentialBackoff, FixedDelay, NeverReconnect, ReconnectionStrategy};
pub use router::{MessageHandler, MessageRouter};
pub use state::{NoOpState, StateHandler};
