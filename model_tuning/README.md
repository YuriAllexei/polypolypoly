# Model Tuning

Market-making quoter model tuning for Polymarket binary markets.

## Installation

```bash
poetry install
```

## Usage

```bash
poetry run model-tuning backtest          # Run a backtest
poetry run model-tuning grid-search       # Run exhaustive parameter grid search
poetry run model-tuning tune --trials 100 # Run parameter optimization (Optuna)
poetry run model-tuning analyze configs/default.yaml
```

---

## Backtesting

The `backtest` command simulates the quoter against market data to evaluate performance.

### How It Works

1. Generates or loads market tick data (oracle prices, bid/ask spreads)
2. At each tick, the quoter generates bid prices for UP and DOWN tokens
3. A fill simulator determines if orders get filled (probabilistic based on edge)
4. Tracks PnL, inventory, and fills throughout the simulation
5. Reports performance metrics at the end

### Running a Backtest

```bash
# Basic backtest with synthetic data (15 min default)
poetry run model-tuning backtest

# With custom parameters from config file
poetry run model-tuning backtest --config configs/default.yaml

# With real market data
poetry run model-tuning backtest --data path/to/market_data.csv

# Shorter/longer duration for synthetic data
poetry run model-tuning backtest --duration 5    # 5 minutes
poetry run model-tuning backtest --duration 30   # 30 minutes

# Verbose output (shows parameter values and sample fills)
poetry run model-tuning backtest --verbose
```

### Backtest Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--config` | `-c` | YAML config file with quoter parameters | None (uses defaults) |
| `--data` | `-d` | CSV file with market data | None (generates synthetic) |
| `--duration` | | Duration in minutes for synthetic data | 15.0 |
| `--seed` | `-s` | Random seed for reproducibility | 42 |
| `--verbose` | `-v` | Show detailed output | False |

### Example Output

```
Running backtest...
       Performance Metrics
┏━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━┓
┃ Metric              ┃   Value ┃
┡━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━┩
│ Total PnL           │ $184.01 │
│ Realized PnL        │ $195.26 │
│ Unrealized PnL      │ $-11.25 │
│ Total Fills         │     158 │
│ UP Fills            │      73 │
│ DOWN Fills          │      85 │
│ Fill Rate           │   43.9% │
│ Avg Spread Captured │    4.4c │
│ Sharpe Ratio        │   64.95 │
│ Max Drawdown        │  $10.26 │
│ Final Imbalance     │   -5.2% │
│ Final Pairs         │    2847 │
│ Avg Combined Cost   │   93.1c │
└─────────────────────┴─────────┘
```

### Config File Format

Create a YAML file with quoter parameters:

```yaml
# configs/my_config.yaml
quoter:
  base_spread: 0.02
  gamma_inv: 0.5
  oracle_sensitivity: 5.0
  edge_threshold: 0.01
  p_informed_base: 0.2
  time_decay_minutes: 5.0
  lambda_size: 1.0
  base_size: 50.0
  min_offset: 0.01
```

### Market Data CSV Format

If using real market data, the CSV must have these columns:

| Column | Description |
|--------|-------------|
| `timestamp` | Time (minutes from start or epoch) |
| `oracle_price` | Oracle price at this tick |
| `threshold` | Market question threshold |
| `best_ask_up` | Best ask price for UP token |
| `best_bid_up` | Best bid price for UP token |
| `best_ask_down` | Best ask price for DOWN token |
| `best_bid_down` | Best bid price for DOWN token |
| `minutes_to_resolution` | Time remaining until resolution |

---

## Grid Search

The `grid-search` command tests all combinations of specified parameter values to find optimal configurations.

### How It Works

1. Define a grid of parameter values to test (e.g., `base_spread: [0.01, 0.02, 0.03]`)
2. Generates all combinations (Cartesian product)
3. Runs a backtest for each combination
4. Ranks results by performance metrics
5. Shows top performers and summary statistics

### Running Grid Search

```bash
# Default grid (108 combinations)
poetry run model-tuning grid-search

# With custom grid configuration
poetry run model-tuning grid-search --config configs/grid_search.yaml

# Save all results to CSV for analysis
poetry run model-tuning grid-search --output results.csv

# Show more/fewer top results
poetry run model-tuning grid-search --top-n 20

# Shorter duration for faster iteration
poetry run model-tuning grid-search --duration 5
```

### Grid Search Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--config` | `-c` | YAML config file with parameter grid | None (uses default grid) |
| `--data` | `-d` | CSV file with market data | None (generates synthetic) |
| `--duration` | | Duration in minutes for synthetic data | 15.0 |
| `--seed` | `-s` | Random seed for reproducibility | 42 |
| `--top-n` | `-n` | Number of top results to display | 10 |
| `--output` | `-o` | Save all results to CSV file | None |

### Grid Config File Format

```yaml
# configs/grid_search.yaml

# Parameters to vary (all combinations tested)
grid:
  base_spread: [0.01, 0.02, 0.03]        # 3 values
  gamma_inv: [0.3, 0.5, 0.7, 1.0]        # 4 values
  oracle_sensitivity: [3.0, 5.0, 10.0]   # 3 values
  edge_threshold: [0.005, 0.01, 0.02]    # 3 values
  # Total: 3 × 4 × 3 × 3 = 108 combinations

# Parameters held constant (not varied)
fixed:
  p_informed_base: 0.2
  time_decay_minutes: 5.0
  lambda_size: 1.0
  base_size: 50.0
  min_offset: 0.01
```

### Example Output

```
Grid Search Configuration
┏━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━┓
┃ Parameter          ┃ Values                     ┃ Count ┃
┡━━━━━━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━━━━━━━━━╇━━━━━━━┩
│ base_spread        │ 0.010, 0.020, 0.030        │     3 │
│ gamma_inv          │ 0.300, 0.500, 0.700, 1.000 │     4 │
│ oracle_sensitivity │ 3.000, 5.000, 10.000       │     3 │
│ edge_threshold     │ 0.005, 0.010, 0.020        │     3 │
└────────────────────┴────────────────────────────┴───────┘

Total combinations: 108

Running grid search... ━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 100% 108/108

Grid Search Complete!
Tested 108 configurations

Top 10 Results by Total PnL:
┏━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━┳━━━━━━━━━┓
┃ base_s… ┃ gamma_… ┃ oracle… ┃ edge_t… ┃ Total   ┃ Fill    ┃        ┃         ┃
┃         ┃         ┃         ┃         ┃ PnL     ┃ Rate    ┃ Sharpe ┃ Imbal.  ┃
┡━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━╇━━━━━━━━━┩
│   0.030 │   0.700 │   3.000 │   0.010 │ $332.83 │   50.0% │  79.81 │   +5.1% │
│   0.030 │   0.700 │  10.000 │   0.010 │ $316.31 │   50.6% │ 156.80 │   +0.4% │
│   0.030 │   0.500 │  10.000 │   0.005 │ $313.51 │   52.2% │ 108.72 │   -5.7% │
│   ...   │   ...   │   ...   │   ...   │   ...   │   ...   │   ...  │   ...   │
└─────────┴─────────┴─────────┴─────────┴─────────┴─────────┴────────┴─────────┘

Summary Statistics:
┏━━━━━━━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━┳━━━━━━━━┳━━━━━━━━━┓
┃ Metric          ┃    Mean ┃    Std ┃    Min ┃     Max ┃
┡━━━━━━━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━╇━━━━━━━━╇━━━━━━━━━┩
│ Total PnL       │ $203.09 │ $62.86 │ $95.58 │ $332.83 │
│ Fill Rate       │   43.6% │   3.5% │  34.7% │   52.2% │
│ Sharpe Ratio    │   86.87 │  36.09 │  26.43 │  173.76 │
│ Max Drawdown    │   $9.86 │  $5.17 │  $2.89 │  $31.16 │
│ Final Imbalance │   -2.6% │   3.9% │ -12.5% │    6.7% │
└─────────────────┴─────────┴────────┴────────┴─────────┘
```

### Analyzing Results

Export to CSV for deeper analysis:

```bash
poetry run model-tuning grid-search --output results.csv
```

The CSV contains all parameters and metrics for each configuration, which you can analyze in pandas, Excel, or any data tool.

---

## Parameter Optimization (Tune)

The `tune` command uses Optuna (TPE sampler) to intelligently search for optimal parameters.

### Grid Search vs Tune

| Aspect | Grid Search | Tune (Optuna) |
|--------|-------------|---------------|
| Search method | Exhaustive (all combinations) | Adaptive (learns from results) |
| Speed | Slower for large grids | Faster for large parameter spaces |
| Coverage | Complete | Sampled |
| Best for | Small grids, understanding parameter effects | Finding optimal params quickly |

### Running Optimization

```bash
# Basic optimization (100 trials)
poetry run model-tuning tune

# More trials for better results
poetry run model-tuning tune --trials 500

# Different optimization objective
poetry run model-tuning tune --objective sharpe
poetry run model-tuning tune --objective risk_adjusted
poetry run model-tuning tune --objective balanced

# Save best parameters to config file
poetry run model-tuning tune --output best_params.yaml
```

### Tune Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--data` | `-d` | CSV file with market data | None (generates synthetic) |
| `--objective` | `-o` | Optimization target | `total_pnl` |
| `--trials` | `-n` | Number of optimization trials | 100 |
| `--duration` | | Duration in minutes for synthetic data | 15.0 |
| `--seed` | `-s` | Random seed for reproducibility | 42 |
| `--output` | | Save best params to YAML file | None |

### Objective Types

| Objective | Description |
|-----------|-------------|
| `total_pnl` | Maximize total profit/loss |
| `sharpe` | Maximize Sharpe ratio (risk-adjusted returns) |
| `risk_adjusted` | Maximize PnL / max_drawdown |
| `balanced` | Weighted combination of PnL, Sharpe, fill rate, and balance |

---

## The 4-Layer Quoter Framework

The quoter uses a 4-layer framework to generate bid prices for UP and DOWN tokens in Polymarket binary markets.

---

### Layer 1: Oracle-Adjusted Offset

Adjusts bid aggressiveness based on oracle price vs threshold.

**Formula:**
```
oracle_adj = distance_pct × oracle_sensitivity
up_offset = base_spread - oracle_adj    (tighter when bullish)
down_offset = base_spread + oracle_adj  (wider when bullish)
```

**Parameter: `oracle_sensitivity`**

Controls how many cents of adjustment per 1% price difference from threshold.

| `oracle_sensitivity` | Meaning |
|---------------------|---------|
| 5.0 | 5c adjustment per 1% price difference |
| 10.0 | 10c adjustment per 1% price difference |

**Example with `oracle_sensitivity = 5.0`:**

| Price vs Threshold | distance_pct | oracle_adj |
|-------------------|--------------|------------|
| +0.1% above | 0.001 | 0.5c |
| +0.5% above | 0.005 | 2.5c |
| +1.0% above | 0.01 | 5c |
| -1.0% below | -0.01 | -5c |

**Effect:**
- When oracle is **bullish** (price > threshold):
  - UP offset **decreases** → tighter bid → more aggressive (want to buy UP)
  - DOWN offset **increases** → wider bid → defensive (protect from dumps)
- When oracle is **bearish** (price < threshold):
  - Opposite effect

---

### Layer 2: Adverse Selection

Widens the base offset as resolution approaches to protect against informed traders.

**Why?** Near resolution, some traders become "informed" - they can see which way the market is going. They dump the losing side on market makers. Wider spreads = pay less when getting dumped on.

**Formula:**
```
p_informed = p_informed_base × exp(-minutes_to_resolution / time_decay_minutes)
base_offset = base_spread × (1 + 3 × p_informed)
```

**Parameters:**

| Parameter | Description | Default |
|-----------|-------------|---------|
| `base_spread` | Starting spread before time adjustment | 0.02 (2c) |
| `p_informed_base` | Base probability of informed trader | 0.2 (20%) |
| `time_decay_minutes` | Time constant for decay | 5.0 min |

**Timeline (with defaults):**

| Minutes to Resolution | p_informed | base_offset |
|----------------------|------------|-------------|
| 14 min (safe) | ~1.2% | 2.07c |
| 7 min (careful) | ~4.9% | 2.29c |
| 3 min (cautious) | ~11% | 2.66c |
| 1 min (DANGER) | ~16% | 2.98c |

**Parameter Effects:**

- **`base_spread`**: Higher = wider spreads overall (more conservative)
- **`p_informed_base`**: Higher = more spread widening near resolution
- **`time_decay_minutes`**:
  - Smaller = spread increases faster as resolution approaches
  - Larger = spread stays flat longer, increases later

**Note:** `p_informed` is capped at 80% to prevent extreme spreads.

**Flow:**
```
Minutes to Resolution → p_informed → base_offset → Layer 1 (oracle adjustment)
```

---

### Layer 3: Inventory Skew

Adjusts **both offsets and sizes** based on inventory imbalance to push toward balanced positions.

**Why?** In binary markets, profit comes from holding PAIRS (1 UP + 1 DOWN = $1 at resolution). Unmatched inventory is risky - if you're overweight UP and DOWN wins, your unmatched UP tokens are worth $0.

**Imbalance Formula:**
```
q = (UP_qty - DOWN_qty) / (UP_qty + DOWN_qty)
```
- `q = +1.0`: 100% UP (extreme overweight UP)
- `q = +0.5`: 75% UP, 25% DOWN
- `q = 0.0`: 50% UP, 50% DOWN (balanced)
- `q = -0.5`: 25% UP, 75% DOWN
- `q = -1.0`: 100% DOWN (extreme overweight DOWN)

**Offset Multiplier Formula:**
```
spread_mult_up = 1 + gamma_inv × q      (>1 when overweight UP → wider offset)
spread_mult_down = 1 - gamma_inv × q    (<1 when overweight UP → tighter offset)

final_offset = raw_offset × spread_mult
```

**Size Formula:**
```
size_up = base_size × exp(-lambda_size × q)    (smaller when overweight UP)
size_down = base_size × exp(+lambda_size × q)  (bigger when overweight UP)
```

**Parameters:**

| Parameter | Description | Default |
|-----------|-------------|---------|
| `gamma_inv` | Offset multiplier sensitivity | 0.5 |
| `lambda_size` | Size adjustment sensitivity | 1.0 |
| `base_size` | Base order size when balanced | 50.0 |

**Example with q = +0.5 (overweight UP):**

| Parameter | UP | DOWN |
|-----------|-----|------|
| spread_mult (γ=0.5) | 1.25 (wider) | 0.75 (tighter) |
| size (λ=1.0, base=50) | 30 (smaller) | 82 (bigger) |

**Effect:** When overweight UP:
- UP offset is **wider** → less aggressive buying UP
- DOWN offset is **tighter** → more aggressive buying DOWN
- UP size is **smaller** → buy less UP
- DOWN size is **bigger** → buy more DOWN
- Result: Inventory rebalances toward 50/50

---

### Layer 4: Edge Check

Gates quotes based on minimum profit margin (edge).

**Why?** Don't place bids too close to the market ask - need sufficient edge to be profitable after fees/slippage.

**Formula:**
```
edge = market_ask - our_bid
if edge < edge_threshold → SKIP (don't quote)
```

**Parameters:**

| Parameter | Description | Default |
|-----------|-------------|---------|
| `edge_threshold` | Minimum edge required to quote | 0.01 (1c) |
| `min_offset` | Floor for offset (from Layer 1) | 0.01 (1c) |

**Example:**
```
market_ask = 0.55
our_bid = 0.53
edge = 0.55 - 0.53 = 0.02 (2c)

If edge_threshold = 0.01 → PASS (2c > 1c, quote is placed)
If edge_threshold = 0.03 → FAIL (2c < 3c, quote is skipped)
```

---

## Parameter Reference

Complete reference for all quoter parameters with ranges and effects.

### Layer 1: Oracle

| Parameter | Range | Default | ↑ Increase | ↓ Decrease |
|-----------|-------|---------|------------|------------|
| `oracle_sensitivity` | 0.0 - 50.0 | 5.0 | More reactive to oracle, larger directional adjustments | Less reactive, offsets stay closer to base |

### Layer 2: Adverse Selection

| Parameter | Range | Default | ↑ Increase | ↓ Decrease |
|-----------|-------|---------|------------|------------|
| `base_spread` | 0.005 - 0.10 | 0.02 | Wider spreads overall, more conservative, fewer fills | Tighter spreads, more aggressive, more fills but more risk |
| `p_informed_base` | 0.0 - 0.8 | 0.2 | More spread widening near resolution | Less time-based widening |
| `time_decay_minutes` | 1.0 - 15.0 | 5.0 | Spread stays flat longer, widens later | Spread widens earlier/faster |

### Layer 3: Inventory Skew

| Parameter | Range | Default | ↑ Increase | ↓ Decrease |
|-----------|-------|---------|------------|------------|
| `gamma_inv` | 0.0 - 3.0 | 0.5 | Stronger offset penalty for imbalanced side | Weaker penalty, offsets less affected by imbalance |
| `lambda_size` | 0.0 - 5.0 | 1.0 | Stronger size reduction for imbalanced side | Weaker size adjustment |
| `base_size` | 1.0 - 500.0 | 50.0 | Larger orders | Smaller orders |

### Layer 4: Edge Check

| Parameter | Range | Default | ↑ Increase | ↓ Decrease |
|-----------|-------|---------|------------|------------|
| `edge_threshold` | 0.001 - 0.05 | 0.01 | More quotes skipped, only high-edge trades | More quotes placed, accepts lower edge |
| `min_offset` | 0.005 - 0.05 | 0.01 | Higher floor on offsets | Lower floor, allows tighter bids |

### Quick Tuning Guide

| Goal | Parameters to Adjust |
|------|---------------------|
| More fills | ↓ base_spread, ↓ edge_threshold, ↓ gamma_inv |
| More conservative | ↑ base_spread, ↑ edge_threshold, ↑ p_informed_base |
| Faster rebalancing | ↑ gamma_inv, ↑ lambda_size |
| More oracle reactive | ↑ oracle_sensitivity |
| Safer near resolution | ↑ p_informed_base, ↓ time_decay_minutes |

---

## Running Tests

```bash
poetry run pytest tests/test_quoter_functions.py -v
```

## Project Structure

```
model_tuning/
├── src/model_tuning/
│   ├── core/           # Quoter logic (models, quoter, utils)
│   ├── tuning/         # Backtester + Optuna optimizer
│   ├── data/           # Data loading
│   └── cli/            # Typer CLI commands
├── tests/              # pytest suite
└── configs/            # YAML configuration
```
