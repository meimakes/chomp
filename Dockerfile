# Build stage
FROM rust:slim AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY dashboard.html login.html ./
RUN cargo build --release --features sse

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /data
COPY --from=builder /app/target/release/chomp /usr/local/bin/chomp

ENV CHOMP_HOST=0.0.0.0
ENV CHOMP_DB_PATH=/data/foods.db

# Railway injects PORT; map it to CHOMP_PORT so clap picks it up
CMD sh -c "CHOMP_PORT=\${PORT:-3000} exec chomp serve --transport sse"
