# API Simplification Proposals

This document outlines proposed improvements to make HyperSockets simpler to use while maintaining its extreme performance and modularity.

## Table of Contents

- [Overview](#overview)
- [Proposal 1: Closure-Based Handlers](#proposal-1-closure-based-handlers)
- [Proposal 2: Default Reconnection Strategy](#proposal-2-default-reconnection-strategy)
- [Proposal 3: Simplified Single-Handler API](#proposal-3-simplified-single-handler-api)
- [Proposal 4: Handler Convenience Macro](#proposal-4-handler-convenience-macro)
- [Proposal 5: Builder Method Grouping](#proposal-5-builder-method-grouping)
- [Proposal 6: Implicit Route Key for Single Handler](#proposal-6-implicit-route-key-for-single-handler)
- [Implementation Priority](#implementation-priority)

---

## Overview

HyperSockets currently requires users to implement traits for even simple use cases. While this provides maximum flexibility and type safety, it can be verbose for common scenarios. These proposals aim to reduce boilerplate while maintaining backward compatibility and zero-cost abstractions.

**Guiding Principles**:
- ✅ Maintain all existing APIs (backward compatible)
- ✅ Zero-cost abstractions (no runtime overhead)
- ✅ Keep complex cases as powerful as they are
- ✅ Make simple cases simpler
- ✅ Preserve type safety and compile-time guarantees

---

## Proposal 1: Closure-Based Handlers

### Problem

Currently, even simple handlers require full trait implementation:

```rust
struct SimpleHandler;

#[async_trait::async_trait]
impl MessageHandler<MyMessage> for SimpleHandler {
    async fn handle(&mut self, message: MyMessage) -> Result<()> {
        println!("Received: {:?}", message);
        Ok(())
    }
}

// Usage
.router(MyRouter, |routing| {
    routing.handler(Route::Main, SimpleHandler)
})
```

For stateless handlers or simple callbacks, this is verbose.

### Proposed Solution

Allow closures and functions as handlers for simple cases:

```rust
// With closure
.router(MyRouter, |routing| {
    routing.handler(Route::Main, |msg| async move {
        println!("Received: {:?}", msg);
        Ok(())
    })
})

// With async function
async fn handle_message(message: MyMessage) -> Result<()> {
    println!("Received: {:?}", message);
    Ok(())
}

.router(MyRouter, |routing| {
    routing.handler(Route::Main, handle_message)
})

// With stateful closure (using Arc)
let counter = Arc::new(AtomicU64::new(0));
let counter_clone = counter.clone();

.router(MyRouter, |routing| {
    routing.handler(Route::Main, move |msg| {
        let n = counter_clone.fetch_add(1, Ordering::Relaxed) + 1;
        async move {
            println!("Message #{}: {:?}", n, msg);
            Ok(())
        }
    })
})
```

### Implementation

Create a wrapper type that implements `MessageHandler`:

```rust
// Internal implementation
struct ClosureHandler<F, Fut, M>
where
    F: FnMut(M) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
    M: Send + Debug + 'static,
{
    f: F,
    _phantom: PhantomData<(Fut, M)>,
}

#[async_trait]
impl<F, Fut, M> MessageHandler<M> for ClosureHandler<F, Fut, M>
where
    F: FnMut(M) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
    M: Send + Debug + 'static,
{
    async fn handle(&mut self, message: M) -> Result<()> {
        (self.f)(message).await
    }
}

// Public API addition to RoutingBuilder
impl<R> RoutingBuilder<R>
where
    R: MessageRouter,
{
    // Existing trait-based method (unchanged)
    pub fn handler<H>(self, route_key: R::RouteKey, handler: H) -> Self
    where
        H: MessageHandler<R::Message>,
    { ... }

    // NEW: Closure-based method
    pub fn handler_fn<F, Fut>(self, route_key: R::RouteKey, f: F) -> Self
    where
        F: FnMut(R::Message) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let handler = ClosureHandler {
            f,
            _phantom: PhantomData,
        };
        self.handler(route_key, handler)
    }
}
```

### Pros

- ✅ Much simpler for common cases
- ✅ No boilerplate for stateless handlers
- ✅ Backward compatible (existing trait-based API unchanged)
- ✅ Zero-cost abstraction (compiles to same code)
- ✅ Still allows complex stateful handlers via traits

### Cons

- ❌ Slightly more complex internal implementation
- ❌ Two ways to do the same thing (trait vs closure)
- ❌ Closure captures can be confusing for beginners

### Breaking Changes

**None** - This is a purely additive change.

### Implementation Complexity

**Medium** - Requires generic wrapper type and additional RoutingBuilder method.

---

## Proposal 2: Default Reconnection Strategy

### Problem

Users must always specify a reconnection strategy, even though most want the same thing:

```rust
.reconnect_strategy(ExponentialBackoff::new(
    Duration::from_secs(1),
    Duration::from_secs(60),
    Some(10),
))
```

### Proposed Solution

Provide sensible default when not specified:

```rust
// Current (still supported)
.reconnect_strategy(ExponentialBackoff::new(...))

// NEW: Use default if not specified
// Default: Exponential backoff 1s -> 60s, 10 attempts
.build().await?  // Uses default strategy
```

Default configuration:
```rust
ExponentialBackoff::new(
    Duration::from_secs(1),   // Initial: 1 second
    Duration::from_secs(60),  // Max: 60 seconds
    Some(10),                 // 10 attempts
)
```

Users can still customize:
```rust
// Unlimited attempts
.reconnect_strategy(ExponentialBackoff::new(
    Duration::from_secs(1),
    Duration::from_secs(60),
    None,  // Unlimited
))

// Fixed delay
.reconnect_strategy(FixedDelay::new(Duration::from_secs(5), Some(5)))

// No reconnection
.reconnect_strategy(NeverReconnect)
```

### Implementation

```rust
// In builder build() method
let reconnect_strategy = self.reconnect_strategy.unwrap_or_else(|| {
    Box::new(ExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
        Some(10),
    ))
});
```

**Already implemented!** Check `libs/hypersockets/src/builder/mod.rs:251-257`

### Pros

- ✅ Reduces boilerplate for common case
- ✅ Users can still override with any strategy
- ✅ Sensible defaults for production use
- ✅ Already implemented!

### Cons

- ❌ "Magical" behavior (implicit config)
- ❌ Users might not realize strategy is configurable

### Breaking Changes

**None** - Default only applies when not specified.

### Implementation Complexity

**None** - Already implemented!

---

## Proposal 3: Simplified Single-Handler API

### Problem

When only one handler is needed (common for simple clients), the routing configuration is verbose:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Main,  // Only one route!
}

.router(MyRouter, |routing| {
    routing.handler(Route::Main, MyHandler)
})
```

Users must:
1. Define a route key enum with one variant
2. Implement `route_key()` to always return that variant
3. Configure routing with closure

### Proposed Solution

Add simplified API for single-handler case:

```rust
// NEW: Simple handler (no routing)
.simple_handler(MyParser, MyHandler)

// Where MyParser is simpler:
struct MyParser;

#[async_trait]
impl MessageParser for MyParser {
    type Message = MyMessage;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        // Just parse, no routing
        ...
    }
}
```

This creates an internal router with a single route key automatically.

### Implementation

```rust
// New trait: MessageParser (without routing)
#[async_trait]
pub trait MessageParser: Send + Sync + 'static {
    type Message: Send + Debug + 'static;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message>;
}

// Internal wrapper that implements MessageRouter
struct SingleRouteRouter<P>
where
    P: MessageParser,
{
    parser: P,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SingleRoute;

#[async_trait]
impl<P> MessageRouter for SingleRouteRouter<P>
where
    P: MessageParser,
{
    type Message = P::Message;
    type RouteKey = SingleRoute;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        self.parser.parse(message).await
    }

    fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
        SingleRoute
    }
}

// Builder method
impl<U, Ro, R, M> WebSocketClientBuilder<U, Ro, R, M> {
    pub fn simple_handler<P, H>(
        self,
        parser: P,
        handler: H,
    ) -> WebSocketClientBuilder<U, HasRouter, SingleRouteRouter<P>, P::Message>
    where
        P: MessageParser,
        H: MessageHandler<P::Message>,
    {
        let router = SingleRouteRouter { parser };
        self.router(router, |routing| {
            routing.handler(SingleRoute, handler)
        })
    }
}
```

### Pros

- ✅ Much simpler for common single-handler case
- ✅ No need to define route key enum
- ✅ Clearer intent (no routing needed)
- ✅ Backward compatible

### Cons

- ❌ Two APIs for similar functionality
- ❌ Users might not discover routing when they need it
- ❌ Adds a new trait (MessageParser)

### Breaking Changes

**None** - Additive only.

### Implementation Complexity

**Medium** - New trait, wrapper router, builder method.

---

## Proposal 4: Handler Convenience Macro

### Problem

Implementing `MessageHandler` trait requires boilerplate:

```rust
struct MyHandler {
    count: u64,
}

#[async_trait::async_trait]
impl MessageHandler<MyMessage> for MyHandler {
    async fn handle(&mut self, message: MyMessage) -> Result<()> {
        self.count += 1;
        println!("Message: {:?}", message);
        Ok(())
    }
}
```

### Proposed Solution

Provide a macro to reduce boilerplate:

```rust
// NEW: Macro-based handler
handler!(MyHandler<MyMessage> {
    count: u64 = 0,
} |self, message| async {
    self.count += 1;
    println!("Message: {:?}", message);
    Ok(())
});

// Expands to the full trait implementation above
```

More examples:

```rust
// Stateless handler
handler!(PrintHandler<MyMessage> |message| async {
    println!("Received: {:?}", message);
    Ok(())
});

// Handler with multiple fields
handler!(StatsHandler<TradeMessage> {
    trade_count: u64 = 0,
    total_volume: f64 = 0.0,
} |self, message| async {
    if let TradeMessage::Trade { price, quantity, .. } = message {
        self.trade_count += 1;
        self.total_volume += price * quantity;
    }
    Ok(())
});
```

### Implementation

```rust
#[macro_export]
macro_rules! handler {
    // Stateless variant
    ($name:ident<$msg:ty> |$message:ident| async $body:block) => {
        struct $name;

        #[async_trait::async_trait]
        impl MessageHandler<$msg> for $name {
            async fn handle(&mut self, $message: $msg) -> Result<()> {
                $body
            }
        }
    };

    // Stateful variant
    ($name:ident<$msg:ty> {
        $($field:ident: $ty:ty = $init:expr),* $(,)?
    } |$self:ident, $message:ident| async $body:block) => {
        struct $name {
            $($field: $ty),*
        }

        impl $name {
            fn new() -> Self {
                Self {
                    $($field: $init),*
                }
            }
        }

        #[async_trait::async_trait]
        impl MessageHandler<$msg> for $name {
            async fn handle(&mut $self, $message: $msg) -> Result<()> {
                $body
            }
        }
    };
}
```

### Pros

- ✅ Significantly less boilerplate
- ✅ Clear, readable syntax
- ✅ Still generates full trait implementation
- ✅ Compile-time checked
- ✅ Optional (users can still use traits directly)

### Cons

- ❌ Macros can be intimidating
- ❌ Rust-analyzer support might be limited
- ❌ Less flexible than manual implementation
- ❌ Error messages might be cryptic

### Breaking Changes

**None** - Optional convenience macro.

### Implementation Complexity

**Medium** - Macro with multiple patterns and edge cases.

---

## Proposal 5: Builder Method Grouping

### Problem

Related configuration scattered across multiple methods:

```rust
.auth(MyAuth)
.headers(MyHeaders)
.heartbeat(Duration::from_secs(30), WsMessage::Text("ping".into()))
.passive_ping(MyDetector)
```

### Proposed Solution

Add grouped configuration methods for common combinations:

```rust
// NEW: Group authentication concerns
.with_auth_and_headers(MyAuth, MyHeaders)

// NEW: Group keep-alive concerns
.with_keep_alive(
    Duration::from_secs(30),  // Heartbeat interval
    WsMessage::Text("ping".into()),  // Heartbeat payload
    MyDetector,  // Passive ping detector
)

// NEW: Quick subscribe
.subscribe_to(vec!["trades", "orders", "book"].iter().map(|ch| {
    WsMessage::Text(format!(r#"{{"channel":"{}"}}"#, ch))
}).collect())
```

### Implementation

```rust
impl<U, R> WebSocketClientBuilder<U, HasRouter, R, R::Message> {
    // Group auth + headers
    pub fn with_auth_and_headers(
        mut self,
        auth: impl AuthProvider + 'static,
        headers: impl HeaderProvider + 'static,
    ) -> Self {
        self.auth = Some(Arc::new(auth));
        self.headers = Some(Arc::new(headers));
        self
    }

    // Group heartbeat + passive ping
    pub fn with_keep_alive(
        mut self,
        interval: Duration,
        payload: WsMessage,
        detector: impl PassivePingDetector + 'static,
    ) -> Self {
        self.heartbeat = Some((interval, payload));
        self.passive_ping = Some(Arc::new(detector));
        self
    }

    // Quick subscribe (sugar for subscriptions)
    pub fn subscribe_to(mut self, messages: Vec<WsMessage>) -> Self {
        self.subscriptions.extend(messages);
        self
    }
}
```

### Pros

- ✅ Less method chaining for common patterns
- ✅ Groups related configuration
- ✅ More discoverable (fewer methods to learn)
- ✅ Backward compatible (existing methods still work)

### Cons

- ❌ More API surface area
- ❌ Might not match all use cases
- ❌ Less flexible (can't configure one without the other)

### Breaking Changes

**None** - Additive convenience methods.

### Implementation Complexity

**Low** - Simple wrapper methods.

---

## Proposal 6: Implicit Route Key for Single Handler

### Problem

When using only one handler, defining route keys feels unnecessary:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Route {
    Main,  // Feels pointless
}

fn route_key(&self, _message: &Self::Message) -> Self::RouteKey {
    Route::Main  // Always returns the same thing
}
```

### Proposed Solution

Allow router to implement `MessageParser` (no routing) and automatically wrap it:

```rust
// Define router WITHOUT routing
struct MyRouter;

#[async_trait]
impl MessageParser for MyRouter {  // Note: MessageParser, not MessageRouter
    type Message = MyMessage;

    async fn parse(&self, message: WsMessage) -> Result<Self::Message> {
        // Just parse, no routing needed
        ...
    }
}

// Builder automatically adds implicit routing
.parser_with_handler(MyRouter, MyHandler)
// Internally creates SingleRouteRouter wrapper
```

This is similar to Proposal 3 but uses different terminology.

### Implementation

See Proposal 3 implementation.

### Pros

- ✅ Very simple for common case
- ✅ Clear separation: Parser vs Router
- ✅ No unnecessary route key definitions

### Cons

- ❌ Two concepts to learn (Parser vs Router)
- ❌ Migration path from simple to routing not obvious

### Breaking Changes

**None** - Additive API.

### Implementation Complexity

**Medium** - Same as Proposal 3.

---

## Implementation Priority

Recommended implementation order based on impact vs complexity:

### Phase 1: Quick Wins (Low Complexity, High Impact)
1. ✅ **Default Reconnection Strategy** - Already implemented!
2. **Handler Convenience Macro** - Optional, high value for users

### Phase 2: Core Simplifications (Medium Complexity, High Impact)
3. **Closure-Based Handlers** - Makes simple cases much easier
4. **Builder Method Grouping** - Low complexity, nice ergonomics

### Phase 3: Advanced Simplifications (Medium-High Complexity)
5. **Simplified Single-Handler API** (Proposal 3 or 6)
   - Choose one approach
   - Most impactful for beginners
   - Requires careful API design

### Phase 4: Polish (Optional)
6. Additional convenience methods based on user feedback
7. More macro variants
8. Documentation and migration guides

---

## Backward Compatibility

**All proposals maintain 100% backward compatibility.** Existing code will continue to work without changes. These are purely additive improvements.

---

## User Migration

Users can adopt these improvements incrementally:

```rust
// Before: Verbose but explicit
struct MyHandler;

#[async_trait]
impl MessageHandler<MyMessage> for MyHandler {
    async fn handle(&mut self, message: MyMessage) -> Result<()> {
        println!("{:?}", message);
        Ok(())
    }
}

.router(MyRouter, |routing| {
    routing.handler(Route::Main, MyHandler)
})
.reconnect_strategy(ExponentialBackoff::new(...))

// After: Simpler but still clear
.router(MyRouter, |routing| {
    routing.handler_fn(Route::Main, |msg| async move {
        println!("{:?}", msg);
        Ok(())
    })
})
// reconnect_strategy now has a default!
```

---

## Conclusion

These proposals aim to make HyperSockets significantly easier to use for common cases while maintaining its power and flexibility for advanced scenarios. The key is providing multiple levels of API:

1. **Simple**: Closures, defaults, macros - for quick prototypes
2. **Intermediate**: Current API - for most production use cases
3. **Advanced**: Full trait customization - for maximum control

Users can start simple and graduate to more complex APIs as their needs grow, all without breaking changes or performance compromises.

**Recommendation**: Implement Phase 1 and 2 first (Closure handlers + convenience methods), gather user feedback, then decide on Phase 3 approach.
