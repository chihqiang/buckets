# ============================================================================
# buckets — 单容器镜像（后端 API + 嵌入式前端）
# 构建：docker build -t buckets:latest .
# ============================================================================

# ---- Stage 1: Build Vue 前端（编译产物将被嵌入 Rust 二进制） ----
FROM node:20-alpine AS builder-web

WORKDIR /build
COPY jssdk/ jssdk/
COPY web/ web/
WORKDIR /build/web
RUN npm ci && npm run build

# ---- Stage 2: Build Rust 后端（前端 dist 编译进二进制） ----
FROM rust:1.89-slim-bookworm AS builder-rust

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY buckets-common/Cargo.toml buckets-common/
COPY buckets-cli/Cargo.toml buckets-cli/
COPY buckets-srv/Cargo.toml buckets-srv/

RUN mkdir -p buckets-common/src && \
    echo 'fn main() {}' > buckets-common/src/lib.rs && \
    mkdir -p buckets-cli/src && \
    echo 'fn main() {}' > buckets-cli/src/main.rs && \
    mkdir -p buckets-srv/src && \
    echo 'fn main() {}' > buckets-srv/src/main.rs

# Web dist 必须在首次编译依赖前存在，否则 rust-embed 找不到文件夹会报错
RUN mkdir -p web/dist && echo '<html></html>' > web/dist/index.html

RUN cargo build --release -p buckets-srv && \
    rm -rf buckets-*/src target/release/.fingerprint/buckets-srv-*

COPY buckets-common buckets-cli buckets-srv/

# 将真正的前端 dist 覆盖 placeholder，然后最终编译
COPY --from=builder-web /build/web/dist web/dist/
RUN cargo build --release -p buckets-srv

# ---- Stage 3: Runtime ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder-rust /build/target/release/buckets-srv /usr/local/bin/buckets-srv

RUN useradd --system --uid 1000 --create-home buckets

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["buckets-srv"]
