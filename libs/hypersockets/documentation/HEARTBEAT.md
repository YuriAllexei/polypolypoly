# Heartbeat Mechanism Documentation

## Overview

The HyperSockets heartbeat mechanism ensures WebSocket connections stay alive by sending periodic messages at configured intervals. This is essential for servers that close idle connections or require regular "ping" messages.

## Architecture

### High-Level Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                        WebSocket Client                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────────────┐         ┌─────────────────────────┐  │
│  │   Heartbeat Task     │         │   Main Message Loop     │  │
│  │   (Tokio spawn)      │         │                         │  │
│  │                      │         │                         │  │
│  │  Tokio Interval      │         │  tokio::select! {       │  │
│  │  Every X seconds:    │         │                         │  │
│  │                      │         │    // Incoming msgs     │  │
│  │  1. ticker.tick()    │         │    msg = read.next()    │  │
│  │  2. Send payload ────┼────────►│                         │  │
│  │  3. Repeat           │ Channel │    // Commands          │  │
│  │                      │         │    cmd = rx.recv()      │  │
│  │  Shutdown signal ◄───┼─────────│                         │  │
│  │  stops task          │         │    // HEARTBEAT         │  │
│  └──────────────────────┘         │    hb = heartbeat_rx ◄──┼──┐
│                                    │    {                    │  │
│                                    │      send(hb) ──────────┼──┼─► WebSocket
│                                    │    }                    │  │   Server
│                                    │  }                      │  │
│                                    └─────────────────────────┘  │
│                                                                 │
│  Unbounded Crossbeam Channel ──────────────────────────────────┘
│  (Never blocks, maximum throughput)
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Details

### 1. Configuration (Builder Pattern)

Both interval AND payload are **required together**:

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .heartbeat(
        Duration::from_secs(30),              // Required: interval
        WsMessage::Text("ping".to_string())   // Required: payload
    )
    .build()
    .await?;
```

### 2. Dedicated Heartbeat Task

When heartbeat is configured, the client spawns a separate Tokio task:

**File**: `libs/hypersockets/src/heartbeat.rs`

```rust
pub async fn heartbeat_task(
    interval: Duration,
    payload: WsMessage,
    heartbeat_tx: Sender<WsMessage>,
    shutdown_rx: Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // Skip first immediate tick
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Send payload via channel
                if heartbeat_tx.send(payload.clone()).is_err() {
                    break; // Channel closed, exit
                }
            }
            _ = async { shutdown_rx.recv().ok() } => {
                break; // Shutdown requested
            }
        }
    }
}
```

**Key Points:**
- Runs independently in its own Tokio task
- Uses `tokio::time::interval` for precise timing
- Skips first immediate tick (waits for first interval)
- Uses `MissedTickBehavior::Skip` - if we're behind, skip missed ticks instead of bursting
- Cleans up gracefully on shutdown or channel closure

### 3. Message Sending (Main Loop Integration)

**File**: `libs/hypersockets/src/client.rs`

The main message loop receives heartbeats via `tokio::select!`:

```rust
tokio::select! {
    // ... handle incoming WebSocket messages ...

    // ... handle command messages ...

    // Handle heartbeat messages from dedicated task
    hb = async {
        if let Some(rx) = heartbeat_rx {
            rx.recv().ok()
        } else {
            std::future::pending().await
        }
    } => {
        if let Some(msg) = hb {
            let tung_msg = ws_message_to_tungstenite(&msg);
            write.send(tung_msg).await?;
            metrics.increment_sent();
        }
    }
}
```

**Key Points:**
- Non-blocking: heartbeat task sends to unbounded channel
- Main loop receives and immediately sends to WebSocket
- Metrics track heartbeat as a sent message
- If heartbeat not configured, `std::future::pending()` prevents this branch from ever completing

### 4. Performance Characteristics

| Aspect | Implementation | Benefit |
|--------|----------------|---------|
| **Concurrency** | Separate Tokio task | Timing independent of message processing |
| **Channels** | Unbounded crossbeam | No backpressure, maximum throughput |
| **Blocking** | Zero blocking | Never delays other operations |
| **Memory** | Payload cloned per send | Minimal - only the configured message |
| **Timing Precision** | `tokio::time::interval` | OS-level precision, not drift-prone |
| **Missed Ticks** | Skip behavior | Prevents message bursts if behind |

## Usage Examples

### Basic Heartbeat

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .heartbeat(
        Duration::from_secs(30),
        WsMessage::Text("ping".to_string())
    )
    .build()
    .await?;
```

### JSON Heartbeat

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .heartbeat(
        Duration::from_secs(15),
        WsMessage::Text(r#"{"type":"heartbeat","timestamp":0}"#.to_string())
    )
    .build()
    .await?;
```

### Binary Heartbeat

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .heartbeat(
        Duration::from_secs(20),
        WsMessage::Binary(vec![0xFF, 0x00, 0x01]) // Custom binary ping
    )
    .build()
    .await?;
```

### High-Frequency Trading (Short Interval)

```rust
let client = hypersockets::builder()
    .url("wss://trade.example.com")
    .parser(MyParser)
    .state(MyState)
    .heartbeat(
        Duration::from_millis(500), // Every 500ms for low latency
        WsMessage::Text("ping".to_string())
    )
    .build()
    .await?;
```

## Monitoring Heartbeats

### Enable Debug Logging

```bash
RUST_LOG=debug cargo run --bin heartbeat_demo
```

You'll see logs like:
```
[DEBUG hypersockets::heartbeat] Heartbeat task started with interval: 5s
[DEBUG hypersockets::heartbeat] Heartbeat tick - sending payload
[DEBUG hypersockets::client] Received heartbeat from heartbeat task, sending to server
[DEBUG hypersockets::client] Heartbeat sent successfully
```

### Check Metrics

```rust
let metrics = client.metrics();
println!("Messages sent: {}", metrics.messages_sent); // Includes heartbeats
```

## Lifecycle

1. **Start**: Heartbeat task spawns when connection is established
2. **Running**: Task sends payload every interval via channel
3. **Main loop**: Receives from channel and sends to WebSocket
4. **Reconnection**: New heartbeat task spawned on reconnect
5. **Shutdown**: Task receives shutdown signal and exits cleanly

## Thread Safety

- ✅ Heartbeat task: Runs on Tokio runtime (async)
- ✅ Channel: Crossbeam unbounded (lock-free, thread-safe)
- ✅ Sending: Via tokio::select! (single-threaded async, no locks needed)

## Testing

Run the heartbeat demonstration:

```bash
# With debug logs
RUST_LOG=debug cargo run --bin heartbeat_demo

# Or basic info logs
cargo run --bin heartbeat_demo
```

The demo:
- Connects to echo.websocket.org
- Sends heartbeat every 5 seconds
- Runs for 30 seconds (~6 heartbeats)
- Shows metrics and confirms heartbeats are working

## Common Patterns

### Cryptocurrency Exchanges

Many crypto exchanges require heartbeats:

```rust
// Binance WebSocket
.heartbeat(
    Duration::from_secs(180), // 3 minutes
    WsMessage::Text(r#"{"method":"ping"}"#.to_string())
)

// Kraken WebSocket
.heartbeat(
    Duration::from_secs(30),
    WsMessage::Text(r#"{"event":"ping"}"#.to_string())
)
```

### Custom Protocols

```rust
// STOMP protocol
.heartbeat(
    Duration::from_secs(10),
    WsMessage::Text("\n".to_string()) // STOMP heartbeat is just newline
)

// MQTT over WebSocket
.heartbeat(
    Duration::from_secs(60),
    WsMessage::Binary(vec![0xC0, 0x00]) // PINGREQ packet
)
```

## Troubleshooting

### Heartbeats Not Sending

1. Check that both interval and payload are configured
2. Enable debug logging: `RUST_LOG=hypersockets=debug`
3. Verify connection is established (check events)
4. Check metrics to see if `messages_sent` increases

### Too Frequent/Infrequent

Adjust the interval:
```rust
.heartbeat(Duration::from_secs(new_interval), payload)
```

### Server Not Responding to Heartbeats

- Verify the payload format matches server expectations
- Check if server requires specific heartbeat message format
- Some servers use WebSocket PING/PONG frames (handled automatically by tungstenite)
- Consider using passive ping detection if server sends pings instead

## See Also

- [Passive Ping Detection](./PASSIVE_PING.md) - For server-initiated pings
- [Reconnection Strategies](./RECONNECTION.md) - Automatic reconnection handling
- [Examples](./bin/) - Working code examples
