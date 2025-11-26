//! Client module - Re-exported from infrastructure layer
//!
//! This maintains backward compatibility while following Clean Architecture.
//! The actual implementation is in the infrastructure layer.

// Re-export everything from infrastructure/client for backward compatibility
pub use crate::infrastructure::client::*;
