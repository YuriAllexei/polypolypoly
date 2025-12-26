# Inventory MM Strategy

Inventory-balanced market making for Up/Down binary markets on Polymarket.

## Overview

This strategy accumulates matched pairs of Up and Down tokens at a combined average cost below $1.00, then merges them for profit. Polymarket binary markets have a fixed payout of $1.00 when Up + Down tokens are merged.

### Profit Formula

```
profit_per_pair = $1.00 - avg_up_price - avg_down_price
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────────────┐   │
│  │ Data Sources│     │   Solver    │     │      Executor       │   │
│  │ (SharedState)────▶│ (Pure Func) │────▶│   (Own Thread)      │   │
│  └─────────────┘     └─────────────┘     └─────────────────────┘   │
│        │                                          │                │
│        ▼                                          ▼                │
│  ┌─────────────┐                         ┌─────────────────────┐   │
│  │   Merger    │                         │   TradingClient     │   │
│  │ (Component) │                         │   (Order Exec)      │   │
│  └─────────────┘                         └─────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

## Components

### Solver (`components/solver/`)

Pure function that takes raw input types and returns actions. No side effects, fully testable.

| File | Purpose |
|------|---------|
| `core.rs` | Main `solve()` function |
| `quotes.rs` | Quote ladder calculation based on imbalance |
| `diff.rs` | Order diffing (cancel/place decisions) |
| `taker.rs` | Taker opportunity detection for rebalancing |
| `profitability.rs` | Validates that quotes maintain profitability |

**Key behavior:**
- When `delta = 0` (balanced): Aggressive on both sides
- When `delta > 0` (heavy Up): Passive on Up, aggressive on Down
- When `delta < 0` (heavy Down): Aggressive on Up, passive on Down

### Executor (`components/executor/`)

Runs on its own thread, receives commands via channel. Doesn't depend on the solver loop.

| File | Purpose |
|------|---------|
| `executor.rs` | Thread management and command processing |
| `commands.rs` | Command types (`ExecuteBatch`, `CancelOrders`, etc.) |

**Key types:**
- `ExecutorHandle` - Send commands to executor
- `ExecutorCommand` - Commands sent via channel
- `ExecutorResult` - Results from command processing

### Merger (`components/merger/`)

Separate component that monitors inventory and triggers merges.

| File | Purpose |
|------|---------|
| `merger.rs` | Merge decision logic and execution |

**Merge criteria:**
1. `pairs_available >= min_merge_size`
2. `|imbalance| <= max_merge_imbalance`
3. `combined_avg_cost < max_combined_cost`

### Types (`types/`)

| File | Purpose |
|------|---------|
| `input.rs` | `SolverInput`, `InventorySnapshot`, `OrderbookSnapshot`, `SolverConfig` |
| `output.rs` | `SolverOutput`, `ExecutorCommand` |
| `order.rs` | `Quote`, `LimitOrder`, `TakerOrder`, `QuoteLadder`, `Side` |

## Configuration

```rust
let config = InventoryMMConfig::new(up_token_id, down_token_id, condition_id)
    .with_num_levels(3)           // Bids per side
    .with_tick_size(0.01)         // Price increment
    .with_base_offset(0.01)       // Offset from best ask when balanced
    .with_min_profit_margin(0.01) // Minimum profit per pair ($0.01)
    .with_max_imbalance(0.8)      // Stop quoting at 80% imbalance
    .with_order_size(100.0)       // Size per order
    .with_min_merge_size(10.0)    // Minimum pairs before merge
    .with_update_interval_ms(100);// Solver tick interval
```

## Usage

```rust
use crate::application::strategies::inventory_mm::{
    InventoryMMConfig,
    InventoryMMStrategy,
    SolverInput,
};

// 1. Create config
let config = InventoryMMConfig::new(up_token_id, down_token_id, condition_id)
    .with_num_levels(3)
    .with_base_offset(0.01);

// 2. Initialize strategy (spawns executor, creates merger)
let mut strategy = InventoryMMStrategy::new(config.clone());
strategy.initialize();

// 3. Main loop
loop {
    // TODO: Replace with actual data extraction when integrated
    let input = extract_solver_input(&config);
    strategy.tick(&input);
    sleep(Duration::from_millis(100));
}

// 4. Shutdown
strategy.shutdown();
```

## Imbalance Calculation

```rust
imbalance = (up_size - down_size) / (up_size + down_size)
// Range: -1.0 (all Down) to +1.0 (all Up)
```

| Imbalance | Meaning | Up Quote | Down Quote |
|-----------|---------|----------|------------|
| -1.0 | All Down | Aggressive | Passive |
| 0.0 | Balanced | Aggressive | Aggressive |
| +1.0 | All Up | Passive | Aggressive |

## Order Diffing

The solver compares current open orders with desired quotes:
- If an order matches a desired quote (price and size), keep it
- If an order doesn't match any quote, cancel it
- If a quote has no matching order, place a new order

This reduces order churn and exchange load.

## Taker Logic

When inventory is imbalanced, the solver looks for taker opportunities to rebalance:

```rust
// When heavy on Up (delta > 0.1), look to take Down liquidity
if delta > 0.1 && !down_ob.best_ask_is_ours {
    // Calculate new avg if we take
    // Only take if combined cost stays profitable
}
```

**Safeguards:**
- Requires existing position on opposite side (to calculate profitability)
- Uses actual take size (capped at `order_size`) in VWAP calculation
- Prevents self-trading via `best_ask_is_ours` check

## Integration Points

| Component | Type | Usage |
|-----------|------|-------|
| PositionTracker | `Arc<RwLock<PositionTracker>>` | Get inventory and avg prices |
| OrderStateStore | `Arc<RwLock<OrderStateStore>>` | Get open orders |
| Orderbook | `Arc<RwLock<HashMap<String, Orderbook>>>` | Get best bid/ask |
| TradingClient | Direct | Execute orders |

## Current Status

### Implemented (Scaffold Complete)
- Solver pure function with quote calculation, diffing, taker detection, profitability validation
- Executor on separate thread with channel-based command processing
- Merger component with configurable merge criteria
- All types and configuration
- **37 unit tests passing**

### TODO (Integration Required)
1. **`extract_solver_input()`** - Currently a stub returning defaults. Needs to read from:
   - `SharedPositionTracker` → `InventorySnapshot`
   - `SharedOrderState` → `OrderSnapshot`
   - `SharedOrderbooks` → `OrderbookSnapshot`

2. **Executor ↔ TradingClient** - Currently logs simulated actions. Needs to call:
   - `trading_client.buy()` / `trading_client.sell()`
   - `trading_client.cancel_orders()`

3. **Merger ↔ CLOB API** - Currently placeholder. Needs to call:
   - Polymarket merge API for actual merge execution

## TBD Items (Awaiting Clarification)

1. **Cancel/Replace Atomicity** - Should be atomic like a git merge
2. **Orderbook Triggers** - Which microstructure changes should trigger re-solve

## File Structure

```
inventory_mm/
├── mod.rs
├── config.rs
├── strategy.rs
├── README.md
├── components/
│   ├── mod.rs
│   ├── solver/
│   │   ├── mod.rs
│   │   ├── core.rs
│   │   ├── quotes.rs
│   │   ├── diff.rs
│   │   ├── taker.rs
│   │   └── profitability.rs
│   ├── executor/
│   │   ├── mod.rs
│   │   ├── executor.rs
│   │   └── commands.rs
│   └── merger/
│       ├── mod.rs
│       └── merger.rs
└── types/
    ├── mod.rs
    ├── input.rs
    ├── output.rs
    └── order.rs
```
