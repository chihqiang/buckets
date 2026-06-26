use chrono::Utc;
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::db::{UploadTask, upload_tasks};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

/// 创建具有自定义过期时间的上传任务
#[allow(clippy::too_many_arguments)]
pub async fn create_upload_task_with_expiration(
    db: &DatabaseConnection,
    object_id: Uuid,
    file_md5: &str,
    file_size: i64,
    chunk_size: i64,
    chunk_count: i32,
    user_id: i64,
    expiration_hours: i64,
) -> Result<UploadTask, AppError> {
    let uuid = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + chrono::TimeDelta::hours(expiration_hours);
    let word_count = (chunk_count as usize).div_ceil(constant::BITMAP_BITS_PER_WORD);
    let bitmap = serde_json::to_string(&vec![0u64; word_count]).unwrap_or_else(|_| "[]".into());

    upload_tasks::ActiveModel {
        uuid: Set(uuid.clone()),
        object_id: Set(object_id.to_string()),
        file_md5: Set(file_md5.to_string()),
        file_size: Set(file_size),
        chunk_size: Set(chunk_size),
        chunk_count: Set(chunk_count as i64),
        user_id: Set(user_id),
        status: Set("initialized".to_string()),
        uploaded_bitmap: Set(bitmap),
        created_at: Set(now),
        updated_at: Set(now),
        expires_at: Set(expires_at),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let task_uuid =
        Uuid::parse_str(&uuid).map_err(|e| AppError::Internal(format!("invalid uuid: {}", e)))?;
    find_upload_task(db, task_uuid)
        .await?
        .ok_or_else(|| AppError::Internal("upload task created but not found".into()))
}

/// 通过 UUID 查找上传任务。
pub async fn find_upload_task(
    db: &DatabaseConnection,
    task_id: Uuid,
) -> Result<Option<UploadTask>, AppError> {
    upload_tasks::Entity::find()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .one(db)
        .await
        .map_err(Into::into)
}

/// 通过文件 MD5 和用户查找最近的活动上传任务。
pub async fn find_upload_by_md5(
    db: &DatabaseConnection,
    file_md5: &str,
    user_id: i64,
) -> Result<Option<UploadTask>, AppError> {
    upload_tasks::Entity::find()
        .filter(upload_tasks::Column::FileMd5.eq(file_md5))
        .filter(upload_tasks::Column::UserId.eq(user_id))
        .filter(upload_tasks::Column::Status.is_not_in(vec!["completed", "expired", "failed"]))
        .order_by(upload_tasks::Column::CreatedAt, Order::Desc)
        .one(db)
        .await
        .map_err(Into::into)
}

/// 更新上传任务的状态。
pub async fn update_upload_status(
    db: &DatabaseConnection,
    task_id: Uuid,
    status: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    upload_tasks::Entity::update_many()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .set(upload_tasks::ActiveModel {
            status: Set(status.to_string()),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await?;
    Ok(())
}

/// 批量获取过期任务并标记它们。
pub async fn expire_and_list_tasks_batch(
    db: &DatabaseConnection,
    batch_size: u32,
) -> Result<Vec<String>, AppError> {
    let txn = db.begin().await.map_err(|e| {
        tracing::error!(error = %e, "expire_and_list_tasks_batch begin tx");
        AppError::DatabaseError(e.to_string())
    })?;
    let now = Utc::now();

    let expired = upload_tasks::Entity::find()
        .filter(upload_tasks::Column::Status.is_in(vec!["initialized", "uploading"]))
        .filter(upload_tasks::Column::ExpiresAt.lt(now))
        .lock_exclusive()
        .all(&txn)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "expire_and_list_tasks_batch query");
            AppError::DatabaseError(e.to_string())
        })?;

    let uuids: Vec<String> = expired.into_iter().map(|t| t.uuid).collect();

    let batch: Vec<String> = if batch_size > 0 && uuids.len() > batch_size as usize {
        uuids[..batch_size as usize].to_vec()
    } else {
        uuids
    };

    if !batch.is_empty() {
        upload_tasks::Entity::update_many()
            .filter(upload_tasks::Column::Uuid.is_in(batch.clone()))
            .set(upload_tasks::ActiveModel {
                status: Set("expired".to_string()),
                updated_at: Set(now),
                ..Default::default()
            })
            .exec(&txn)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "expire_and_list_tasks_batch update");
                AppError::DatabaseError(e.to_string())
            })?;
    }

    txn.commit().await.map_err(|e| {
        tracing::error!(error = %e, "expire_and_list_tasks_batch commit");
        AppError::DatabaseError(e.to_string())
    })?;

    Ok(batch)
}

pub async fn create_tus_upload_task(
    db: &DatabaseConnection,
    object_id: Uuid,
    file_size: i64,
    user_id: i64,
    expiration_hours: i64,
    is_deferred: bool,
) -> Result<UploadTask, AppError> {
    let uuid = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + chrono::TimeDelta::hours(expiration_hours);

    upload_tasks::ActiveModel {
        uuid: Set(uuid.clone()),
        object_id: Set(object_id.to_string()),
        file_md5: Set(String::new()),
        file_size: Set(file_size),
        chunk_size: Set(0),
        chunk_count: Set(0),
        user_id: Set(user_id),
        status: Set("initialized".to_string()),
        uploaded_bitmap: Set("[]".to_string()),
        upload_method: Set("tus".to_string()),
        current_offset: Set(0),
        is_deferred: Set(is_deferred),
        created_at: Set(now),
        updated_at: Set(now),
        expires_at: Set(expires_at),
        last_activity_at: Set(None),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let task_uuid =
        Uuid::parse_str(&uuid).map_err(|e| AppError::Internal(format!("invalid uuid: {}", e)))?;
    find_upload_task(db, task_uuid)
        .await?
        .ok_or_else(|| AppError::Internal("upload task created but not found".into()))
}

/// 设置 tus 上传完成时的文件大小（Upload-Defer-Length 扩展使用）。
pub async fn set_upload_file_size(
    db: &DatabaseConnection,
    task_id: Uuid,
    file_size: i64,
) -> Result<(), AppError> {
    let now = Utc::now();
    upload_tasks::Entity::update_many()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .set(upload_tasks::ActiveModel {
            file_size: Set(file_size),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await?;
    Ok(())
}

/// 原子地更新 tus 上传的当前偏移量和可选的传输状态。
/// 也更新 last_activity_at 以保持会话活跃。
pub async fn update_tus_offset(
    db: &DatabaseConnection,
    task_id: Uuid,
    new_offset: i64,
    status: Option<&str>,
) -> Result<(), AppError> {
    let now = Utc::now();
    let mut active = upload_tasks::ActiveModel {
        current_offset: Set(new_offset),
        last_activity_at: Set(Some(now.timestamp())),
        updated_at: Set(now),
        ..Default::default()
    };
    if let Some(s) = status {
        active.status = Set(s.to_string());
    }
    upload_tasks::Entity::update_many()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .set(active)
        .exec(db)
        .await?;
    Ok(())
}

/// 在合并/完成时设置任务的 MD5。
pub async fn set_task_md5(
    db: &DatabaseConnection,
    task_id: Uuid,
    md5: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    upload_tasks::Entity::update_many()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .set(upload_tasks::ActiveModel {
            file_md5: Set(md5.to_string()),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await?;
    Ok(())
}
