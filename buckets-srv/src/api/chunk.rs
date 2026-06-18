//! 网关分块上传和状态 API 处理器。

use crate::app::AppState;
use crate::db::UserId;
use crate::middleware::auth::SecretKeyCache;
use crate::service::chunk_svc;
use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{
    ApiResponse, ChunkStatusResponse, ChunkUploadResponse, api_ok,
};

/// 二进制分块上传的仅查询参数（task_id、chunk_index、chunk_md5）。
/// 会话签名字段从自定义 HTTP 头部中提取，而不是从
/// URL 查询参数中提取，以避免 URL 长度限制和日志/历史记录暴露。
#[derive(serde::Deserialize)]
pub struct BinaryChunkQuery {
    pub task_id: uuid::Uuid,
    pub chunk_index: u32,
    pub chunk_md5: String,
}

/// 提取必需的头部值，如果缺失或无效则返回 BadRequest。
fn required_header(headers: &HeaderMap, name: &str) -> Result<String, AppError> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::BadRequest(format!("missing or invalid header: {name}")))
}

/// 二进制分块上传——请求体为原始字节，无 Base64/JSON 开销。
/// 使用会话级别签名（此上传中的所有分块使用一个签名）。
///
/// 会话签名参数通过自定义 HTTP 头部传递
///（X-Session-Signature、X-Session-Timestamp、X-Session-Salt），
/// 以避免 URL 长度限制和暴露在访问日志/浏览器历史中。
///
/// 通过临时文件将请求体直接流式写入磁盘，避免将整个分块
///（最多 256 MiB）加载到内存中。使用 axum::body::Body 进行流式
/// 写入，因此并发的大分块上传不会导致 OOM。
pub async fn upload_chunk_binary(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Extension(sk_cache): Extension<SecretKeyCache>,
    Query(query): Query<BinaryChunkQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ApiResponse<ChunkUploadResponse>>, AppError> {
    let session_signature = required_header(&headers, constant::HEADER_SESSION_SIGNATURE)?;
    let session_timestamp: i64 = required_header(&headers, constant::HEADER_SESSION_TIMESTAMP)?
        .parse()
        .map_err(|_| AppError::BadRequest("invalid session timestamp header".into()))?;
    let session_salt = required_header(&headers, constant::HEADER_SESSION_SALT)?;

    let result = chunk_svc::upload_chunk_binary_stream(
        &state.db,
        &sk_cache,
        uid.0,
        query.task_id,
        query.chunk_index,
        query.chunk_md5,
        session_signature,
        session_timestamp,
        session_salt,
        body,
    )
    .await?;
    Ok(api_ok(result))
}

/// 查询上传任务的上传状态。
pub async fn chunk_status(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Json(req): Json<buckets_common::model::api::ChunkStatusReq>,
) -> Result<Json<ApiResponse<ChunkStatusResponse>>, AppError> {
    let result = chunk_svc::chunk_status(&state.db, uid.0, req.task_id).await?;
    Ok(api_ok(result))
}
