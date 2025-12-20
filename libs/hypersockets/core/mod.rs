// ! # HyperSockets
//!
//! A high-performance, modular WebSocket client built for extreme configurability
//! and maximum throughput.
//!
//! ## Features
//!
//! - **Lock-free architecture**: Atomic state management and unbounded crossbeam channels
//! - **Type-state builder**: Compile-time guarantees for required configuration
//! - **Parallel message parsing**: Each message parsed in dedicated task
//! - **Modular design**: Pluggable auth, heartbeat, passive ping, reconnection strategies
//! - **Performance-focused**: Zero-copy where possible, minimal allocations
//!
//! ## Example
//!
//! ```rust,ignore
//! use hypersockets::WebSocketClient;
//! use crate::traits::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let client = WebSocketClient::builder()
//!         .url("wss://api.example.com")
//!         .parser(MyParser)
//!         .state(MyState::new())
//!         .heartbeat(Duration::from_secs(30), WsMessage::Text("ping".into()))
//!         .reconnect_strategy(ExponentialBackoff::new(
//!             Duration::from_secs(1),
//!             Duration::from_secs(60),
//!             None, // unlimited retries
//!         ))
//!         .build()
//!         .await?;
//!
//!     // Send a message
//!     client.send(WsMessage::Text("hello".into()))?;
//!
//!     // Receive events
//!     while let Ok(event) = client.recv_event() {
//!         println!("Event: {:?}", event);
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod builder;
pub mod client;
pub mod config;
pub mod connection_state;
pub mod heartbeat;
pub mod pong_tracker;

// Re-export main types
pub use builder::{states, RoutingBuilder, WebSocketClientBuilder};
pub use client::{ClientEvent, Metrics, WebSocketClient};
pub use config::ClientConfig;
pub use connection_state::{AtomicConnectionState, AtomicMetrics, ConnectionState};
pub use pong_tracker::PongTracker;

// Re-export traits for convenience
pub use crate::traits::*;

/// Create a new WebSocket client builder
///
/// This is a convenience function for starting the builder pattern.
///
/// # Example
/// ```ignore
/// let client = hypersockets::builder()
///     .url("wss://api.example.com")
///     .router(MyRouter, |routing| {
///         routing
///             .handler(MessageType::Trade, TradeHandler::new())
///             .handler(MessageType::Book, BookHandler::new())
///     })
///     .heartbeat(Duration::from_secs(30), WsMessage::Text("ping".into()))
///     .passive_ping(TextPassivePing::new("ping", WsMessage::Text("pong".into())))
///     .build()
///     .await?;
/// ```
pub fn builder() -> WebSocketClientBuilder<
    builder::states::NoUrl,
    builder::states::NoRouter,
    (),
    (),
> {
    WebSocketClientBuilder::new()
}
