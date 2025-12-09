# Polymarket Trading Bot

A high-performance Rust bot for Polymarket featuring event synchronization, real-time orderbook tracking via WebSocket, and opportunity detection.

## Architecture

This project follows **Clean Architecture** with strict layer separation:

```
┌─────────────────────────┐
│   Presentation          │  src/bin/ (sniper, polymarket_events)
│   (Binaries)            │  src/bin_common/ (shared utilities)
└──────────┬──────────────┘
           │
           ▼
┌─────────────────────────┐
│   Application           │  libs/polymarket/application/
│   (Use Cases)           │  - Facades: SniperApp, EventSyncApp
└──────────┬──────────────┘  - Services: Sync, Tracker
           │
           ▼
┌─────────────────────────┐      ┌─────────────────────────┐
│   Infrastructure        │◄─────│   Domain                │
│   (External I/O)        │      │   (Business Logic)      │
│   - Database (sqlx)     │      │   - Entities            │
│   - API Clients         │      │   - Models              │
│   - WebSocket           │      │   - Pure Logic          │
│   - Config              │      │                         │
└─────────────────────────┘      └─────────────────────────┘
```

## Features

- **Real-time Orderbook Tracking**: WebSocket connection to Polymarket CLOB
- **Event Synchronization**: Automatic sync from Gamma API to PostgreSQL
- **Opportunity Detection**: Identifies markets with favorable ask prices
- **Clean Architecture**: Layered design with clear separation of concerns
- **Docker Ready**: Compose profiles for flexible deployment
- **High Performance**: Async Rust with tokio runtime

## Project Structure

```
polymarket-arb-bot/
├── src/
│   ├── bin/                          # Executable binaries
│   │   ├── polymarket_events.rs      # Event synchronization daemon
│   │   ├── sniper.rs                 # Pluggable strategy runner
│   │   └── test_orderbook.rs         # WebSocket diagnostic tool
│   ├── bin_common/                   # Shared binary utilities
│   └── lib.rs                        # Library exports
│
├── libs/
│   ├── polymarket/                   # Core business logic
│   │   ├── domain/                   # Entities & business rules
│   │   ├── application/              # Use cases & orchestration
│   │   └── infrastructure/           # Database, API clients, config
│   └── hypersockets/                 # Custom WebSocket library
│
├── config/
│   ├── events_config.yaml            # Events syncer configuration
│   └── strategies_config.yaml        # Strategies runner configuration
│
├── docker-compose.yml                # Docker services
├── Dockerfile                        # Container build
└── Cargo.toml                        # Workspace manifest
```

## Binaries

### 1. polymarket_events
Synchronizes event data from Polymarket Gamma API to PostgreSQL.

- Fetches active events with pagination
- Updates `events` table and links to `markets` via `event_markets`
- Runs continuously with 60-second sync interval

### 2. sniper (Strategy Runner)
Pluggable strategy runner supporting multiple trading strategies.

- Runs configurable strategies from `strategies_config.yaml`
- Strategy selection priority: `STRATEGY_NAME` env var > CLI arg > config file
- Supports running multiple strategies in parallel (separate containers)
- Graceful shutdown with Ctrl+C

**Available Strategies:**
| Strategy | Description |
|----------|-------------|
| `up_or_down` | Monitors recurring crypto price prediction markets |

**Usage:**
```bash
# Via CLI argument
./sniper up_or_down

# Via environment variable (Docker-friendly)
STRATEGY_NAME=up_or_down ./sniper
```

### 3. test_orderbook
Diagnostic tool for testing WebSocket orderbook connections.

```bash
cargo run --bin test_orderbook -- <token_id_1> <token_id_2> [outcome_1] [outcome_2]
```

## Prerequisites

- **Rust**: Install from [rustup.rs](https://rustup.rs/)
- **Docker & Docker Compose**: For containerized deployment
- **PostgreSQL**: Version 16+ (or use Docker)

## Setup

### 1. Clone and Build

```bash
cd polymarket-arb-bot
cargo build --release
```

### 2. Configure Environment

```bash
cp .env.example .env
```

Edit `.env` with your settings:

```env
# Database
DATABASE_URL=postgres://postgres:postgres@postgres:5432/polymarket
POSTGRES_USER=postgres
POSTGRES_PASSWORD=postgres
POSTGRES_DB=polymarket

# Logging
RUST_LOG=info
```

### 3. Configure Strategies

Edit `config/strategies_config.yaml`:

```yaml
# Log level: error, warn, info, debug, trace
log_level: "info"

# Up or Down strategy settings
up_or_down:
  # Time window in seconds before market ends to trigger alert
  delta_t_seconds: 300
  # How often to poll database for new markets (seconds)
  poll_interval_secs: 60
```

**Note:** The strategy to run must be specified via `STRATEGY_NAME` env var or CLI argument.

### 4. Docker Deployment

The project uses Docker Compose with **profiles** for flexible deployment:

| Service | Profile | Description |
|---------|---------|-------------|
| `postgres` | *(always runs)* | PostgreSQL database |
| `polymarket-events` | `events`, `strategies` | Event synchronization daemon |
| `sniper-up-or-down` | `strategies` | Up or Down strategy runner |

**Commands:**

```bash
# Build containers
docker compose build

# Start only postgres (background)
docker compose up -d

# Start postgres + events syncer
docker compose --profile events up

# Start postgres + events syncer + all strategies
docker compose --profile strategies up

# Build and start
docker compose --profile strategies up --build

# Reset database (drops all data)
docker compose down -v
docker compose up -d
```

**Typical workflow:**
```bash
# Start strategies (includes events syncer automatically)
docker compose --profile strategies up
```

**Running Multiple Strategies:**

To run multiple strategies in parallel, add more services to `docker-compose.yml`:

```yaml
# Example: Add a second strategy
sniper-another-strategy:
  profiles: ["strategies"]
  build: .
  container_name: sniper-another-strategy
  command: ["./sniper"]
  depends_on:
    postgres:
      condition: service_healthy
    polymarket-events:
      condition: service_started
  env_file:
    - .env
  environment:
    - DATABASE_URL=${DATABASE_URL}
    - STRATEGY_NAME=another_strategy
    - STRATEGIES_CONFIG_PATH=/etc/polymarket/strategies_config.yaml
  volumes:
    - ./config/strategies_config.yaml:/etc/polymarket/strategies_config.yaml
```

All services with `profiles: ["strategies"]` will start together when running:
```bash
docker compose --profile strategies up
```

### 5. Run Locally (without Docker)

```bash
# Start PostgreSQL separately, then:

# Run event syncer
cargo run --release --bin polymarket_events

# Run strategy runner (strategy is required)
cargo run --release --bin sniper -- up_or_down

# Or via environment variable
STRATEGY_NAME=up_or_down cargo run --release --bin sniper
```

## Database Schema

The system uses PostgreSQL with the following tables:

| Table | Description |
|-------|-------------|
| `markets` | Market data (question, outcomes, token_ids, resolution_time) |
| `events` | Event metadata (title, description, category, volume) |
| `event_markets` | Junction table linking events to markets |
| `opportunities` | Detected trading opportunities |

## How It Works

```
1. [Event Syncer] (Every 60s)
   ├─ Fetch active events from Gamma API
   ├─ Upsert events into database
   └─ Link events to their markets

2. [Strategy Runner] (Continuous)
   ├─ Poll database for markets matching strategy criteria
   ├─ Track markets approaching resolution
   ├─ Execute strategy-specific logic
   └─ Log alerts when markets enter time window
```

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Check code
cargo clippy
cargo fmt

# Run specific binary
cargo run --bin sniper
cargo run --bin polymarket_events
```

## Troubleshooting

### Database Connection Failed
```
Error: Database connection error
```
- Verify `DATABASE_URL` in `.env`
- Ensure PostgreSQL is running
- Check network connectivity to database host

### No Markets Found
```
No markets found matching criteria
```
- Increase `delta_t_seconds` in strategies config
- Ensure `polymarket_events` has synced data
- Check database has active markets with required tags

## License

This project is for educational purposes. Use at your own risk.

---

**Note**: This bot currently detects opportunities but does not execute trades automatically.
