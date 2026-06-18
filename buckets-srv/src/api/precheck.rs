//! 网关预检 API 处理器——检查去重和可续传上传。

use crate::app::AppState;
use crate::db::UserId;
use crate::service::file_svc;
use axum::Json;
use axum::extract::{Extension, State};
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, PrecheckRequest, PrecheckResult};

/// 上传前预检文件：去重检查和续传支持。
pub async fn precheck_file(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Json(req): Json<PrecheckRequest>,
) -> Result<Json<ApiResponse<PrecheckResult>>, AppError> {
    let result = file_svc::precheck(
        &state.db,
        &req.file_md5,
        req.file_size,
        uid.0,
        req.chunk_size,
        &req.file_name,
    )
    .await?;

    // 创建新任务时增加并发上传计数器
    if result.task_id.is_some()
        && !result.exists
        && let Some(ref limiter) = state.rate_limiter
    {
        limiter.increment_concurrent(uid.0);
    }

    Ok(Json(ApiResponse {
        code: 200,
        message: "ok".into(),
        data: Some(result),
    }))
}
