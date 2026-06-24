use crate::dao;
use base64::Engine;
use chrono::Datelike;
use md5::{Digest, Md5};
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::db::{ObjectStatus, TaskStatus, UploadTask, objects, upload_tasks};
use buckets_common::utils::{image, path};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// 存储在 staging 目录中的元数据。
#[derive(Serialize, Deserialize)]
struct TusMeta {
    file_name: String,
    extension: String,
    content_type: Option<String>,
}

/// 获取 tus 上传的暂存数据文件路径。
fn tus_staging_data_path(task_id: &Uuid) -> PathBuf {
    staging_dir(task_id).join("data")
}

/// 获取 tus 元数据文件路径。
fn tus_meta_path(task_id: &Uuid) -> PathBuf {
    staging_dir(task_id).join("meta.json")
}

/// 获取 tus 暂存目录路径。
fn staging_dir(task_id: &Uuid) -> PathBuf {
    PathBuf::from(constant::staging_dir())
        .join(constant::TUS_STAGING_SUBDIR)
        .join(task_id.to_string())
}

/// 解析 Tus Upload-Metadata 头部。
/// 格式：`filename <base64>,content_type <base64>,key_without_value`
pub fn parse_upload_metadata(header: &str) -> Vec<(String, String)> {
    header
        .split(',')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let mut parts = pair.splitn(2, ' ');
            let key = parts.next()?.to_string();
            let value = parts
                .next()
                .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            Some((key, value))
        })
        .collect()
}

/// 创建一个新的 tus 上传任务。
pub async fn create_upload(
    db: &DatabaseConnection,
    user_id: i64,
    file_size: i64,
    metadata_header: &str,
    is_deferred: bool,
) -> Result<(UploadTask, String), AppError> {
    let object_id = Uuid::new_v4();

    // 从元数据解析文件名和 content_type
    let meta_map: std::collections::HashMap<String, String> = parse_upload_metadata(metadata_header)
        .into_iter()
        .collect();

    let raw_filename = meta_map.get("filename").cloned().unwrap_or_else(|| object_id.to_string());
    let extension = path::get_extension(&raw_filename).unwrap_or_default();
    let content_type = meta_map.get("content_type").cloned();

    let now = chrono::Utc::now();
    let output_path = path::get_object_storage_path(
        &object_id,
        user_id,
        now.year(),
        now.month(),
        now.day(),
        &extension,
    );

    let expiration_hours = if is_deferred {
        constant::MIN_UPLOAD_EXPIRATION_HOURS
    } else {
        let size_gb = file_size as f64 / constant::GB_DIVISOR;
        (constant::MIN_UPLOAD_EXPIRATION_HOURS as f64
            + (size_gb / 10.0).ceil() * constant::EXPIRATION_SCALE_HOURS_PER_10GB as f64)
            as i64
    };

    let task = dao::create_tus_upload_task(db, object_id, file_size, user_id, expiration_hours, is_deferred)
        .await?;

    // 创建 staging 目录并写入元数据
    let dir = staging_dir(&Uuid::parse_str(&task.uuid).unwrap());
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::StorageError(format!("create tus staging dir: {}", e)))?;

    let meta = TusMeta {
        file_name: raw_filename,
        extension,
        content_type,
    };
    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| AppError::Internal(format!("serialize meta: {}", e)))?;
    tokio::fs::write(tus_meta_path(&Uuid::parse_str(&task.uuid).unwrap()), &meta_json)
        .await
        .map_err(|e| AppError::StorageError(format!("write meta: {}", e)))?;

    let object_key = output_path.to_string_lossy().to_string();
    Ok((task, object_key))
}

/// 将数据追加到 tus 暂存文件并更新偏移量。
/// 如果这是首个数据，将状态从 "initialized" 更新为 "uploading"。
///
/// `upload_length` 仅在 Upload-Defer-Length 扩展中使用：
/// 最终 PATCH 请求携带 `Upload-Length` 头部以告知服务端文件总大小。
pub async fn append_data(
    db: &DatabaseConnection,
    user_id: i64,
    task_id: Uuid,
    expected_offset: i64,
    upload_length: Option<i64>,
    body: axum::body::Body,
) -> Result<i64, AppError> {
    use futures::StreamExt;

    let task = dao::find_upload_task(db, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != user_id {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    if task.status_enum().is_terminal() {
        return Err(AppError::BadRequest("upload already completed".into()));
    }

    if task.current_offset != expected_offset {
        return Err(AppError::Conflict(format!(
            "offset mismatch: expected {}, got {}",
            task.current_offset, expected_offset
        )));
    }

    // 如果 task 使用了 Upload-Defer-Length，且本次 PATCH 提供了 Upload-Length，
    // 则先更新 file_size。
    let effective_file_size = if task.is_deferred {
        if let Some(lsize) = upload_length {
            if lsize <= 0 {
                return Err(AppError::BadRequest("Upload-Length must be positive".into()));
            }
            if lsize < expected_offset {
                return Err(AppError::BadRequest(format!(
                    "Upload-Length {} is less than current offset {}",
                    lsize, expected_offset
                )));
            }
            dao::set_upload_file_size(db, task_id, lsize).await?;
            lsize
        } else {
            // 仍未知道文件大小——跳过上限校验
            i64::MAX
        }
    } else {
        if upload_length.is_some() {
            return Err(AppError::BadRequest(
                "Upload-Length not allowed for non-deferred upload".into(),
            ));
        }
        if expected_offset >= task.file_size {
            return Err(AppError::BadRequest(
                "upload already at or past file size".into(),
            ));
        }
        task.file_size
    };

    let data_path = tus_staging_data_path(&task_id);
    if let Some(parent) = data_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::StorageError(format!("create dir: {}", e)))?;
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&data_path)
        .await
        .map_err(|e| AppError::StorageError(format!("open staging file: {}", e)))?;

    let mut stream = body.into_data_stream();
    let mut bytes_written: i64 = 0;
    while let Some(chunk_result) = stream.next().await {
        let data = chunk_result.map_err(|e| AppError::BadRequest(format!("read body stream: {}", e)))?;

        bytes_written += data.len() as i64;

        let max_allowed = effective_file_size - expected_offset;
        if bytes_written > max_allowed {
            return Err(AppError::BadRequest(format!(
                "data exceeds remaining file size (remaining: {}, sent: {})",
                max_allowed, bytes_written
            )));
        }

        if let Err(e) = file.write_all(&data).await {
            return Err(AppError::StorageError(format!("write staging: {}", e)));
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::StorageError(format!("flush staging: {}", e)))?;
    drop(file);

    let new_offset = expected_offset + bytes_written;
    let is_complete = new_offset >= effective_file_size;

    let status = if task.status == constant::STATUS_INITIALIZED {
        Some(constant::STATUS_UPLOADING)
    } else if is_complete {
        Some(TaskStatus::Completed.as_str())
    } else {
        None
    };

    dao::update_tus_offset(db, task_id, new_offset, status).await?;

    if is_complete {
        let db_clone = db.clone();
        let task_id_clone = task_id;
        let user_id_clone = user_id;
        let object_id_str = task.object_id.clone();

        tokio::spawn(async move {
            if let Err(e) = complete_upload(&db_clone, task_id_clone, user_id_clone, &object_id_str).await {
                tracing::error!(error = %e, task_id = %task_id_clone, "tus upload completion failed");
                let _ = dao::update_upload_status(&db_clone, task_id_clone, TaskStatus::Failed.as_str()).await;
            }
        });
    }

    Ok(new_offset)
}

/// 完成 tus 上传：计算 MD5、创建对象记录、移动文件到存储路径。
async fn complete_upload(
    db: &DatabaseConnection,
    task_id: Uuid,
    user_id: i64,
    object_id_str: &str,
) -> Result<(), AppError> {
    let object_id = Uuid::parse_str(object_id_str)
        .map_err(|_| AppError::Internal("invalid object_id in task".into()))?;

    let dir = staging_dir(&task_id);
    let data_path = dir.join("data");
    let meta_path = dir.join("meta.json");

    let meta: TusMeta = tokio::task::spawn_blocking({
        let mp = meta_path.clone();
        move || -> Result<TusMeta, AppError> {
            let content = std::fs::read_to_string(&mp)
                .map_err(|e| AppError::StorageError(format!("read meta: {}", e)))?;
            serde_json::from_str(&content)
                .map_err(|e| AppError::Internal(format!("parse meta: {}", e)))
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("meta read panicked: {}", e)))??;

    // 在 spawn_blocking 中计算 MD5
    let staging = data_path.clone();
    let (computed_md5, file_len) = tokio::task::spawn_blocking(move || compute_file_md5(&staging))
        .await
        .map_err(|e| AppError::Internal(format!("md5 compute panicked: {}", e)))??;

    dao::set_task_md5(db, task_id, &computed_md5).await?;

    let now = chrono::Utc::now();

    let output_path = path::get_object_storage_path(
        &object_id,
        user_id,
        now.year(),
        now.month(),
        now.day(),
        &meta.extension,
    );

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::StorageError(format!("create object dir: {}", e)))?;
    }

    // 移动 staging 文件到最终路径
    tokio::fs::rename(&data_path, &output_path)
        .await
        .map_err(|e| AppError::StorageError(format!("rename to final path: {}", e)))?;

    // 清理 staging 目录
    let _ = tokio::fs::remove_dir_all(&dir).await;

    let storage_path_str = output_path.to_string_lossy().to_string();
    let uuid_str = object_id.to_string();
    let effective_content_type: Option<String> = if meta.content_type.is_some() {
        meta.content_type
    } else if !meta.extension.is_empty() {
        let mime = mime_guess::from_ext(&meta.extension).first_or_octet_stream();
        if mime.essence_str() == "application/octet-stream" {
            None
        } else {
            Some(mime.to_string())
        }
    } else {
        None
    };

    let (image_width, image_height, image_type_str) = if effective_content_type.as_deref().is_some_and(|ct| ct.starts_with("image/")) {
        let path = output_path.clone();
        tokio::task::spawn_blocking(move || image::detect_image_dims(&path))
            .await
            .map_err(|e| AppError::Internal(format!("image detect panicked: {}", e)))?
            .unwrap_or((0, 0, String::new()))
    } else {
        (0i64, 0i64, String::new())
    };

    let txn = db.begin().await?;

    let result = objects::Entity::insert(objects::ActiveModel {
        uuid: Set(uuid_str.clone()),
        name: Set(meta.file_name),
        size: Set(file_len as i64),
        md5: Set(computed_md5),
        content_type: Set(effective_content_type),
        extension: Set(Some(meta.extension)),
        bucket: Set(constant::DEFAULT_BUCKET.to_string()),
        storage_path: Set(storage_path_str.clone()),
        image_width: Set(image_width),
        image_height: Set(image_height),
        image_type: Set(image_type_str),
        status: Set(ObjectStatus::Active.as_str().to_string()),
        upload_method: Set(constant::TUS_UPLOAD_METHOD.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(&txn)
    .await?;

    let object_internal_id = result.last_insert_id;
    dao::insert_user_object(&txn, user_id, object_internal_id).await?;

    upload_tasks::Entity::update_many()
        .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
        .set(upload_tasks::ActiveModel {
            status: Set(TaskStatus::Completed.as_str().to_string()),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(&txn)
        .await?;

    txn.commit().await?;

    Ok(())
}

/// 使用同步 I/O 和小缓冲区计算文件的 MD5。
fn compute_file_md5(path: &std::path::Path) -> Result<(String, u64), AppError> {
    use std::io::{BufReader, Read};
    let file = std::fs::File::open(path)
        .map_err(|e| AppError::StorageError(format!("open for md5: {}", e)))?;
    let metadata = file
        .metadata()
        .map_err(|e| AppError::StorageError(format!("metadata: {}", e)))?;
    let file_len = metadata.len();
    let mut hasher = Md5::new();
    let mut buffer = [0u8; constant::CHUNK_STREAM_BUFFER_SIZE];
    let mut reader = BufReader::new(file);
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| AppError::StorageError(format!("read for md5: {}", e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok((hex::encode(hasher.finalize()), file_len))
}
