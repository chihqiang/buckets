//! 用户管理 API——仅超级管理员。
//!
//! 所有端点都需要超级管理员权限；由认证中间件通过
//! [`ADMIN_REQUIRED_PATHS`] 强制执行。处理器使用 `Extension<AdminUserId>`
//! 读取已由中间件验证的管理员用户 ID。

use crate::app::AppState;
use crate::dao::{self, UserRow};
use crate::middleware::auth::AdminUserId;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use buckets_common::error::AppError;
use buckets_common::model::api::{
    ApiResponse, CreateUserRequest, PaginatedResponse, UpdateUserRequest, UserInfo, api_ok,
};
use buckets_common::utils::password;
use serde::Deserialize;

/// 用户列表的查询参数。
#[derive(Deserialize)]
pub struct UserListQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}

fn into_user_info(u: UserRow) -> UserInfo {
    UserInfo {
        id: u.id,
        email: u.email,
        created_at: u.created_at,
        updated_at: u.updated_at,
    }
}

/// GET /api/v1/users — 列出所有用户（分页）。
pub async fn list_users(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Query(q): Query<UserListQuery>,
) -> Result<Json<ApiResponse<PaginatedResponse<UserInfo>>>, AppError> {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).min(100);

    let (users, total) = dao::list_users(&state.db, page, page_size).await?;

    Ok(api_ok(PaginatedResponse {
        items: users.into_iter().map(into_user_info).collect(),
        total,
        page,
        page_size,
    }))
}

/// GET /api/v1/users/:id — 获取单个用户。
pub async fn get_user(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let user = dao::get_user(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;
    Ok(api_ok(into_user_info(user)))
}

/// POST /api/v1/users — 创建新用户。
pub async fn create_user(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let password_hash = password::hash_password(&req.password)?;
    let secret_key = dao::generate_secret_key();

    let user_id = dao::create_user(&state.db, &req.email, &password_hash, &secret_key).await?;

    // 回查已创建的用户
    let user = dao::get_user(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::Internal("user created but not found".into()))?;

    Ok(api_ok(into_user_info(user)))
}

/// PUT /api/v1/users/:id — 更新用户邮箱和/或密码。
pub async fn update_user(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Path(user_id): Path<i64>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    let password_hash = req
        .password
        .as_ref()
        .map(|p| password::hash_password(p))
        .transpose()?;

    let updated = dao::update_user(
        &state.db,
        user_id,
        req.email.as_deref(),
        password_hash.as_deref(),
    )
    .await?;

    if !updated {
        return Err(AppError::NotFound("user not found".into()));
    }

    let user = dao::get_user(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::Internal("user not found after update".into()))?;

    Ok(api_ok(into_user_info(user)))
}

/// DELETE /api/v1/users/:id — 删除用户。
pub async fn delete_user(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let deleted = dao::delete_user(&state.db, user_id).await?;
    if !deleted {
        return Err(AppError::NotFound("user not found".into()));
    }
    Ok(api_ok(()))
}

/// POST /api/v1/users/:id/reset-secret-key — 重置用户的 secret_key。
pub async fn reset_user_secret_key(
    State(state): State<AppState>,
    Extension(_admin): Extension<AdminUserId>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let new_key = dao::generate_secret_key();
    let updated = dao::reset_user_secret_key(&state.db, user_id, &new_key).await?;
    if !updated {
        return Err(AppError::NotFound("user not found".into()));
    }
    tracing::info!(user_id = user_id, "secret key reset");
    Ok(api_ok(()))
}
