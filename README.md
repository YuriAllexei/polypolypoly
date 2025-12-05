# Polymarket Trading Bot

A high-performance Rust bot for Polymarket featuring event synchronization, real-time orderbook tracking via WebSocket, and opportunity detection.

## Architecture

This project follows **Clean Architecture** with strict layer separation:

```
┌─────────────────────────┐
│   Presentation          │  src/bin/ (market_sniper, polymarket_events)
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
│   │   ├── market_sniper.rs          # Market monitoring & opportunity detection
│   │   ├── polymarket_events.rs      # Event synchronization daemon
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
│   ├── sniper_config.yaml            # Sniper configuration
│   └── sniper_config.example.yaml    # Example config
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

### 2. market_sniper
Monitors markets approaching resolution and detects trading opportunities.

- Polls database for markets expiring within configured time window
- Spawns WebSocket trackers for real-time orderbook updates
- Detects opportunities when ask prices fall below probability threshold
- Stores detected opportunities in database

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

# Config path (for Docker)
SNIPER_CONFIG_PATH=/etc/polymarket/sniper_config.yaml
```

### 3. Configure Sniper

Edit `config/sniper_config.yaml`:

```yaml
# Probability threshold for opportunity detection (0.0 to 1.0)
probability: 0.95

# Time window in seconds - markets expiring within this window are tracked
delta_t_seconds: 43200  # 12 hours

# How often to poll database for new markets (seconds)
loop_interval_secs: 60
```

### 4. Docker Deployment

The project uses Docker Compose with **profiles** for flexible deployment:

| Service | Profile | Description |
|---------|---------|-------------|
| `postgres` | *(always runs)* | PostgreSQL database |
| `polymarket-events` | `events` | Event synchronization daemon |
| `market-sniper` | `sniper` | Market monitoring & tracking |

**Commands:**

```bash
# Build containers
docker compose build

# Start only postgres (background)
docker compose up -d

# Start postgres + events syncer
docker compose --profile events up

# Start postgres + market sniper
docker compose --profile sniper up

# Start everything
docker compose --profile events --profile sniper up

# Build and start
docker compose --profile events up --build

# Reset database (drops all data)
docker compose down -v
docker compose up -d
```

**Typical workflow:**
```bash
# Terminal 1: Start postgres and events syncer
docker compose --profile events up

# Terminal 2: Start market sniper
docker compose --profile sniper up
```

### 5. Run Locally (without Docker)

```bash
# Start PostgreSQL separately, then:

# Run event syncer
cargo run --release --bin polymarket_events

# Run market sniper
cargo run --release --bin market_sniper
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

2. [Market Sniper] (Continuous)
   ├─ Poll database for markets expiring soon
   ├─ For each new market:
   │   ├─ Parse token IDs and outcomes
   │   ├─ Connect to WebSocket
   │   └─ Subscribe to orderbook updates
   ├─ Monitor live orderbook prices
   ├─ Detect opportunities (ask < threshold)
   └─ Store opportunities in database

3. [WebSocket Tracker]
   ├─ Receive real-time orderbook snapshots
   ├─ Process price level updates
   ├─ Track best bid/ask per outcome
   └─ Trigger opportunity detection
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
cargo run --bin market_sniper
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
No markets found expiring within window
```
- Increase `delta_t_seconds` in sniper config
- Ensure `polymarket_events` has synced data
- Check database has active markets

### WebSocket Connection Issues
```
WebSocket connection failed
```
- Check internet connectivity
- Verify Polymarket CLOB WebSocket is accessible
- Review logs for specific error messages

## License

This project is for educational purposes. Use at your own risk.

---

**Note**: This bot currently detects opportunities but does not execute trades automatically.
