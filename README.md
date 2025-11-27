# Polymarket Trading Bot - Clean Architecture

A high-performance Rust bot for Polymarket built with **Clean Architecture** principles, featuring market monitoring, event synchronization, and real-time trading capabilities.

## 🏗️ Architecture

This project follows **Clean Architecture** (Arquitectura Limpia) with strict layer separation:

```
┌─────────────────────┐
│   Presentation      │  bin/ (market_sniper, polymarket_events)
│   (Binaries)        │  src/ (bin_common utilities)
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   Application       │  libs/polymarket/application/
│   (Use Cases)       │  - Facades: SniperApp, EventSyncApp
└──────┬──────────────┘  - Services: Strategy, Filter, Sync
       │
       ▼
┌─────────────────────┐      ┌─────────────────────┐
│  Infrastructure     │◄─────│     Domain          │
│  (External I/O)     │      │  (Business Logic)   │
│  - Database         │      │  - Entities         │
│  - API Clients      │      │  - Errors           │
│  - Config           │      │  - Pure Logic       │
└─────────────────────┘      └─────────────────────┘
```

📖 **See detailed documentation:**
- [ARCHITECTURE.md](./ARCHITECTURE.md) - Complete project architecture
- [BIN_ARCHITECTURE.md](./BIN_ARCHITECTURE.md) - Clean Architecture in binaries

## Features

- 🏗️ **Clean Architecture**: Layered design with clear separation of concerns
- 🚀 **High Performance**: Built in Rust with async/await and WebSockets
- 🤖 **LLM-Powered**: Uses local Ollama for intelligent market filtering
- 💾 **PostgreSQL**: Robust data persistence
- 🛡️ **Risk Management**: Position limits, max bet sizes, daily loss limits
- 📊 **Real-time Monitoring**: WebSocket tracking of market orderbooks
- ⚡ **Fast Execution**: Optimized for low-latency trading
- 🔐 **Secure**: EIP-712 signatures for Polymarket authentication
- 🐳 **Docker Ready**: Complete containerization with docker-compose

## Project Structure

## Project Structure

```
polypolypoly/
├── src/                          # 📦 Main Package (Standard Rust Layout)
│   ├── lib.rs                   # Library root: re-exports + bin_common
│   │
│   ├── bin/                     # 📱 Binary executables
│   │   ├── market_sniper.rs    # Market monitoring bot
│   │   ├── polymarket_events.rs # Event synchronization
│   │   └── test_orderbook.rs   # WebSocket testing utility
│   │
│   └── bin_common/             # 🔧 Shared binary utilities
│       ├── cli.rs              # Config loading, CLI utilities
│       └── runner.rs           # Binary execution patterns
│
├── examples/                    # 📚 Usage examples
│   └── simple_config.rs        # Config loading demo
│
├── tests/                       # 🧪 Integration tests
│   └── config_loading.rs       # Config utilities tests
│
├── libs/                        # 📦 Workspace members
│   ├── polymarket/             # 🎯 Core Business Logic (Clean Architecture)
│   │   ├── domain/             # Entities, business rules (pure)
│   │   │   ├── models.rs       # DbMarket, DbEvent, SniperMarket
│   │   │   ├── orderbook.rs    # OrderBook, PriceLevel
│   │   │   ├── filter.rs       # FilterError, CacheError
│   │   │   └── strategy.rs     # ExecutedTrade
│   │   │
│   │   ├── infrastructure/     # External services implementation
│   │   │   ├── client/         # API clients (CLOB, Gamma)
│   │   │   ├── database/       # PostgreSQL access (sqlx)
│   │   │   ├── config/         # YAML configuration
│   │   │   ├── cache.rs        # In-memory cache
│   │   │   ├── ollama.rs       # LLM client
│   │   │   ├── logging.rs      # Tracing setup
│   │   │   ├── shutdown.rs     # Graceful shutdown
│   │   │   └── heartbeat.rs    # Monitoring
│   │   │
│   │   └── application/        # Use cases orchestration
│   │       ├── facade.rs       # 🎯 SniperApp, EventSyncApp
│   │       ├── sync/           # Event/Market sync services
│   │       ├── strategy/       # Trading strategy
│   │       └── filter/         # LLM filtering
│   │
│   └── hypersockets/           # 🔌 WebSocket library
│       ├── core/               # Connection management
│       ├── manager/            # Multi-connection manager
│       └── traits/             # Abstractions
│
├── config/                      # ⚙️ Configuration Files
│   ├── sniper_config.yaml      # Market sniper config
│   └── config.yaml             # Event sync config
│
├── docker-compose.yml           # 🐳 Docker services
├── Dockerfile                   # Container build
└── Cargo.toml                  # Workspace definition
```

### Layer Dependencies

```
bin/ (Presentation)
  ↓ uses
src/bin_common (Utilities)
  ↓ uses
application/ (Use Cases)
  ↓ uses
infrastructure/ (External) ← implements → domain/ (Business Logic)
```

**Key principle:** Inner layers NEVER depend on outer layers.
└── docker-compose.yml           # Ollama setup
```

## Prerequisites

- **Rust**: Install from [rustup.rs](https://rustup.rs/)
- **Docker**: For running Ollama LLM
- **Polymarket Account**: With funded USDC on Polygon
- **Private Key**: Ethereum wallet private key

## Setup

### 1. Clone and Build

```bash
# Clone the repository (adjust path as needed)
cd polymarket-arb-bot

# Build the project
cargo build --release
```

### 2. Configure Environment Variables

```bash
# Copy example env file
cp .env.example .env

# Edit .env and add your credentials
nano .env
```

**Required variables:**
- `PRIVATE_KEY`: Your Ethereum wallet private key (with 0x prefix)
- `WALLET_ADDRESS`: Your Ethereum wallet address (with 0x prefix)

**⚠️ Security Warning**: Never commit `.env` to version control!

### 3. Configure Bot Settings

Edit `config.yaml` to customize:

```yaml
llm:
  endpoint: "http://localhost:11434"  # Ollama endpoint
  model: "llama3.2"                   # LLM model to use
  prompt: |                           # Customize market filtering criteria
    Identify prediction markets suitable for last-minute arbitrage...

trading:
  probability_threshold: 0.98         # Only trade when ≥98% probable
  seconds_before_resolution: 10       # Trade 10s before close
  bet_amount_usd: 50.0               # $50 per trade

risk:
  max_concurrent_positions: 10        # Max 10 open positions
  max_bet_per_market: 100.0          # Max $100 per market
  daily_loss_limit: 500.0            # Stop if lose $500 in a day
  min_profit_cents: 50.0             # Skip trades < 50¢ profit

scanner:
  poll_interval_secs: 30             # Scan every 30 seconds
  min_resolution_window_mins: 60     # Only track markets resolving within 1 hour
```

### 4. Start Ollama

```bash
# Start Ollama container
docker-compose up -d

# Pull the LLM model (only needed once)
docker exec -it polymarket-ollama ollama pull llama3.2

# Verify model is ready
docker exec -it polymarket-ollama ollama list
```

**Alternative models:**
- `llama3.2` (recommended, ~2GB)
- `llama3.2:1b` (smaller, faster)
- `mistral` (alternative)

### 5. Run the Binaries

```bash
# Market monitoring bot
cargo run --release --bin market_sniper

# Event synchronization
cargo run --release --bin polymarket_events

# WebSocket orderbook testing
cargo run --release --bin test_orderbook

# Or run built binaries directly
./target/release/market_sniper
./target/release/polymarket_events
```

### 6. Run Examples & Tests

```bash
# Run config loading example
cargo run --example simple_config

# Run integration tests
cargo test --test config_loading

# Run all tests
cargo test
```

## How It Works

### Workflow

```
1. [Startup]
   ├─ Load config.yaml + .env
   ├─ Authenticate with Polymarket
   ├─ Connect to Ollama
   └─ Load market cache

2. [Market Scanner] (Every 30s)
   ├─ Fetch all markets from Polymarket
   ├─ Filter by resolution time (< 60 min)
   ├─ Check cache for each market
   │  ├─ Cached + compatible → Monitor
   │  ├─ Cached + incompatible → Skip
   │  └─ Not cached → Send to LLM
   └─ Update cache with LLM results

3. [Resolution Monitor] (Every 1s)
   ├─ Check markets approaching resolution
   ├─ 10s before resolution → Get orderbook
   ├─ If winning side ≥ 98¢:
   │  ├─ Risk check (limits OK?)
   │  ├─ Profit check (≥ 50¢?)
   │  ├─ Create market order
   │  ├─ Sign with EIP-712
   │  └─ Execute trade
   └─ Record position

4. [Position Management]
   ├─ Wait for market resolution
   ├─ Claim winnings ($1.00 per share)
   └─ Update daily P&L
```

### Example Execution

```
🔍 Scanning for new markets...
   Found 47 markets resolving within 60 minutes
   ✓ Cache hit (compatible): Bitcoin Up or Down - Nov 22, 4:15-4:30AM
   ✗ Cache hit (incompatible): Will it rain tomorrow?
   ? Cache miss: Solana Up or Down - Nov 22, 4:30-4:45AM

🤖 Querying LLM for 1 uncached market...
   ✓ LLM result: Solana Up or Down - Nov 22, 4:30-4:45AM

⏰ Trade window open for: Bitcoin Up or Down - Nov 22, 4:15-4:30AM
   Resolves at: 2025-11-22T04:30:00Z

🎯 EXECUTING TRADE: Bitcoin Up or Down - BUY @ 0.9850 for $50.00 (expected profit: $0.76)
✅ Order executed successfully. Order ID: 0x123...

🎉 Trade executed!

📊 Daily Stats:
   Trades: 1 (1 wins, 0 losses, 100.0% win rate)
   P&L: $0.76
   Open positions: 0
```

## Cache System

The bot uses a JSON cache (`market_cache.json`) to store LLM filtering decisions:

```json
{
  "Bitcoin Up or Down - Nov 22, 4:15-4:30AM": {
    "market_id": "0x123...",
    "question": "Bitcoin Up or Down - Nov 22, 4:15-4:30AM",
    "compatible": true,
    "checked_at": "2025-11-22T10:00:00Z",
    "resolution_time": "2025-11-22T04:30:00Z"
  }
}
```

**Benefits:**
- Reduces LLM API calls by ~95%
- Faster market scanning
- Works offline for cached markets

**Maintenance:**
- Auto-saves after LLM queries
- Auto-cleanup of entries >7 days old
- Manual edit supported

## Risk Management

The bot enforces multiple safety limits:

1. **Max Concurrent Positions**: Limits open positions (default: 10)
2. **Max Bet Per Market**: Caps single trade size (default: $100)
3. **Daily Loss Limit**: Halts trading if losses exceed threshold (default: $500)
4. **Minimum Profit**: Skips trades with low expected profit (default: 50¢)

When daily loss limit is hit:
```
🛑 TRADING HALTED due to risk limits
Daily loss limit reached: -$500.00
```

Reset happens automatically at midnight (UTC) or manually:
```rust
risk_manager.reset_daily();
```

## Performance Optimization

### Speed Optimizations

1. **Parallel Market Scanning**: Fetches and filters markets concurrently
2. **WebSocket for Orderbooks**: Real-time price updates (vs polling)
3. **Cache-First Strategy**: Checks cache before LLM
4. **Compiled Binary**: Rust performance (~100x faster than Python)

### Expected Performance

- Market scan: ~1-2 seconds (for 100 markets)
- LLM filter: ~2-5 seconds (for 10 uncached markets)
- Order execution: ~100-200ms (network latency)

## Troubleshooting

### Ollama Not Available

```
❌ Failed to connect to Ollama: Connection refused
```

**Solution:**
```bash
# Check if Ollama is running
docker ps | grep ollama

# Start if not running
docker-compose up -d

# Check logs
docker logs polymarket-ollama
```

### Model Not Found

```
❌ LLM model llama3.2 not found
```

**Solution:**
```bash
# Pull the model
docker exec -it polymarket-ollama ollama pull llama3.2

# List available models
docker exec -it polymarket-ollama ollama list
```

### Authentication Failed

```
❌ Failed to create API key: 401 Unauthorized
```

**Solution:**
- Verify `PRIVATE_KEY` in `.env` is correct (66 chars with 0x)
- Verify `WALLET_ADDRESS` matches the private key
- Ensure wallet has USDC on Polygon

### No Markets Found

```
Found 0 markets resolving within 60 minutes
```

**Solution:**
- Increase `min_resolution_window_mins` in config.yaml
- Check Polymarket has active markets
- Verify `poll_interval_secs` isn't too frequent

## Development

### Build for Development

```bash
cargo build
cargo run --bin polymarket_arb
```

### Run Tests

```bash
cargo test
```

### Check Code

```bash
cargo clippy
cargo fmt
```

### Add Dependencies

Edit `Cargo.toml` and add to `[workspace.dependencies]`.

## Monitoring

### Logs

The bot outputs structured logs:

- 🔍 Market scanning
- 🤖 LLM filtering
- ⏰ Trade opportunities
- 🎯 Order execution
- ✅ Trade success
- ❌ Trade failure
- 📊 Daily statistics

### Metrics to Watch

- **Win Rate**: Should be >95% (trading near-certain outcomes)
- **Profit per Trade**: Typically $0.50-$2.00
- **Cache Hit Rate**: Should be >90% after initial scan
- **Execution Speed**: <500ms from decision to order

## Safety & Best Practices

1. **Start Small**: Begin with low `bet_amount_usd` ($10-20)
2. **Test First**: Run for a few hours before increasing position sizes
3. **Monitor Closely**: Watch for unexpected losses or errors
4. **Set Conservative Limits**: High `probability_threshold` (≥0.98)
5. **Backup Keys**: Keep private key secure and backed up
6. **Review Trades**: Periodically check executed trades on Polymarket

## Limitations

- **Market Availability**: Depends on Polymarket having suitable markets
- **Competition**: Other bots may compete for the same opportunities
- **Network Latency**: Higher latency reduces profitability
- **Gas Fees**: Polygon fees are low but non-zero
- **Slippage**: Price may move between check and execution

## Future Enhancements

- [ ] WebSocket orderbook streaming (currently using REST)
- [ ] Position monitoring and auto-claim winnings
- [ ] Multi-market simultaneous execution
- [ ] Advanced LLM prompts for better filtering
- [ ] Telegram/Discord notifications
- [ ] Web dashboard for monitoring
- [ ] Backtesting framework
- [ ] More exchanges (Kalshi, PredictIt)

## License

This project is for educational purposes. Use at your own risk.

## Support

For issues or questions:
- Check the troubleshooting section above
- Review logs for error messages
- Ensure all prerequisites are met

## Acknowledgments

- **Polymarket**: For providing the prediction market platform
- **Ollama**: For local LLM inference
- **HyperSockets**: For WebSocket client library
- **Ethers-rs**: For Ethereum signing

---

**Disclaimer**: This bot is provided as-is for educational purposes. Trading involves risk. Always test with small amounts first.
