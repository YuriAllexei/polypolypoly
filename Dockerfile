# Build stage
FROM rust:1.91.1 as builder

WORKDIR /usr/src/app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests first to cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY libs ./libs
COPY src ./src

# Build the application
# We build all binaries (now in src/bin/)
RUN cargo build --release

# Runtime stage
FROM ubuntu

WORKDIR /usr/local/bin

# Install runtime dependencies
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

# Copy binaries from builder
COPY --from=builder /usr/src/app/target/release/polymarket_events .
COPY --from=builder /usr/src/app/target/release/market_sniper .

# Create config directory for volume mounts
RUN mkdir -p /etc/polymarket
