//! 应用状态和路由器构建。
//!
//! 组装 Axum 路由器，包含所有中间件层：CORS、追踪、
//! 请求日志记录、认证、速率限制、压缩和请求超时。
//!
//! 在 `/api/v1` 下挂载路由组：
//! - 认证路由（登录/刷新/登出/验证）
//! - 上传路由（STS、预检、分块上传、合并、对象 CRUD）
//! - 管理路由（用户/文件管理、超级管理员门控）

use crate::api;
use crate::config::AppConfig;
use crate::middleware;
use crate::middleware::auth::{AuthCache, BasicAuthFailLimiter, JwtFailLimiter, SecretKeyCache};
use crate::middleware::ratelimit::RateLimiter;
use axum::http::HeaderValue;
use axum::response::IntoResponse;
use axum::{Router, middleware as mw};
use buckets_common::constant;
use sea_orm::DatabaseConnection;
use std::time::Duration;
use tower_http::{compression::CompressionLayer, cors::CorsLayer};

/// 传递给所有请求处理器的共享应用状态。
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub cfg: AppConfig,
    pub rate_limiter: Option<RateLimiter>,
    pub auth_cache: AuthCache,
    pub secret_key_cache: SecretKeyCache,
    pub jwt_fail_limiter: JwtFailLimiter,
    /// 按 IP 的 Basic Auth 失败速率限制器（防止密码暴力破解）
    pub basic_auth_fail_limiter: BasicAuthFailLimiter,
}

// 允许提取器从 AppState 访问各个字段。
impl axum::extract::FromRef<AppState> for DatabaseConnection {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

impl axum::extract::FromRef<AppState> for AppConfig {
    fn from_ref(state: &AppState) -> Self {
        state.cfg.clone()
    }
}

/// 构建包含所有中间件和路由的 Axum 路由器。
pub fn create_router(state: AppState, cfg: &AppConfig) -> Router {
    // CORS 配置
    let cors = if cfg.cors_allowed_origins.is_empty() {
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        let origins: Vec<HeaderValue> = cfg
            .cors_allowed_origins
            .iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
                axum::http::HeaderName::from_static(constant::HEADER_SESSION_SIGNATURE),
                axum::http::HeaderName::from_static(constant::HEADER_SESSION_TIMESTAMP),
                axum::http::HeaderName::from_static(constant::HEADER_SESSION_SALT),
                axum::http::HeaderName::from_static(constant::HEADER_TUS_RESUMABLE),
                axum::http::HeaderName::from_static(constant::HEADER_UPLOAD_LENGTH),
                axum::http::HeaderName::from_static(constant::HEADER_UPLOAD_OFFSET),
                axum::http::HeaderName::from_static(constant::HEADER_UPLOAD_METADATA),
                axum::http::HeaderName::from_static(constant::HEADER_UPLOAD_DEFER_LENGTH),
            ])
    };

    let auth_cache = state.auth_cache.clone();
    let secret_key_cache = state.secret_key_cache.clone();
    let jwt_fail_limiter = state.jwt_fail_limiter.clone();
    let basic_auth_fail_limiter = state.basic_auth_fail_limiter.clone();
    let db = state.db.clone();
    let app_cfg = state.cfg.clone();

    // 将共享扩展注入所有请求
    let extensions_layer = mw::from_fn(
        move |mut req: axum::extract::Request, next: axum::middleware::Next| {
            let cache = auth_cache.clone();
            let sk_cache = secret_key_cache.clone();
            let pool = db.clone();
            let cfg = app_cfg.clone();
            let jwt_lim = jwt_fail_limiter.clone();
            let basic_lim = basic_auth_fail_limiter.clone();
            async move {
                req.extensions_mut().insert(cache);
                req.extensions_mut().insert(sk_cache);
                req.extensions_mut().insert(pool);
                req.extensions_mut().insert(cfg);
                req.extensions_mut().insert(jwt_lim);
                req.extensions_mut().insert(basic_lim);
                next.run(req).await
            }
        },
    );

    // 统一 API 路由，使用统一认证中间件
    // extensions_layer 必须在外部（在 auth_layer 之后添加），
    // 以便共享扩展（pool、cfg、caches）对 auth_layer 可用。
    let api_routes = api::routes()
        // 统一认证中间件（支持网关和 Web 两种模式）
        .layer(mw::from_fn(middleware::auth::auth_layer))
        .layer(extensions_layer);

    // Tus OPTIONS 不需要认证（协议要求）
    let tus_options_route = Router::new()
        .route("/upload/tus", axum::routing::options(api::tus::tus_options));

    let mut router = Router::new()
        .nest("/api/v1", api_routes)
        .nest("/api/v1", tus_options_route)
        .route("/health", axum::routing::get(health_check));

    router = router.fallback(crate::embed::fallback);

    router
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(mw::from_fn(middleware::logger::request_logger))
        .layer(mw::from_fn(middleware::trace::trace_layer))
        // 请求超时，防止挂起的连接消耗资源。
        // 用 tokio::time::timeout 包装每个请求；超时返回 408。
        .layer(mw::from_fn(request_timeout_middleware))
        .with_state(state)
}

/// 将每个请求包装在 tokio::time::timeout 中的中间件。
/// 如果处理器未及时完成，返回 HTTP 408 请求超时。
/// 这防止慢速/挂起的客户端无限消耗连接资源。
async fn request_timeout_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let timeout_dur = Duration::from_secs(buckets_common::constant::REQUEST_TIMEOUT_SECS);
    match tokio::time::timeout(timeout_dur, next.run(request)).await {
        Ok(response) => response,
        Err(_elapsed) => {
            tracing::warn!(
                "request timed out after {}s",
                buckets_common::constant::REQUEST_TIMEOUT_SECS
            );
            axum::http::StatusCode::REQUEST_TIMEOUT.into_response()
        }
    }
}

/// 健康检查端点——返回数据库状态和磁盘空间。
/// 当超过阈值时，同时报告磁盘使用百分比和降级状态。
async fn health_check(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<serde_json::Value> {
    let db_ok = state.db.ping().await.is_ok();

    let (disk_available, disk_usage_pct) = buckets_common::utils::validate::disk_space_info();

    // 当磁盘使用率超过阈值（90%）时标记为降级
    let disk_degraded = disk_usage_pct.map(|pct| pct > 90.0).unwrap_or(false);
    let status = if db_ok && !disk_degraded {
        "ok"
    } else {
        "degraded"
    };

    axum::Json(serde_json::json!({
        "status": status,
        "db_ok": db_ok,
        "disk_available_bytes": disk_available,
        "disk_usage_percent": disk_usage_pct,
    }))
}
