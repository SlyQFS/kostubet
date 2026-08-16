# Stage 1: Build static binary
FROM rust:alpine AS builder

# musl-dev/build-base: C toolchain for the bundled SQLite (libsqlite3-sys);
# openssl is NOT needed: both reqwest and sqlx use rustls.
RUN apk add --no-cache \
    musl-dev \
    build-base

WORKDIR /usr/src/kostubet-github

# Dependency caching layer: build a dummy binary first so that `src/` changes
# do not invalidate the compiled dependency cache (~10 min -> seconds).
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --locked && rm -rf src
# Remove the dummy binary fingerprint so cargo rebuilds the real binary below.
RUN rm target/release/deps/kostubet_github* target/release/kostubet-github

COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

# Stage 2: Lightweight runtime image
FROM alpine:3.20

RUN apk add --no-cache \
    ca-certificates \
    tzdata \
    libgcc

WORKDIR /data

COPY --from=builder /usr/src/kostubet-github/target/release/kostubet-github /usr/local/bin/kostubet-github

ENV RUST_LOG="kostubet_github=info,teloxide=info"
ENV DB_PATH="/data/kostubet.db"

ENTRYPOINT ["/usr/local/bin/kostubet-github"]
