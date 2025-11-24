# Smart Reconnection in HyperSockets

HyperSockets implements **smart reconnection** that automatically handles connection failures and restores full client state on reconnection.

## Overview

When a WebSocket connection is lost, HyperSockets automatically:
1. ✅ Waits for the configured reconnection delay offset
2. ✅ Applies the reconnection strategy delay
3. ✅ Reconnects to the WebSocket server
4. ✅ **Re-authenticates** (if auth is configured)
5. ✅ **Re-subscribes** to all channels
6. ✅ **Restarts heartbeat** task
7. ✅ **Preserves all handlers and routers**

Everything is reinstated - your client resumes exactly where it left off!

## Configuration

### Reconnection Delay Offset

The **reconnection_delay_offset** is applied immediately after disconnection, before the reconnection strategy delay:

```rust
let client = hypersockets::builder()
    .url("wss://api.example.com")
    .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    // Wait 2 seconds after disconnect
    .reconnection_delay_offset(Duration::from_secs(2))
    // Then apply exponential backoff
    .reconnect_strategy(ExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
        Some(10),
    ))
    .build()
    .await?;
```

**Timeline:**
```
Disconnect → [Offset: 2s] → [Strategy Delay: 1s, 2s, 4s...] → Reconnect
```

### Reconnection Strategy

Choose from built-in strategies or implement your own:

```rust
// Exponential backoff: 1s, 2s, 4s, 8s, 16s, ... up to 60s max, 10 retries
.reconnect_strategy(ExponentialBackoff::new(
    Duration::from_secs(1),   // Initial delay
    Duration::from_secs(60),  // Max delay
    Some(10),                 // Max attempts (None = unlimited)
))

// Fixed delay: Always wait 5 seconds, unlimited retries
.reconnect_strategy(FixedDelay::new(Duration::from_secs(5), None))

// Never reconnect
.reconnect_strategy(NeverReconnect)
```

## Smart Reconnection Flow

### Without Authentication

```
┌─────────────────────────────────────────────────────────────┐
│ DISCONNECTED                                                │
├─────────────────────────────────────────────────────────────┤
│ 1. Wait reconnection_delay_offset                           │
│ 2. Wait reconnection strategy delay                         │
│ 3. Attempt reconnection                                     │
├─────────────────────────────────────────────────────────────┤
│ CONNECTED                                                   │
├─────────────────────────────────────────────────────────────┤
│ 4. Send subscriptions (if configured)                       │
│ 5. Start heartbeat task (if configured)                     │
│ 6. Resume message processing                                │
│    ✓ Handlers still active                                  │
│    ✓ Router still active                                    │
│    ✓ Passive ping still active                              │
└─────────────────────────────────────────────────────────────┘
```

### With Authentication (Smart Reconnection)

```
┌─────────────────────────────────────────────────────────────┐
│ DISCONNECTED                                                │
├─────────────────────────────────────────────────────────────┤
│ 1. Wait reconnection_delay_offset                           │
│ 2. Wait reconnection strategy delay                         │
│ 3. Attempt reconnection                                     │
├─────────────────────────────────────────────────────────────┤
│ CONNECTED                                                   │
├─────────────────────────────────────────────────────────────┤
│ 4. ⚡ SEND AUTH MESSAGE (FIRST!)                            │
│ 5. Send subscriptions (after auth)                          │
│ 6. Start heartbeat task                                     │
│ 7. Resume message processing                                │
│    ✓ Handlers still active                                  │
│    ✓ Router still active                                    │
│    ✓ Passive ping still active                              │
└─────────────────────────────────────────────────────────────┘
```

**Key Point:** If authentication is configured, it's **always sent first** on every reconnection!

## State Preservation

### What Gets Preserved

✅ **Handlers** - All message handlers continue processing
✅ **Router** - Message routing logic unchanged
✅ **Subscriptions** - All subscriptions automatically re-sent
✅ **Heartbeat** - New heartbeat task spawned with same config
✅ **Passive Ping** - Detection and response continues
✅ **Configuration** - All settings preserved

### What Gets Reset

🔄 **Heartbeat Task** - New task spawned (old task is stopped)
🔄 **Connection Metrics** - Reconnection counter incremented
🔄 **Connection State** - Updated through state machine

## Complete Example

```rust
use hypersockets::*;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route { Main }

#[derive(Debug, Clone)]
struct Message(String);

struct Router;

#[async_trait::async_trait]
impl MessageRouter for Router {
    type Message = Message;
    type RouteKey = Route;

    async fn parse(&self, msg: WsMessage) -> Result<Self::Message> {
        Ok(Message(msg.as_text().unwrap_or("").to_string()))
    }

    fn route_key(&self, _: &Self::Message) -> Self::RouteKey {
        Route::Main
    }
}

struct Handler;

#[async_trait::async_trait]
impl MessageHandler<Message> for Handler {
    async fn handle(&mut self, msg: Message) -> Result<()> {
        println!("Received: {}", msg.0);
        Ok(())
    }
}

// Custom auth that logs each authentication
struct MyAuth;

#[async_trait::async_trait]
impl AuthProvider for MyAuth {
    async fn get_auth_message(&self) -> Result<Option<WsMessage>> {
        println!("🔐 Authenticating...");
        Ok(Some(WsMessage::Text(r#"{"type":"auth","key":"secret"}"#.to_string())))
    }

    async fn validate_auth_response(&self, _: &WsMessage) -> Result<bool> {
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = hypersockets::builder()
        .url("wss://api.example.com")
        .router(Router, |routing| routing.handler(Route::Main, Handler))
        // Auth will be sent on EVERY connection/reconnection
        .auth(MyAuth)
        // Wait 2s after disconnect before reconnecting
        .reconnection_delay_offset(Duration::from_secs(2))
        // Exponential backoff with 10 max retries
        .reconnect_strategy(ExponentialBackoff::new(
            Duration::from_secs(1),
            Duration::from_secs(60),
            Some(10),
        ))
        // Subscriptions sent after auth on every connection
        .subscription(WsMessage::Text(r#"{"action":"subscribe","channel":"trades"}"#.to_string()))
        .subscription(WsMessage::Text(r#"{"action":"subscribe","channel":"orders"}"#.to_string()))
        // Heartbeat restarted on every connection
        .heartbeat(Duration::from_secs(30), WsMessage::Text("ping".to_string()))
        .build()
        .await?;

    // Monitor events
    loop {
        while let Some(event) = client.try_recv_event() {
            match event {
                ClientEvent::Connected => println!("✓ Connected!"),
                ClientEvent::Disconnected => println!("✗ Disconnected"),
                ClientEvent::Reconnecting(n) => println!("⟳ Reconnecting (attempt {})", n),
                ClientEvent::Error(e) => println!("⚠ Error: {}", e),
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

## Monitoring Reconnections

Track reconnection attempts using events:

```rust
loop {
    match client.recv_event() {
        Ok(ClientEvent::Disconnected) => {
            println!("Connection lost!");
        }
        Ok(ClientEvent::Reconnecting(attempt)) => {
            println!("Reconnection attempt #{}", attempt);
        }
        Ok(ClientEvent::Connected) => {
            println!("Reconnected successfully!");
            // Auth already sent
            // Subscriptions already sent
            // Heartbeat already started
            // Handlers ready!
        }
        Ok(ClientEvent::Error(e)) => {
            eprintln!("Error: {}", e);
        }
        Err(_) => break,
    }
}
```

Check metrics:

```rust
let metrics = client.metrics();
println!("Reconnections: {}", metrics.reconnect_count);
println!("State: {:?}", metrics.connection_state);
```

## Best Practices

1. **Set appropriate offset** - Give servers time to clean up:
   ```rust
   .reconnection_delay_offset(Duration::from_secs(2))
   ```

2. **Limit retries for critical systems** - Avoid infinite loops:
   ```rust
   .reconnect_strategy(ExponentialBackoff::new(
       Duration::from_secs(1),
       Duration::from_secs(60),
       Some(10),  // Stop after 10 attempts
   ))
   ```

3. **Always configure auth if required** - It's automatic:
   ```rust
   .auth(MyAuth)  // Sent on every connection!
   ```

4. **Use exponential backoff** - Be nice to servers:
   ```rust
   .reconnect_strategy(ExponentialBackoff::new(...))
   ```

5. **Monitor events** - Track reconnection health:
   ```rust
   if let ClientEvent::Reconnecting(n) = event {
       if n > 5 {
           warn!("Multiple reconnection attempts!");
       }
   }
   ```

## Architecture

The reconnection system is designed for:

- **Zero data loss** - All handlers preserved
- **Automatic recovery** - No manual intervention
- **Server-friendly** - Configurable delays
- **Stateful** - Auth and subscriptions automatic
- **Observable** - Events for monitoring

Every reconnection fully restores your client state!
