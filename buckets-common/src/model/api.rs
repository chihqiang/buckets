//! 网关和 CLI 之间共享的 API 请求/响应 DTO。
//!
//! 所有响应类型派生 `Serialize`，请求类型派生 `Deserialize`。
//! 已废弃的逐分块签名类型保留为注释以供参考。

use serde::{Deserialize, Serialize};

#[cfg(feature = "axum")]
use axum::Json;

/// 标准 API 响应封装。
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: u32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

/// 预检端点的响应。指示文件是否已存在（去重）或上传正在进行中（续传）。
#[derive(Serialize)]
pub struct PrecheckResult {
    pub exists: bool,
    pub object_id: Option<String>,
    pub storage_path: Option<String>,
    pub task_id: Option<String>,
    pub uploaded_chunks: Vec<u32>,
    pub chunk_size: u64,
}

/// STS（安全令牌服务）响应。签发一个会话级别签名，
/// 授权此上传会话的所有分块上传。
#[derive(Serialize)]
pub struct StsResult {
    pub task_id: String,
    pub object_key: String,
    // 会话级别签名：此上传中所有分块使用一个签名
    pub session_signature: String,
    pub session_timestamp: i64,
    pub session_salt: String,
}

/// 单个分块上传操作的响应。
#[derive(Serialize)]
pub struct ChunkUploadResponse {
    pub chunk_index: u32,
    pub status: String,
    pub md5: String,
}

/// 分块上传进度查询的响应。
#[derive(Serialize)]
pub struct ChunkStatusResponse {
    pub task_id: String,
    pub chunk_count: i64,
    pub uploaded_count: u32,
    pub missing_chunks: Vec<u32>,
    pub is_complete: bool,
}

/// 分块合并成功后的响应。
#[derive(Serialize)]
pub struct MergeResult {
    pub object_id: String,
    pub storage_path: String,
    pub size: u64,
    pub md5: String,
}

/// 合并被接受进行异步处理时的响应。
#[derive(Serialize)]
pub struct MergeAcceptedResult {
    pub task_id: String,
    pub message: String,
}

/// 合并状态轮询的响应。
#[derive(Serialize)]
pub struct MergeStatusResponse {
    pub task_id: String,
    pub status: String,
    pub storage_path: Option<String>,
    /// 状态为"failed"时的错误消息，否则为 null。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 为新上传会话请求签发 STS 令牌。
#[derive(Deserialize)]
pub struct StsRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_md5: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: u64,
}

/// 上传前预检文件的请求。检查去重和进行中的上传。
#[derive(Deserialize)]
pub struct PrecheckRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_md5: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: u64,
}

fn default_chunk_size() -> u64 {
    8 * 1024 * 1024 // 8 MiB
}

/// 查询指定上传任务的分块上传状态的请求。
#[derive(Deserialize)]
pub struct ChunkStatusReq {
    pub task_id: uuid::Uuid,
}

/// 将上传的所有分块合并到最终对象文件的请求。
#[derive(Deserialize)]
pub struct MergeRequest {
    pub task_id: uuid::Uuid,
    pub file_name: String,
    pub file_md5: String,
    pub file_size: u64,
    pub content_type: Option<String>,
}

// 逐分块签名请求/验证签名请求/签名响应已移除。
// 会话级别签名现在通过 STS 响应处理。

// ============================================================================
// 认证令牌 API 类型（Web 登录）
// ============================================================================

/// 登录请求体。
#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// 登录响应体。
#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    /// 用户是否为超级管理员（可以管理所有用户/文件）
    pub is_super_admin: bool,
}

/// 令牌刷新请求体。
#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// 凭据验证响应体。
#[derive(Serialize)]
pub struct VerifyResponse {
    pub user_id: u64,
}

/// 构造 200 OK API 响应。
#[cfg(feature = "axum")]
pub fn api_ok<T: serde::Serialize>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse {
        code: 200,
        message: "ok".into(),
        data: Some(data),
    })
}

// ============================================================================
// 用户管理 DTO（buckets-srv）
// ============================================================================

/// 列表/详情中返回的用户信息——`secret_key` 不会暴露。
#[derive(Serialize)]
pub struct UserInfo {
    pub id: u64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// 创建新用户的请求。
#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub password: String,
}

/// 更新用户的请求。
#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub password: Option<String>,
}

/// 分页响应封装。
#[derive(Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: u64,
    pub page_size: u64,
}

// ============================================================================
// 文件管理 DTO（buckets-srv）
// ============================================================================

/// 对象列表中返回的对象信息。
#[derive(Serialize)]
pub struct ObjectInfo {
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub size: i64,
    pub md5: String,
    pub content_type: Option<String>,
    pub extension: Option<String>,
    pub bucket: String,
    pub storage_path: Option<String>,
    pub image_width: i64,
    pub image_height: i64,
    pub image_type: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// 文件列表的查询参数。
#[derive(Deserialize)]
pub struct FileListQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
}
