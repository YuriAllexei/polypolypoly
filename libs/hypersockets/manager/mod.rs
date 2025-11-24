//! # HyperSockets Manager
//!
//! Multi-client supervisor for managing multiple WebSocket connections
//! with centralized control and health monitoring.

pub mod manager;

pub use manager::ClientManager;
pub use crate::core::*;
pub use crate::traits::*;
