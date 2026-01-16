# Real Data Simulator

The Real Data Simulator replays historical Polymarket orderbook data and fills against your quoter to evaluate performance with actual market conditions.

## Overview

Unlike the synthetic backtester which generates random data, this simulator uses **real market data**:
- Actual orderbook snapshots over time
- Real fills that occurred in the market
- Historical oracle prices

This allows you to test how your quoter would have performed in real market conditions.

## How It Works

1. Load timestamped orderbook snapshots, fills, and oracle prices
2. For each orderbook snapshot at time T:
   - Generate quotes using your `InventoryMMQuoter`
   - Find fills that occurred between T and T+1
   - Match SELL fills against your bids (if sell price ≤ your bid)
   - Update inventory with matched fills
3. Track position state (inventory, avg costs, pairs) over time

### Fill Matching Logic

A fill matches your quote if:
- The fill is a **SELL** (someone selling into your bid)
- The fill's `outcome` matches your quote side (up/down)
- The fill `price <= your_bid_price` (they sold at or below your bid)
- You have remaining quote size available

**Assumption:** You are always first in queue at your price level.

---

## Data Formats

### Orderbooks JSON

Array of orderbook snapshots, each containing UP and DOWN books with timestamp:

```json
[
  {
    "up": {
      "asks": [{"price": 0.56, "size": 100}, {"price": 0.57, "size": 200}],
      "bids": [{"price": 0.54, "size": 150}, {"price": 0.53, "size": 300}]
    },
    "down": {
      "asks": [{"price": 0.46, "size": 100}, {"price": 0.47, "size": 200}],
      "bids": [{"price": 0.44, "size": 150}, {"price": 0.43, "size": 300}]
    },
    "timestamp": 1704067200.0
  },
  {
    "up": { ... },
    "down": { ... },
    "timestamp": 1704067260.0
  }
]
```

**Fields:**
- `up.asks` / `up.bids`: UP token orderbook levels
- `down.asks` / `down.bids`: DOWN token orderbook levels
- `price`: Price at this level (0.01 to 0.99)
- `size`: Total size at this level
- `timestamp`: Unix timestamp (seconds)

### Fills JSON

Array of trade fills that occurred in the market:

```json
[
  {
    "price": 0.55,
    "size": 100,
    "side": "buy",
    "timestamp": 1704067230.0,
    "outcome": "up"
  },
  {
    "price": 0.44,
    "size": 50,
    "side": "buy",
    "timestamp": 1704067245.0,
    "outcome": "down"
  }
]
```

**Fields:**
- `price`: Fill price
- `size`: Fill size
- `side`: `"buy"` or `"sell"` (only `"buy"` fills match your bids)
- `timestamp`: Unix timestamp
- `outcome`: `"up"` or `"down"` (which market the fill is for)

### Oracle JSON

Array of oracle price snapshots:

```json
[
  {
    "price": 97500.0,
    "threshold": 98000.0,
    "timestamp": 1704067200.0
  },
  {
    "price": 97650.0,
    "threshold": 98000.0,
    "timestamp": 1704067260.0
  }
]
```

**Fields:**
- `price`: Current oracle price (e.g., BTC price from exchange)
- `threshold`: Market question threshold (e.g., "Will BTC be above $98,000?")
- `timestamp`: Unix timestamp

---

## CLI Usage

### Basic Usage

```bash
poetry run model-tuning simulate \
  --orderbooks data/orderbooks.json \
  --fills data/fills.json \
  --oracle data/oracle.json
```

### With Custom Quoter Config

```bash
poetry run model-tuning simulate \
  --orderbooks data/orderbooks.json \
  --fills data/fills.json \
  --oracle data/oracle.json \
  --config configs/aggressive.yaml
```

### Export Position History

```bash
poetry run model-tuning simulate \
  --orderbooks data/orderbooks.json \
  --fills data/fills.json \
  --oracle data/oracle.json \
  --output position_history.csv
```

### Verbose Output (Show Fills)

```bash
poetry run model-tuning simulate \
  --orderbooks data/orderbooks.json \
  --fills data/fills.json \
  --oracle data/oracle.json \
  --verbose
```

### All Options

```
Options:
  -o, --orderbooks PATH   Path to orderbooks JSON file [required]
  -f, --fills PATH        Path to fills JSON file [required]
  -r, --oracle PATH       Path to oracle JSON file [required]
  -c, --config PATH       Path to quoter config YAML
  --resolution FLOAT      Resolution timestamp (Unix)
  --output PATH           Save position history to CSV
  -v, --verbose           Verbose output (show sample fills)
```

---

## Python API Usage

For use in notebooks or scripts:

```python
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.simulation import (
    RealDataSimulator,
    load_simulation_data,
)

# Load data from files
orderbooks, fills, oracle = load_simulation_data(
    "data/orderbooks.json",
    "data/fills.json",
    "data/oracle.json",
)

# Create quoter with custom params
params = QuoterParams(
    base_spread=0.02,
    gamma_inv=0.5,
    oracle_sensitivity=5.0,
)
quoter = InventoryMMQuoter(params)

# Run simulation
simulator = RealDataSimulator()
result = simulator.run(
    quoter=quoter,
    orderbooks=orderbooks,
    fills=fills,
    oracle=oracle,
)

# Access results
print(f"Total fills: {result.total_fills}")
print(f"UP fills: {result.up_fills}")
print(f"DOWN fills: {result.down_fills}")
print(f"Final inventory: {result.final_inventory}")
print(f"Potential PnL: ${result.final_pnl_potential:.2f}")
```

### Working with Results

```python
# Access final inventory
inv = result.final_inventory
print(f"UP: {inv.up_qty:.1f} @ {inv.up_avg:.3f}")
print(f"DOWN: {inv.down_qty:.1f} @ {inv.down_avg:.3f}")
print(f"Pairs: {inv.pairs:.1f}")
print(f"Combined avg: {inv.combined_avg:.3f}")
print(f"Potential profit/pair: ${inv.potential_profit:.4f}")

# Access position history (for plotting)
import pandas as pd
df = pd.DataFrame([ps.model_dump() for ps in result.position_history])
df.plot(x="timestamp", y=["up_qty", "down_qty"])

# Access matched fills
for mf in result.matched_fills[:5]:
    print(f"{mf.outcome.upper()} {mf.size:.1f} @ {mf.price:.3f}")
```

### Creating Data Programmatically

```python
from model_tuning.simulation import (
    OrderbookSnapshot,
    Orderbook,
    OrderbookLevel,
    RealFill,
    OracleSnapshot,
)

# Create orderbook snapshots
orderbooks = [
    OrderbookSnapshot(
        up=Orderbook(
            asks=[OrderbookLevel(price=0.56, size=100)],
            bids=[OrderbookLevel(price=0.54, size=100)],
        ),
        down=Orderbook(
            asks=[OrderbookLevel(price=0.46, size=100)],
            bids=[OrderbookLevel(price=0.44, size=100)],
        ),
        timestamp=1000.0,
    ),
]

# Create fills
fills = [
    RealFill(price=0.55, size=50, side="buy", timestamp=1030.0, outcome="up"),
]

# Create oracle
oracle = [
    OracleSnapshot(price=97500.0, threshold=98000.0, timestamp=1000.0),
]

# Run simulation
result = simulator.run(quoter, orderbooks, fills, oracle)
```

---

## Output Interpretation

### Simulation Results Table

| Metric | Description |
|--------|-------------|
| **UP Position** | Quantity and average cost of UP tokens |
| **DOWN Position** | Quantity and average cost of DOWN tokens |
| **Pairs** | `min(up_qty, down_qty)` - redeemable pairs |
| **Combined Avg** | `up_avg + down_avg` - cost per pair |
| **Potential Profit/Pair** | `$1.00 - combined_avg` |
| **Total Potential PnL** | `pairs × potential_profit` |
| **Total Fills** | Number of fills that matched your quotes |
| **UP/DOWN Fills** | Breakdown by outcome |
| **Total Volume** | Total size filled |
| **Imbalance** | `(up - down) / (up + down)` from -100% to +100% |

### Understanding Profitability

In Polymarket binary markets:
- At resolution: UP + DOWN = $1.00
- Your profit = `$1.00 - (avg_cost_up + avg_cost_down)` per pair
- **Green** combined avg (< $1.00) = profitable
- **Red** combined avg (> $1.00) = underwater

### Position History CSV

When using `--output`, the CSV contains:

| Column | Description |
|--------|-------------|
| `timestamp` | Unix timestamp |
| `up_qty` | UP token quantity |
| `down_qty` | DOWN token quantity |
| `up_avg` | UP average cost |
| `down_avg` | DOWN average cost |
| `pairs` | Redeemable pairs |
| `combined_avg` | Cost per pair |
| `potential_profit` | Profit per pair |

---

## Example: Full Workflow

```bash
# 1. Prepare your data files
#    - orderbooks.json: from your orderbook cache
#    - fills.json: from trade history
#    - oracle.json: from price feed cache

# 2. Run simulation with default params
poetry run model-tuning simulate \
  -o data/orderbooks.json \
  -f data/fills.json \
  -r data/oracle.json \
  -v

# 3. Try different quoter configs
poetry run model-tuning simulate \
  -o data/orderbooks.json \
  -f data/fills.json \
  -r data/oracle.json \
  -c configs/conservative.yaml

# 4. Export for analysis
poetry run model-tuning simulate \
  -o data/orderbooks.json \
  -f data/fills.json \
  -r data/oracle.json \
  --output results/position_history.csv
```

---

## Differences from Synthetic Backtester

| Feature | `backtest` (synthetic) | `simulate` (real data) |
|---------|------------------------|------------------------|
| Data source | Generated randomly | Real market data |
| Fill simulation | Probabilistic model | Deterministic matching |
| Orderbook | Single best bid/ask | Full depth |
| Queue priority | Random | First in queue |
| Use case | Parameter exploration | Real performance validation |
