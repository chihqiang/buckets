use crate::app::AppState;
use crate::db::UserId;
use crate::service::tus_svc;
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use buckets_common::constant;
use buckets_common::error::AppError;
use uuid::Uuid;

/// 为 tus 响应构建基础头部。
fn tus_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        constant::HEADER_TUS_RESUMABLE,
        constant::TUS_PROTOCOL_VERSION.parse().unwrap(),
    );
    headers
}

/// 将 tus 操作结果包装成带有 tus 头部的 HTTP 响应。
fn into_tus_response(result: Result<(StatusCode, Option<HeaderMap>), AppError>) -> Response {
    match result {
        Ok((status, extra_headers)) => {
            let mut headers = tus_headers();
            if let Some(extra) = extra_headers {
                headers.extend(extra);
            }
            (status, headers).into_response()
        }
        Err(err) => {
            let status = err.status_code();
            let mut headers = tus_headers();
            headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            let body = serde_json::json!({
                "code": status,
                "message": err.to_string(),
                "data": null,
            });
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                headers,
                axum::Json(body),
            )
                .into_response()
        }
    }
}

/// OPTIONS —— 返回服务器能力。
pub async fn tus_options() -> Response {
    let mut headers = tus_headers();
    headers.insert(
        constant::HEADER_TUS_VERSION,
        constant::TUS_PROTOCOL_VERSION.parse().unwrap(),
    );
    headers.insert(
        constant::HEADER_TUS_EXTENSION,
        constant::TUS_SUPPORTED_EXTENSIONS.parse().unwrap(),
    );
    headers.insert(
        constant::HEADER_TUS_MAX_SIZE,
        constant::TUS_DEFAULT_MAX_SIZE.to_string().parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    (StatusCode::NO_CONTENT, headers).into_response()
}

/// POST —— 创建新的 tus 上传。
pub async fn tus_create(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    headers: HeaderMap,
) -> Response {
    let result = try_tus_create(state, uid, headers).await;
    into_tus_response(result)
}

async fn try_tus_create(
    state: AppState,
    uid: UserId,
    headers: HeaderMap,
) -> Result<(StatusCode, Option<HeaderMap>), AppError> {
    if !headers.contains_key(constant::HEADER_TUS_RESUMABLE) {
        return Err(AppError::BadRequest("missing Tus-Resumable header".into()));
    }

    let is_deferred = headers
        .get(constant::HEADER_UPLOAD_DEFER_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim() == "1")
        .unwrap_or(false);

    let upload_length = if is_deferred {
        if headers.contains_key(constant::HEADER_UPLOAD_LENGTH) {
            return Err(AppError::BadRequest(
                "Upload-Length must not be provided when Upload-Defer-Length is 1".into(),
            ));
        }
        0
    } else {
        let len = headers
            .get(constant::HEADER_UPLOAD_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| AppError::BadRequest("missing or invalid Upload-Length header".into()))?;

        if len < 0 {
            return Err(AppError::BadRequest("Upload-Length must be non-negative".into()));
        }
        len
    };

    let metadata_header = headers
        .get(constant::HEADER_UPLOAD_METADATA)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let (task, _object_key) = tus_svc::create_upload(&state.db, uid.0, upload_length, metadata_header, is_deferred).await?;

    let location = format!("/api/v1/upload/tus/{}", task.uuid);
    let mut extra = HeaderMap::new();
    extra.insert(header::LOCATION, location.parse().unwrap());

    Ok((StatusCode::CREATED, Some(extra)))
}

/// HEAD —— 获取上传的当前偏移量。
pub async fn tus_head(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(task_id): Path<String>,
) -> Response {
    let result = try_tus_head(state, uid, task_id).await;
    into_tus_response(result)
}

async fn try_tus_head(
    state: AppState,
    uid: UserId,
    task_id: String,
) -> Result<(StatusCode, Option<HeaderMap>), AppError> {
    let task_uuid = Uuid::parse_str(&task_id)
        .map_err(|_| AppError::BadRequest("invalid task id".into()))?;

    let task = crate::dao::find_upload_task(&state.db, task_uuid)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != uid.0 {
        return Err(AppError::Forbidden("task does not belong to user".into()));
    }

    if task.status_enum().is_terminal() && task.status != constant::STATUS_COMPLETED {
        return Err(AppError::NotFound("upload expired or failed".into()));
    }

    let mut extra = HeaderMap::new();
    extra.insert(
        constant::HEADER_UPLOAD_OFFSET,
        task.current_offset.to_string().parse().unwrap(),
    );
    // Upload-Defer-Length: 在客户端告知文件大小之前，不返回 Upload-Length
    if !task.is_deferred || task.file_size > 0 {
        extra.insert(
            constant::HEADER_UPLOAD_LENGTH,
            task.file_size.to_string().parse().unwrap(),
        );
    }
    extra.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());

    Ok((StatusCode::OK, Some(extra)))
}

/// PATCH —— 上传数据到现有资源。
pub async fn tus_patch(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    let result = try_tus_patch(state, uid, task_id, headers, body).await;
    into_tus_response(result)
}

async fn try_tus_patch(
    state: AppState,
    uid: UserId,
    task_id: String,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<(StatusCode, Option<HeaderMap>), AppError> {
    if !headers.contains_key(constant::HEADER_TUS_RESUMABLE) {
        return Err(AppError::BadRequest("missing Tus-Resumable header".into()));
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type != "application/offset+octet-stream" {
        return Err(AppError::BadRequest(
            "Content-Type must be application/offset+octet-stream".into(),
        ));
    }

    let upload_offset = headers
        .get(constant::HEADER_UPLOAD_OFFSET)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| AppError::BadRequest("missing or invalid Upload-Offset header".into()))?;

    let task_uuid = Uuid::parse_str(&task_id)
        .map_err(|_| AppError::BadRequest("invalid task id".into()))?;

    let upload_length = headers
        .get(constant::HEADER_UPLOAD_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok());

    let new_offset = tus_svc::append_data(&state.db, uid.0, task_uuid, upload_offset, upload_length, body).await?;

    let mut extra = HeaderMap::new();
    extra.insert(
        constant::HEADER_UPLOAD_OFFSET,
        new_offset.to_string().parse().unwrap(),
    );
    extra.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());

    Ok((StatusCode::NO_CONTENT, Some(extra)))
}

/// DELETE —— 终止上传并清理资源。
pub async fn tus_terminate(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(task_id): Path<String>,
) -> Response {
    let result = try_tus_terminate(state, uid, task_id).await;
    into_tus_response(result)
}

async fn try_tus_terminate(
    state: AppState,
    uid: UserId,
    task_id: String,
) -> Result<(StatusCode, Option<HeaderMap>), AppError> {
    let task_uuid = Uuid::parse_str(&task_id)
        .map_err(|_| AppError::BadRequest("invalid task id".into()))?;

    let task = crate::dao::find_upload_task(&state.db, task_uuid)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != uid.0 {
        return Err(AppError::Forbidden("task does not belong to user".into()));
    }

    let staging_dir = std::path::PathBuf::from(constant::staging_dir())
        .join(constant::TUS_STAGING_SUBDIR)
        .join(&task_id);
    if staging_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    }

    let _ = crate::dao::update_upload_status(&state.db, task_uuid, "expired").await;

    Ok((StatusCode::NO_CONTENT, None))
}
