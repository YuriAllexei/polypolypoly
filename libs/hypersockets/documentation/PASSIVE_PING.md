# Passive Ping Detection & Response

## Overview

Passive ping detection allows HyperSockets to automatically detect and respond to server-initiated "ping" messages that are sent as regular WebSocket data messages (not WebSocket PING frames).

Many WebSocket APIs, especially cryptocurrency exchanges, send ping messages as JSON or text data rather than using the WebSocket protocol's built-in PING/PONG frames.

## Key Requirement

**Both inbound detection AND outbound response are REQUIRED together.**

When you configure passive ping:
- You MUST specify how to detect pings (inbound pattern matching)
- You MUST specify what to respond with (outbound pong message)

## Architecture

### Message Flow

```text
┌──────────────────────────────────────────────────────────────────┐
│                      WebSocket Client                            │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Every Received Message:                                         │
│                                                                  │
│  WebSocket ──["ping"]──> │                                       │
│                          │                                       │
│                          ├─> is_ping(message) ?                  │
│                          │   ├─> NO  ──> Parse ──> State         │
│                          │   │                                   │
│                          │   └─> YES ──┐                         │
│                          │             │                         │
│                          │             ├─> get_pong_response()   │
│                          │             │                         │
│  WebSocket <─["pong"]────┴─────────────┘                         │
│                          │                                       │
│                          └─> continue (skip parsing)             │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

**Important Flow Details:**
1. **EVERY message** is checked if passive ping is configured
2. If `is_ping()` returns `true`:
   - `get_pong_response()` is called immediately
   - Response is sent to server automatically
   - Message is NOT passed to parser (saves processing)
   - Loop continues with next message
3. If `is_ping()` returns `false`:
   - Message is parsed normally
   - State handler processes it

## Implementation

### File Locations

- **Trait Definition**: `libs/hypersockets-traits/src/passive_ping.rs`
- **Client Integration**: `libs/hypersockets/src/client.rs:375-397`
- **Builder Config**: `libs/hypersockets/src/builder/mod.rs:170-207`

### The PassivePingDetector Trait

```rust
pub trait PassivePingDetector: Send + Sync {
    /// Check if a message is a passive ping (INBOUND)
    fn is_ping(&self, message: &WsMessage) -> bool;

    /// Get the response message (OUTBOUND - REQUIRED)
    fn get_pong_response(&self) -> WsMessage;
}
```

**Key Points:**
- `is_ping()` is called for EVERY received message
- Keep `is_ping()` fast - it's on the hot path!
- `get_pong_response()` returns `WsMessage`, not `Option<WsMessage>`
- Both methods are required - you cannot have detection without response

### Built-in Detectors

#### 1. TextPassivePing

Simple text-based detection using exact string matching.

```rust
use hypersockets::*;

let detector = TextPassivePing::new(
    "ping",                              // Inbound: detect this exact text
    WsMessage::Text("pong".to_string())  // Outbound: send this response
);

let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .passive_ping(detector)
    .build()
    .await?;
```

#### 2. JsonPassivePing

JSON-based detection checking for specific field/value pairs.

```rust
use hypersockets::*;

// Detect {"type":"ping"} and respond with {"type":"pong"}
let detector = JsonPassivePing::new(
    "type",                                          // Field name to check
    "ping",                                          // Value that indicates ping
    WsMessage::Text(r#"{"type":"pong"}"#.to_string()) // Response
);

let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .passive_ping(detector)
    .build()
    .await?;
```

## Usage Examples

### Basic Text Ping/Pong

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .passive_ping(TextPassivePing::new(
        "ping",
        WsMessage::Text("pong".to_string())
    ))
    .build()
    .await?;
```

### JSON Ping Detection

```rust
// Binance-style ping
let client = hypersockets::builder()
    .url("wss://stream.binance.com")
    .parser(MyParser)
    .state(MyState)
    .passive_ping(JsonPassivePing::new(
        "e",      // Event field
        "ping",   // Event value
        WsMessage::Text(r#"{"e":"pong"}"#.to_string())
    ))
    .build()
    .await?;
```

### Custom Detector

For complex detection logic, implement the trait:

```rust
struct CustomPingDetector;

impl PassivePingDetector for CustomPingDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if let Some(text) = message.as_text() {
            // Custom logic - check multiple conditions
            if text.contains("heartbeat") || text.contains("keepalive") {
                return true;
            }

            // Check JSON structure
            if let Ok(json) = serde_json::from_str::<Value>(text) {
                if json.get("action") == Some(&json!("ping")) {
                    return true;
                }
            }
        }
        false
    }

    fn get_pong_response(&self) -> WsMessage {
        // REQUIRED: Return your pong message
        WsMessage::Text(r#"{"action":"pong","timestamp":0}"#.to_string())
    }
}

// Use it
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    .passive_ping(CustomPingDetector)
    .build()
    .await?;
```

### Binary Ping Detection

```rust
struct BinaryPingDetector;

impl PassivePingDetector for BinaryPingDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if let Some(data) = message.as_binary() {
            // Detect specific binary pattern
            data.len() == 2 && data[0] == 0xFF && data[1] == 0x00
        } else {
            false
        }
    }

    fn get_pong_response(&self) -> WsMessage {
        // Binary pong response
        WsMessage::Binary(vec![0xFF, 0x01])
    }
}
```

## Real-World Examples

### Cryptocurrency Exchanges

#### Binance

```rust
.passive_ping(JsonPassivePing::new(
    "e",
    "ping",
    WsMessage::Text(r#"{"e":"pong"}"#.to_string())
))
```

#### Kraken

```rust
.passive_ping(JsonPassivePing::new(
    "event",
    "ping",
    WsMessage::Text(r#"{"event":"pong"}"#.to_string())
))
```

#### OKX

```rust
struct OkxPingDetector;

impl PassivePingDetector for OkxPingDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        message.as_text().map_or(false, |t| t == "ping")
    }

    fn get_pong_response(&self) -> WsMessage {
        WsMessage::Text("pong".to_string())
    }
}
```

#### Coinbase

```rust
.passive_ping(JsonPassivePing::new(
    "type",
    "ping",
    WsMessage::Text(r#"{"type":"pong"}"#.to_string())
))
```

### Custom Protocols

#### STOMP over WebSocket

```rust
struct StompPingDetector;

impl PassivePingDetector for StompPingDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        message.as_text().map_or(false, |t| t.starts_with("PING"))
    }

    fn get_pong_response(&self) -> WsMessage {
        WsMessage::Text("PONG\n".to_string())
    }
}
```

#### Socket.IO-like Protocol

```rust
.passive_ping(JsonPassivePing::new(
    "type",
    "2", // Socket.IO ping packet type
    WsMessage::Text(r#"{"type":"3"}"#.to_string()) // Pong is type 3
))
```

## Performance Considerations

### Hot Path Optimization

Since `is_ping()` is called for **EVERY** message:

✅ **DO:**
- Keep detection logic simple and fast
- Use early returns for quick rejections
- Avoid heavy allocations in `is_ping()`
- Cache parsed JSON if you need to check multiple fields

❌ **DON'T:**
- Perform complex regex matching
- Make network calls
- Do heavy computation
- Allocate unnecessarily

### Example: Optimized Detection

```rust
impl PassivePingDetector for FastDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        // Fast path: check text first
        let Some(text) = message.as_text() else {
            return false; // Binary messages can't be our text ping
        };

        // Quick length check before expensive operations
        if text.len() > 1000 {
            return false; // Our pings are small
        };

        // Now do the real check
        text.contains("\"type\":\"ping\"")
    }

    fn get_pong_response(&self) -> WsMessage {
        // Pre-allocated, cheap to clone
        WsMessage::Text(r#"{"type":"pong"}"#.to_string())
    }
}
```

## Monitoring

### Enable Debug Logs

```bash
RUST_LOG=hypersockets=debug cargo run
```

You'll see:
```
[DEBUG hypersockets::client] Passive ping detected from server
[DEBUG hypersockets::client] Passive pong sent successfully
```

### Track in Metrics

Pong responses are counted in `messages_sent`:

```rust
let metrics = client.metrics();
println!("Messages sent: {}", metrics.messages_sent); // Includes pongs
```

### Custom Tracking

```rust
use std::sync::atomic::{AtomicU64, Ordering};

struct TrackingDetector {
    ping_count: Arc<AtomicU64>,
}

impl PassivePingDetector for TrackingDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if message.as_text().map_or(false, |t| t == "ping") {
            self.ping_count.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn get_pong_response(&self) -> WsMessage {
        WsMessage::Text("pong".to_string())
    }
}

// Later: check count
println!("Pings detected: {}", ping_count.load(Ordering::Relaxed));
```

## Testing

Run the passive ping demonstration:

```bash
# With debug logs
RUST_LOG=debug cargo run --bin passive_ping_demo

# Or basic info logs
cargo run --bin passive_ping_demo
```

The demo:
- Sends test messages with "TEST_PING"
- Echo server echoes them back
- Passive ping detector detects them
- Automatically sends "TEST_PONG" responses
- Shows detection and response working

## Common Patterns

### Multiple Detection Patterns

```rust
struct MultiPatternDetector;

impl PassivePingDetector for MultiPatternDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if let Some(text) = message.as_text() {
            // Check multiple patterns
            text == "ping" ||
            text == "heartbeat" ||
            text.contains(r#""type":"ping""#) ||
            text.contains(r#""event":"ping""#)
        } else {
            false
        }
    }

    fn get_pong_response(&self) -> WsMessage {
        WsMessage::Text("pong".to_string())
    }
}
```

### Stateful Detection

```rust
struct StatefulDetector {
    seen_pings: Arc<Mutex<HashSet<String>>>,
}

impl PassivePingDetector for StatefulDetector {
    fn is_ping(&self, message: &WsMessage) -> bool {
        if let Some(text) = message.as_text() {
            if let Ok(json) = serde_json::from_str::<Value>(text) {
                if let Some(ping_id) = json.get("ping_id").and_then(|v| v.as_str()) {
                    let mut seen = self.seen_pings.lock().unwrap();
                    if !seen.contains(ping_id) {
                        seen.insert(ping_id.to_string());
                        return true;
                    }
                }
            }
        }
        false
    }

    fn get_pong_response(&self) -> WsMessage {
        WsMessage::Text(r#"{"type":"pong"}"#.to_string())
    }
}
```

## Troubleshooting

### Pings Not Being Detected

1. Enable debug logging: `RUST_LOG=hypersockets=debug`
2. Check that `is_ping()` logic matches actual server messages
3. Verify the message format (text vs binary)
4. Print messages in your parser to see what's being received

### Pongs Not Being Sent

1. Check that `get_pong_response()` returns correct format
2. Verify server expects the response you're sending
3. Check metrics to confirm messages are being sent
4. Look for WebSocket errors in logs

### Parser Receiving Ping Messages

If your parser is receiving ping messages, passive ping detection isn't working:
1. Ensure passive ping is configured: `.passive_ping(detector)`
2. Verify `is_ping()` returns `true` for the messages
3. Check debug logs for "Passive ping detected"

## Difference from Heartbeat

| Feature | Heartbeat | Passive Ping |
|---------|-----------|--------------|
| **Direction** | Client → Server | Server → Client (detection) |
| **Trigger** | Time interval | Incoming message |
| **Purpose** | Keep connection alive | Respond to server pings |
| **When to use** | Server expects periodic pings | Server sends ping messages |
| **Implementation** | Dedicated task | Message loop check |
| **Both required?** | Yes (interval + payload) | Yes (detection + response) |

You can use **both** together:
- Heartbeat for sending periodic pings to keep connection alive
- Passive ping for responding to server-initiated pings

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .parser(MyParser)
    .state(MyState)
    // Client-initiated heartbeat every 30s
    .heartbeat(
        Duration::from_secs(30),
        WsMessage::Text("ping".to_string())
    )
    // Server-initiated passive ping detection
    .passive_ping(TextPassivePing::new(
        "server_ping",
        WsMessage::Text("client_pong".to_string())
    ))
    .build()
    .await?;
```

## See Also

- [Heartbeat Mechanism](./HEARTBEAT.md) - Client-initiated periodic pings
- [Reconnection Strategies](./RECONNECTION.md) - Automatic reconnection
- [Examples](./bin/) - Working code examples
