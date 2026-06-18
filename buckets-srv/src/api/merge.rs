//! 网关合并 API 处理器——将所有上传的分块合并到最终对象。
//!
//! 合并现在是**异步的**：端点立即返回 202 Accepted，
//! 合并操作在后台运行。客户端轮询 `/upload/merge/status` 以检查完成状态。

use crate::app::AppState;
use crate::dao;
use crate::db::UserId;
use crate::service::file_svc;
use axum::Json;
use axum::extract::{Extension, Query, State};
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{
    ApiResponse, MergeAcceptedResult, MergeRequest, MergeStatusResponse,
};
use buckets_common::model::db::TaskStatus;
use serde::Deserialize;

/// 请求合并——在后台启动合并，立即返回 202 Accepted。
pub async fn merge_chunks(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Json(req): Json<MergeRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    // 验证上传任务存在且属于用户
    let task = dao::find_upload_task(&state.db, req.task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != uid.0 {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    // 防止重复合并
    if task.status_enum() == TaskStatus::Merging || task.status_enum() == TaskStatus::Completed {
        return Err(AppError::BadRequest(format!(
            "upload task is already {}",
            task.status
        )));
    }

    let task_id = req.task_id;
    let file_name = req.file_name.clone();
    let file_md5 = req.file_md5.clone();
    let file_size = req.file_size;
    let content_type = req.content_type.clone();
    let db = state.db.clone();

    // 在启动前标记为 merging——后台任务可能不会立即启动
    //（tokio 运行时调度），我们需要客户端第一次轮询时
    // 看到 'merging' 而不是 'uploading'。
    let _ = dao::update_upload_status(&state.db, task_id, TaskStatus::Merging.as_str()).await;

    // 在后台启动合并——不阻塞 HTTP 响应
    let limiter = state.rate_limiter.clone();
    tokio::spawn(async move {
        let result = file_svc::merge(
            &db,
            uid.0,
            &file_name,
            &file_md5,
            file_size,
            content_type.as_deref(),
            task_id,
        )
        .await;

        match &result {
            Ok(_) => tracing::info!(%task_id, "merge completed successfully"),
            Err(e) => {
                // 合并失败——确保状态已更新，以便客户端
                // 轮询循环终止而不是永远挂起。
                tracing::error!(%task_id, error = %e, "merge failed");
                let _ = dao::update_upload_status(&db, task_id, TaskStatus::Failed.as_str()).await;
            }
        }

        // 合并完成时始终递减并发计数器
        if let Some(ref limiter) = limiter {
            limiter.decrement_concurrent(uid.0);
        }
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(ApiResponse {
            code: 202,
            message: "merge accepted".into(),
            data: Some(MergeAcceptedResult {
                task_id: task_id.to_string(),
                message: "merge started, poll /upload/merge/status for completion".into(),
            }),
        }),
    ))
}

/// 合并状态轮询的查询参数。
#[derive(Deserialize)]
pub struct MergeStatusQuery {
    pub task_id: uuid::Uuid,
}

/// 轮询合并状态——返回合并操作的当前状态。
pub async fn merge_status(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Query(params): Query<MergeStatusQuery>,
) -> Result<Json<ApiResponse<MergeStatusResponse>>, AppError> {
    let task = dao::find_upload_task(&state.db, params.task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != uid.0 {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    let (status, storage_path) = match task.status_enum() {
        TaskStatus::Merging => (constant::STATUS_MERGING.into(), None),
        TaskStatus::Completed => {
            let obj = dao::find_object_by_uuid(&state.db, &task.object_id)
                .await?
                .map(|o| o.storage_path);
            (constant::STATUS_COMPLETED.into(), obj)
        }
        TaskStatus::Failed => (constant::STATUS_FAILED.into(), None),
        s => (s.to_string(), None),
    };

    Ok(Json(ApiResponse {
        code: 200,
        message: "ok".into(),
        data: Some(MergeStatusResponse {
            task_id: task.uuid,
            status,
            storage_path,
            error: if task.status == "failed" {
                Some("merge failed".into())
            } else {
                None
            },
        }),
    }))
}
