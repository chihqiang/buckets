//! 对象操作处理器：元数据查询、删除、文件下载。

use crate::app::AppState;
use crate::dao;
use crate::db::UserId;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::Json;
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, api_ok};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

/// 通过自增 ID 获取对象元数据（通过 user_objects 限定所有者范围）。
pub async fn get_object_info(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(id): Path<i64>,
) -> Result<Json<ApiResponse<buckets_common::model::db::ObjectMeta>>, AppError> {
    let obj = dao::find_object_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound("object not found".into()))?;

    if !dao::check_user_owns_object_by_id(&state.db, uid.0, id).await? {
        return Err(AppError::Forbidden("object does not belong to user".into()));
    }

    Ok(api_ok(obj))
}

/// 通过自增 ID 移除用户-对象关联。
/// 如果用户是最后一个所有者，对象也被清理。
pub async fn delete_object(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    dao::delete_user_object_by_id(&state.db, uid.0, id).await?;
    Ok(api_ok(()))
}

/// 解析 HTTP Range 头中的 bytes 范围。
/// 支持格式: `bytes=start-end`、`bytes=start-`、`bytes=-suffix`
fn parse_byte_range(range: &str, file_size: u64) -> Option<(u64, u64)> {
    let range = range.strip_prefix("bytes=")?;
    let (start_part, end_part) = range.split_once('-')?;

    let (start, end) = if start_part.is_empty() {
        let suffix: u64 = end_part.parse().ok()?;
        if suffix == 0 || suffix > file_size {
            return None;
        }
        (file_size - suffix, file_size - 1)
    } else {
        let s: u64 = start_part.parse().ok()?;
        if s >= file_size {
            return None;
        }
        if end_part.is_empty() {
            (s, file_size - 1)
        } else {
            let e: u64 = end_part.parse().ok()?;
            if e < s {
                return None;
            }
            (s, e.min(file_size - 1))
        }
    };

    Some((start, end))
}

/// GET /api/v1/object/{id}/download — 下载文件，支持断点续传。
pub async fn download_object(
    State(state): State<AppState>,
    Extension(uid): Extension<UserId>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let obj = dao::find_object_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound("object not found".into()))?;

    if !dao::check_user_owns_object_by_id(&state.db, uid.0, id).await? {
        return Err(AppError::Forbidden("object does not belong to user".into()));
    }

    let path = std::path::Path::new(&obj.storage_path);
    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
        tracing::error!(storage_path = %obj.storage_path, error = %e, "download file not found on disk");
        AppError::Internal("file not found on storage".into())
    })?;

    let file_size = obj.size as u64;
    let content_type = obj
        .content_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());

    let content_disposition = format!("inline; filename=\"{}\"", obj.name);

    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    let mut res_headers = axum::http::HeaderMap::new();
    res_headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    res_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    res_headers.insert(header::CONTENT_DISPOSITION, content_disposition.parse().unwrap());

    if let Some(range_str) = range_header
        && let Some((start, end)) = parse_byte_range(range_str, file_size)
    {
        let length = end - start + 1;
        file.seek(SeekFrom::Start(start)).await?;
        let limited = file.take(length);
        let stream = ReaderStream::new(limited);

        res_headers.insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());
        res_headers.insert(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", start, end, file_size).parse().unwrap(),
        );

        return Ok((
            StatusCode::PARTIAL_CONTENT,
            res_headers,
            Body::from_stream(stream),
        ));
    }

    res_headers.insert(header::CONTENT_LENGTH, file_size.to_string().parse().unwrap());
    Ok((
        StatusCode::OK,
        res_headers,
        Body::from_stream(ReaderStream::new(file)),
    ))
}
