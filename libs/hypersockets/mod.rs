//! # HyperSockets
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

pub mod traits;
pub mod core;
pub mod manager;

// Re-export all traits
pub use traits::*;

// Re-export core client functionality
pub use core::{
    builder, client, config, connection_state, heartbeat,
    builder::{states, RoutingBuilder, WebSocketClientBuilder},
    client::{ClientEvent, Metrics, WebSocketClient},
    config::ClientConfig,
    connection_state::{AtomicConnectionState, AtomicMetrics, ConnectionState},
};

// Re-export manager
pub use manager::ClientManager;

// Convenience function
pub use core::builder as client_builder;

/// Type alias for Result with HyperSocketError
pub type Result<T> = std::result::Result<T, traits::HyperSocketError>;
