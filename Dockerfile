FROM node:20-alpine AS builder-web
WORKDIR /build
COPY jssdk/package.json jssdk/package-lock.json jssdk/
COPY web/package.json web/package-lock.json web/
RUN npm ci --prefix jssdk && npm ci --prefix web
COPY jssdk/ jssdk/
COPY web/ web/
RUN npm run build --prefix jssdk && npm run build --prefix web

FROM rust:1.89-slim-bookworm AS builder-rust
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY buckets-common/ buckets-common/
COPY buckets-cli/ buckets-cli/
COPY buckets-srv/ buckets-srv/
COPY migration/ migration/
COPY --from=builder-web /build/web/dist web/dist/
RUN cargo build --release --bin buckets-srv --bin buckets-cli --bin migration

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder-rust /build/target/release/buckets-srv /usr/local/bin/buckets-srv
COPY --from=builder-rust /build/target/release/buckets-cli /usr/local/bin/buckets-cli
COPY --from=builder-rust /build/target/release/migration /usr/local/bin/migration
RUN useradd --system --uid 1000 --create-home buckets
EXPOSE 8080
ENTRYPOINT ["buckets-srv"]
