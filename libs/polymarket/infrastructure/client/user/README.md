# User Order Manager

Thread-safe order and trade state management for Polymarket's User WebSocket channel.

## Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      SharedOrderState                            │
│                   Arc<RwLock<OrderStateStore>>                   │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │  AssetOrderBook │  │  AssetOrderBook │  │  AssetOrderBook │  │
│  │    (asset-1)    │  │    (asset-2)    │  │    (asset-N)    │  │
│  ├─────────────────┤  ├─────────────────┤  ├─────────────────┤  │
│  │ Bids: HashMap   │  │ Bids: HashMap   │  │ Bids: HashMap   │  │
│  │ Asks: HashMap   │  │ Asks: HashMap   │  │ Asks: HashMap   │  │
│  │ Fills: Vec      │  │ Fills: Vec      │  │ Fills: Vec      │  │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│  order_to_asset: HashMap<OrderId, AssetId>   (O(1) lookup)      │
│  seen_trade_ids: HashSet<TradeId>            (deduplication)    │
│  callback: Arc<dyn OrderEventCallback>       (event dispatch)   │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use polymarket::infrastructure::client::user::*;

// 1. Start with REST hydration (recommended)
let state = spawn_user_order_tracker(
    shutdown_flag,
    &rest_client,
    &auth,
    None, // or Some(callback)
).await?;

// 2. Query orders
let open_orders = state.read().get_open_orders("asset-123");
let bids = state.read().get_bids("asset-123");
let asks = state.read().get_asks("asset-123");

// 3. Query by order ID
if let Some(order) = state.read().get_order("order-abc") {
    println!("Status: {}, Remaining: {}", order.status, order.remaining_size());
}
```

## Features

| Feature | Description |
|---------|-------------|
| **Bid/Ask Separation** | Orders stored in separate maps per side for efficient querying |
| **Dual Indexing** | O(1) lookup by both `order_id` and `asset_id` |
| **Trade Deduplication** | Automatic rejection of duplicate trade messages |
| **REST Hydration** | Bootstrap state from REST API before WebSocket connection |
| **Memory Management** | Configurable pruning of completed orders and old trades |
| **Callback System** | Real-time notifications fired outside lock scope |
| **Thread-Safe** | `parking_lot::RwLock` for concurrent read access |

## Types

### Enums

```rust
enum Side { Buy, Sell }
enum OrderStatus { Open, PartiallyFilled, Filled, Cancelled }
enum TradeStatus { Matched, Mined, Confirmed, Retrying, Failed }
enum OrderType { GTC, FOK, GTD, FAK }
```

### Order

```rust
struct Order {
    order_id: String,
    asset_id: String,
    market: String,
    side: Side,
    outcome: String,          // "YES" or "NO"
    price: f64,
    original_size: f64,
    size_matched: f64,
    status: OrderStatus,
    order_type: OrderType,
    maker_address: String,
    owner: String,
    associate_trades: Vec<String>,
    created_at: String,
    expiration: String,       // Unix timestamp, "0" = no expiration
    timestamp: String,
}

// Helper methods
order.remaining_size()        // original_size - size_matched
order.is_open()               // Open or PartiallyFilled
order.is_expired(now_unix)    // Check against expiration
```

### Fill

```rust
struct Fill {
    trade_id: String,
    asset_id: String,
    market: String,
    side: Side,
    outcome: String,
    price: f64,
    size: f64,
    status: TradeStatus,
    taker_order_id: String,
    trader_side: String,      // "TAKER" or "MAKER"
    fee_rate_bps: f64,
    transaction_hash: Option<String>,
    maker_orders: Vec<MakerOrderInfo>,
    match_time: String,
    timestamp: String,
    owner: String,
}
```

## API Reference

### Query Methods

```rust
// Per-asset queries
state.read().get_bids(asset_id)         -> Vec<Order>
state.read().get_asks(asset_id)         -> Vec<Order>
state.read().get_fills(asset_id)        -> Vec<Fill>
state.read().get_open_orders(asset_id)  -> Vec<Order>

// By order ID
state.read().get_order(order_id)        -> Option<Order>

// Aggregations
state.read().total_bid_size(asset_id)   -> f64
state.read().total_ask_size(asset_id)   -> f64
state.read().total_fill_volume(asset_id) -> f64

// Global stats
state.read().order_count()              -> usize
state.read().fill_count()               -> usize
state.read().asset_count()              -> usize
state.read().asset_ids()                -> Vec<String>
```

### Memory Management

```rust
// Prune completed orders (keeps N most recent per asset)
state.write().prune_completed_orders(keep_last_n);

// Prune old trades (keeps N most recent per asset)
state.write().prune_old_trades(keep_last_n);
```

Trade deduplication is automatically capped at 10,000 entries (FIFO eviction).

## Callbacks

Implement `OrderEventCallback` for real-time notifications:

```rust
struct MyStrategy {
    tx: mpsc::Sender<OrderEvent>,
}

impl OrderEventCallback for MyStrategy {
    fn on_order_placed(&self, order: &Order) {
        let _ = self.tx.try_send(OrderEvent::Placed(order.clone()));
    }

    fn on_order_updated(&self, order: &Order) {
        let _ = self.tx.try_send(OrderEvent::Updated(order.clone()));
    }

    fn on_order_filled(&self, order: &Order) {
        let _ = self.tx.try_send(OrderEvent::Filled(order.clone()));
    }

    fn on_order_cancelled(&self, order: &Order) {
        let _ = self.tx.try_send(OrderEvent::Cancelled(order.clone()));
    }

    fn on_trade(&self, fill: &Fill) {
        let _ = self.tx.try_send(OrderEvent::Trade(fill.clone()));
    }
}

// Usage
let callback = Arc::new(MyStrategy { tx });
let state = spawn_user_order_tracker(shutdown, &rest, &auth, Some(callback)).await?;
```

### Callback Safety

- Callbacks are fired **outside** the lock scope (no deadlock risk)
- Safe to read from `SharedOrderState` within callbacks
- Avoid write operations or blocking I/O in callbacks
- For expensive work, queue to a background task

## Thread Safety

```rust
// Multiple readers allowed
let bids = state.read().get_bids("asset-1");
let asks = state.read().get_asks("asset-1");

// Single writer blocks readers
state.write().prune_completed_orders(100);
```

The WebSocket handler holds the write lock only during `process_order()` / `process_trade()`, then releases before firing callbacks.

## Module Structure

```
client/user/
├── mod.rs           # Public exports
├── order_manager.rs # State management (this module)
├── user_ws.rs       # WebSocket client & handler
├── types.rs         # Message types (OrderMessage, TradeMessage)
└── README.md        # This file
```

## Validation

Invalid messages are silently rejected:
- Empty `order_id` or `asset_id`
- `original_size <= 0` for orders
- `size <= 0` for trades
- Duplicate trade IDs
