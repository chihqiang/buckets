//! 直接上传 API 处理器——通过 multipart/form-data 上传文件。
//!
//! 浏览器自动在 multipart 字段中携带文件名和内容类型，
//! 无需自定义 HTTP 头。

use crate::app::AppState;
use crate::db::UserId;
use crate::service::file_svc;
use axum::extract::{Extension, Multipart, State};
use axum::Json;
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, MergeResult, api_ok};

/// 直接上传文件——请求体为 multipart/form-data。
///
/// 表单字段名固定为 `file`，浏览器自动填充文件名和内容类型。
pub async fn direct_upload(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<MergeResult>>, AppError> {
    let mut file_name = String::new();
    let mut content_type: Option<String> = None;
    let mut data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {}", e)))?
    {
        if field.name() == Some("file") {
            file_name = field
                .file_name()
                .ok_or_else(|| AppError::BadRequest("missing filename in multipart field".into()))?
                .to_string();
            content_type = field
                .content_type()
                .map(|m| m.to_string());
            data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("read file data: {}", e)))?
                    .to_vec(),
            );
        }
    }

    if file_name.is_empty() {
        return Err(AppError::BadRequest("missing file field in multipart form".into()));
    }

    let file_data = data.ok_or_else(|| AppError::BadRequest("missing file data".into()))?;
    let result = file_svc::direct_upload_bytes(
        &state.db,
        uid.0,
        &file_name,
        content_type.as_deref(),
        &file_data,
    )
    .await?;
    Ok(api_ok(result))
}
