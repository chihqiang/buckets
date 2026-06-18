//! # buckets-srv
//!
//! buckets 私有 OSS 的统一服务器。
//!
//! 在单个进程中同时提供 Web 管理（用户/文件管理）和网关（分块上传）功能。

mod api;
mod app;
mod config;
mod dao;
mod db;
mod embed;
mod middleware;
mod service;
mod storage;
mod task;

use crate::middleware::ratelimit::RateLimiter;
use buckets_common::constant;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    println!("buckets-srv starting...");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = config::AppConfig::from_env()?;

    // 如果在生产环境中未覆盖密钥，则发出警告
    let secret_key = std::env::var(constant::ENV_SECRET_KEY).unwrap_or_default();
    let refresh_key = std::env::var(constant::ENV_REFRESH_TOKEN_KEY).unwrap_or_default();
    if secret_key.is_empty() {
        tracing::warn!(
            "SECRET_KEY is not set — using hardcoded default. Set it via environment variable for production!"
        );
    }
    if refresh_key.is_empty() {
        tracing::warn!(
            "REFRESH_TOKEN_KEY is not set — using hardcoded default. Set it via environment variable for production!"
        );
    }

    if cfg.super_admin_ids.is_empty() {
        tracing::warn!("SUPER_ADMIN_IDS is empty — no user will have admin access");
    } else {
        tracing::info!(ids = ?cfg.super_admin_ids, "super admin IDs loaded");
    }

    tracing::info!("Connecting to database...");
    let db_pool = db::create_pool(cfg.database_url(), cfg.db_max_conn).await?;

    // 确保存储目录存在
    storage::fs::ensure_directories().await?;

    // 使用 CancellationToken 实现优雅关闭
    let cancellation = CancellationToken::new();

    // 认证缓存（缓存 email:password 验证结果）
    let auth_cache = middleware::auth::new_auth_cache();
    middleware::auth::start_auth_cache_cleaner(auth_cache.clone(), cancellation.clone());

    // 密钥缓存（避免逐分块查询数据库进行签名验证）
    let secret_key_cache = middleware::auth::new_secret_key_cache();
    middleware::auth::start_secret_key_cache_cleaner(
        secret_key_cache.clone(),
        cancellation.clone(),
    );

    // 后台任务（延迟启动以避免数据库连接池预热竞争）
    let gc_pool = db_pool.clone();
    let gc_cancel = cancellation.clone();
    let ref_pool = db_pool.clone();
    let ref_cancel = cancellation.clone();
    tokio::spawn(async move {
        // 等待数据库连接池预热后再启动后台任务。
        tokio::time::sleep(Duration::from_secs(5)).await;
        task::gc_clean::start_gc_cleaner(gc_pool, gc_cancel).await;
    });
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        task::ref_check::start_ref_checker(ref_pool, ref_cancel).await;
    });

    // 内存位图刷新 + 缓存清理后台任务
    service::chunk_svc::start_bitmap_flush_task(db_pool.clone(), cancellation.clone());
    service::chunk_svc::start_bitmap_cache_cleanup(cancellation.clone());

    // 速率限制器（网关）
    let rate_limiter = if cfg.rate_limit_enabled {
        tracing::info!(
            rps = cfg.rate_limit.max_requests_per_second,
            burst = cfg.rate_limit.burst_size,
            concurrent = cfg.rate_limit.max_concurrent_uploads,
            daily_quota = cfg.rate_limit.daily_upload_quota,
            "Upload rate limiting enabled"
        );
        let limiter = RateLimiter::new(cfg.rate_limit.clone(), db_pool.clone());
        limiter.init_concurrent_counters().await;
        RateLimiter::start_cleanup_task(std::sync::Arc::new(limiter.clone()), cancellation.clone());
        Some(limiter)
    } else {
        tracing::info!("Upload rate limiting disabled");
        None
    };

    // JWT 验证失败速率限制器——防止通过格式错误的 JWT 令牌
    // 进行暴力破解用户 ID 探测。
    // 默认：突发 5 次失败，每秒补充 0.2 个令牌（每 5 秒 1 个）。
    let jwt_fail_limiter = middleware::auth::JwtFailLimiter::new(5.0, 0.2);
    std::sync::Arc::new(jwt_fail_limiter.clone()).start_cleanup(cancellation.clone());
    tracing::info!("JWT failure rate limiter: burst=5, refill=0.2/s");

    // Basic Auth 失败速率限制器——防止通过重复 Basic Auth 尝试
    // 进行暴力破解密码猜测。使用相同的令牌桶算法。
    // 默认：突发 10 次失败，每秒补充 0.5 个令牌（每 2 秒 1 个）。
    let basic_auth_fail_limiter =
        std::sync::Arc::new(middleware::auth::JwtFailLimiter::new(10.0, 0.5));
    basic_auth_fail_limiter
        .clone()
        .start_cleanup(cancellation.clone());
    tracing::info!("Basic Auth failure rate limiter: burst=10, refill=0.5/s");

    let state = app::AppState {
        db: db_pool.clone(),
        cfg: cfg.clone(),
        auth_cache,
        secret_key_cache,
        rate_limiter,
        jwt_fail_limiter,
        basic_auth_fail_limiter,
    };

    let router = app::create_router(state, &cfg);

    tracing::info!("Starting server on {}:{}", cfg.host, cfg.port);
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // 监听 SIGTERM/SIGINT 并触发优雅关闭
    let shutdown_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
        shutdown_cancel.cancel();
    });

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            cancellation.cancelled().await;
        })
        .await
        .map_err(|e| anyhow::anyhow!("server error: {}", e))
}
