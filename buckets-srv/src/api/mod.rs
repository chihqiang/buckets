//! 统一 API 路由定义。
//!
//! 路由分为：
//! - **认证路由**：登录、刷新、登出、验证（网关和 Web 共享）
//! - **上传路由**：上传（STS、预检、分块、合并）、对象 CRUD
//! - **管理路由**：用户管理（超级管理员）、文件管理
//!
//! 挂载在 `/api/v1` 下：
//!   /api/v1/auth/*         — 统一认证
//!   /api/v1/upload/*       — 分块上传
//!   /api/v1/object/*       — 对象 CRUD
//!   /api/v1/users/*        — 用户管理（超级管理员）
//!   /api/v1/objects/*      — 文件管理

pub mod auth;
pub mod chunk;
pub mod objects;
pub mod merge;
pub mod precheck;
pub mod sts;
pub mod users;

use crate::app::AppState;
use crate::middleware::ratelimit;
use axum::extract::DefaultBodyLimit;
use axum::middleware as mw;
use axum::{
    Router,
    routing::{delete, get, post, put},
};

/// 构建组合的 API 路由。
/// 认证中间件在 app.rs 中应用。此处仅定义路由树。
pub fn routes() -> Router<AppState> {
    // 认证路由（登录/刷新跳过认证中间件；登出/验证需要认证）
    let auth_routes = Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/verify", post(auth::verify_credentials));

    // 非上传对象路由（无速率限制）
    let object_routes = Router::new()
        .route("/object/{object_id}", get(sts::get_object_info))
        .route("/object/{object_id}", delete(sts::delete_object));

    // 从环境变量读取最大分块大小，回退到 256 MiB 默认值
    let max_body = std::env::var(buckets_common::constant::ENV_MAX_CHUNK_SIZE)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(buckets_common::constant::DEFAULT_MAX_CHUNK_SIZE)
        .saturating_add(buckets_common::constant::BODY_LIMIT_OVERHEAD)
        as usize;

    // 上传路由（含速率限制）
    let upload_routes = Router::new()
        .route("/upload/sts", post(sts::get_sts_token))
        .route("/upload/precheck", post(precheck::precheck_file))
        .route(
            "/upload/chunk/upload-binary",
            post(chunk::upload_chunk_binary),
        )
        .route("/upload/chunk/status", post(chunk::chunk_status))
        .route("/upload/merge", post(merge::merge_chunks))
        .route("/upload/merge/status", get(merge::merge_status))
        // 应用于所有上传端点的上传速率限制中间件
        .layer(mw::from_fn(ratelimit::upload_ratelimit))
        // 请求体限制，防止超大请求导致 OOM
        .layer(DefaultBodyLimit::max(max_body));

    // Web 管理路由
    let web_admin_routes = Router::new()
        .route("/users", get(users::list_users))
        .route("/users", post(users::create_user))
        .route("/users/{id}", get(users::get_user))
        .route("/users/{id}", put(users::update_user))
        .route("/users/{id}", delete(users::delete_user))
        .route(
            "/users/{id}/reset-secret-key",
            post(users::reset_user_secret_key),
        )
        .route("/objects", get(objects::list_objects))
        .route("/objects/{id}", delete(objects::delete_object));

    Router::new()
        .merge(auth_routes)
        .merge(object_routes)
        .merge(upload_routes)
        .merge(web_admin_routes)
}
