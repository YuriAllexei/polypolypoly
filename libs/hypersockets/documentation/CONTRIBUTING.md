# Contributing to HyperSockets

Thank you for your interest in contributing to HyperSockets! This document provides guidelines and information for contributors.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)
- [Performance Guidelines](#performance-guidelines)
- [Documentation](#documentation)

---

## Getting Started

### Prerequisites

- Rust 1.70 or later
- Familiarity with async Rust and Tokio
- Understanding of WebSocket protocol
- Basic knowledge of atomic operations and channels

### Areas for Contribution

- 🐛 **Bug Fixes** - Fix issues, improve error handling
- ✨ **Features** - Implement new capabilities
- 📚 **Documentation** - Improve docs, add examples
- 🎯 **Performance** - Optimize hot paths, reduce allocations
- 🧪 **Testing** - Add tests, improve coverage
- 🔧 **API Improvements** - Simplifications from [API_IMPROVEMENTS.md](API_IMPROVEMENTS.md)

---

## Development Setup

### Clone and Build

```bash
git clone https://github.com/yourusername/hypersockets.git
cd hypersockets

# Build all workspace members
cargo build --all

# Run tests
cargo test --all

# Run examples
cargo run --bin basic_example
cargo run --bin routing_example
```

### Development Tools

**Recommended**:
```bash
# Install cargo-watch for auto-rebuilds
cargo install cargo-watch

# Install clippy for linting
rustup component add clippy

# Install rustfmt for formatting
rustup component add rustfmt
```

**Usage**:
```bash
# Auto-rebuild on changes
cargo watch -x build

# Run clippy
cargo clippy --all -- -D warnings

# Format code
cargo fmt --all
```

---

## Project Structure

```
hypersockets/
├── libs/
│   ├── hypersockets-traits/       # Core trait definitions
│   │   ├── src/
│   │   │   ├── lib.rs              # Module exports
│   │   │   ├── router.rs           # MessageRouter, MessageHandler
│   │   │   ├── auth.rs             # AuthProvider
│   │   │   ├── headers.rs          # HeaderProvider
│   │   │   ├── passive_ping.rs     # PassivePingDetector
│   │   │   ├── reconnect.rs        # ReconnectionStrategy
│   │   │   ├── parser.rs           # WsMessage type
│   │   │   ├── state.rs            # StateHandler (legacy)
│   │   │   └── error.rs            # Error types
│   │   └── Cargo.toml
│   │
│   ├── hypersockets/               # Main WebSocket client
│   │   ├── src/
│   │   │   ├── lib.rs              # Public API exports
│   │   │   ├── client.rs           # WebSocketClient impl
│   │   │   ├── config.rs           # ClientConfig
│   │   │   ├── builder/            # Builder pattern
│   │   │   │   ├── mod.rs          # Builder implementation
│   │   │   │   └── states.rs       # Type-state markers
│   │   │   ├── connection_state.rs # Atomic state management
│   │   │   └── heartbeat.rs        # Heartbeat task
│   │   └── Cargo.toml
│   │
│   └── hypersockets-manager/       # Multi-client manager
│       ├── src/
│       │   ├── lib.rs
│       │   └── manager.rs          # ClientManager impl
│       └── Cargo.toml
│
├── bin/                            # Example binaries
│   ├── basic_example.rs
│   ├── routing_example.rs
│   ├── advanced_example.rs
│   ├── heartbeat_demo.rs
│   ├── passive_ping_demo.rs
│   ├── headers_example.rs
│   └── multi_client_example.rs
│
├── Cargo.toml                      # Workspace definition
├── README.md                       # Main documentation
├── HEARTBEAT.md                    # Heartbeat guide
├── PASSIVE_PING.md                 # Passive ping guide
├── SMART_RECONNECTION.md           # Reconnection guide
├── ARCHITECTURE.md                 # Internal design
├── API_IMPROVEMENTS.md             # Simplification proposals
└── CONTRIBUTING.md                 # This file
```

### Key Files

- **`libs/hypersockets/src/client.rs`** - Core client logic, message loop
- **`libs/hypersockets/src/builder/mod.rs`** - Builder pattern implementation
- **`libs/hypersockets-traits/src/router.rs`** - Routing system traits
- **`libs/hypersockets/src/connection_state.rs`** - Atomic state management

---

## Coding Standards

### Rust Style

Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/):

```rust
// ✅ Good: Clear naming, documented
/// Sends a message through the WebSocket connection.
///
/// # Errors
/// Returns an error if the client is shut down.
pub fn send(&self, message: WsMessage) -> Result<()> {
    self.command_tx
        .send(ClientCommand::Send(message))
        .map_err(|e| HyperSocketError::ChannelSend(e.to_string()))
}

// ❌ Bad: Unclear, undocumented
pub fn snd(&self, m: WsMessage) -> Result<()> {
    self.cmd_tx.send(ClientCommand::Send(m)).map_err(|e| HyperSocketError::ChannelSend(e.to_string()))
}
```

### Naming Conventions

- **Types**: `PascalCase` (e.g., `WebSocketClient`, `MessageRouter`)
- **Functions**: `snake_case` (e.g., `connect_async`, `send_message`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `MAX_MESSAGE_SIZE`)
- **Traits**: Descriptive nouns (e.g., `MessageRouter`, `AuthProvider`)

### Code Organization

```rust
// Order: use statements, types, impl blocks, functions

// 1. Imports (grouped and sorted)
use std::sync::Arc;
use std::time::Duration;

use hypersockets_traits::*;
use tokio::task::JoinHandle;

// 2. Type definitions
pub struct WebSocketClient<R, M> { ... }

pub enum ClientEvent { ... }

// 3. Implementation blocks
impl<R, M> WebSocketClient<R, M> { ... }

// 4. Helper functions
fn helper_function() { ... }
```

### Error Handling

```rust
// ✅ Good: Descriptive errors
Err(HyperSocketError::Parse(format!(
    "Invalid message type: expected 'trade', got '{}'",
    msg_type
)))

// ❌ Bad: Generic errors
Err(HyperSocketError::Parse("invalid".into()))
```

---

## Testing

### Unit Tests

Place tests in the same file as the code being tested:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_transitions() {
        let state = AtomicConnectionState::new(ConnectionState::Disconnected);
        assert!(state.is_disconnected());

        state.set(ConnectionState::Connected);
        assert!(state.is_connected());
    }

    #[tokio::test]
    async fn test_router_parse() {
        let router = MyRouter;
        let msg = WsMessage::Text(r#"{"type":"test"}"#.into());
        let parsed = router.parse(msg).await.unwrap();
        assert_eq!(router.route_key(&parsed), MessageType::Test);
    }
}
```

### Integration Tests

Create integration tests in `tests/` directory:

```bash
tests/
├── integration_test.rs
├── router_test.rs
└── reconnection_test.rs
```

```rust
use hypersockets::*;
use std::time::Duration;

#[tokio::test]
async fn test_full_client_lifecycle() {
    let client = hypersockets::builder()
        .url("wss://echo.websocket.org")
        .router(TestRouter, |routing| {
            routing.handler(Route::Main, TestHandler::new())
        })
        .build()
        .await
        .unwrap();

    client.send(WsMessage::Text("test".into())).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let metrics = client.metrics();
    assert!(metrics.messages_sent > 0);

    client.shutdown().await.unwrap();
}
```

### Running Tests

```bash
# Run all tests
cargo test --all

# Run specific test
cargo test test_connection_state

# Run with output
cargo test -- --nocapture

# Run integration tests only
cargo test --test integration_test
```

---

## Pull Request Process

### Before Submitting

1. ✅ **Format code**: `cargo fmt --all`
2. ✅ **Run clippy**: `cargo clippy --all -- -D warnings`
3. ✅ **Run tests**: `cargo test --all`
4. ✅ **Update documentation**: Add/update docs for new features
5. ✅ **Add examples**: If adding features, add or update examples

### PR Template

```markdown
## Description
Brief description of changes.

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Performance improvement
- [ ] Documentation update
- [ ] Code refactoring

## Testing
Describe how you tested the changes.

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Comments added for complex code
- [ ] Documentation updated
- [ ] Tests added/updated
- [ ] All tests passing
- [ ] No new warnings from clippy
```

### Review Process

1. **Automated Checks** - CI runs tests, clippy, fmt
2. **Code Review** - Maintainer reviews code
3. **Discussion** - Address feedback, make changes
4. **Approval** - Once approved, PR is merged

---

## Performance Guidelines

### Hot Path Optimization

Code on the hot path (every message) must be optimized:

```rust
// ✅ Good: Fast, inline, no allocations
#[inline]
pub fn is_ping(&self, message: &WsMessage) -> bool {
    if let Some(text) = message.as_text() {
        text == self.ping_text
    } else {
        false
    }
}

// ❌ Bad: Slow, allocations, unnecessary work
pub fn is_ping(&self, message: &WsMessage) -> bool {
    let json: serde_json::Value = serde_json::from_str(
        message.as_text().unwrap_or("")
    ).ok()?;
    json["type"].as_str() == Some("ping")
}
```

### Memory Allocation

Minimize allocations in performance-critical code:

```rust
// ✅ Good: Arc-shared, no clones
let config = Arc::new(config);
let config_clone = Arc::clone(&config);

// ❌ Bad: Full clone
let config_clone = config.clone();
```

### Atomic Operations

Use appropriate memory ordering:

```rust
// ✅ Good: Relaxed for counters
self.count.fetch_add(1, Ordering::Relaxed)

// ✅ Good: Acquire/Release for synchronization
self.state.store(new_state, Ordering::Release);
let state = self.state.load(Ordering::Acquire);

// ❌ Bad: SeqCst everywhere (too strong, slower)
self.count.fetch_add(1, Ordering::SeqCst)
```

### Benchmarking

Add benchmarks for performance-critical code:

```rust
#[cfg(test)]
mod benches {
    use super::*;
    use criterion::{black_box, Criterion};

    pub fn bench_parse(c: &mut Criterion) {
        let router = MyRouter;
        let msg = WsMessage::Text(r#"{"type":"test"}"#.into());

        c.bench_function("parse_message", |b| {
            b.iter(|| {
                let _ = black_box(router.parse(black_box(msg.clone())));
            });
        });
    }
}
```

---

## Documentation

### Code Documentation

All public APIs must be documented:

```rust
/// Trait for parsing and routing WebSocket messages.
///
/// Implementations define how raw WebSocket messages are parsed into
/// typed messages and how those messages are routed to handlers.
///
/// # Type Parameters
/// - `Message`: The parsed message type
/// - `RouteKey`: The key used to route messages to handlers
///
/// # Examples
/// ```
/// struct MyRouter;
///
/// #[async_trait]
/// impl MessageRouter for MyRouter {
///     type Message = MyMessage;
///     type RouteKey = Route;
///
///     async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
///         // Parse logic...
///         Ok(MyMessage::Text(message.as_text()?.to_string()))
///     }
///
///     fn route_key(&self, message: &Self::Message) -> Self::RouteKey {
///         Route::Main
///     }
/// }
/// ```
#[async_trait]
pub trait MessageRouter: Send + Sync + 'static {
    /// The parsed message type.
    type Message: Send + Debug + 'static;

    /// The route key type used to determine which handler receives the message.
    type RouteKey: Hash + Eq + Clone + Send + Sync + Debug + 'static;

    /// Parse a raw WebSocket message into a typed message.
    ///
    /// # Errors
    /// Returns an error if parsing fails.
    async fn parse(&self, message: WsMessage) -> Result<Self::Message>;

    /// Determine the route key for a parsed message.
    ///
    /// This method must be fast as it's called for every message.
    fn route_key(&self, message: &Self::Message) -> Self::RouteKey;
}
```

### Documentation Guidelines

1. **Public API**: Comprehensive docs with examples
2. **Internal code**: Comments explaining "why", not "what"
3. **Complex logic**: Step-by-step explanation
4. **Performance notes**: Document hot paths and optimizations

### Examples

Every feature should have an example:

```bash
# Add example to bin/ directory
bin/my_feature_example.rs

# Add to workspace Cargo.toml
[[bin]]
name = "my_feature_example"
path = "bin/my_feature_example.rs"

# Document in README.md
### My Feature Example

See [bin/my_feature_example.rs](bin/my_feature_example.rs)

**Run**: `cargo run --bin my_feature_example`
```

---

## Questions?

- **Issues**: [GitHub Issues](https://github.com/yourusername/hypersockets/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourusername/hypersockets/discussions)
- **Documentation**: [README.md](README.md), [ARCHITECTURE.md](ARCHITECTURE.md)

---

**Thank you for contributing to HyperSockets!** 🎉
