//! 网关分块上传服务层。
//!
//! 处理二进制分块上传，包含会话级别签名验证、
//! 会话活跃度检查、通过 tmp+rename 的原子写入和分块状态查询。
//!
//! 通过小缓冲区将请求体流式写入临时文件，从文件（而非内存）计算 MD5。
//! 每个任务的内存位图，批量刷新到数据库以减少数据库压力。
//! 直接从 axum::Body 将请求体流式写入磁盘以避免 OOM。
//! 将 last_activity_at 更新与位图刷新一起批量处理。

use crate::dao;
use dashmap::DashMap;
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{ChunkStatusResponse, ChunkUploadResponse};
use buckets_common::model::db::{TaskStatus, upload_tasks};
use buckets_common::utils::crypto::{self, SessionSignInput};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, Statement,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

// 跟踪需要 last_activity_at 更新的任务，与位图刷新一起批量处理
use std::collections::HashSet;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// 内存位图缓存（优化 #3）
// ---------------------------------------------------------------------------

/// 每个任务的内存位图状态——避免逐分块更新数据库。
pub(crate) struct TaskBitmap {
    /// 以 u64 字数组表示的位图。
    pub(crate) words: Vec<u64>,
    /// 此任务的分块数。
    chunk_count: u32,
    /// 脏标记：如果内存与数据库不同则为 true。
    dirty: bool,
}

/// 全局内存位图缓存。以 task_id（字符串）为键。
static BITMAP_CACHE: std::sync::OnceLock<DashMap<String, Arc<RwLock<TaskBitmap>>>> =
    std::sync::OnceLock::new();

// 跟踪需要批量更新 last_activity_at 的任务 ID。
// 不必逐分块访问数据库，而是累积后与位图一起批量刷新。
static PENDING_ACTIVITY: std::sync::OnceLock<Mutex<HashSet<uuid::Uuid>>> =
    std::sync::OnceLock::new();

fn pending_activity() -> &'static Mutex<HashSet<uuid::Uuid>> {
    PENDING_ACTIVITY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 跟踪脏任务 ID，避免刷新时遍历整个 DashMap。
/// 当数千个任务活跃时，遍历所有条目仅查找少数脏条目会导致严重的锁争用。
/// 此 HashSet 提供 O(1) 的脏检查，因此刷新只处理实际需要处理的条目。
static DIRTY_TASK_IDS: std::sync::OnceLock<Mutex<HashSet<String>>> = std::sync::OnceLock::new();

fn dirty_task_ids() -> &'static Mutex<HashSet<String>> {
    DIRTY_TASK_IDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 获取全局位图缓存的引用（由 file_svc 在合并时使用）。
pub(crate) fn bitmap_cache() -> &'static DashMap<String, Arc<RwLock<TaskBitmap>>> {
    BITMAP_CACHE.get_or_init(DashMap::new)
}

/// 确保指定任务存在内存位图条目。首次访问时从数据库加载。
/// 同时在后台任务中淘汰过期的条目（参见 start_bitmap_cache_cleanup）。
async fn get_or_init_bitmap(
    task_id: uuid::Uuid,
    chunk_count: u32,
    db_bitmap: Vec<u64>,
) -> Arc<RwLock<TaskBitmap>> {
    let key = task_id.to_string();
    // 使用 DashMap entry API 避免 TOCTOU 竞态（#5 修复）
    bitmap_cache()
        .entry(key)
        .or_insert_with(|| {
            Arc::new(RwLock::new(TaskBitmap {
                words: db_bitmap,
                chunk_count,
                dirty: false,
            }))
        })
        .value()
        .clone()
}

/// 在内存位图中设置一个位并标记为脏。
/// 同时跟踪脏任务集合中的 task_id，以实现高效刷新。
async fn set_bit_in_memory(bitmap: &Arc<RwLock<TaskBitmap>>, chunk_index: u32, task_id: &str) {
    let mut guard = bitmap.write().await;
    let word_index = chunk_index as usize / constant::BITMAP_BITS_PER_WORD;
    let bit = chunk_index % constant::BITMAP_BITS_PER_WORD as u32;
    if word_index < guard.words.len() {
        guard.words[word_index] |= 1u64 << bit;
        guard.dirty = true;
        // 跟踪脏 task_id，实现 O(1) 的刷新查找
        dirty_task_ids().lock().unwrap().insert(task_id.to_string());
    }
}

/// 批量将脏位图刷新到数据库。定期调用和在任务完成时调用。
/// 同时批量更新自上次刷新以来接收分块的任务的 last_activity_at，
/// 减少逐分块数据库更新。
///
/// 使用单独的 HashSet<String> 跟踪脏任务 ID，避免在数千个任务
/// 活跃但只有少数是脏的情况下遍历整个 DashMap。
async fn flush_dirty_bitmaps(db: &DatabaseConnection) {
    // 清空待处理的活动任务并批量更新 last_activity_at
    let pending_tasks: Vec<uuid::Uuid> = {
        let mut set = pending_activity().lock().unwrap();
        let tasks: Vec<_> = set.drain().collect();
        tasks
    };
    if !pending_tasks.is_empty() {
        let now = chrono::Utc::now();
        let uuid_strs: Vec<String> = pending_tasks.iter().map(|id| id.to_string()).collect();
        let placeholders: Vec<String> = uuid_strs.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "UPDATE upload_tasks SET last_activity_at = ?, updated_at = ? WHERE uuid IN ({})",
            placeholders.join(",")
        );
        let mut values: Vec<sea_orm::Value> = vec![(now.timestamp()).into(), now.into()];
        values.extend(uuid_strs.into_iter().map(|s| s.into()));
        if let Err(e) = db
            .execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::MySql,
                &sql,
                values,
            ))
            .await
        {
            tracing::error!(error = %e, count = pending_tasks.len(), "batch last_activity_at update failed");
        }
    }

    // 清空脏任务 ID，只处理这些条目。
    // 这避免了在数千个任务存在但只有少数有脏位图时遍历整个 DashMap。
    let dirty_ids: Vec<String> = {
        let mut set = dirty_task_ids().lock().unwrap();
        let ids: Vec<_> = set.drain().collect();
        ids
    };

    if dirty_ids.is_empty() {
        return;
    }

    let mut tasks_to_remove = Vec::new();
    for key in &dirty_ids {
        let bitmap = match bitmap_cache().get(key) {
            Some(entry) => entry.value().clone(),
            None => continue,
        };

        // 在写锁下提取数据，然后在数据库 I/O 前释放。
        let task_id = match uuid::Uuid::parse_str(key) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let (bitmap_json, should_remove) = {
            let mut guard = bitmap.write().await;
            if !guard.dirty {
                continue;
            }
            let json = serde_json::to_string(&guard.words).unwrap_or_else(|_| "[]".into());
            guard.dirty = false;
            let all_done = (0..guard.chunk_count).all(|i| {
                let w = i as usize / constant::BITMAP_BITS_PER_WORD;
                let b = i % constant::BITMAP_BITS_PER_WORD as u32;
                w < guard.words.len() && (guard.words[w] & (1u64 << b)) != 0
            });
            (json, all_done)
        };
        // 锁已释放——现在执行数据库 I/O。
        let now = chrono::Utc::now();
        let result = upload_tasks::Entity::update_many()
            .filter(upload_tasks::Column::Uuid.eq(task_id.to_string()))
            .set(upload_tasks::ActiveModel {
                uploaded_bitmap: Set(bitmap_json),
                updated_at: Set(now),
                ..Default::default()
            })
            .exec(db)
            .await;
        match result {
            Ok(_) => {
                if should_remove {
                    tasks_to_remove.push(key.clone());
                }
            }
            Err(e) => {
                tracing::error!(error = %e, %task_id, "failed to flush bitmap to DB");
            }
        }
    }
    // 从缓存和脏集合中移除已完成的条目
    for key in &tasks_to_remove {
        bitmap_cache().remove(key);
    }
}

/// 启动周期性位图刷新任务。每隔几秒将脏位图刷新到数据库。
/// 接受 [`CancellationToken`] 以实现优雅关闭：取消时，在退出前
/// 执行所有脏位图的最终刷新，避免数据丢失。
pub fn start_bitmap_flush_task(db: DatabaseConnection, cancellation: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            constant::BITMAP_FLUSH_INTERVAL_SECS,
        ));
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::info!("bitmap flush task shutting down, performing final flush...");
                    flush_dirty_bitmaps(&db).await;
                    tracing::info!("bitmap flush task final flush complete");
                    break;
                }
                _ = interval.tick() => {
                    flush_dirty_bitmaps(&db).await;
                }
            }
        }
    });
}

/// 启动周期性清理过期的位图缓存条目（长时间未操作的任务）。
/// 对缓存大小实施硬限制，在超出限制时淘汰干净（非脏）条目，防止无限制的内存增长。
pub fn start_bitmap_cache_cleanup(cancellation: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            constant::BITMAP_CACHE_CLEANUP_INTERVAL_SECS,
        ));
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::debug!("bitmap cache cleanup shutting down");
                    break;
                }
                _ = interval.tick() => {
                    let cache_len = bitmap_cache().len();
                    // 硬限制——超出时即使未完全完成也淘汰干净条目。
                    // 已完成的条目已被 flush_dirty_bitmaps 移除。
                    // 这里处理大量进行中的上传累积的边界情况。
                    if cache_len > constant::BITMAP_CACHE_MAX_ENTRIES {
                        tracing::warn!(
                            "bitmap cache size {} exceeds hard limit {}, evicting clean entries",
                            cache_len,
                            constant::BITMAP_CACHE_MAX_ENTRIES
                        );
                        let mut to_remove = Vec::new();
                        for entry in bitmap_cache().iter() {
                            let (key, bitmap) = entry.pair();
                            let guard = bitmap.read().await;
                            if !guard.dirty {
                                to_remove.push(key.clone());
                            }
                            drop(guard);
                            // 收集到足够的条目后停止
                            if to_remove.len() >= cache_len.saturating_sub(constant::BITMAP_CACHE_MAX_ENTRIES) {
                                break;
                            }
                        }
                        for key in &to_remove {
                            bitmap_cache().remove(key);
                        }
                        tracing::info!(
                            "bitmap cache evicted {} entries, {} remaining",
                            to_remove.len(),
                            bitmap_cache().len()
                        );
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 核心上传逻辑
// ---------------------------------------------------------------------------

/// 流式处理变体：通过 axum::Body 流直接将请求体写入磁盘，
/// 避免将整个分块加载到内存中。修复 #1（OOM 风险）。
#[allow(clippy::too_many_arguments)]
pub async fn upload_chunk_binary_stream(
    db: &DatabaseConnection,
    secret_key_cache: &crate::middleware::auth::SecretKeyCache,
    user_id: u64,
    task_id: uuid::Uuid,
    chunk_index: u32,
    chunk_md5: String,
    session_signature: String,
    session_timestamp: i64,
    session_salt: String,
    body: axum::body::Body,
) -> Result<ChunkUploadResponse, AppError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let task = dao::find_upload_task(db, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != user_id {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    // 为此任务的第一个分块初始化位图缓存
    let bitmap = get_or_init_bitmap(task_id, task.chunk_count as u32, task.parse_bitmap()).await;

    // 验证会话级别签名——使用缓存的 secret_key 避免逐分块数据库查询
    let secret_key = {
        if let Some(cached) =
            crate::middleware::auth::get_cached_secret_key(secret_key_cache, user_id)
        {
            cached
        } else {
            let sk = dao::get_user_secret_key(db, user_id).await?;
            crate::middleware::auth::cache_secret_key(secret_key_cache, user_id, sk.clone());
            sk
        }
    };
    let session_input = SessionSignInput {
        user_id,
        task_id: task_id.to_string(),
        file_md5: task.file_md5.clone(),
        chunk_size: task.chunk_size as u64,
        timestamp: session_timestamp,
        salt: session_salt,
    };
    if !crypto::verify_session_signature(&secret_key, &session_input, &session_signature)? {
        return Err(AppError::SignatureInvalid);
    }
    crypto::verify_session_timestamp(session_timestamp)?;

    // 检查会话活跃度
    check_session_liveness(db, task_id, &task).await?;

    let staging_dir = buckets_common::utils::path::get_chunk_staging_dir(&task_id);
    tokio::fs::create_dir_all(&staging_dir)
        .await
        .map_err(|e| AppError::StorageError(format!("create staging dir: {}", e)))?;

    let chunk_path = buckets_common::utils::path::get_chunk_staging_path(&task_id, chunk_index);
    if chunk_path.exists() {
        // 分块已在磁盘上——只需更新内存位图
        // （位图已在上方用任务元数据初始化）
        set_bit_in_memory(&bitmap, chunk_index, &task_id.to_string()).await;
        return Ok(ChunkUploadResponse {
            chunk_index,
            status: constant::CHUNK_STATUS_ALREADY_EXISTS.into(),
            md5: chunk_md5,
        });
    }

    let tmp_path = chunk_path.with_extension(constant::TEMP_FILE_EXTENSION);

    // 将请求体直接流式写入临时文件——绝不将完整分块保留在内存中
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| AppError::StorageError(format!("create tmp file: {}", e)))?;

    let mut stream = body.into_data_stream();
    let mut total_bytes: u64 = 0;
    while let Some(chunk_result) = stream.next().await {
        let data = chunk_result.map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            AppError::BadRequest(format!("read body stream: {}", e))
        })?;

        total_bytes += data.len() as u64;
        if total_bytes > task.chunk_size as u64 {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::BadRequest(format!(
                "chunk size exceeds limit {}",
                task.chunk_size
            )));
        }

        if let Err(e) = file.write_all(&data).await {
            let _ = std::fs::remove_file(&tmp_path);
            if let Some(enospc) = buckets_common::utils::validate::check_enospc(&e) {
                return Err(enospc);
            }
            return Err(AppError::StorageError(format!("write chunk stream: {}", e)));
        }
    }
    // 刷新并同步，确保在 MD5 计算前数据已写入磁盘
    file.flush().await.map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        AppError::StorageError(format!("flush chunk: {}", e))
    })?;
    drop(file);

    // 从磁盘上的临时文件计算 MD5
    let computed_md5 = compute_file_md5_sync(&tmp_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        AppError::StorageError(format!("compute chunk md5: {}", e))
    })?;

    if computed_md5 != chunk_md5 {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::HashMismatch {
            expected: chunk_md5,
            actual: computed_md5,
        });
    }

    // 原子重命名
    tokio::fs::rename(&tmp_path, &chunk_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            AppError::StorageError(format!("rename chunk: {}", e))
        })?;

    // 写入分块 MD5 sidecar 文件，供合并时使用
    if let Err(e) = super::file_svc::write_chunk_md5_sidecar(&task_id, chunk_index, &computed_md5) {
        tracing::warn!(error = %e, chunk_index, "failed to write chunk md5 sidecar");
    }

    // 更新内存位图（已在上方用任务元数据初始化）
    set_bit_in_memory(&bitmap, chunk_index, &task_id.to_string()).await;

    // 将 last_activity_at 更新与位图刷新一起批量处理
    {
        let mut set = pending_activity().lock().unwrap();
        set.insert(task_id);
    }

    if task.status_enum() == TaskStatus::Initialized {
        dao::update_upload_status(db, task_id, TaskStatus::Uploading.as_str()).await?;
    }

    Ok(ChunkUploadResponse {
        chunk_index,
        status: constant::CHUNK_STATUS_UPLOADED.into(),
        md5: chunk_md5,
    })
}

/// 使用同步 I/O 和小缓冲区（64 KiB）计算文件的 MD5。
/// 用于分块 MD5 验证——避免将整个分块加载到内存中。
fn compute_file_md5_sync(path: &std::path::Path) -> Result<String, std::io::Error> {
    use md5::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = md5::Md5::new();
    let mut buffer = [0u8; constant::CHUNK_STREAM_BUFFER_SIZE];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 检查上传会话是否仍然活跃（有最近的活动）。
/// 如果 last_activity_at 超出超时窗口，会话被视为已过期。
/// 对于非常大的文件，超时时间会根据文件大小动态调整。
async fn check_session_liveness(
    db: &DatabaseConnection,
    task_id: uuid::Uuid,
    task: &buckets_common::model::db::UploadTask,
) -> Result<(), AppError> {
    // 如果是第一个分块（状态=initialized），跳过活跃度检查
    if task.status_enum() == TaskStatus::Initialized {
        return Ok(());
    }

    // 对于"uploading"任务，检查 last_activity_at 是否在超时范围内
    if let Some(last_activity) = task.last_activity_at {
        let now = chrono::Utc::now().timestamp();
        let elapsed = now - last_activity;

        // 动态超时：根据文件大小调整
        // 基础：1 小时。每 10GB 文件大小增加 1 小时，上限为 48 小时。
        let size_gb = task.file_size as f64 / constant::GB_DIVISOR;
        let extra_hours = (size_gb / 10.0).ceil() as i64 * constant::LIVENESS_SCALE_SECS_PER_10GB;
        let timeout = (constant::SESSION_ACTIVITY_TIMEOUT_SECS + extra_hours)
            .min(constant::MAX_SESSION_ACTIVITY_TIMEOUT_SECS);

        if elapsed > timeout {
            // 标记为已过期
            let _ = dao::update_upload_status(db, task_id, TaskStatus::Expired.as_str()).await;
            return Err(AppError::BadRequest(format!(
                "upload session expired: no activity for {} seconds (timeout: {} seconds). Start a new upload.",
                elapsed, timeout
            )));
        }
    }

    Ok(())
}

/// 查询指定上传任务的上传进度。
/// 返回已上传的分块数和缺失的分块索引列表。
pub async fn chunk_status(
    db: &DatabaseConnection,
    user_id: u64,
    task_id: uuid::Uuid,
) -> Result<ChunkStatusResponse, AppError> {
    let task = dao::find_upload_task(db, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != user_id {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    // 先检查内存缓存，回退到数据库位图
    let key = task_id.to_string();
    let bitmap = if let Some(entry) = bitmap_cache().get(&key) {
        entry.value().read().await.words.clone()
    } else {
        task.parse_bitmap()
    };

    let total = task.chunk_count as u32;
    let mut uploaded_count = 0u32;
    let mut missing = Vec::new();

    for i in 0..total {
        let word = i as usize / constant::BITMAP_BITS_PER_WORD;
        let bit = i % constant::BITMAP_BITS_PER_WORD as u32;
        if word < bitmap.len() && (bitmap[word] & (1u64 << bit)) != 0 {
            uploaded_count += 1;
        } else {
            missing.push(i);
        }
    }

    Ok(ChunkStatusResponse {
        task_id: task.uuid,
        chunk_count: task.chunk_count,
        uploaded_count,
        missing_chunks: missing,
        is_complete: uploaded_count == total,
    })
}
