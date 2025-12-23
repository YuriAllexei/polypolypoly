# Market Merger Strategy

Accumulate balanced Up/Down positions at combined cost < $1.00, then merge for guaranteed profit.

## Overview

### Profit Mechanics

Polymarket's Up/Down crypto prediction markets always have exactly two outcomes that sum to $1.00. By accumulating both tokens at a combined average cost below $1.00, we can merge the pair for a guaranteed profit:

```
Profit per pair = $1.00 - (avg_cost_up + avg_cost_down)
```

**Example:**
- Buy 100 Up tokens @ $0.48 average = $48.00 spent
- Buy 100 Down tokens @ $0.47 average = $47.00 spent
- Total spent: $95.00 for 100 pairs
- Merge 100 pairs → receive $100.00
- **Profit: $5.00 (5.26% return)**

### Core Invariants

1. **Profitability Constraint**: `combined_cost < $1.00` always maintained
2. **Self-Trade Prevention (STP)**: `up_bid + down_bid < $1.00` automatically enforced
3. **Balance Target**: Maintain ~50/50 Up/Down positions for maximum merge capacity

## Strategy Logic Flow

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ Market Discovery│────▶│ Quote Calculator │────▶│ Quote Manager   │
│ (strategy.rs)   │     │ (3-level ladder) │     │ (place bids)    │
└─────────────────┘     └──────────────────┘     └─────────────────┘
                                                          │
┌─────────────────┐     ┌──────────────────┐              ▼
│ Merge Checker   │◀────│ Position Tracker │◀──── Fill Events
│ (when balanced) │     │ (avg cost, size) │      (WebSocket)
└─────────────────┘     └──────────────────┘
         │
         ▼
  Merge via CTF Contract
```

## Components

### Types (`types/`)

| File | Type | Purpose |
|------|------|---------|
| `context.rs` | `MarketContext` | Immutable market metadata (token IDs, tick size, end time, asset, timeframe) |
| `state.rs` | `MarketState` | Mutable position tracking (sizes, avg costs, active bids, sizing phase) |
| `state.rs` | `BidInfo` | Tracks placed bid orders (order_id, price, size, level, timestamp) |
| `state.rs` | `SizingPhase` | Bootstrap/Confirmed/Scaled sizing tiers |
| `quote.rs` | `Quote` | Single bid quote (token_id, price, size, level) |
| `quote.rs` | `QuoteLadder` | Complete bid ladder with up_bids and down_bids |
| `quote.rs` | `TakerOpportunity` | Scored opportunity for aggressive fill |

#### MarketContext (`context.rs`)

Immutable metadata parsed from `DbMarket`:

```rust
pub struct MarketContext {
    pub market_id: String,
    pub condition_id: String,      // For CTF merge calls
    pub up_token_id: String,
    pub down_token_id: String,
    pub tick_size: f64,
    pub precision: u8,
    pub market_end_time: DateTime<Utc>,
    pub crypto_asset: CryptoAsset, // BTC, ETH, SOL, etc.
    pub timeframe: Timeframe,      // Daily, Weekly, Monthly
    pub market_question: String,
}
```

#### MarketState (`state.rs`)

Mutable state tracking positions and bids:

```rust
pub struct MarketState {
    // Position tracking
    pub up_size: f64,
    pub up_avg_cost: f64,
    pub down_size: f64,
    pub down_avg_cost: f64,

    // Active bids (level -> BidInfo)
    pub up_bids: HashMap<u8, BidInfo>,
    pub down_bids: HashMap<u8, BidInfo>,

    // Sizing phase
    pub phase: SizingPhase,

    // Metrics
    pub fill_count: u64,
    pub merged_pairs: u64,
    pub realized_profit: f64,
}
```

Key methods:
- `combined_cost()` - Returns `up_avg_cost + down_avg_cost`
- `is_profitable()` - Returns `combined_cost() < 1.0`
- `mergeable_pairs()` - Returns `min(up_size, down_size)`
- `imbalance()` - Returns position imbalance ratio (0.0 = balanced)
- `apply_fill()` - Updates position with volume-weighted average cost
- `record_merge()` - Reduces positions, records profit

#### Quote Types (`quote.rs`)

```rust
pub struct Quote {
    pub token_id: String,
    pub price: f64,
    pub size: f64,
    pub level: u8,  // 0 = best, 1 = mid, 2 = deep
}

pub struct QuoteLadder {
    pub up_bids: Vec<Quote>,    // Up to 3 levels
    pub down_bids: Vec<Quote>,  // Up to 3 levels
}

pub struct TakerOpportunity {
    pub token_id: String,
    pub is_up: bool,
    pub price: f64,
    pub size: f64,
    pub score: f64,           // Higher = better opportunity
    pub reasons: Vec<String>, // Why this opportunity scored well
}
```

### Services (`services/`)

| File | Service | Purpose |
|------|---------|---------|
| `quote_calculator.rs` | `QuoteCalculator` | Calculate bid prices respecting profitability constraint |
| `size_calculator.rs` | `SizeCalculator` | Dynamic sizing based on phase and balance |
| `opportunity_scanner.rs` | `OpportunityScanner` | Score-based taker opportunity detection |
| `merge_checker.rs` | `MergeChecker` | Decide when to merge accumulated positions |

#### QuoteCalculator (`quote_calculator.rs`)

Calculates bid prices for a 3-level ladder:

```rust
impl QuoteCalculator {
    pub fn calculate_bids(
        &self,
        ctx: &MarketContext,
        state: &MarketState,
        up_ob: &Orderbook,
        down_ob: &Orderbook,
    ) -> QuoteLadder
}
```

**Pricing Logic:**
1. **Level 0 (best)**: `best_bid - tick_size` (top of book)
2. **Level 1 (mid)**: `best_bid - (2 * tick_size)`
3. **Level 2 (deep)**: `best_bid - (3 * tick_size)` (for size)

**Constraints Applied:**
- Each bid respects: `up_bid + down_best_bid < max_combined_cost`
- Skews bids toward underweight side when position is imbalanced
- Clamps prices to `[min_bid_price, max_bid_price]` from config

#### SizeCalculator (`size_calculator.rs`)

Manages dynamic sizing phases:

```rust
pub enum SizingPhase {
    Bootstrap,  // position < $100, use 1% sizes
    Confirmed,  // $100-500, use 3% sizes
    Scaled,     // > $500, use 5% sizes
}

impl SizeCalculator {
    pub fn update_phase(&self, state: &mut MarketState);
    pub fn calculate_sizes(
        &self,
        state: &MarketState,
        balance: f64,
        ladder: &mut QuoteLadder,
    );
}
```

**Size Distribution:**
- Level 0: 50% of phase allocation
- Level 1: 30% of phase allocation
- Level 2: 20% of phase allocation

#### OpportunityScanner (`opportunity_scanner.rs`)

Scores potential taker opportunities using multiple factors:

```rust
impl OpportunityScanner {
    pub fn scan(
        &self,
        ctx: &MarketContext,
        state: &MarketState,
        up_ob: &Orderbook,
        down_ob: &Orderbook,
    ) -> Option<TakerOpportunity>
}
```

**Scoring Factors (configurable weights):**

| Factor | Weight | Description |
|--------|--------|-------------|
| Profit Margin | 0.4 | How much below $1.00 combined cost would be |
| Price vs Bid | 0.2 | Discount from our current best bid |
| Delta Coverage | 0.2 | How much it reduces position imbalance |
| Avg Improvement | 0.2 | Improvement to average cost |

Only returns opportunities above `min_opportunity_score` threshold.

#### MergeChecker (`merge_checker.rs`)

Decides when to merge accumulated positions:

```rust
pub struct MergeDecision {
    pub should_merge: bool,
    pub pairs: u64,
    pub expected_profit: f64,
    pub reasons: Vec<String>,
}

impl MergeChecker {
    pub fn should_merge(&self, state: &MarketState) -> MergeDecision;
}
```

**Merge Conditions (all must be true):**
1. `mergeable_pairs >= min_merge_pairs` (default: 10)
2. `imbalance <= max_merge_imbalance` (default: 0.2)
3. `|up_avg_cost - down_avg_cost| <= merge_cost_spread_max` (default: 0.05)
4. `profit_per_pair >= merge_profit_threshold` (default: 0.02)

### Tracker (`tracker/`)

| File | Component | Purpose |
|------|-----------|---------|
| `market_tracker.rs` | `run_accumulator` | Main loop for single market |
| `market_tracker.rs` | `AccumulatorContext` | Shared context for accumulator |
| `quote_manager.rs` | `QuoteManager` | Bid placement, cancellation, staleness |

#### AccumulatorContext (`market_tracker.rs`)

Shared context passed to each market's accumulator:

```rust
pub struct AccumulatorContext {
    pub shutdown_flag: Arc<AtomicBool>,
    pub trading: Arc<TradingClient>,
    pub balance_manager: Arc<RwLock<BalanceManager>>,
    pub order_state: Arc<tokio::sync::RwLock<OrderStateStore>>,
}
```

#### run_accumulator (`market_tracker.rs`)

Main accumulation loop per market:

```rust
pub async fn run_accumulator(
    market: DbMarket,
    config: MarketMergerConfig,
    ctx: AccumulatorContext,
) -> anyhow::Result<()>
```

**Loop Steps:**
1. Check shutdown flag
2. Check trading halt (cancel all bids if halted)
3. Get orderbooks (TODO: WebSocket integration)
4. Sync positions from fill events
5. Update sizing phase
6. Check merge conditions → execute if ready
7. Scan for taker opportunities → execute if found
8. Refresh quote ladder periodically

#### QuoteManager (`quote_manager.rs`)

Manages bid lifecycle:

```rust
impl QuoteManager {
    pub async fn update_bids(
        &self,
        ctx: &MarketContext,
        state: &mut MarketState,
        ladder: &QuoteLadder,
    ) -> anyhow::Result<()>;

    pub async fn cancel_all(
        &self,
        state: &mut MarketState,
    ) -> anyhow::Result<()>;
}
```

**Bid Management:**
- Cancels bids if price changed > `price_tolerance` (default: 0.005)
- Cancels bids older than `max_bid_age_secs` (default: 30s)
- Places new bids for missing levels
- Tracks order IDs in state for cancellation

### Strategy (`strategy.rs`)

Implements the `Strategy` trait:

```rust
pub struct MarketMergerStrategy {
    config: MarketMergerConfig,
    tracked_market_ids: HashSet<String>,
    active_markets: Vec<TrackedMarket>,
    accumulator_tasks: HashMap<String, JoinHandle<()>>,
    order_state: Arc<RwLock<OrderStateStore>>,
}

#[async_trait]
impl Strategy for MarketMergerStrategy {
    fn name(&self) -> &str { "market_merger" }
    async fn initialize(&mut self, ctx: &StrategyContext) -> StrategyResult<()>;
    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()>;
    async fn stop(&mut self) -> StrategyResult<()>;
}
```

**Strategy Loop:**
1. Fetch markets with tags: `["Up or Down", "Crypto Prices", "Recurring"]`
2. Filter by configured assets and timeframes
3. Spawn accumulator task per eligible market
4. Clean up completed/ended tasks
5. Repeat at `poll_interval_secs`

## Configuration

All parameters in `MarketMergerConfig`:

```rust
pub struct MarketMergerConfig {
    // Market selection
    pub assets: Vec<String>,           // ["BTC", "ETH", "SOL"]
    pub timeframes: Vec<String>,       // ["Daily", "Weekly"]
    pub poll_interval_secs: f64,       // 60.0

    // Quote ladder
    pub bid_levels: u8,                // 3
    pub level_spacing_ticks: u8,       // 1
    pub min_bid_price: f64,            // 0.15
    pub max_bid_price: f64,            // 0.50
    pub max_combined_cost: f64,        // 0.98
    pub quote_refresh_ms: u64,         // 1000

    // Dynamic sizing
    pub bootstrap_threshold: f64,      // 100.0
    pub confirmed_threshold: f64,      // 500.0
    pub bootstrap_size_pct: f64,       // 0.01
    pub confirmed_size_pct: f64,       // 0.03
    pub scaled_size_pct: f64,          // 0.05

    // Opportunity-based taker
    pub min_opportunity_score: f64,    // 0.6
    pub profit_margin_weight: f64,     // 0.4
    pub price_vs_bid_weight: f64,      // 0.2
    pub delta_coverage_weight: f64,    // 0.2
    pub avg_improvement_weight: f64,   // 0.2

    // Merge conditions
    pub min_merge_pairs: u64,          // 10
    pub max_merge_imbalance: f64,      // 0.2
    pub merge_cost_spread_max: f64,    // 0.05
    pub merge_profit_threshold: f64,   // 0.02
}
```

## TODOs / Integration Points

### 1. WebSocket Orderbook Integration

`market_tracker.rs:get_orderbooks()` currently returns empty orderbooks. Needs integration with the orderbook WebSocket manager:

```rust
// TODO: Replace placeholder with actual WebSocket connection
async fn get_orderbooks(ctx: &MarketContext) -> anyhow::Result<(Orderbook, Orderbook)> {
    // Connect to orderbook WebSocket and get live data
}
```

### 2. Merge Execution

The merge check identifies when to merge, but execution needs CTF contract interaction:

```rust
// In market_tracker.rs
if merge_decision.should_merge {
    // TODO: User implements merge execution
    // execute_merge(&market_ctx, merge_decision.pairs).await?;
}
```

Merge requires calling the CTF `redeemPositions` function with the `condition_id`.

### 3. Position Hydration

On restart, positions should be loaded from Polymarket Data API:

```rust
// TODO: Hydrate positions from Polymarket Data API (for restart recovery)
// hydrate_positions_from_api(&mut state, &market_ctx).await?;
```

### 4. Fill Event Processing

`sync_positions_from_fills()` reads from `OrderStateStore`. Ensure the WebSocket fill handler writes fills there:

```rust
// Fill events should call:
order_state.write().await.record_fill(fill);
```

## Usage

### Running the Strategy

1. Configure in `config/strategies.toml` or equivalent:

```toml
[market_merger]
assets = ["BTC", "ETH"]
timeframes = ["Daily"]
max_combined_cost = 0.97  # 3% profit margin
```

2. Select strategy type when starting:

```rust
let strategy = StrategyType::MarketMerger;
```

### Monitoring

Key metrics to watch:
- `state.combined_cost()` - Should stay below $0.98
- `state.imbalance()` - Should stay below 0.2
- `state.realized_profit` - Cumulative profit from merges
- Active bid count per market

### Safety Features

- **Trading Halt**: If `BalanceManager::is_halted()`, all bids are canceled
- **Shutdown**: Clean cancellation of all bids on shutdown
- **STP Enforcement**: Config's `max_combined_cost` prevents self-trade
- **Stale Bid Cleanup**: Bids older than 30s are refreshed
