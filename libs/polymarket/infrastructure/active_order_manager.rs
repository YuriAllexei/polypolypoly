//! Active Order Manager
//!
//! Polls Polymarket REST API for active orders every 1 second.
//! Provides a source of truth for order state.

use crate::infrastructure::client::clob::TradingClient;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct ActiveOrder {
    pub order_id: String,
    pub asset_id: String,
    pub market: String,
    pub side: String,
    pub price: f64,
    pub original_size: f64,
    pub size_matched: f64,
}

impl ActiveOrder {
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            order_id: value.get("id")?.as_str()?.to_string(),
            asset_id: value.get("asset_id")?.as_str()?.to_string(),
            market: value.get("market")?.as_str().unwrap_or("").to_string(),
            side: value.get("side")?.as_str()?.to_string(),
            price: value.get("price")?.as_str()?.parse().ok()?,
            original_size: value.get("original_size")?.as_str()?.parse().ok()?,
            size_matched: value.get("size_matched")?.as_str()?.parse().unwrap_or(0.0),
        })
    }

    pub fn remaining_size(&self) -> f64 {
        self.original_size - self.size_matched
    }
}

pub struct ActiveOrderManager {
    orders: Arc<RwLock<HashMap<String, ActiveOrder>>>,
    orders_by_token: Arc<RwLock<HashMap<String, Vec<String>>>>,
    task_handle: Option<JoinHandle<()>>,
}

impl ActiveOrderManager {
    pub fn new() -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new())),
            orders_by_token: Arc::new(RwLock::new(HashMap::new())),
            task_handle: None,
        }
    }

    pub async fn start(
        &mut self,
        trading: Arc<TradingClient>,
        shutdown_flag: Arc<AtomicBool>,
    ) -> anyhow::Result<()> {
        let initial = trading.get_orders(None).await?;
        self.update_orders(&initial);
        info!(
            "ActiveOrderManager started: {} orders",
            self.orders.read().unwrap().len()
        );

        let orders = Arc::clone(&self.orders);
        let orders_by_token = Arc::clone(&self.orders_by_token);

        let handle = tokio::spawn(async move {
            while shutdown_flag.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_secs(1)).await;

                if !shutdown_flag.load(Ordering::Acquire) {
                    break;
                }

                match trading.get_orders(None).await {
                    Ok(api_orders) => {
                        let mut new_orders = HashMap::new();
                        let mut new_by_token: HashMap<String, Vec<String>> = HashMap::new();

                        for value in &api_orders {
                            if let Some(order) = ActiveOrder::from_json(value) {
                                new_by_token
                                    .entry(order.asset_id.clone())
                                    .or_default()
                                    .push(order.order_id.clone());
                                new_orders.insert(order.order_id.clone(), order);
                            }
                        }

                        *orders.write().unwrap() = new_orders;
                        *orders_by_token.write().unwrap() = new_by_token;

                        debug!(
                            "ActiveOrderManager: {} active orders",
                            orders.read().unwrap().len()
                        );
                    }
                    Err(e) => {
                        warn!("ActiveOrderManager: Failed to fetch orders: {}", e);
                    }
                }
            }
            info!("ActiveOrderManager stopped");
        });

        self.task_handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }

    fn update_orders(&self, api_orders: &[serde_json::Value]) {
        let mut orders = self.orders.write().unwrap();
        let mut by_token = self.orders_by_token.write().unwrap();
        orders.clear();
        by_token.clear();

        for value in api_orders {
            if let Some(order) = ActiveOrder::from_json(value) {
                by_token
                    .entry(order.asset_id.clone())
                    .or_default()
                    .push(order.order_id.clone());
                orders.insert(order.order_id.clone(), order);
            }
        }
    }

    pub fn has_order(&self, order_id: &str) -> bool {
        self.orders.read().unwrap().contains_key(order_id)
    }

    pub fn get_order(&self, order_id: &str) -> Option<ActiveOrder> {
        self.orders.read().unwrap().get(order_id).cloned()
    }

    pub fn get_orders_for_token(&self, token_id: &str) -> Vec<ActiveOrder> {
        let orders = self.orders.read().unwrap();
        let by_token = self.orders_by_token.read().unwrap();
        by_token
            .get(token_id)
            .map(|ids| ids.iter().filter_map(|id| orders.get(id).cloned()).collect())
            .unwrap_or_default()
    }

    pub fn order_count(&self) -> usize {
        self.orders.read().unwrap().len()
    }
}

impl Default for ActiveOrderManager {
    fn default() -> Self {
        Self::new()
    }
}
