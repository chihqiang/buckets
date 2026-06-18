//! 统一认证处理器：登录、刷新、登出、验证。
//!
//! 由网关和 Web 管理路由共享。
//! 所有令牌均为标准 JWT（HS256）。

use crate::app::AppState;
use crate::dao;
use crate::db::UserId;
use axum::Json;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{
    ApiResponse, LoginRequest, LoginResponse, RefreshRequest, VerifyResponse, api_ok,
};
use buckets_common::utils::crypto;

/// POST /api/v1/auth/login
///
/// 验证 email+password，返回 JWT 访问令牌 + 刷新令牌对。
/// 访问令牌使用用户的用户特定密钥（HS256）签名。
/// 刷新令牌使用全局服务器密钥（HS256）签名。
pub async fn login(
    state: State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let user = dao::verify_user(&state.db, &req.email, &req.password)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "verify_user failed");
            AppError::Internal("auth error".into())
        })?
        .ok_or(AppError::Unauthorized)?;

    let secret_key = user.secret_key.as_str();

    let (token, _) = crypto::generate_access_token(secret_key.as_bytes(), user.id)?;
    let (refresh_token, _) = crypto::generate_refresh_token(user.id)?;

    let is_super_admin = state.cfg.super_admin_ids.contains(&user.id);

    Ok(api_ok(LoginResponse {
        token,
        refresh_token,
        expires_in: constant::AUTH_TOKEN_TTL_SECS,
        is_super_admin,
    }))
}

/// POST /api/v1/auth/refresh
///
/// 用刷新令牌换取新的 JWT 访问令牌 + 刷新令牌对。
pub async fn refresh(
    state: State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    // 使用全局服务器密钥验证刷新令牌。
    let (user_id, _jti) =
        crypto::verify_refresh_token(&req.refresh_token).ok_or(AppError::Unauthorized)?;

    // 获取用户的密钥（用于签发新的访问令牌）
    let secret_key = dao::get_user_secret_key(&state.db, user_id).await?;

    // 签发新的一对
    let (token, _) = crypto::generate_access_token(secret_key.as_bytes(), user_id)?;
    let (refresh_token, _) = crypto::generate_refresh_token(user_id)?;

    let is_super_admin = state.cfg.super_admin_ids.contains(&user_id);

    Ok(api_ok(LoginResponse {
        token,
        refresh_token,
        expires_in: constant::AUTH_TOKEN_TTL_SECS,
        is_super_admin,
    }))
}

/// POST /api/v1/auth/logout
///
/// 令牌通过 `Authorization: Bearer <token>` 头部传递。
/// 需要认证。
pub async fn logout(
    _state: State<AppState>,
    Extension(uid): Extension<UserId>,
    _headers: HeaderMap,
) -> Result<Json<ApiResponse<()>>, AppError> {
    tracing::info!(user_id = uid.0, "user logged out");
    Ok(api_ok(()))
}

/// POST /api/v1/auth/verify
///
/// 专用凭据验证端点。需要认证。
/// 如果凭据有效，返回认证用户的 ID。
/// 这比之前发送虚拟 STS 请求的方法更简洁。
pub async fn verify_credentials(
    Extension(uid): Extension<UserId>,
) -> Result<Json<ApiResponse<VerifyResponse>>, AppError> {
    Ok(api_ok(VerifyResponse { user_id: uid.0 }))
}
