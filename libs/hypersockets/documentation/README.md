# HyperSockets

> **High-performance WebSocket client for Rust** — Built for HFT and cryptocurrency trading systems

HyperSockets is a production-ready WebSocket client designed for **high-frequency trading**, **cryptocurrency exchanges**, and **real-time data streams** where maximum throughput, ordered processing, and graceful shutdown are critical.

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

## Why HyperSockets?

- ⚡ **Graceful Shutdown** — Zero-overhead atomic flag coordination (~1ns checks), clean exit in <150ms
- 🚀 **HFT-Optimized** — Hybrid threading (async I/O + OS threads), lock-free channels, parallel parsing
- 🎯 **Smart Routing** — Per-type ordering with cross-type parallelism for maximum throughput
- 🔒 **Zero Message Loss** — Unbounded channels absorb bursts, no dropped messages
- 🔄 **Intelligent Reconnection** — Exponential backoff with state restoration (default: enabled)
- 🔐 **Dynamic Auth** — Fresh authentication on every connection/reconnection
- 💓 **Passive Ping** — Hot-path ping detection (checked before parsing for performance)
- 🎛️ **Type-Safe Builder** — Compile-time correctness guarantees via type-state pattern
- 🏢 **Battle-Tested** — Production use with Hyperliquid, Lighter, and other major exchanges

---

## Table of Contents

- [Quick Start](#quick-start)
- [Graceful Shutdown](#graceful-shutdown-new)
- [Real-World Examples](#real-world-examples)
- [Architecture](#architecture)
- [Performance Characteristics](#performance-characteristics)
- [Feature Guide](#feature-guide)
  - [Message Routing](#message-routing)
  - [Authentication](#authentication)
  - [Headers](#headers)
  - [Heartbeat](#heartbeat)
  - [Passive Ping Detection](#passive-ping-detection)
  - [Reconnection Strategies](#reconnection-strategies)
  - [Multi-Client Management](#multi-client-management)
- [Complete API Reference](#complete-api-reference)
- [Examples](#examples)
- [Documentation](#documentation)

---

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hypersockets = { path = "path/to/hypersockets/libs/hypersockets" }
hypersockets-traits = { path = "path/to/hypersockets/libs/hypersockets-traits" }
tokio = { version = "1.41", features = ["full"] }
async-trait = "0.1"
```

### Simple Echo Client

```rust
use hypersockets::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// 1. Define your message type
#[derive(Debug, Clone)]
enum MyMessage {
    Echo(String),
}

// 2. Define route keys
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Main,
}

// 3. Implement MessageRouter
struct MyRouter;

#[async_trait::async_trait]
impl MessageRouter for MyRouter {
    type Message = MyMessage;
    type RouteKey = Route;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        match message {
            WsMessage::Text(text) => Ok(MyMessage::Echo(text)),
            WsMessage::Binary(_) => Ok(MyMessage::Echo("Binary message".into())),
        }
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        Route::Main
    }
}

// 4. Implement MessageHandler
struct MyHandler;

impl MessageHandler<MyMessage> for MyHandler {
    fn handle(&mut self, message: MyMessage) -> Result<()> {
        println!("Received: {:?}", message);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 5. Build client with graceful shutdown support
    let shutdown_flag = Arc::new(AtomicBool::new(true));
    let shutdown_flag_signal = Arc::clone(&shutdown_flag);

    // Handle Ctrl-C for graceful shutdown
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl-C");
        println!("Shutting down gracefully...");
        shutdown_flag_signal.store(false, Ordering::Release);
    });

    let client = hypersockets::builder()
        .url("wss://echo.websocket.org")
        .router(MyRouter, |routing| {
            routing.handler(Route::Main, MyHandler)
        })
        .shutdown_flag(shutdown_flag)
        .build()
        .await?;

    // 6. Send a message
    client.send(WsMessage::Text("Hello!".to_string()))?;

    // 7. Wait for shutdown signal, then clean exit
    tokio::signal::ctrl_c().await?;
    client.shutdown().await?;

    Ok(())
}
```

---

## Graceful Shutdown (NEW!)

HyperSockets provides **zero-overhead graceful shutdown** using atomic flag coordination. When shutdown is triggered, all components stop immediately and cleanly.

### Features

- **~1 nanosecond** per atomic check (Acquire/Release ordering)
- **<150ms** total shutdown time (I/O stop + parse task grace + handler exit)
- **Zero message loss** for in-flight messages
- **No hung threads** — All components respond to shutdown flag
- **Ctrl-C handling** — Clean SIGINT/SIGTERM support

### How It Works

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Create shutdown flag (shared across all components)
let shutdown_flag = Arc::new(AtomicBool::new(true));
let shutdown_flag_signal = Arc::clone(&shutdown_flag);

// Build client with shutdown flag
let client = hypersockets::builder()
    .url("wss://api.example.com/ws")
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .shutdown_flag(shutdown_flag)
    .build()
    .await?;

// Set up Ctrl-C handler
tokio::spawn(async move {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let mut sigterm = signal(SignalKind::terminate()).unwrap();

        tokio::select! {
            _ = sigint.recv() => println!("SIGINT received"),
            _ = sigterm.recv() => println!("SIGTERM received"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.unwrap();
    }

    // Trigger graceful shutdown
    shutdown_flag_signal.store(false, Ordering::Release);
});

// ... application runs ...

// When ready to shutdown
client.shutdown().await?;
```

### Shutdown Sequence

```
1. Set shutdown_flag to false             [~1ns, atomic write]
2. Message loop detects flag              [<1ms, next iteration]
3. WebSocket connection closes            [~1ms]
4. Wait for in-flight parse tasks         [100ms grace period]
5. Handler threads detect flag            [<50ms, via recv_timeout]
6. All threads joined                     [<10ms]
Total: ~140-150ms
```

### Performance Impact

| Component | Check Frequency | Cost per Check | Impact |
|-----------|----------------|----------------|---------|
| Message Loop | Per iteration | ~1ns | 0.0001% |
| Parse Tasks | After parsing | ~1ns | 0.0001% |
| Handler Threads | Every 50ms timeout | ~1ns | 0% (idle only) |

**Total overhead during normal operation: < 0.0001% (unmeasurable)**

---

## Real-World Examples

### Hyperliquid Orderbook (Production)

Multi-stream L2 orderbook client for Hyperliquid exchange with **per-market parallel processing**:

```bash
cargo run --bin hyperliquid_orderbook
```

**Features demonstrated:**
- 10+ simultaneous orderbook streams (BTC, ETH, SOL, etc.)
- **Per-market parallelization**: Each market (BTC, SOL, etc.) processes on its own dedicated thread
- **11 handler threads total**: 10 markets + 1 for pong messages = maximum throughput
- JSON message parsing and routing
- Graceful Ctrl-C shutdown
- Metrics tracking

**Architecture:**
```
WebSocket I/O (async) → Router → 11 dedicated threads
                                  ├─ BTC orderbook thread
                                  ├─ SOL orderbook thread
                                  ├─ ETH orderbook thread
                                  ├─ ... (7 more markets)
                                  └─ Pong handler thread
```

This enables **true parallel processing**: BTC updates don't block SOL updates, all markets process concurrently for maximum throughput.

See: [`bin/hyperliquid_orderbook.rs`](bin/hyperliquid_orderbook.rs)

### Lighter Orderbook (Production)

L2 orderbook client for Lighter exchange with passive ping/pong:

```bash
cargo run --bin lighter_orderbook
```

**Features demonstrated:**
- Multiple orderbook channels
- JsonPassivePing for server pings
- Automatic pong responses
- Pretty-printed JSON output

See: [`bin/lighter_orderbook.rs`](bin/lighter_orderbook.rs)

---

## Architecture

HyperSockets uses a **hybrid threading model** optimized for HFT:

```
┌─────────────────────────────────────────────────────────────────┐
│                     WebSocket I/O (Tokio Async)                  │
│                                                                   │
│  • Single async task per client                                  │
│  • Non-blocking message reception                                │
│  • Connection management, reconnection                           │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Passive Ping Check (Sync)                     │
│                                                                   │
│  • Checked for EVERY message before parsing                      │
│  • If ping → send pong immediately, skip parsing                 │
│  • Hot-path optimization                                         │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                  Parse Task (Tokio Spawn, Parallel)              │
│                                                                   │
│  • One task spawned per message                                  │
│  • Parallel message parsing (CPU utilization)                    │
│  • Checks shutdown flag before routing                           │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│              Crossbeam Channel (Unbounded, Lock-Free)            │
│                                                                   │
│  • MPSC unbounded channels                                       │
│  • Zero contention, no blocking                                  │
│  • Absorbs message bursts                                        │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│            Handler Thread (OS Thread, Sequential)                │
│                                                                   │
│  • One dedicated OS thread per route key                         │
│  • FIFO ordering per route                                       │
│  • Parallel processing across routes                             │
│  • Checks shutdown flag every 50ms                               │
└─────────────────────────────────────────────────────────────────┘
```

### Threading Model

| Component | Type | Count | Purpose |
|-----------|------|-------|---------|
| WebSocket I/O | Tokio async task | 1 per client | Non-blocking message reception |
| Parse Tasks | Tokio spawn | 1 per message | Parallel message parsing |
| Handler Threads | OS thread | 1 per route key | Sequential processing per type |
| Heartbeat | Tokio spawn | 0-1 per client | Optional keep-alive |

### Message Flow Guarantees

- **Same route key** → Sequential FIFO processing
- **Different route keys** → Parallel processing
- **Zero message loss** → Unbounded channels
- **Deterministic ordering** → Per-type FIFO guarantee

### Thread Count and Scalability

**Key Concept:** One OS thread is created per unique `RouteKey`. The number of threads is determined by how many unique route keys your router produces.

| Scenario | Unique Route Keys | Handler Threads | Notes |
|----------|------------------|-----------------|-------|
| Single route (`Route::Main`) | 1 | 1 thread | All messages sequential |
| Type-based routing (3 types) | 3 | 3 threads | Orderbook, Trades, Ticker parallel |
| Per-market routing (10 markets) | 10 | 10 threads | BTC, ETH, SOL, etc. parallel |
| Per-market + per-type (10 markets × 3 types) | 30 | 30 threads | Maximum parallelism |
| Hyperliquid example (10 markets + pong) | 11 | 11 threads | Real production example |
| Extended example (100 markets) | 100 | 100 threads | Still perfectly fine! |

**Thread Limits and Resource Usage:**

- **CPU Cores vs Thread Count**: `nproc` shows CPU cores (e.g., 32), NOT the thread limit
  - CPU cores = How many threads can run **simultaneously**
  - Thread limit = How many threads can **exist** (~10,000+ on modern systems)
  - WebSocket handlers are **I/O-bound**, not CPU-bound (mostly waiting for messages)

- **Resource Usage per Thread**:
  - Stack memory: ~2MB per thread (100 threads = ~200MB)
  - CPU usage when idle: ~0% (blocked on `recv_timeout`)
  - CPU usage when processing: Depends on handler workload

- **I/O-Bound vs CPU-Bound**:
  - **I/O-Bound** (WebSocket handlers): Thread spends 99%+ time waiting for network I/O
    - 100 threads with 1 msg/sec each = ~2-5% total CPU usage
    - Thread count limited by memory, not CPU
  - **CPU-Bound** (heavy computation): Thread uses CPU continuously
    - Limited by CPU cores (32 cores = ~32 threads max efficiency)

**When to Use Many Threads:**
- ✅ **Per-market orderbooks** (each market independent, I/O-bound)
- ✅ **Per-exchange connections** (different exchanges, I/O-bound)
- ✅ **Per-instrument streaming** (tickers, trades, candles)

**When to Limit Threads:**
- ⚠️ **CPU-intensive handlers** (complex calculations, ML inference)
- ⚠️ **Shared mutable state** (requires synchronization, reduces benefit)
- ⚠️ **Memory constrained environments** (<1GB RAM available)

**Rule of Thumb:**
- < 50 threads: Don't even think about it
- 50-200 threads: Perfectly normal for multi-market trading systems
- 200-500 threads: Fine if I/O-bound, monitor memory
- \> 500 threads: Consider async alternatives or batching

---

## Performance Characteristics

### Why HyperSockets is Fast

1. **Lock-Free Architecture**
   - Atomic state management (AtomicU8, AtomicU64)
   - Unbounded crossbeam channels (MPSC, no contention)
   - No mutexes in hot path

2. **Hybrid Threading Model**
   - Async I/O for WebSocket (non-blocking)
   - OS threads for handlers (maximum throughput)
   - Parallel message parsing (CPU utilization)

3. **Zero-Copy Where Possible**
   - Arc-based message sharing
   - Minimal clones in hot path
   - Direct channel sends (no intermediate buffers)

4. **Hot-Path Optimizations**
   - Passive ping check before parsing
   - Shutdown flag checks (~1ns atomic load)
   - Fast-path for common cases

### Measured Performance

| Operation | Cost | Notes |
|-----------|------|-------|
| Atomic state read | ~1ns | Acquire ordering |
| Channel send | ~100ns | Crossbeam unbounded |
| Shutdown check | ~1ns | Per message, per handler |
| Handler recv_timeout | 50ms | Only during idle periods |

### Handler Thread Performance

**Common Question:** *"Why are we waiting 50ms? Isn't that hindering performance?"*

**Answer:** `recv_timeout(50ms)` does **NOT** add latency to message processing. It returns **instantly** when a message is available.

**How `recv_timeout` Works:**

```rust
loop {
    // Returns INSTANTLY if message in channel
    // Only waits if channel is EMPTY
    match receiver.recv_timeout(Duration::from_millis(50)) {
        Ok(message) => {
            // Process immediately (0μs delay)
            handler.handle(message)?;
        }
        Err(RecvTimeoutError::Timeout) => {
            // Channel was empty for 50ms
            // Check shutdown flag, loop again
        }
    }
}
```

**Timeline Examples:**

**High Message Rate (1-50ms between messages):**
```
0ms    - recv_timeout called
0μs    - Message available, returns instantly  ← ZERO DELAY
0.1ms  - Handler processes message
0.2ms  - recv_timeout called again
0μs    - Next message available, returns instantly  ← ZERO DELAY
...
```

**Low Message Rate (idle periods):**
```
0ms    - recv_timeout called
50ms   - Timeout (channel empty) ← Used for shutdown responsiveness
50ms   - Check shutdown flag
50ms   - recv_timeout called again
75ms   - Message arrives, returns instantly (after 25ms idle)
75.1ms - Handler processes message
```

**Performance Impact:**
- **Message processing latency**: 0μs added (returns instantly when message available)
- **Shutdown responsiveness**: <50ms (checks flag on timeout)
- **CPU usage when idle**: ~0% (thread blocked, not spinning)

**Why 50ms timeout?**
- Balance between shutdown responsiveness (<50ms) and CPU efficiency
- Could use 10ms for faster shutdown, or 100ms for lower overhead
- Does NOT affect message processing speed (still instant)

### Suitable For

- High-frequency trading systems
- Cryptocurrency exchange integrations
- Real-time data streams (orderbooks, trades)
- Low-latency applications
- Burst traffic handling (market events)

---

## Feature Guide

### Message Routing

HyperSockets provides **per-type ordering with cross-type parallelism**. Messages of the same type are processed sequentially (FIFO), while different types are processed in parallel.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Orderbook,
    Trades,
    Ticker,
}

struct MyRouter;

#[async_trait::async_trait]
impl MessageRouter for MyRouter {
    type Message = ExchangeMessage;
    type RouteKey = Route;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        // Parse WebSocket message into typed message
        let text = message.as_text().ok_or("Not text")?;
        serde_json::from_str(text).map_err(|e| e.into())
    }

    fn route_key(&self, message: &Self::Message) -> Self::RouteKey {
        // Determine which handler should process this message
        match message {
            ExchangeMessage::Orderbook(_) => Route::Orderbook,
            ExchangeMessage::Trade(_) => Route::Trades,
            ExchangeMessage::Ticker(_) => Route::Ticker,
        }
    }
}

// Build with multiple handlers
let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .router(MyRouter, |routing| {
        routing
            .handler(Route::Orderbook, OrderbookHandler)
            .handler(Route::Trades, TradeHandler)
            .handler(Route::Ticker, TickerHandler)
    })
    .build()
    .await?;
```

**Guarantees:**
- All `Orderbook` messages processed in order on dedicated thread
- All `Trades` messages processed in order on dedicated thread
- All `Ticker` messages processed in order on dedicated thread
- Orderbook, Trades, and Ticker processed **in parallel**

#### Per-Market Parallelization

For **multi-market orderbooks**, you want each market (BTC, ETH, SOL, etc.) to process independently without blocking each other. This is achieved by making the market identifier part of the `RouteKey`.

**❌ Before: Sequential Processing (Single Thread)**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Main,  // All markets use same route = 1 thread
}

fn route_key(&self, message: &OrderbookMessage) -> Route {
    Route::Main  // BTC, ETH, SOL all sequential 😢
}
```

**Result:** All markets processed sequentially on 1 thread. BTC blocks ETH blocks SOL.

**✅ After: Parallel Processing (Per-Market Threads)**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Orderbook(String),  // Market name is part of route key
}

fn route_key(&self, message: &OrderbookMessage) -> Route {
    match message {
        OrderbookMessage::Subscription { coin, .. } => {
            Route::Orderbook(coin.clone())  // "BTC", "ETH", "SOL"
        }
    }
}

// Register handler for each market
.router(MyRouter, move |mut routing| {
    for coin in &["BTC", "ETH", "SOL", "HYPE", "DOGE"] {
        routing = routing.handler(
            Route::Orderbook(coin.to_string()),
            OrderbookHandler::new(coin.to_string()),
        );
    }
    routing
})
```

**Result:** 5 markets = 5 threads. All markets process **in parallel** 🚀

**Architecture Comparison:**

```
BEFORE (Sequential):
WebSocket → Router → Route::Main → 1 thread handles all markets
                                    ├─ BTC update 1
                                    ├─ ETH update 1 (waits for BTC)
                                    ├─ SOL update 1 (waits for ETH)
                                    ├─ BTC update 2 (waits for SOL)
                                    └─ ... (all sequential)

AFTER (Parallel):
WebSocket → Router → Route::Orderbook("BTC") → BTC thread
                  ├─ Route::Orderbook("ETH") → ETH thread
                  ├─ Route::Orderbook("SOL") → SOL thread
                  ├─ Route::Orderbook("HYPE") → HYPE thread
                  └─ Route::Orderbook("DOGE") → DOGE thread
                     (all process concurrently)
```

**Real Example:** See [`bin/hyperliquid_orderbook.rs`](bin/hyperliquid_orderbook.rs) for production implementation with 10+ markets.

**Key Insight:** The `RouteKey` determines parallelism. Different route keys = different threads = parallel processing.

---

### Authentication

Dynamic authentication that's **fresh on every connection** (including reconnections):

```rust
use async_trait::async_trait;

struct MyAuth {
    api_key: String,
    api_secret: String,
}

#[async_trait]
impl AuthProvider for MyAuth {
    async fn get_auth_message(&self) -> Result<Option<WsMessage>> {
        // Generate fresh HMAC signature with current timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis();

        let signature = compute_hmac(&self.api_secret, &timestamp.to_string());

        let auth_msg = json!({
            "op": "login",
            "args": [{
                "apiKey": self.api_key,
                "timestamp": timestamp,
                "signature": signature,
            }]
        });

        Ok(Some(WsMessage::Text(auth_msg.to_string())))
    }

    async fn validate_auth_response(&self, response: &WsMessage) -> Result<bool> {
        // Validate server's auth response
        if let Some(text) = response.as_text() {
            let resp: serde_json::Value = serde_json::from_str(text)?;
            Ok(resp["event"] == "login" && resp["success"] == true)
        } else {
            Ok(false)
        }
    }
}

// Use with client
let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .auth(MyAuth {
        api_key: env::var("API_KEY")?,
        api_secret: env::var("API_SECRET")?,
    })
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**Key Points:**
- Called on **initial connection** and **every reconnection**
- Supports rotating keys, time-based signatures, JWTs
- Sent **before** subscriptions

---

### Headers

Dynamic HTTP headers for WebSocket upgrade request:

```rust
use async_trait::async_trait;
use std::collections::HashMap;

struct MyHeaders {
    api_key: String,
}

#[async_trait]
impl HeaderProvider for MyHeaders {
    async fn get_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();

        // Add API key header
        headers.insert("X-API-Key".to_string(), self.api_key.clone());

        // Add timestamp (fresh on each connection)
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        headers.insert("X-Timestamp".to_string(), timestamp.to_string());

        headers
    }
}

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .headers(MyHeaders {
        api_key: env::var("API_KEY")?,
    })
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**Use Cases:**
- API keys in headers
- Timestamps, nonces for request signing
- Custom authentication schemes

---

### Heartbeat

Dedicated task for periodic keep-alive messages:

```rust
use std::time::Duration;

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .heartbeat(
        Duration::from_secs(30),  // Send every 30 seconds
        WsMessage::Text(r#"{"op":"ping"}"#.to_string())
    )
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**Features:**
- Dedicated Tokio task (non-blocking)
- Automatically restarted on reconnection
- MissedTickBehavior::Skip (no burst on lag)

---

### Passive Ping Detection

**Hot-path optimization**: Detect server pings and respond with pongs **before parsing**, saving CPU cycles.

#### Text-Based Ping

```rust
use hypersockets_traits::TextPassivePing;

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .passive_ping(TextPassivePing::new(
        "ping",  // Incoming ping message
        WsMessage::Text("pong".to_string())  // Outgoing pong response
    ))
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

#### JSON-Based Ping

```rust
use hypersockets_traits::JsonPassivePing;

let client = hypersockets::builder()
    .url("wss://mainnet.zklighter.elliot.ai/stream")
    .passive_ping(JsonPassivePing::new(
        "type",        // JSON field to check
        "ping",        // Expected value for ping
        WsMessage::Text(r#"{"type":"pong"}"#.to_string())  // Pong response
    ))
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**How It Works:**
1. Every WebSocket message checked before parsing
2. If `is_ping()` returns true → send pong immediately, skip parsing
3. If not ping → proceed to normal parsing

**Performance:**
- Saves CPU by skipping parse for pings
- Immediate pong response (no parse delay)
- Zero-cost abstraction (trait-based)

---

### Reconnection Strategies

**Default:** ExponentialBackoff is enabled automatically if not specified.

#### Exponential Backoff (Recommended)

```rust
use hypersockets_traits::ExponentialBackoff;
use std::time::Duration;

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .reconnect_strategy(ExponentialBackoff::new(
        Duration::from_secs(1),   // Initial delay
        Duration::from_secs(60),  // Max delay
        Some(10)                  // Max attempts (None = infinite)
    ))
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**Delay sequence:** 1s → 2s → 4s → 8s → 16s → 32s → 60s (capped)

#### Fixed Delay

```rust
use hypersockets_traits::FixedDelay;
use std::time::Duration;

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .reconnect_strategy(FixedDelay::new(
        Duration::from_secs(5),  // Always wait 5s
        Some(20)                 // Max 20 attempts
    ))
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

#### No Reconnection

```rust
use hypersockets_traits::NeverReconnect;

let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .reconnect_strategy(NeverReconnect)
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

#### Reconnection Delay Offset

Wait a fixed period **after disconnect** before applying reconnection strategy:

```rust
let client = hypersockets::builder()
    .url("wss://api.exchange.com/ws")
    .reconnection_delay_offset(Duration::from_secs(2))  // Wait 2s after disconnect
    .reconnect_strategy(ExponentialBackoff::new(...))   // Then apply strategy
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    .build()
    .await?;
```

**Use case:** Give server time to clean up resources after disconnect.

---

### Multi-Client Management

Manage multiple WebSocket connections with centralized control:

```rust
use hypersockets_manager::ClientManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Create shutdown flag for coordinated shutdown
let shutdown_flag = Arc::new(AtomicBool::new(true));
let manager = ClientManager::new(Arc::clone(&shutdown_flag));

// Get manager's halted flag for connection state monitoring
let halted_flag = manager.halted_flag();

// Build clients with manager's flags
let binance_client = hypersockets::builder()
    .url("wss://binance.com/ws")
    .router(MyRouter, |r| r.handler(Route::Main, MyHandler))
    .shutdown_flag(Arc::clone(&shutdown_flag))
    .halted_flag(Arc::clone(&halted_flag))  // Track connection state
    .build()
    .await?;

// Add clients
manager.add_client("binance", binance_client)?;
manager.add_client("coinbase", coinbase_client)?;
manager.add_client("kraken", kraken_client)?;

// Send to specific client
manager.send_to("binance", WsMessage::Text(subscribe_msg))?;

// Broadcast to all
manager.broadcast(WsMessage::Text(ping_msg))?;

// Monitor connection health
if manager.is_halted() {
    println!("⚠️ Some connections down: {:?}", manager.get_disconnected_clients());
} else {
    println!("✅ All connections healthy");
}

// Shutdown all clients gracefully
manager.shutdown().await?;
```

**Connection State Monitoring:**

The `ClientManager` automatically tracks connection health via the `halted_flag`:
- **`halted_flag`**: Atomic flag indicating when ANY client is disconnected (but not shutting down)
- **`is_halted()`**: Returns `true` if any client is disconnected
- **`get_disconnected_clients()`**: Returns list of disconnected client IDs
- **`disconnected_count()`**: Returns count of disconnected clients

**How it works:**
1. Manager owns an internal `halted_flag: Arc<AtomicBool>`
2. Clients are built with this flag via `.halted_flag(manager.halted_flag())`
3. Background task monitors connection events
4. When a client disconnects: adds to disconnected set, sets `halted_flag = true`
5. When a client reconnects: removes from set, clears `halted_flag = false` if all connected

**Example: Connection monitoring loop**

```rust
tokio::spawn({
    let manager = manager.clone();
    let halted_flag = manager.halted_flag();

    async move {
        loop {
            if halted_flag.load(Ordering::Acquire) {
                let disconnected = manager.get_disconnected_clients();
                eprintln!("⚠️ {} connections down: {:?}",
                          disconnected.len(), disconnected);
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
});
```

See: [`bin/multi_client_example.rs`](bin/multi_client_example.rs)

---

## Complete API Reference

### Builder Methods

```rust
hypersockets::builder()
    // Required
    .url(url: impl Into<String>)
    .router<R, F>(router: R, configure: F)

    // Optional
    .auth(provider: impl AuthProvider + 'static)
    .headers(provider: impl HeaderProvider + 'static)
    .heartbeat(interval: Duration, payload: WsMessage)
    .passive_ping(detector: impl PassivePingDetector + 'static)
    .reconnect_strategy(strategy: impl ReconnectionStrategy + 'static)
    .reconnection_delay_offset(offset: Duration)
    .subscription(message: WsMessage)
    .subscriptions(messages: Vec<WsMessage>)
    .shutdown_flag(flag: Arc<AtomicBool>)
    .halted_flag(flag: Arc<AtomicBool>)

    // Build
    .build()
    .await?
```

### Traits to Implement

#### MessageRouter (Required)

```rust
#[async_trait::async_trait]
pub trait MessageRouter: Send + Sync + 'static {
    type Message: Send + std::fmt::Debug + 'static;
    type RouteKey: Clone + Eq + std::hash::Hash + Send + std::fmt::Debug + 'static;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message>;
    fn route_key(&self, message: &Self::Message) -> Self::RouteKey;
}
```

#### MessageHandler (Required per route)

```rust
pub trait MessageHandler<M>: Send + 'static {
    fn handle(&mut self, message: M) -> Result<()>;
}
```

#### AuthProvider (Optional)

```rust
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync + 'static {
    async fn get_auth_message(&self) -> Result<Option<WsMessage>>;
    async fn validate_auth_response(&self, response: &WsMessage) -> Result<bool>;
}
```

#### HeaderProvider (Optional)

```rust
#[async_trait::async_trait]
pub trait HeaderProvider: Send + Sync + 'static {
    async fn get_headers(&self) -> Headers;  // HashMap<String, String>
}
```

#### PassivePingDetector (Optional)

```rust
pub trait PassivePingDetector: Send + Sync + 'static {
    fn is_ping(&self, message: &WsMessage) -> bool;
    fn get_pong_response(&self) -> WsMessage;
}
```

#### ReconnectionStrategy (Optional)

```rust
pub trait ReconnectionStrategy: Send + Sync + 'static {
    fn next_delay(&self, attempt: usize) -> Option<Duration>;
}
```

### Built-In Implementations

- `TextPassivePing` — Exact text match for pings
- `JsonPassivePing` — JSON field/value match for pings
- `ExponentialBackoff` — Exponentially increasing delays
- `FixedDelay` — Fixed delay between attempts
- `NeverReconnect` — No reconnection

---

## Examples

All examples are located in the `bin/` directory:

### Production Examples

| Example | Description | Features |
|---------|-------------|----------|
| `hyperliquid_orderbook.rs` | Hyperliquid L2 orderbooks (10+ symbols) | Multi-stream, JSON routing, Ctrl-C handling |
| `lighter_orderbook.rs` | Lighter L2 orderbooks with passive ping | JsonPassivePing, multi-channel, graceful shutdown |

### Feature Demonstrations

| Example | Description | Features |
|---------|-------------|----------|
| `basic_example.rs` | Simple echo client | Basic setup, heartbeat, metrics |
| `passive_ping_demo.rs` | Passive ping/pong handling | TextPassivePing, JsonPassivePing |
| `routing_example.rs` | Multi-handler routing | Different handlers per route |
| `heartbeat_demo.rs` | Keep-alive heartbeat | Periodic ping messages |
| `headers_example.rs` | Dynamic header generation | HeaderProvider, API keys |
| `advanced_example.rs` | All features combined | Auth, headers, heartbeat, routing |
| `multi_client_example.rs` | Multiple connections | ClientManager usage |

### Running Examples

```bash
# Production examples
cargo run --bin hyperliquid_orderbook
cargo run --bin lighter_orderbook

# Feature demos
cargo run --bin basic_example
cargo run --bin passive_ping_demo
cargo run --bin routing_example
```

---

## Documentation

- **[ARCHITECTURE.md](./ARCHITECTURE.md)** — Internal design deep dive, threading model, concurrency
- **[API_IMPROVEMENTS.md](./API_IMPROVEMENTS.md)** — Future API improvements roadmap
- **[HEARTBEAT.md](./HEARTBEAT.md)** — Heartbeat mechanism details
- **[PASSIVE_PING.md](./PASSIVE_PING.md)** — Passive ping detection guide
- **[SMART_RECONNECTION.md](./SMART_RECONNECTION.md)** — Reconnection strategies
- **[CONTRIBUTING.md](./CONTRIBUTING.md)** — Development guidelines

---

## Best Practices

### 1. Always Use Graceful Shutdown

```rust
let shutdown_flag = Arc::new(AtomicBool::new(true));

// Set up signal handler
tokio::spawn({
    let flag = Arc::clone(&shutdown_flag);
    async move {
        tokio::signal::ctrl_c().await.unwrap();
        flag.store(false, Ordering::Release);
    }
});

// Build client with flag
let client = hypersockets::builder()
    .url(url)
    .shutdown_flag(shutdown_flag)
    // ...
    .build()
    .await?;

// Clean shutdown
client.shutdown().await?;
```

### 2. Use Unbounded Channels (Already Default)

HyperSockets uses unbounded crossbeam channels by default. **Do not** implement bounded channels in your handlers unless you have a specific reason.

**Why unbounded?**
- Zero message loss during bursts
- No backpressure blocking I/O task
- Suitable for HFT (absorb market events)

### 3. Keep Handlers Fast

Handler threads process messages sequentially. Long-running handlers block subsequent messages of the same type.

**Good:**
```rust
impl MessageHandler<OrderbookUpdate> for MyHandler {
    fn handle(&mut self, update: OrderbookUpdate) -> Result<()> {
        // Fast operations only
        self.orderbook.apply_update(update);  // O(log n)
        self.metrics.increment();
        Ok(())
    }
}
```

**Bad:**
```rust
impl MessageHandler<OrderbookUpdate> for MyHandler {
    fn handle(&mut self, update: OrderbookUpdate) -> Result<()> {
        // Slow! Blocks all orderbook updates
        tokio::runtime::Runtime::new()?.block_on(async {
            send_to_database(update).await?;
        })
    }
}
```

**Solution for slow operations:**
- Spawn tasks or use separate channels to offload work
- Handler thread only does critical path work

### 4. Use Passive Ping When Possible

If the exchange sends periodic pings, use passive ping detection instead of heartbeat:

```rust
// Good: Passive ping (server sends pings)
.passive_ping(JsonPassivePing::new("type", "ping", pong_msg))

// Less ideal: Active heartbeat (you send pings)
.heartbeat(Duration::from_secs(30), ping_msg)
```

Passive ping is more efficient (checked before parsing).

### 5. Monitor Metrics

```rust
let metrics = client.metrics();
println!("Sent: {}, Received: {}, Reconnects: {}",
    metrics.messages_sent,
    metrics.messages_received,
    metrics.reconnect_count
);
```

### 6. Handle Reconnection State

After reconnection, resubscribe to channels:

```rust
// Subscriptions are automatically sent after reconnection
.subscription(WsMessage::Text(subscribe_orderbook))
.subscription(WsMessage::Text(subscribe_trades))
```

No need to manually resubscribe — the library handles it.

---

## Troubleshooting

### Client Hangs on Shutdown

**Symptom:** `client.shutdown().await` never completes.

**Cause:** Shutdown flag not set, or not using shutdown flag.

**Solution:**
```rust
// Always use shutdown flag
let shutdown_flag = Arc::new(AtomicBool::new(true));
let client = builder.shutdown_flag(shutdown_flag).build().await?;

// Set flag before calling shutdown
shutdown_flag.store(false, Ordering::Release);
client.shutdown().await?;  // Now completes quickly
```

### Messages Not Being Processed

**Symptom:** WebSocket receives messages but handlers don't run.

**Cause:** Router not routing to any handler, or handler not registered.

**Solution:**
```rust
// Ensure route_key returns a key that has a handler
fn route_key(&self, message: &MyMessage) -> RouteKey {
    Route::Main  // This key MUST have a handler
}

// Register handler for this key
.router(MyRouter, |routing| {
    routing.handler(Route::Main, MyHandler)  // Must match route_key
})
```

### Connection Keeps Dropping

**Symptom:** Repeated reconnection attempts.

**Cause:** No heartbeat or passive ping configured.

**Solution:**
```rust
// Option 1: Use heartbeat (you send pings)
.heartbeat(Duration::from_secs(30), ping_msg)

// Option 2: Use passive ping (server sends pings)
.passive_ping(JsonPassivePing::new("type", "ping", pong_msg))
```

### Parse Errors

**Symptom:** "Parse error" logs but no handler execution.

**Cause:** `parse()` returning `Err`.

**Solution:**
```rust
async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
    match message {
        WsMessage::Text(text) => {
            // Add proper error handling
            serde_json::from_str(&text)
                .map_err(|e| HyperSocketError::Parse(format!("JSON error: {}", e)))
        }
        WsMessage::Binary(_) => {
            Err(HyperSocketError::Parse("Binary not supported".into()))
        }
    }
}
```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

---

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

---

**Built with ❤️ for high-frequency trading and real-time systems**
