//! Main application state and logic for the visualizer

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::RwLock;
use tokio::runtime::Handle;
use tracing::{info, warn, error};

use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::client::clob::sniper_ws::SharedOrderbooks;
use crate::infrastructure::client::user::{
    SharedOrderState,
    PositionTracker, SharedPositionTracker, PositionTrackerBridge,
    spawn_user_order_tracker,
};
use crate::infrastructure::MarketDatabase;

use super::state::MarketInfo;
use crate::application::strategies::inventory_mm::quoter::{
    QuoterWsConfig, QuoterWsClient, build_quoter_ws_client, wait_for_snapshot,
};

/// Main application state
pub struct App {
    /// Order state (our orders)
    pub order_state: SharedOrderState,
    /// Position tracker
    pub position_tracker: SharedPositionTracker,
    /// Orderbooks per market (condition_id -> SharedOrderbooks)
    pub orderbooks: HashMap<String, SharedOrderbooks>,
    /// Database for market metadata
    pub database: Arc<MarketDatabase>,
    /// Trading client for order operations
    trading_client: TradingClient,
    /// WebSocket clients (keep them alive)
    ws_clients: Vec<QuoterWsClient>,
    /// Markets we're active in
    pub markets: Vec<MarketInfo>,
    /// Currently selected market index
    pub selected_index: usize,
    /// Whether to quit
    pub should_quit: bool,
    /// Shutdown flag for WebSocket tasks
    shutdown_flag: Arc<AtomicBool>,
    /// Tokio runtime handle
    runtime: Handle,
    /// Whether initialization completed successfully
    pub initialized: bool,
    /// Status message to show in footer
    pub status_message: Option<String>,
}

impl App {
    /// Initialize the application with real-time components
    pub async fn initialize(runtime: Handle, database_url: &str) -> Result<Self> {
        // true = keep running, false = shutdown requested
        let shutdown_flag = Arc::new(AtomicBool::new(true));

        // Initialize database connection
        info!("[Visualizer] Connecting to database...");
        let database = Arc::new(MarketDatabase::new(database_url).await?);
        info!("[Visualizer] Database connected");

        // Initialize trading client (for REST and auth)
        info!("[Visualizer] Initializing trading client...");
        let trading_client = TradingClient::from_env().await?;
        let rest_client = trading_client.rest();
        let auth = trading_client.auth();

        // Create position tracker
        info!("[Visualizer] Creating position tracker...");
        let position_tracker: SharedPositionTracker = Arc::new(RwLock::new(PositionTracker::new()));

        // Create bridge to forward fills to position tracker
        let bridge = Arc::new(PositionTrackerBridge::new(position_tracker.clone()));

        // Start OMS with WebSocket and REST hydration
        info!("[Visualizer] Starting order tracker...");
        let order_state = spawn_user_order_tracker(
            shutdown_flag.clone(),
            rest_client,
            auth,
            Some(bridge),
        )
        .await?;

        // Wait a moment for orders to hydrate
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Discover markets from orders using database
        info!("[Visualizer] Discovering markets from database...");
        let markets = Self::discover_markets_from_db(&order_state, &database).await;
        info!("[Visualizer] Found {} markets with active orders", markets.len());

        // Connect orderbook WebSockets for each market
        let mut orderbooks = HashMap::new();
        let mut ws_clients = Vec::new();

        for market in &markets {
            info!("[Visualizer] Connecting orderbook for {}...", market.display_name);

            let market_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

            let ws_config = QuoterWsConfig::new(
                market.market_id.clone(),
                market.up_token_id.clone(),
                market.down_token_id.clone(),
            );

            match build_quoter_ws_client(&ws_config, market_orderbooks.clone()).await {
                Ok(ws_client) => {
                    // Wait for initial snapshot
                    let got_snapshot = wait_for_snapshot(
                        &ws_client,
                        &shutdown_flag,
                        &market.market_id,
                        Duration::from_secs(3),
                    )
                    .await;

                    if got_snapshot {
                        info!("[Visualizer] Orderbook connected for {}", market.display_name);
                        orderbooks.insert(market.condition_id.clone(), market_orderbooks);
                        ws_clients.push(ws_client);
                    } else {
                        warn!("[Visualizer] Failed to get snapshot for {}", market.display_name);
                    }
                }
                Err(e) => {
                    error!("[Visualizer] Failed to connect orderbook for {}: {}", market.display_name, e);
                }
            }
        }

        Ok(Self {
            order_state,
            position_tracker,
            orderbooks,
            database,
            trading_client,
            ws_clients,
            markets,
            selected_index: 0,
            should_quit: false,
            shutdown_flag,
            runtime,
            initialized: true,
            status_message: None,
        })
    }

    /// Discover markets from current orders using the database for metadata
    async fn discover_markets_from_db(
        order_state: &SharedOrderState,
        database: &MarketDatabase,
    ) -> Vec<MarketInfo> {
        // Step 1: Get unique condition_ids from orders
        let condition_ids: HashSet<String> = {
            let oms = order_state.read();
            let mut ids = HashSet::new();

            for asset_id in oms.asset_ids() {
                let bids = oms.get_bids(&asset_id);
                let asks = oms.get_asks(&asset_id);

                for order in bids.iter().chain(asks.iter()) {
                    if order.is_open() && !order.market.is_empty() {
                        ids.insert(order.market.clone());
                    }
                }
            }
            ids
        };

        info!("[Visualizer] Found {} unique markets from orders", condition_ids.len());

        // Step 2: Query database for each market to get full metadata
        let mut markets = Vec::new();
        for condition_id in condition_ids {
            match database.get_market_by_condition(&condition_id).await {
                Ok(db_market) => {
                    // Parse outcomes and token_ids from database
                    let outcomes = db_market.parse_outcomes().unwrap_or_default();
                    let token_ids = db_market.parse_token_ids().unwrap_or_default();

                    if outcomes.len() >= 2 && token_ids.len() >= 2 {
                        // Determine which outcome is UP (Yes) and which is DOWN (No)
                        // outcomes and token_ids are parallel arrays
                        let (up_idx, down_idx) = if Self::is_up_outcome(&outcomes[0]) {
                            (0, 1)
                        } else {
                            (1, 0)
                        };

                        info!(
                            "[Visualizer] Market '{}' - UP: {} ({}), DOWN: {} ({})",
                            &db_market.question[..30.min(db_market.question.len())],
                            &outcomes[up_idx],
                            &token_ids[up_idx][..8.min(token_ids[up_idx].len())],
                            &outcomes[down_idx],
                            &token_ids[down_idx][..8.min(token_ids[down_idx].len())]
                        );

                        markets.push(MarketInfo::new(
                            condition_id.clone(),
                            db_market.id.clone(),
                            db_market.question.clone(),
                            token_ids[up_idx].clone(),
                            token_ids[down_idx].clone(),
                            outcomes[up_idx].clone(),
                            outcomes[down_idx].clone(),
                        ));
                    } else {
                        warn!(
                            "[Visualizer] Market {} has insufficient outcomes ({}) or token_ids ({})",
                            &condition_id[..8.min(condition_id.len())],
                            outcomes.len(),
                            token_ids.len()
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "[Visualizer] Market {} not found in database: {}",
                        &condition_id[..8.min(condition_id.len())],
                        e
                    );
                }
            }
        }

        markets
    }

    /// Check if an outcome label indicates UP/Yes side
    fn is_up_outcome(outcome: &str) -> bool {
        let lower = outcome.to_lowercase();
        lower == "yes" || lower.contains("up") || lower.contains("above") || lower.contains("over")
    }

    /// Navigate to next market
    pub fn next_market(&mut self) {
        if !self.markets.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.markets.len();
        }
    }

    /// Navigate to previous market
    pub fn prev_market(&mut self) {
        if !self.markets.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.markets.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Remove markets that have no orders and no positions
    fn remove_inactive_markets(&mut self) {
        let mut indices_to_remove: Vec<usize> = Vec::new();

        for (i, market) in self.markets.iter().enumerate() {
            let order_count = self.get_market_order_count(market);
            let (up_pos, down_pos) = self.get_market_positions(market);

            // Check if market is inactive (no orders AND no positions)
            let has_orders = order_count > 0;
            let has_positions = up_pos.abs() > 0.01 || down_pos.abs() > 0.01;

            if !has_orders && !has_positions {
                indices_to_remove.push(i);
            }
        }

        // Remove in reverse order to maintain correct indices
        for i in indices_to_remove.into_iter().rev() {
            let market = self.markets.remove(i);
            self.orderbooks.remove(&market.condition_id);
        }

        // Adjust selected_index if needed
        if !self.markets.is_empty() {
            if self.selected_index >= self.markets.len() {
                self.selected_index = self.markets.len() - 1;
            }
        } else {
            self.selected_index = 0;
        }
    }

    /// Refresh markets: remove inactive ones and add new ones from orders
    /// Called periodically to keep market list in sync
    pub fn refresh_markets(&mut self) {
        // First, remove markets that are no longer active
        self.remove_inactive_markets();
        let order_state = self.order_state.clone();
        let database = self.database.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        // Run async discovery on runtime
        let discovered_markets = self.runtime.block_on(async {
            Self::discover_markets_from_db(&order_state, &database).await
        });

        // Find markets we don't already have
        let existing_ids: HashSet<String> = self.markets.iter()
            .map(|m| m.condition_id.clone())
            .collect();

        for market in discovered_markets {
            if !existing_ids.contains(&market.condition_id) {
                // Connect orderbook WebSocket for new market
                let market_orderbooks: SharedOrderbooks = Arc::new(RwLock::new(HashMap::new()));

                let ws_config = QuoterWsConfig::new(
                    market.market_id.clone(),
                    market.up_token_id.clone(),
                    market.down_token_id.clone(),
                );

                match self.runtime.block_on(async {
                    build_quoter_ws_client(&ws_config, market_orderbooks.clone()).await
                }) {
                    Ok(ws_client) => {
                        // Wait for initial snapshot
                        let got_snapshot = self.runtime.block_on(async {
                            wait_for_snapshot(
                                &ws_client,
                                &shutdown_flag,
                                &market.market_id,
                                Duration::from_secs(3),
                            )
                            .await
                        });

                        if got_snapshot {
                            self.orderbooks.insert(market.condition_id.clone(), market_orderbooks);
                            self.ws_clients.push(ws_client);
                            self.markets.push(market);
                        }
                    }
                    Err(_) => {
                        // Silently ignore connection failures - will retry on next refresh
                    }
                }
            }
        }
    }

    /// Get currently selected market
    pub fn get_selected_market(&self) -> Option<&MarketInfo> {
        self.markets.get(self.selected_index)
    }

    /// Check if OMS is connected
    pub fn is_oms_connected(&self) -> bool {
        self.initialized
    }

    /// Get total open order count
    pub fn get_total_order_count(&self) -> usize {
        let oms = self.order_state.read();
        let mut count = 0;
        for asset_id in oms.asset_ids() {
            for order in oms.get_bids(&asset_id) {
                if order.is_open() {
                    count += 1;
                }
            }
            for order in oms.get_asks(&asset_id) {
                if order.is_open() {
                    count += 1;
                }
            }
        }
        count
    }

    /// Get orderbook levels for a token
    /// Returns (asks, bids, spread) - asks sorted low to high, bids sorted high to low
    pub fn get_orderbook_levels(&self, token_id: &str) -> (Vec<(f64, f64)>, Vec<(f64, f64)>, Option<f64>) {
        // Find the orderbook for this token
        for obs in self.orderbooks.values() {
            let obs_read = obs.read();
            if let Some(orderbook) = obs_read.get(token_id) {
                let asks: Vec<(f64, f64)> = orderbook.asks.levels().to_vec();
                let bids: Vec<(f64, f64)> = orderbook.bids.levels().to_vec();
                let spread = orderbook.spread();
                return (asks, bids, spread);
            }
        }

        (Vec::new(), Vec::new(), None)
    }

    /// Get our orders for a token as (price, size) tuples
    pub fn get_our_orders_for_token(&self, token_id: &str) -> Vec<(f64, f64)> {
        let oms = self.order_state.read();
        let mut orders = Vec::new();

        // Get bids
        for order in oms.get_bids(token_id) {
            if order.is_open() {
                orders.push((order.price, order.remaining_size()));
            }
        }

        // Get asks
        for order in oms.get_asks(token_id) {
            if order.is_open() {
                orders.push((order.price, order.remaining_size()));
            }
        }

        orders
    }

    /// Get order count for a specific market (both UP and DOWN tokens)
    pub fn get_market_order_count(&self, market: &MarketInfo) -> usize {
        let oms = self.order_state.read();
        let mut count = 0;

        // Count open orders for UP token
        for order in oms.get_bids(&market.up_token_id) {
            if order.is_open() {
                count += 1;
            }
        }
        for order in oms.get_asks(&market.up_token_id) {
            if order.is_open() {
                count += 1;
            }
        }

        // Count open orders for DOWN token (if different from UP)
        if market.down_token_id != market.up_token_id {
            for order in oms.get_bids(&market.down_token_id) {
                if order.is_open() {
                    count += 1;
                }
            }
            for order in oms.get_asks(&market.down_token_id) {
                if order.is_open() {
                    count += 1;
                }
            }
        }

        count
    }

    /// Get order counts for UP and DOWN tokens separately
    pub fn get_market_order_counts(&self, market: &MarketInfo) -> (usize, usize) {
        let oms = self.order_state.read();

        // Count UP token orders
        let mut up_count = 0;
        for order in oms.get_bids(&market.up_token_id) {
            if order.is_open() {
                up_count += 1;
            }
        }
        for order in oms.get_asks(&market.up_token_id) {
            if order.is_open() {
                up_count += 1;
            }
        }

        // Count DOWN token orders
        let mut down_count = 0;
        if market.down_token_id != market.up_token_id {
            for order in oms.get_bids(&market.down_token_id) {
                if order.is_open() {
                    down_count += 1;
                }
            }
            for order in oms.get_asks(&market.down_token_id) {
                if order.is_open() {
                    down_count += 1;
                }
            }
        }

        (up_count, down_count)
    }

    /// Get position sizes for a market (up_size, down_size)
    pub fn get_market_positions(&self, market: &MarketInfo) -> (f64, f64) {
        let tracker = self.position_tracker.read();

        let up_size = tracker
            .get_position(&market.up_token_id)
            .map(|p| p.size)
            .unwrap_or(0.0);

        let down_size = tracker
            .get_position(&market.down_token_id)
            .map(|p| p.size)
            .unwrap_or(0.0);

        (up_size, down_size)
    }

    /// Get full position info including avg entry price
    /// Returns (up_size, up_avg_price, down_size, down_avg_price)
    pub fn get_market_position_details(&self, market: &MarketInfo) -> (f64, f64, f64, f64) {
        let tracker = self.position_tracker.read();

        let (up_size, up_avg) = tracker
            .get_position(&market.up_token_id)
            .map(|p| (p.size, p.avg_entry_price))
            .unwrap_or((0.0, 0.0));

        let (down_size, down_avg) = tracker
            .get_position(&market.down_token_id)
            .map(|p| (p.size, p.avg_entry_price))
            .unwrap_or((0.0, 0.0));

        (up_size, up_avg, down_size, down_avg)
    }

    /// Get position summary string for footer
    pub fn get_position_summary(&self) -> String {
        if let Some(market) = self.get_selected_market() {
            let tracker = self.position_tracker.read();

            let up_pos = tracker.get_position(&market.up_token_id);
            let down_pos = tracker.get_position(&market.down_token_id);

            let up_str = match up_pos {
                Some(p) if p.size.abs() > 0.01 => format!("UP: {:.1} @ {:.4}", p.size, p.avg_entry_price),
                _ => "UP: 0".to_string(),
            };

            let down_str = match down_pos {
                Some(p) if p.size.abs() > 0.01 => format!("DOWN: {:.1} @ {:.4}", p.size, p.avg_entry_price),
                _ => "DOWN: 0".to_string(),
            };

            format!("{} | {}", up_str, down_str)
        } else {
            "No market selected".to_string()
        }
    }

    /// Cancel all open orders
    pub fn cancel_all_orders(&self) {
        let _ = self.runtime.block_on(async {
            self.trading_client.cancel_all().await
        });
        // OMS will update automatically via WebSocket when orders are cancelled
    }

    /// Dump all inventory for the selected market using aggressive FAK orders
    pub fn dump_inventory(&mut self) {
        let Some(market) = self.get_selected_market() else {
            self.status_message = Some("No market selected".to_string());
            return;
        };

        let up_token = market.up_token_id.clone();
        let down_token = market.down_token_id.clone();
        let (up_pos, down_pos) = self.get_market_positions(market);

        if up_pos.abs() < 1.0 && down_pos.abs() < 1.0 {
            self.status_message = Some("Position too small to dump".to_string());
            return;
        }

        let result = self.runtime.block_on(async {
            let mut msgs = Vec::new();

            // Dump UP position (floor to whole number)
            let up_size = up_pos.floor();
            if up_size >= 1.0 {
                match self.trading_client.sell_fak(&up_token, 0.01, up_size).await {
                    Ok(_) => msgs.push(format!("Sold {} UP", up_size as i64)),
                    Err(e) => msgs.push(format!("UP err: {}", e)),
                }
            }

            // Dump DOWN position (floor to whole number)
            let down_size = down_pos.floor();
            if down_size >= 1.0 {
                match self.trading_client.sell_fak(&down_token, 0.01, down_size).await {
                    Ok(_) => msgs.push(format!("Sold {} DOWN", down_size as i64)),
                    Err(e) => msgs.push(format!("DOWN err: {}", e)),
                }
            }

            if msgs.is_empty() {
                "No full shares to dump".to_string()
            } else {
                msgs.join(", ")
            }
        });

        self.status_message = Some(result);
    }

    /// Shutdown the application
    pub fn shutdown(&mut self) {
        info!("[Visualizer] Shutting down...");
        self.shutdown_flag.store(false, Ordering::Release);

        // Shutdown WebSocket clients by draining and consuming them
        let clients = std::mem::take(&mut self.ws_clients);
        for ws_client in clients {
            let _ = self.runtime.block_on(ws_client.shutdown());
        }
    }
}
