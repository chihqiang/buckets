//! 文件管理 API——Web 管理。
//!
//! - 超级管理员可以看到所有文件（跨所有用户）。
//! - 普通用户只能看到自己的文件。
//! - DELETE 移除用户-对象关联（不是物理文件）。

use crate::app::AppState;
use crate::dao::{self, ObjectRow};
use crate::db::UserId;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, ObjectInfo, PaginatedResponse, api_ok};
use serde::Deserialize;

/// 文件列表查询，可选的 user_id 过滤器（仅超级管理员）。
#[derive(Deserialize)]
pub struct FileListQueryExt {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    /// 按用户 ID 过滤（仅超级管理员；普通用户忽略）。
    pub user_id: Option<u64>,
}

fn into_object_info(f: ObjectRow) -> ObjectInfo {
    ObjectInfo {
        id: f.id,
        uuid: f.uuid,
        name: f.name,
        size: f.size,
        md5: f.md5,
        content_type: f.content_type,
        extension: f.extension,
        bucket: f.bucket,
        storage_path: f.storage_path,
        image_width: f.image_width,
        image_height: f.image_height,
        image_type: f.image_type,
        status: f.status,
        created_at: f.created_at,
        updated_at: f.updated_at,
    }
}

/// GET /api/v1/objects — 列出文件。
///
/// 超级管理员：查看所有文件，可按 `?user_id=` 过滤。
/// 普通用户：仅查看自己的文件。
pub async fn list_objects(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Query(q): Query<FileListQueryExt>,
) -> Result<Json<ApiResponse<PaginatedResponse<ObjectInfo>>>, AppError> {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).min(100);

    let is_super_admin = state.cfg.super_admin_ids.contains(&uid.0);

    let (files, total) = if is_super_admin {
        // 超级管理员——可以查看所有文件，可按 user_id 过滤
        if let Some(filter_user_id) = q.user_id {
            dao::list_objects_by_user(&state.db, filter_user_id, page, page_size).await?
        } else {
            dao::list_all_objects(&state.db, page, page_size).await?
        }
    } else {
        // 普通用户——仅查看自己的文件
        dao::list_objects_by_user(&state.db, uid.0, page, page_size).await?
    };

    Ok(api_ok(PaginatedResponse {
        items: files.into_iter().map(into_object_info).collect(),
        total,
        page,
        page_size,
    }))
}

/// DELETE /api/v1/objects/:id — 移除文件关联。
///
/// 移除当前用户的 user_objects 关联。
/// 如果是最后一个所有者，对象也会被软删除。
pub async fn delete_object(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(object_uuid): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    // 验证对象存在且用户拥有它
    let owns = dao::check_user_owns_object_by_uuid(&state.db, uid.0, &object_uuid).await?;
    if !owns {
        return Err(AppError::NotFound("file not found".into()));
    }

    // 检查是否是最后一个所有者
    let is_last = dao::remove_user_object_by_uuid(&state.db, uid.0, &object_uuid).await?;
    if is_last {
        dao::soft_delete_object(&state.db, &object_uuid).await?;
    }

    tracing::info!(user_id = uid.0, object_id = object_uuid, "file deleted");

    Ok(api_ok(()))
}
