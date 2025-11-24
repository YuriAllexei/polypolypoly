//! Message Routing System
//!
//! This module provides the core traits for conditional message routing with
//! per-message-type ordering guarantees while allowing cross-type parallelism.
//!
//! # Architecture
//!
//! ```text
//! WebSocket → PassivePing? → Router → Route Key → Channel → Handler Task
//!                                         ↓              ↓
//!                                    TypeA Channel → TypeA Handler (sequential)
//!                                    TypeB Channel → TypeB Handler (sequential)
//!                                    TypeC Channel → TypeC Handler (sequential)
//!                                         ↑
//!                                   (parallel across types)
//! ```
//!
//! # Ordering Guarantees
//!
//! - **Per-Type Sequential**: Messages of the same type processed in order
//! - **Cross-Type Parallel**: Different types processed concurrently
//! - **Lock-Free**: Unbounded crossbeam channels, no backpressure

use crate::{Result, WsMessage};
use async_trait::async_trait;
use std::fmt::Debug;
use std::hash::Hash;

/// Message router that parses WebSocket messages and determines routing
///
/// The router has two responsibilities:
/// 1. Parse the raw WebSocket message into a typed message
/// 2. Extract a route key that determines which handler processes it
///
/// # Type Parameters
/// - `M`: The parsed message type (must be Send + Debug)
/// - `K`: The route key type (must be Hash + Eq + Clone + Send + Debug)
///
/// # Example
///
/// ```ignore
/// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// enum MessageType {
///     Trade,
///     OrderUpdate,
///     BookSnapshot,
/// }
///
/// #[derive(Debug)]
/// enum ExchangeMessage {
///     Trade { symbol: String, price: f64 },
///     OrderUpdate { order_id: String, status: String },
///     BookSnapshot { symbol: String, bids: Vec<(f64, f64)> },
/// }
///
/// struct ExchangeRouter;
///
/// #[async_trait]
/// impl MessageRouter for ExchangeRouter {
///     type Message = ExchangeMessage;
///     type RouteKey = MessageType;
///
///     async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
///         // Parse JSON and return typed message
///     }
///
///     fn route_key(&self, message: &Self::Message) -> Self::RouteKey {
///         match message {
///             ExchangeMessage::Trade { .. } => MessageType::Trade,
///             ExchangeMessage::OrderUpdate { .. } => MessageType::OrderUpdate,
///             ExchangeMessage::BookSnapshot { .. } => MessageType::BookSnapshot,
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait MessageRouter: Send + Sync + 'static {
    /// The parsed message type
    type Message: Send + Debug + 'static;

    /// The route key type (determines which handler processes the message)
    type RouteKey: Hash + Eq + Clone + Send + Sync + Debug + 'static;

    /// Parse a raw WebSocket message into a typed message
    ///
    /// This is called for every non-ping message received from the WebSocket.
    /// Parsing errors should return `Err(HyperSocketError)`.
    ///
    /// # Performance
    /// This is on the hot path - keep parsing fast!
    async fn parse(&self, message: WsMessage) -> Result<Self::Message>;

    /// Extract the route key from a parsed message
    ///
    /// The route key determines which handler task will process this message.
    /// Messages with the same route key are processed sequentially in order.
    /// Messages with different route keys are processed in parallel.
    ///
    /// # Performance
    /// This is on the hot path - should be a simple match/field access!
    fn route_key(&self, message: &Self::Message) -> Self::RouteKey;
}

/// Message handler that processes typed messages sequentially
///
/// Each handler runs in its own dedicated OS thread and processes messages
/// sequentially in the order they were received. Multiple handlers for
/// different message types run in parallel on separate threads.
///
/// # Type Parameters
/// - `M`: The message type this handler processes
///
/// # Performance
/// Handlers run on dedicated OS threads (not async tasks) for maximum
/// performance when processing messages. This is optimal for CPU-intensive
/// operations like order book updates, pricing calculations, and state updates
/// with lock-free primitives.
///
/// # Example
///
/// ```ignore
/// struct TradeHandler {
///     trades_processed: Arc<AtomicU64>,
/// }
///
/// impl MessageHandler<ExchangeMessage> for TradeHandler {
///     fn handle(&mut self, message: ExchangeMessage) -> Result<()> {
///         if let ExchangeMessage::Trade { symbol, price } = message {
///             println!("Trade: {} @ ${}", symbol, price);
///             self.trades_processed.fetch_add(1, Ordering::Relaxed);
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait MessageHandler<M>: Send + 'static
where
    M: Send + Debug + 'static,
{
    /// Handle a parsed message
    ///
    /// This is called sequentially for each message routed to this handler.
    /// Messages are guaranteed to be processed in order for this handler.
    ///
    /// **Important**: This method runs on a dedicated OS thread, not in an
    /// async context. It should perform blocking operations directly without
    /// using async/await.
    ///
    /// # Errors
    /// If this returns an error, it will be logged but the handler thread
    /// continues processing subsequent messages.
    fn handle(&mut self, message: M) -> Result<()>;
}
