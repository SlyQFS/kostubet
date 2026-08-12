# Stage 1: Build static binary
FROM rust:1.80-alpine as builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    build-base \
    openssl-dev \
    sqlite-dev \
    ca-certificates

WORKDIR /usr/src/kostubet-github

COPY Cargo.toml ./
COPY src ./src

# Build highly optimized release binary
RUN cargo build --release

# Stage 2: Lightweight runtime image (~20MB)
FROM alpine:3.20

RUN apk add --no-cache \
    ca-certificates \
    sqlite-libs \
    tzdata \
    libgcc

WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/kostubet-github/target/release/kostubet-github /app/kostubet-github

# Default logging filter
ENV RUST_LOG="kostubet_github=info,teloxide=info"

ENTRYPOINT ["/app/kostubet-github"]
