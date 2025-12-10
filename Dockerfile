# Build stage
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for better caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies only (this layer gets cached)
RUN cargo build --release && rm -rf src

# Copy actual source code
COPY src ./src
COPY templates ./templates

# Touch main.rs to ensure it rebuilds with actual code
RUN touch src/main.rs

# Build the actual application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/moar-news /app/moar-news

# Copy static assets and config
COPY static ./static
COPY feeds.toml ./feeds.toml

# Create directory for SQLite database
RUN mkdir -p /data

# Set environment variables
ENV RUST_LOG=moar_news=info,tower_http=info
ENV DATABASE_PATH=/data/moar_news.db

EXPOSE 3000

CMD ["/app/moar-news"]
