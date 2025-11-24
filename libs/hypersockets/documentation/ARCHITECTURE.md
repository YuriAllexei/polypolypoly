# HyperSockets Architecture

This document provides an in-depth look at HyperSockets' internal design, implementation details, and architectural decisions.

## Table of Contents

- [Overview](#overview)
- [Core Architecture](#core-architecture)
- [Type System Design](#type-system-design)
- [Message Flow](#message-flow)
- [Concurrency Model](#concurrency-model)
- [Memory Management](#memory-management)
- [Performance Optimization](#performance-optimization)
- [State Management](#state-management)
- [Error Handling](#error-handling)
- [Design Rationale](#design-rationale)
- [Future Considerations](#future-considerations)

---

## Overview

HyperSockets is built on three core principles:

1. **Zero-cost abstractions** - Generics and traits compile away to direct calls
2. **Lock-free hot paths** - Atomics and channels eliminate contention
3. **Unbounded by design** - Trading memory for throughput in financial contexts

### Design Goals

- Maximum throughput for burst message volumes
- Per-type message ordering (FIFO within type)
- Cross-type parallelism (different types processed simultaneously)
- Automatic reconnection with full state restoration
- Type-safe configuration at compile time

---

## Core Architecture

### Three-Layer Design

```
┌─────────────────────────────────────────────────────┐
│  Presentation Layer (Builder, Public API)          │
│  - Type-state builder pattern                       │
│  - Public client methods                            │
│  - Event streaming                                  │
└────────────────┬────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────┐
│  Control Layer (Client Logic)                       │
│  - Main message loop                                │
│  - Connection management                            │
│  - Reconnection logic                               │
│  - Command processing                               │
└────────────────┬────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────┐
│  Transport Layer (WebSocket, Channels)              │
│  - Tungstenite WebSocket                            │
│  - Crossbeam unbounded channels                     │
│  - Tokio async runtime                              │
└─────────────────────────────────────────────────────┘
```

### Component Relationships

```
WebSocketClient
  ├─ ClientConfig (Arc)
  │    ├─ MessageRouter (Arc)
  │    ├─ Route Senders (HashMap<RouteKey, Sender>)
  │    ├─ AuthProvider (Option<Arc>)
  │    ├─ HeaderProvider (Option<Arc>)
  │    ├─ PassivePingDetector (Option<Arc>)
  │    ├─ ReconnectionStrategy (Box)
  │    └─ Subscriptions (Vec)
  ├─ AtomicConnectionState (Arc)
  ├─ AtomicMetrics (Arc)
  ├─ Command Channel (Sender/Receiver)
  ├─ Event Channel (Sender/Receiver)
  ├─ Main Task (JoinHandle)
  └─ Handler Tasks (Vec<JoinHandle>)
```

---

## Type System Design

### Type-State Pattern

The builder uses phantom types to enforce compile-time constraints:

```rust
pub struct WebSocketClientBuilder<U, Ro, R, M>
where
    U: UrlState,      // NoUrl | HasUrl
    Ro: RouterState,  // NoRouter | HasRouter
{
    // ... fields ...
}

// Type progression:
// 1. WebSocketClientBuilder<NoUrl, NoRouter, (), ()>
// 2. .url() -> WebSocketClientBuilder<HasUrl, NoRouter, (), ()>
// 3. .router() -> WebSocketClientBuilder<HasUrl, HasRouter, R, M>
// 4. .build() -> Only available at step 3
```

**Benefits**:
- Compile-time guarantee of required configuration
- Impossible to build invalid client
- IDE autocomplete guides correct usage
- Zero runtime overhead (types erased at compile time)

### Generic Type Parameters

```rust
pub struct WebSocketClient<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
```

**Why Generics**:
- **Zero-cost abstraction**: No dynamic dispatch
- **Monomorphization**: Compiler generates specialized code per type
- **Type safety**: Message/RouteKey types checked at compile time
- **Performance**: Direct function calls, no v-table lookups

**Trade-offs**:
- ✅ Maximum performance (no virtual calls)
- ✅ Type safety
- ❌ Code size (one copy per <R, M> combination)
- ❌ Longer compile times

---

## Message Flow

### Detailed Flow Diagram

```
┌──────────────────────────────────────────────────────────────┐
│ 1. WebSocket receives message                                │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 2. tungstenite_to_ws_message()                               │
│    Convert tungstenite::Message → WsMessage                  │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 3. Passive Ping Check (HOT PATH)                             │
│    if detector.is_ping(&message) {                           │
│      send pong immediately                                   │
│      SKIP parsing ────────────┐                              │
│    }                           │                              │
└──────────────────┬─────────────┼──────────────────────────────┘
                   │             │
                   │             └─> [End, message consumed]
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 4. Spawn Parse Task                                          │
│    tokio::spawn(async move {                                 │
└──────────────────┬───────────────────────────────────────────┘
                   │ (parallel, non-blocking)
┌──────────────────▼───────────────────────────────────────────┐
│ 5. Router.parse(message) → Result<Message>                   │
│    - Parse JSON, extract fields                              │
│    - Validate structure                                      │
│    - Transform to typed message                              │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 6. Router.route_key(&message) → RouteKey                     │
│    - Determine handler by message type                       │
│    - Fast, synchronous (no await)                            │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 7. Lookup handler channel                                    │
│    route_senders.get(&route_key)                             │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 8. Send to channel (lock-free)                               │
│    sender.send(message)                                      │
│    Unbounded channel, never blocks                           │
└──────────────────┬───────────────────────────────────────────┘
                   │
┌──────────────────▼───────────────────────────────────────────┐
│ 9. Handler Task receives (FIFO queue)                        │
│    while let Ok(message) = receiver.recv() {                 │
└──────────────────┬───────────────────────────────────────────┘
                   │ (sequential per route key)
┌──────────────────▼───────────────────────────────────────────┐
│ 10. Handler.handle(message)                                  │
│     User-defined async processing                            │
└──────────────────────────────────────────────────────────────┘
```

### Parallelism Visualization

```
Time →

Message A (RouteKey::Trade)    ┌──Parse──┐     ┌Handler─────┐
                                │ (async) │ ──► │ (sequential)│
                                └─────────┘     └────────────┘

Message B (RouteKey::Trade)          ┌──Parse──┐   ┌Handler─┐
                                      │ (async) │──►│(queued)│
                                      └─────────┘   └────────┘

Message C (RouteKey::Order)    ┌──Parse──┐     ┌Handler─────┐
                                │ (async) │ ──► │ (parallel) │
                                └─────────┘     └────────────┘

Message D (RouteKey::Book)     ┌──Parse──┐     ┌Handler─────┐
                                │ (async) │ ──► │ (parallel) │
                                └─────────┘     └────────────┘

Key:
- Parsing: Parallel (each in own task)
- Same RouteKey: Sequential (queued in channel)
- Different RouteKey: Parallel (different channels/tasks)
```

---

## Concurrency Model

### Task Architecture

```
Main Runtime
  │
  ├─ Main Client Task
  │    ├─ Connection loop
  │    ├─ Message receive loop
  │    ├─ Command handling
  │    └─ Reconnection logic
  │
  ├─ Parse Tasks (spawned per message)
  │    ├─ Parse Task 1 ──┐
  │    ├─ Parse Task 2 ──┤ (parallel)
  │    └─ Parse Task 3 ──┘
  │
  ├─ Handler Tasks (one per route key)
  │    ├─ Handler Task A (RouteKey::Trade)
  │    ├─ Handler Task B (RouteKey::Order)
  │    └─ Handler Task C (RouteKey::Book)
  │
  └─ Heartbeat Task (if configured)
       └─ Interval timer + send loop
```

### Thread Safety

**Lock-Free Components**:
- `AtomicConnectionState` - Atomic U8 with Acquire/Release ordering
- `AtomicMetrics` - Atomic U64 counters with Relaxed ordering
- Crossbeam channels - Lock-free MPSC implementation

**Synchronized Components**:
- None in hot path!
- ClientManager uses RwLock for client map (cold path)

### Ordering Guarantees

**What is guaranteed**:
- Messages with same RouteKey processed in FIFO order
- Handler sees messages in order they were routed
- Reconnection preserves handler task (same queue continues)

**What is NOT guaranteed**:
- Order across different RouteKeys
- Parse order relative to receive order (parsing is parallel)
- Timing of parse completion (non-deterministic)

---

## Memory Management

### Allocation Strategy

**One-time allocations** (setup):
- Client config (Arc)
- Builder fields
- Channel allocations
- Task spawning

**Per-message allocations** (hot path):
- WsMessage clone for routing (necessary)
- Parse task spawn (minimal overhead)
- Parsed message (user-defined, varies)

**Avoided allocations** (optimized):
- No String allocations in passive ping check
- No copies of Arc-wrapped config
- No mutex allocations
- Reused channel capacity

### Arc Usage

```rust
// Config shared across all tasks (read-only)
config: Arc<ClientConfig<R, M>>

// Router shared across parse tasks
router: Arc<R>

// Providers shared (called on reconnection)
auth: Option<Arc<dyn AuthProvider>>
headers: Option<Arc<dyn HeaderProvider>>
```

**Why Arc**:
- **Cheap cloning**: Atomic reference count increment
- **No copies**: All tasks share same instance
- **Thread-safe**: Reference counting is atomic

**Cost**: One atomic increment per clone, one atomic decrement per drop

### Channel Memory

```rust
// Unbounded crossbeam channels
crossbeam_channel::unbounded<M>()
```

**Memory growth**:
- Channel is a linked list of segments
- Grows as messages queue up
- No memory limit (unbounded)

**Why unbounded**:
- Trading scenario: Bursts of thousands of messages per second
- Bounded channels would drop messages or block sender
- Memory growth acceptable (messages processed quickly)

**Monitoring**:
Users should track `messages_received` vs handler processing rate to detect issues.

---

## Performance Optimization

### Hot Path Identification

**Hot path code** (executed for every message):
```rust
// 1. Passive ping check
detector.is_ping(&message)  // Must be O(1)

// 2. Route key extraction
router.route_key(&message)  // Must be O(1)

// 3. Channel send
sender.send(message)  // Lock-free, O(1)
```

**Cold path code** (infrequent):
- Connection establishment
- Reconnection logic
- Auth generation
- Builder configuration

### Inlining Strategy

```rust
#[inline]
pub fn is_connected(&self) -> bool {
    self.state.is_connected()
}

#[inline]
pub fn get(&self) -> ConnectionState {
    ConnectionState::from_u8(self.state.load(Ordering::Acquire))
}
```

**When to inline**:
- Small functions (<10 lines)
- Hot path functions
- Accessor methods
- State checks

### Memory Ordering

```rust
// Metrics: Relaxed (no synchronization needed)
self.messages_sent.fetch_add(1, Ordering::Relaxed);

// State: Acquire/Release (synchronization needed)
self.state.store(new_state, Ordering::Release);
let state = self.state.load(Ordering::Acquire);
```

**Ordering costs** (from fastest to slowest):
1. **Relaxed**: No synchronization, just atomic operation
2. **Acquire/Release**: Synchronizes with Release/Acquire pair
3. **SeqCst**: Full sequential consistency (slowest, not used)

### Avoiding Allocations

```rust
// ✅ Good: No allocation
if let Some(text) = message.as_text() {
    text == "ping"
}

// ❌ Bad: String allocation
message.as_text().unwrap_or("").to_string() == "ping".to_string()
```

---

## State Management

### Connection State Machine

```
Initial State: Disconnected

┌─────────────┐
│ Disconnected│◄───────────────┐
└──────┬──────┘                │
       │                       │
       │ connect()             │ disconnect
       │                       │
┌──────▼──────┐                │
│ Connecting  │                │
└──────┬──────┘                │
       │                       │
       │ success               │
       │                       │
┌──────▼──────┐                │
│  Connected  ├────────────────┘
└──────┬──────┘
       │
       │ error/disconnect
       │
┌──────▼────────┐
│ Reconnecting  │──┐
└───────────────┘  │
       ▲           │
       └───────────┘
        (retry loop)

Special State:
┌──────────────┐
│ShuttingDown  │ (terminal, no reconnection)
└──────────────┘
```

### Atomic State Implementation

```rust
#[repr(u8)]
pub enum ConnectionState {
    Disconnected = 0,
    Connecting = 1,
    Connected = 2,
    Reconnecting = 3,
    ShuttingDown = 4,
}

pub struct AtomicConnectionState {
    state: AtomicU8,
}

impl AtomicConnectionState {
    #[inline]
    pub fn get(&self) -> ConnectionState {
        ConnectionState::from_u8(self.state.load(Ordering::Acquire))
    }

    #[inline]
    pub fn set(&self, new: ConnectionState) {
        self.state.store(new as u8, Ordering::Release);
    }
}
```

**Why U8**:
- Atomic operations on U8 supported on all platforms
- Smaller than bool or larger integers
- 5 states fit in 3 bits (plenty of room)

---

## Error Handling

### Error Types

```rust
pub enum HyperSocketError {
    WebSocket(String),         // WebSocket protocol errors
    Auth(String),              // Authentication failures
    Parse(String),             // Message parsing errors
    ChannelSend(String),       // Channel send failures
    ConnectionClosed(String),  // Connection closed unexpectedly
    InvalidConfiguration(String), // Builder configuration errors
}
```

### Error Propagation

```rust
// Errors in parse task are logged, not propagated
tokio::spawn(async move {
    match router.parse(ws_msg).await {
        Ok(message) => { /* route */ },
        Err(e) => error!("Parse error: {}", e),  // Log, don't crash
    }
});

// Errors in handler are logged, task continues
async fn handler_task(receiver: Receiver<M>) {
    while let Ok(message) = receiver.recv() {
        if let Err(e) = handler.handle(message).await {
            error!("Handler error: {}", e);  // Log, continue
        }
    }
}
```

### Recovery Strategy

- **Parse errors**: Logged, message discarded, continue processing
- **Handler errors**: Logged, handler continues with next message
- **Connection errors**: Trigger reconnection if strategy allows
- **Fatal errors**: Shutdown client, user notified via events

---

## Design Rationale

### Why Unbounded Channels?

**Context**: Cryptocurrency exchanges during high volatility

**Problem with bounded channels**:
```rust
// Bounded channel (say, capacity 1000)
let (tx, rx) = bounded(1000);

// During burst: 5000 messages/second
for msg in messages {
    tx.send(msg)?;  // BLOCKS after 1000 messages!
    // Or drops messages if using try_send
}
```

**Solution: Unbounded channels**:
```rust
// Unbounded channel
let (tx, rx) = unbounded();

// During burst: 5000 messages/second
for msg in messages {
    tx.send(msg)?;  // NEVER blocks, queue grows
}
```

**Trade-offs**:
- ✅ Zero message loss
- ✅ No backpressure blocking WebSocket receive
- ✅ Handlers process at their own pace
- ❌ Memory can grow if handler too slow
- ❌ No automatic throttling

**Monitoring**: Users track metrics to detect slow handlers.

### Why Arc for Config?

**Alternatives considered**:

1. **Clone config per task** - Wasteful, large struct
2. **Global static** - Not flexible, lifetime issues
3. **Arc** - Cheap reference count, shared immutable data ✅

**Cost analysis**:
- Arc::clone() = atomic increment (~5 CPU cycles)
- Config copy = hundreds of bytes, deep copies
- Arc wins by orders of magnitude

### Why Generics instead of Trait Objects?

**Trait objects** (`Box<dyn MessageRouter>`):
```rust
pub struct WebSocketClient {
    router: Box<dyn MessageRouter>,  // Dynamic dispatch
}
```

**Generics** (`<R: MessageRouter>`):
```rust
pub struct WebSocketClient<R: MessageRouter> {
    router: R,  // Static dispatch
}
```

**Comparison**:

| Aspect | Trait Objects | Generics |
|--------|---------------|----------|
| Dispatch | Virtual call (~10-20ns) | Direct call (~1ns) |
| Code size | Single copy | Copy per type |
| Flexibility | Runtime polymorphism | Compile-time only |
| Performance | Slower (v-table lookup) | Faster (inlined) |

**Decision**: Generics for maximum performance (financial trading scenario).

### Why Type-State Pattern?

**Alternative**: Runtime validation
```rust
impl WebSocketClientBuilder {
    pub fn build(self) -> Result<WebSocketClient> {
        if self.url.is_none() {
            return Err("URL not set");  // Runtime error
        }
        if self.router.is_none() {
            return Err("Router not set");  // Runtime error
        }
        // ...
    }
}
```

**Type-state approach**:
```rust
impl WebSocketClientBuilder<HasUrl, HasRouter, R, M> {
    pub fn build(self) -> WebSocketClient<R, M> {
        // Impossible to call without URL and Router!
        // Compiler enforces correctness
    }
}
```

**Benefits**:
- Errors caught at compile time
- IDE autocomplete guides user
- Impossible states are unrepresentable
- Zero runtime cost

---

## Future Considerations

### Potential Optimizations

1. **Object pooling** for WsMessage allocations
2. **Bounded channels with backpressure signals** for slow handler detection
3. **SIMD optimizations** for passive ping detection
4. **Custom allocator** for channel segments

### API Evolution

1. **Closure-based handlers** - Reduce boilerplate (see API_IMPROVEMENTS.md)
2. **Async closures** - When stabilized in Rust
3. **GATs (Generic Associated Types)** - More flexible router API

### Monitoring Enhancements

1. **Queue depth tracking** - Per-handler channel depth metrics
2. **Parse time tracking** - Identify slow parsers
3. **Handler performance metrics** - Time per handle() call
4. **Memory usage tracking** - Total memory consumed by channels

### Platform Support

**Current**: All platforms supported by Tokio and crossbeam

**Future considerations**:
- **WASM support** - Browser-based WebSocket clients
- **no_std support** - Embedded systems (challenging with async)
- **Alternative runtimes** - async-std, smol (currently Tokio-only)

---

## Conclusion

HyperSockets' architecture prioritizes:

1. **Performance** - Lock-free, zero-cost abstractions, optimized hot paths
2. **Safety** - Type-state builder, comprehensive error handling
3. **Flexibility** - Trait-based modularity, generic design
4. **Reliability** - Automatic reconnection, state restoration

The design makes deliberate trade-offs (unbounded channels, generic bloat) that align with the target use case: high-frequency trading and cryptocurrency exchange connections where throughput and reliability outweigh memory and compile time concerns.

For questions or deeper technical discussions, see [CONTRIBUTING.md](CONTRIBUTING.md) or open a GitHub discussion.
