//! 统一认证中间件——支持 JWT Bearer 令牌和 Basic Auth。
//!
//! 认证优先级：
//! 1. `Authorization: Bearer <JWT 令牌>` 头部（标准，网关和 Web 均适用）
//! 2. `Authorization: Basic base64(email:password)` 头部（CLI，缓存 30 分钟）
//!
//! 登录和刷新端点跳过认证。
//! [`ADMIN_REQUIRED_PATHS`] 中的路径额外需要超级管理员权限。
//!
//! ## JWT 验证设计
//!
//! 访问令牌在 JWT **头部**嵌入 `kid`（user_id）。验证器
//! 检查未经验证的头部以路由到正确的用户特定密钥，
//! 然后执行完整的 HS256 验证。这避免了旧模式：
//! 在验证前手动 base64url 解码载荷，这种方式：
//! - 允许任意 user_id 探测（任何人都可以构造包含任意
//!   user_id 的 JWT 并在签名验证前触发数据库查询）。
//! - 脆弱且依赖内部 JWT 格式假设。
//!
//! 对失败的 JWT 验证实施按 IP 速率限制，额外防止
//! 对有效用户 ID 进行暴力破解探测。

use crate::config::AppConfig;
use crate::db;
use crate::db::UserId;
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use dashmap::DashMap;
use buckets_common::constant;
use buckets_common::utils::crypto;
use sea_orm::DatabaseConnection;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// 类型
// ============================================================================

/// 缓存的已验证凭据：hash(email:password) -> (user_id, expires_at)
pub struct AuthCacheEntry {
    user_id: i64,
    expires_at: Instant,
}

/// 线程安全的 Basic Auth 凭据认证缓存。
pub type AuthCache = Arc<DashMap<String, AuthCacheEntry>>;

/// 按 IP 的 Basic Auth 失败速率限制器——防止通过重复的 Basic Auth 尝试
/// 加不同密码进行暴力破解密码猜测。
/// 复用与 [`JwtFailLimiter`] 相同的令牌桶算法。
pub type BasicAuthFailLimiter = Arc<JwtFailLimiter>;

pub fn new_auth_cache() -> AuthCache {
    Arc::new(DashMap::new())
}

// ============================================================================
// 密钥缓存——避免逐分块数据库查询以进行签名验证
// ============================================================================

/// 缓存的密钥条目：(secret_key_string, expires_at)。
pub struct SecretKeyCacheEntry {
    pub secret_key: String,
    expires_at: Instant,
}

/// 线程安全的用户密钥缓存，避免在分块上传期间重复查询数据库。
/// 以 user_id 字符串为键，TTL 与 [`AUTH_CACHE_TTL_SECS`] 相同。
pub type SecretKeyCache = Arc<DashMap<String, SecretKeyCacheEntry>>;

pub fn new_secret_key_cache() -> SecretKeyCache {
    Arc::new(DashMap::new())
}

/// 查找 `user_id` 的缓存密钥。未命中或过期时返回 `None`。
pub fn get_cached_secret_key(cache: &SecretKeyCache, user_id: i64) -> Option<String> {
    let key = user_id.to_string();
    cache.get(&key).and_then(|entry| {
        if entry.expires_at > Instant::now() {
            Some(entry.secret_key.clone())
        } else {
            None
        }
    })
}

/// 将密钥以默认 TTL 插入缓存。
pub fn cache_secret_key(cache: &SecretKeyCache, user_id: i64, secret_key: String) {
    let key = user_id.to_string();
    let ttl = Duration::from_secs(constant::AUTH_CACHE_TTL_SECS);
    cache.insert(
        key,
        SecretKeyCacheEntry {
            secret_key,
            expires_at: Instant::now() + ttl,
        },
    );
}

/// 启动周期性清理过期密钥缓存条目。
pub fn start_secret_key_cache_cleaner(
    cache: SecretKeyCache,
    cancellation: tokio_util::sync::CancellationToken,
) {
    let ttl = Duration::from_secs(constant::AUTH_CACHE_TTL_SECS);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(ttl);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::debug!("secret key cache cleaner shutting down");
                    break;
                }
                _ = interval.tick() => {
                    let now = Instant::now();
                    cache.retain(|_, entry| entry.expires_at > now);
                }
            }
        }
    });
}

// ============================================================================
// JWT 验证失败速率限制器（按 IP 令牌桶）
// ============================================================================

/// 失败 JWT 验证的按 IP 速率限制状态。
#[derive(Debug)]
struct JwtFailBucket {
    tokens: f64,
    last_refill: Instant,
}

/// 共享的 JWT 失败速率限制器——防止通过格式错误的 JWT
/// 进行暴力破解探测有效用户 ID。
#[derive(Clone)]
pub struct JwtFailLimiter {
    buckets: Arc<DashMap<IpAddr, JwtFailBucket>>,
    /// 限制前允许的最大验证失败次数（突发容量）。
    max_failures: f64,
    /// 补充速率：每秒令牌数。
    refill_rate: f64,
}

impl JwtFailLimiter {
    /// 创建新的 JWT 失败速率限制器。
    /// `max_failures`：突发容量（例如限制前 5 次失败尝试）。
    /// `refill_rate`：每秒补充的令牌数（例如 0.2 = 每 5 秒 1 个令牌）。
    pub fn new(max_failures: f64, refill_rate: f64) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            max_failures,
            refill_rate,
        }
    }

    /// 检查来自 `ip` 的失败验证是否应被允许（返回 true）还是被限制（返回 false）。
    fn check_and_record_failure(&self, ip: IpAddr) -> bool {
        let mut bucket = self.buckets.entry(ip).or_insert_with(|| JwtFailBucket {
            tokens: self.max_failures,
            last_refill: Instant::now(),
        });

        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.last_refill = now;
        bucket.tokens = (bucket.tokens + elapsed * self.refill_rate).min(self.max_failures);

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 启动周期性清理非活动桶以防止内存泄漏。
    pub fn start_cleanup(self: Arc<Self>, cancellation: tokio_util::sync::CancellationToken) {
        tokio::spawn(async move {
            let interval = Duration::from_secs(3600);
            let mut ticker = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        tracing::debug!("jwt fail limiter cleanup shutting down");
                        break;
                    }
                    _ = ticker.tick() => {
                        let cutoff = Instant::now() - interval;
                        let before = self.buckets.len();
                        self.buckets.retain(|_, b| b.last_refill > cutoff);
                        let after = self.buckets.len();
                        if before > after {
                            tracing::info!("jwt fail limiter cleaned: {} -> {} entries", before, after);
                        }
                    }
                }
            }
        });
    }
}

// ============================================================================
// 令牌验证
// ============================================================================

/// 验证 JWT Bearer 令牌并返回 (user_id, jti)。
/// 如果令牌无效或过期则返回 None。
///
/// 使用 JWT 头部的 `kid` 字段确定使用哪个用户特定密钥，
/// 避免手动解码载荷。回退到 `SecretKeyCache` 以跳过
/// 对最近看到的用户的数据库查询。
async fn verify_bearer_token(
    token: &str,
    db: &Option<DatabaseConnection>,
    secret_key_cache: &Option<SecretKeyCache>,
) -> Option<(i64, String)> {
    // 解码（未经验证的）头部以提取 `kid`（user_id）并验证算法。
    // 这是我们做的唯一预验证解析。
    let header = jsonwebtoken::decode_header(token).ok()?;
    if header.alg != jsonwebtoken::Algorithm::HS256 {
        return None;
    }

    // 从 `kid`（密钥 ID）头部字段提取 user_id。
    // 由 `generate_access_token` 生成的访问令牌始终包含此项。
    let user_id: i64 = header.kid.as_deref()?.parse().ok()?;

    // 先尝试缓存，然后回退到数据库获取密钥。
    let secret_key = if let Some(cache) = secret_key_cache {
        if let Some(cached) = get_cached_secret_key(cache, user_id) {
            cached
        } else {
            let db = db.as_ref()?;
            let sk = db::get_user_secret_key(db, user_id).await.ok()?;
            cache_secret_key(cache, user_id, sk.clone());
            sk
        }
    } else {
        let db = db.as_ref()?;
        db::get_user_secret_key(db, user_id).await.ok()?
    };

    // 现在用正确的用户特定密钥验证完整令牌。
    crypto::verify_access_token(secret_key.as_bytes(), token)
}

pub fn start_auth_cache_cleaner(
    cache: AuthCache,
    cancellation: tokio_util::sync::CancellationToken,
) {
    let ttl = Duration::from_secs(constant::AUTH_CACHE_TTL_SECS);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(ttl);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::debug!("auth cache cleaner shutting down");
                    break;
                }
                _ = interval.tick() => {
                    let now = Instant::now();
                    cache.retain(|_, entry| entry.expires_at > now);
                    tracing::debug!("auth cache cleaned");
                }
            }
        }
    });
}

// ============================================================================
// AdminUserId 提取器
// ============================================================================

/// 提取超级管理员用户 ID。仅在 `SUPER_ADMIN_IDS` 中的用户通过。
#[derive(Debug, Clone, Copy)]
pub struct AdminUserId(#[allow(dead_code)] pub i64);

impl<S> FromRequestParts<S> for AdminUserId
where
    S: Send + Sync,
    AppConfig: axum::extract::FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let uid = parts
            .extensions
            .get::<UserId>()
            .copied()
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "not authenticated").into_response())?;

        let cfg = AppConfig::from_ref(state);

        if cfg.super_admin_ids.contains(&uid.0) {
            Ok(AdminUserId(uid.0))
        } else {
            Err((StatusCode::FORBIDDEN, "admin access required").into_response())
        }
    }
}

// ============================================================================
// 统一认证中间件
// ============================================================================

/// 统一认证中间件。
///
/// 1. 跳过 `/auth/login` 和 `/auth/refresh`。
/// 2. 尝试 `Authorization: Bearer <JWT>`。
/// 3. 回退到 `Authorization: Basic base64(email:password)`（CLI）。
/// 4. 如果路径以 [`ADMIN_REQUIRED_PATHS`] 前缀开头，则验证
///    认证用户是否为超级管理员；否则以 403 拒绝。
///
/// JWT 验证失败按客户端 IP 通过 [`JwtFailLimiter`] 进行速率限制，
/// 以防止暴力破解用户 ID 探测。
pub async fn auth_layer(req: axum::extract::Request, next: Next) -> Result<Response, Response> {
    let path = req.uri().path().to_string();
    tracing::debug!("auth middleware path={}", path);

    // 跳过登录和刷新的认证（前缀匹配，与管理检查一致）
    if constant::AUTH_SKIP_PATHS
        .iter()
        .any(|p| path.starts_with(p))
    {
        return Ok(next.run(req).await);
    }

    let (parts, body) = req.into_parts();

    // 克隆共享的扩展
    let db = parts.extensions.get::<DatabaseConnection>().cloned();
    let auth_cache = parts.extensions.get::<AuthCache>().cloned();
    let cfg = parts.extensions.get::<AppConfig>().cloned();
    let secret_key_cache = parts.extensions.get::<SecretKeyCache>().cloned();
    let jwt_fail_limiter = parts.extensions.get::<JwtFailLimiter>().cloned();
    let basic_auth_fail_limiter = parts.extensions.get::<BasicAuthFailLimiter>().cloned();

    // ── 第 1 步：尝试 Authorization 头部 ──
    let auth_header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION.as_str())
        .and_then(|v| v.to_str().ok());

    if let Some(header) = auth_header {
        // ── Bearer 令牌（JWT）──
        if let Some(token) = header.strip_prefix(constant::AUTH_SCHEME_BEARER) {
            let token = token.trim().to_string();
            if token.is_empty() {
                return Err((StatusCode::UNAUTHORIZED, "empty bearer token").into_response());
            }

            match verify_bearer_token(&token, &db, &secret_key_cache).await {
                Some((user_id, _jti)) => {
                    let is_admin = cfg
                        .as_ref()
                        .map(|c| c.super_admin_ids.contains(&user_id))
                        .unwrap_or(false);
                    return finish_auth(parts, body, next, user_id, is_admin, &path).await;
                }
                None => {
                    // 按客户端 IP 限制 JWT 验证失败速率，
                    // 防止暴力破解探测有效用户 ID。
                    if let Some(ref limiter) = jwt_fail_limiter {
                        let client_ip = extract_client_ip(&parts);
                        if !limiter.check_and_record_failure(client_ip) {
                            tracing::warn!(ip = %client_ip, "JWT verification rate limited");
                            return Err((
                                StatusCode::TOO_MANY_REQUESTS,
                                "too many auth attempts, slow down",
                            )
                                .into_response());
                        }
                    }
                    return Err(
                        (StatusCode::UNAUTHORIZED, "invalid or expired token").into_response()
                    );
                }
            }
        }

        // ── Basic Auth（email:password）用于 CLI ──
        if let Some(basic) = header.strip_prefix(constant::AUTH_SCHEME_BASIC) {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(basic.trim())
                .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid base64").into_response())?;

            let credentials = String::from_utf8(decoded)
                .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid utf8").into_response())?;

            let (email, password) = credentials
                .split_once(constant::CREDENTIAL_SEPARATOR)
                .ok_or_else(|| {
                    (
                        StatusCode::UNAUTHORIZED,
                        "invalid format, expected email:password",
                    )
                        .into_response()
                })?;

            // 为缓存键哈希凭据
            let cache_key = {
                let mut hasher = Sha256::new();
                hasher.update(email.as_bytes());
                hasher.update(constant::CREDENTIAL_SEPARATOR.as_bytes());
                hasher.update(password.as_bytes());
                hex::encode(hasher.finalize())
            };

            // 检查缓存
            let cached_user_id = auth_cache.as_ref().and_then(|cache| {
                cache.get(&cache_key).and_then(|entry| {
                    if entry.expires_at > Instant::now() {
                        Some(entry.user_id)
                    } else {
                        None
                    }
                })
            });

            if cached_user_id.is_none()
                && let Some(cache) = auth_cache.as_ref()
                && cache.contains_key(&cache_key)
            {
                cache.remove(&cache_key);
            }

            if let Some(user_id) = cached_user_id {
                let is_admin = cfg
                    .as_ref()
                    .map(|c| c.super_admin_ids.contains(&user_id))
                    .unwrap_or(false);
                return finish_auth(parts, body, next, user_id, is_admin, &path).await;
            }

            // 缓存未命中——查询数据库验证
            let db = db.ok_or_else(|| {
                (StatusCode::INTERNAL_SERVER_ERROR, "db not available").into_response()
            })?;

            match db::verify_user(&db, email, password).await {
                Ok(Some(user)) => {
                    if let Some(cache) = auth_cache.as_ref() {
                        let ttl = Duration::from_secs(constant::AUTH_CACHE_TTL_SECS);
                        cache.insert(
                            cache_key,
                            AuthCacheEntry {
                                user_id: user.id,
                                expires_at: Instant::now() + ttl,
                            },
                        );
                    }
                    let is_admin = cfg
                        .as_ref()
                        .map(|c| c.super_admin_ids.contains(&user.id))
                        .unwrap_or(false);
                    finish_auth(parts, body, next, user.id, is_admin, &path).await
                }
                Ok(None) => {
                    // 按客户端 IP 限制 Basic Auth 失败速率，
                    // 防止暴力破解密码猜测。
                    if let Some(ref limiter) = basic_auth_fail_limiter {
                        let client_ip = extract_client_ip(&parts);
                        if !limiter.check_and_record_failure(client_ip) {
                            tracing::warn!(ip = %client_ip, "Basic Auth rate limited");
                            return Err((
                                StatusCode::TOO_MANY_REQUESTS,
                                "too many auth attempts, slow down",
                            )
                                .into_response());
                        }
                    }
                    Err((StatusCode::UNAUTHORIZED, "invalid credentials").into_response())
                }
                Err(_) => Err((StatusCode::INTERNAL_SERVER_ERROR, "auth error").into_response()),
            }
        } else {
            // 未知的认证方案
            Err((
                StatusCode::UNAUTHORIZED,
                "unsupported auth scheme, use Bearer or Basic",
            )
                .into_response())
        }
    } else {
        // 没有 Authorization 头部
        Err((StatusCode::UNAUTHORIZED, "missing Authorization header").into_response())
    }
}

/// 完成认证：插入 `UserId`（如适用也插入 `AdminUserId`），
/// 然后在转发到处理器前检查管理员要求。
async fn finish_auth(
    mut parts: Parts,
    body: axum::body::Body,
    next: Next,
    user_id: i64,
    is_admin: bool,
    path: &str,
) -> Result<Response, Response> {
    // 在处理前检查需要管理员权限的路径
    if constant::ADMIN_REQUIRED_PATHS
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        if !is_admin {
            return Err((StatusCode::FORBIDDEN, "admin access required").into_response());
        }
        parts.extensions.insert(AdminUserId(user_id));
    }

    parts.extensions.insert(UserId(user_id));
    let req = axum::extract::Request::from_parts(parts, body);
    Ok(next.run(req).await)
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 从请求部分提取客户端 IP 地址。
///
/// 检查 `X-Forwarded-For`（第一个条目），然后 `X-Real-IP`，然后回退到
/// 套接字对端地址。当没有可用地址时返回 `0.0.0.0`
///（这仍然允许速率限制工作，只是将所有未知客户端分组）。
fn extract_client_ip(parts: &Parts) -> IpAddr {
    // X-Forwarded-For：客户端、代理1、代理2、...
    if let Some(fwd) = parts.headers.get("x-forwarded-for")
        && let Ok(val) = fwd.to_str()
        && let Some(first) = val.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // X-Real-IP（单个 IP，通常由 nginx 设置）
    if let Some(real) = parts.headers.get("x-real-ip")
        && let Ok(val) = real.to_str()
        && let Ok(ip) = val.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // 回退：使用连接信息中的对端套接字地址
    // 在 Axum 中，当使用 `into_make_service_with_connect_info` 的 `axum::serve` 时，
    // 可通过 `ConnectInfo` 扩展获得。
    if let Some(addr) = parts.extensions.get::<std::net::SocketAddr>() {
        return addr.ip();
    }

    // 最后手段：一个仍然可以对未知客户端分组的哨兵值
    IpAddr::from([0, 0, 0, 0])
}
