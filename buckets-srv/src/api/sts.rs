//! 网关 STS 和对象 API 处理器：STS 令牌签发、对象信息、对象删除。

use crate::app::AppState;
use crate::dao;
use crate::db::UserId;
use crate::service::auth_svc;
use axum::Json;
use axum::extract::{Extension, Path, State};
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, StsRequest, StsResult, api_ok};

/// 为新上传会话签发 STS 令牌。
pub async fn get_sts_token(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Json(req): Json<StsRequest>,
) -> Result<Json<ApiResponse<StsResult>>, AppError> {
    let result = auth_svc::issue_sts(&state.db, uid.0, &req).await?;
    Ok(api_ok(result))
}

/// 通过 ID 获取对象元数据（通过 user_objects 限定所有者范围）。
pub async fn get_object_info(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(object_id): Path<String>,
) -> Result<Json<ApiResponse<buckets_common::model::db::ObjectMeta>>, AppError> {
    let obj = dao::find_object_by_uuid(&state.db, &object_id)
        .await?
        .ok_or(AppError::NotFound("object not found".into()))?;

    // 通过 user_objects 关联表检查所有权
    if !dao::check_user_owns_object_by_uuid(&state.db, uid.0, &object_id).await? {
        return Err(AppError::Forbidden("object does not belong to user".into()));
    }

    Ok(api_ok(obj))
}

/// 通过 ID 软删除对象（通过 user_objects 限定所有者范围）。
/// 如果用户是最后一个所有者，对象被标记为已删除。
/// 如果还有其他所有者，仅移除用户-对象关联。
pub async fn delete_object(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(object_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    // 通过 user_objects 关联表检查所有权
    if !dao::check_user_owns_object_by_uuid(&state.db, uid.0, &object_id).await? {
        return Err(AppError::Forbidden("object does not belong to user".into()));
    }

    // 移除用户-对象关联；如果是最后一个所有者，软删除对象
    let is_last = dao::remove_user_object_by_uuid(&state.db, uid.0, &object_id).await?;
    if is_last {
        dao::soft_delete_object(&state.db, &object_id).await?;
    }

    Ok(api_ok(()))
}
