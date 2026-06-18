//! 整个 buckets 应用的统一错误类型。
//!
//! `AppError` 将每个变体映射到 HTTP 状态码，并实现
//! `IntoResponse` 以直接在 Axum 处理器中使用。内部错误
//!（存储、数据库、内部）的详细信息在 API 响应中被清理，
//! 但在日志中完整记录以用于调试。

/// 应用全局错误枚举，包含 HTTP 状态码语义。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal error")]
    Internal(String),

    #[error("chunk already exists")]
    ChunkAlreadyExists,

    #[error("chunk not found: {0}")]
    ChunkNotFound(String),

    #[error("upload incomplete: missing {0} chunks")]
    UploadIncomplete(u32),

    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("signature expired")]
    SignatureExpired,

    #[error("signature invalid")]
    SignatureInvalid,

    #[error("file too large: {0}")]
    FileTooLarge(String),

    #[error("invalid file type: {0}")]
    InvalidFileType(String),

    #[error("storage error")]
    StorageError(String),

    #[error("database error")]
    DatabaseError(String),
}

// 仅供内部日志使用——暴露完整的错误详情。
impl AppError {
    /// 返回完整的内部错误消息（仅用于日志记录）。
    /// 敏感信息不应返回给 API 调用方。
    pub fn internal_message(&self) -> String {
        match self {
            AppError::Internal(msg)
            | AppError::StorageError(msg)
            | AppError::DatabaseError(msg) => msg.clone(),
            _ => self.to_string(),
        }
    }
}

impl AppError {
    /// 将错误变体映射到 HTTP 状态码。
    pub fn status_code(&self) -> u16 {
        match self {
            AppError::BadRequest(_) => 400,
            AppError::Unauthorized => 401,
            AppError::Forbidden(_) => 403,
            AppError::NotFound(_) => 404,
            AppError::Conflict(_) => 409,
            AppError::SignatureExpired => 401,
            AppError::SignatureInvalid => 401,
            AppError::FileTooLarge(_) => 413,
            AppError::UploadIncomplete(_) => 409,
            AppError::HashMismatch { .. } => 409,
            AppError::ChunkNotFound(_) => 404,
            AppError::ChunkAlreadyExists => 409,
            AppError::InvalidFileType(_) => 415,
            AppError::StorageError(_) => 500,
            AppError::DatabaseError(_) => 500,
            AppError::Internal(_) => 500,
        }
    }
}

/// 为 `AppError` 实现 `IntoResponse`，消除样板处理器代码。
/// 内部错误被清理：只返回通用消息，而完整错误详情通过 `tracing::error!` 记录。
/// 所有错误都在服务端记录以用于调试。
#[cfg(feature = "axum")]
impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status_code = self.status_code();
        let (status, message) = match &self {
            AppError::StorageError(msg)
            | AppError::DatabaseError(msg)
            | AppError::Internal(msg) => {
                tracing::error!(status = status_code, error = %msg, "request failed");
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
            _ => {
                match status_code {
                    500..=599 => {
                        tracing::error!(status = status_code, error = %self, "request failed")
                    }
                    400..=499 => {
                        tracing::warn!(status = status_code, error = %self, "request failed")
                    }
                    _ => tracing::info!(status = status_code, error = %self, "request failed"),
                }
                (
                    axum::http::StatusCode::from_u16(status_code)
                        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
                    self.to_string(),
                )
            }
        };

        let body = serde_json::json!({
            "code": status.as_u16(),
            "message": message,
            "data": null,
        });

        (status, axum::Json(body)).into_response()
    }
}

/// 将 `sea_orm::DbErr` 转换为 `DatabaseError` 变体。
impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError::DatabaseError(e.to_string())
    }
}

/// 将 `std::io::Error` 转换为 `StorageError` 变体。
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::StorageError(e.to_string())
    }
}
