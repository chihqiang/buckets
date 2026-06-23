//! 认证和 STS（安全令牌服务）逻辑。
//!
//! 签发会话级别的 HMAC 签名，授权上传中的所有分块。

use crate::dao;
use chrono::Datelike;
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{StsRequest, StsResult};
use buckets_common::utils::crypto::{self, SessionSignInput};
use sea_orm::DatabaseConnection;

/// 签发 STS 令牌：生成任务 ID 和会话级别签名，
/// 授权此会话的所有后续分块上传。
/// 同时在数据库中创建上传任务，以便分块上传可以找到它。
/// 验证 file_size > 0 以防止无意义的任务。
pub async fn issue_sts(
    db: &DatabaseConnection,
    user_id: i64,
    req: &StsRequest,
) -> Result<StsResult, AppError> {
    // 基本验证
    if req.file_size == 0 {
        return Err(AppError::BadRequest(
            "file_size must be greater than 0".into(),
        ));
    }

    let object_id = uuid::Uuid::new_v4();
    let chunk_size = req.chunk_size;
    let chunk_count = req.file_size.div_ceil(chunk_size) as u32;

    // 动态过期时间：与预检逻辑相同
    let size_gb = req.file_size as f64 / constant::GB_DIVISOR;
    let expiration_hours = (constant::MIN_UPLOAD_EXPIRATION_HOURS as f64
        + (size_gb / 10.0).ceil() * constant::EXPIRATION_SCALE_HOURS_PER_10GB as f64)
        as i64;

    // 在数据库中创建上传任务，以便分块上传可以引用它
    let task = dao::create_upload_task_with_expiration(
        db,
        object_id,
        &req.file_md5,
        req.file_size as i64,
        chunk_size as i64,
        chunk_count as i32,
        user_id,
        expiration_hours,
    )
    .await?;

    let now = chrono::Utc::now();
    let ext = buckets_common::utils::path::get_extension(&req.file_name).unwrap_or_default();
    let object_key = buckets_common::utils::path::get_object_storage_path(
        &object_id,
        user_id,
        now.year(),
        now.month(),
        now.day(),
        &ext,
    );
    let object_key = object_key.to_string_lossy().to_string();

    // 生成会话级别签名：所有分块使用一个签名
    let secret_key = dao::get_user_secret_key(db, user_id).await?;
    let timestamp = chrono::Utc::now().timestamp();
    let salt = uuid::Uuid::new_v4().to_string();
    let session_input = SessionSignInput {
        user_id,
        task_id: task.uuid.clone(),
        file_md5: req.file_md5.clone(),
        chunk_size,
        timestamp,
        salt: salt.clone(),
    };
    let session_signature = crypto::generate_session_signature(&secret_key, &session_input)?;

    Ok(StsResult {
        task_id: task.uuid,
        object_key,
        session_signature,
        session_timestamp: timestamp,
        session_salt: salt,
    })
}
