use super::types::OrderBook;
use hypersockets::core::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Message types from Polymarket WebSocket
#[derive(Debug, Clone)]
pub enum PolymarketMessage {
    /// Market orderbook update
    Market {
        asset_id: String,
        orderbook: OrderBook,
    },

    /// User-specific order update
    UserOrder {
        order_id: String,
        status: String,
    },

    /// Pong response
    Pong,

    /// Other/Unknown message
    Other { text: String },
}

/// Route key for message routing
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Route {
    Market(String),  // Per-market orderbook (by asset_id)
    UserOrders,      // User order updates
    Pong,            // Pong responses
}

/// Polymarket WebSocket router
pub struct PolymarketRouter;

#[async_trait::async_trait]
impl MessageRouter for PolymarketRouter {
    type Message = PolymarketMessage;
    type RouteKey = Route;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        if let Some(text) = message.as_text() {
            // Try to parse as JSON
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                // Check message type
                if let Some(msg_type) = json.get("type").and_then(|v| v.as_str()) {
                    match msg_type {
                        "market" => {
                            // Orderbook update
                            let asset_id = json
                                .get("asset_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();

                            // Try to deserialize orderbook
                            if let Ok(orderbook) = serde_json::from_value::<OrderBook>(json.clone()) {
                                return Ok(PolymarketMessage::Market {
                                    asset_id,
                                    orderbook,
                                });
                            }
                        }
                        "user" => {
                            // User order update
                            let order_id = json
                                .get("order_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();

                            let status = json
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();

                            return Ok(PolymarketMessage::UserOrder { order_id, status });
                        }
                        "pong" => {
                            return Ok(PolymarketMessage::Pong);
                        }
                        _ => {}
                    }
                }
            }

            // Unknown message
            Ok(PolymarketMessage::Other {
                text: text.to_string(),
            })
        } else {
            Ok(PolymarketMessage::Other {
                text: "Binary data".to_string(),
            })
        }
    }

    fn route_key(&self, message: &Self::Message) -> Self::RouteKey {
        match message {
            PolymarketMessage::Market { asset_id, .. } => Route::Market(asset_id.clone()),
            PolymarketMessage::UserOrder { .. } => Route::UserOrders,
            PolymarketMessage::Pong => Route::Pong,
            PolymarketMessage::Other { .. } => Route::Market("OTHER".to_string()),
        }
    }
}

/// Handler for orderbook updates
pub struct OrderbookHandler {
    pub asset_id: String,
    pub orderbook: Arc<RwLock<Option<OrderBook>>>,
    message_count: u64,
}

impl OrderbookHandler {
    pub fn new(asset_id: String) -> Self {
        Self {
            asset_id,
            orderbook: Arc::new(RwLock::new(None)),
            message_count: 0,
        }
    }

    /// Get current orderbook
    pub fn get_orderbook(&self) -> Option<OrderBook> {
        self.orderbook.read().ok()?.clone()
    }
}

impl MessageHandler<PolymarketMessage> for OrderbookHandler {
    fn handle(&mut self, message: PolymarketMessage) -> hypersockets::Result<()> {
        if let PolymarketMessage::Market { orderbook, .. } = message {
            self.message_count += 1;

            debug!(
                "[{}] Orderbook update #{} - {} bids, {} asks",
                self.asset_id,
                self.message_count,
                orderbook.bids.len(),
                orderbook.asks.len()
            );

            // Update stored orderbook
            if let Ok(mut ob) = self.orderbook.write() {
                *ob = Some(orderbook);
            }
        }

        Ok(())
    }
}

/// Handler for pong messages
pub struct PongHandler {
    pong_count: u64,
}

impl PongHandler {
    pub fn new() -> Self {
        Self { pong_count: 0 }
    }
}

impl MessageHandler<PolymarketMessage> for PongHandler {
    fn handle(&mut self, message: PolymarketMessage) -> hypersockets::Result<()> {
        if let PolymarketMessage::Pong = message {
            self.pong_count += 1;
            debug!("Pong received (#{}) - Connection alive", self.pong_count);
        }
        Ok(())
    }
}

/// Handler for user orders
pub struct UserOrderHandler {
    order_count: u64,
}

impl UserOrderHandler {
    pub fn new() -> Self {
        Self { order_count: 0 }
    }
}

impl MessageHandler<PolymarketMessage> for UserOrderHandler {
    fn handle(&mut self, message: PolymarketMessage) -> hypersockets::Result<()> {
        if let PolymarketMessage::UserOrder { order_id, status } = message {
            self.order_count += 1;
            debug!("Order {} status: {}", order_id, status);
        }
        Ok(())
    }
}

/// Subscription message for Polymarket WebSocket
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionMessage {
    #[serde(rename = "type")]
    pub msg_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets_ids: Option<Vec<String>>,
}

impl SubscriptionMessage {
    /// Subscribe to market orderbook updates
    pub fn subscribe_market(asset_ids: Vec<String>) -> WsMessage {
        let msg = Self {
            msg_type: "subscribe".to_string(),
            market: Some("market".to_string()),
            assets_ids: Some(asset_ids),
        };

        WsMessage::Text(serde_json::to_string(&msg).unwrap())
    }

    /// Subscribe to user orders
    pub fn subscribe_user() -> WsMessage {
        let msg = Self {
            msg_type: "subscribe".to_string(),
            market: Some("user".to_string()),
            assets_ids: None,
        };

        WsMessage::Text(serde_json::to_string(&msg).unwrap())
    }
}

/// Orderbook manager - tracks multiple markets
pub struct OrderbookManager {
    handlers: HashMap<String, Arc<RwLock<Option<OrderBook>>>>,
}

impl OrderbookManager {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a new asset to track
    pub fn register_asset(&mut self, asset_id: String, orderbook_ref: Arc<RwLock<Option<OrderBook>>>) {
        self.handlers.insert(asset_id, orderbook_ref);
    }

    /// Get orderbook for an asset
    pub fn get_orderbook(&self, asset_id: &str) -> Option<OrderBook> {
        self.handlers
            .get(asset_id)?
            .read()
            .ok()?
            .clone()
    }

    /// Get all tracked asset IDs
    pub fn tracked_assets(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

impl Default for OrderbookManager {
    fn default() -> Self {
        Self::new()
    }
}
