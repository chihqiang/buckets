//! 网关上文件上传服务层。
//!
//! 处理上传预检（去重+续传）和分块合并，
//! 使用卸载的阻塞 I/O 以避免饿死异步运行时。
//!
//! 分块 MD5 在上传期间存储在 sidecar 文件中，无需在合并时
//!   重新读取分块来计算 MD5。这大致将零拷贝合并路径的
//!   磁盘 I/O 减少了一半。
//! 合并失败时保留临时文件以便重试，而不是删除它们。
//!   只有 GC 会在过期后清理它们。
//! 分块写入时的磁盘已满错误（ENOSPC）被捕获并返回
//!   用户友好的错误消息。

use crate::dao;
use chrono::Datelike;
use md5::{Digest, Md5};
use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::api::{MergeResult, PrecheckResult};
use buckets_common::model::db::{ObjectStatus, TaskStatus};
use buckets_common::model::db::{objects, upload_tasks};
use buckets_common::utils::path;
use buckets_common::utils::validate;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

// 信号量用于限制并发合并操作，防止同时合并多个大文件时
// 线程池耗尽。
static MERGE_SEMAPHORE: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();

fn merge_semaphore() -> Arc<Semaphore> {
    MERGE_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(constant::MAX_CONCURRENT_MERGES)))
        .clone()
}

/// 分块 MD5 sidecar 文件的扩展名。
const CHUNK_MD5_SIDECAR_EXTENSION: &str = "md5";

/// 上传前预检文件：验证、检查去重、
/// 检查是否存在进行中的上传（续传），或创建新任务。
pub async fn precheck(
    db: &DatabaseConnection,
    file_md5: &str,
    file_size: u64,
    user_id: u64,
    chunk_size: u64,
    file_name: &str,
) -> Result<PrecheckResult, AppError> {
    // 验证文件扩展名
    validate::validate_file_extension(file_name)?;

    // 在 spawn_blocking 中执行磁盘空间预检，避免阻塞 tokio 运行时
    let _available = tokio::task::spawn_blocking(move || validate::check_disk_space(file_size))
        .await
        .map_err(|e| AppError::Internal(format!("disk check panicked: {}", e)))??;

    // 对超大文件的软限制警告
    if file_size > constant::FILE_SIZE_SOFT_LIMIT_WARN {
        tracing::warn!(
            file_size = file_size,
            "file exceeds 1 TiB soft limit — ensure sufficient disk space and monitoring"
        );
    }

    // 全局去重检查（任何用户上传过相同文件则触发即时完成）
    // 使用 MD5 + file_size 作为去重键，降低碰撞风险。
    let existing =
        dao::find_object_by_md5(db, file_md5, constant::DEFAULT_BUCKET, file_size as i64).await?;
    if let Some(obj) = existing {
        // 确保当前用户与此对象关联
        dao::insert_user_object(db, user_id, obj.id).await?;
        return Ok(PrecheckResult {
            exists: true,
            object_id: Some(obj.uuid.clone()),
            storage_path: Some(obj.storage_path.clone()),
            task_id: None,
            uploaded_chunks: vec![],
            chunk_size,
        });
    }

    let existing_upload = dao::find_upload_by_md5(db, file_md5, user_id).await?;
    let should_create_new = match existing_upload {
        Some(task) if task.chunk_size as u64 == chunk_size => {
            // 来自 STS 的新建任务——还没有分块上传，暂存目录
            // 不存在（在第一个分块上传时创建）。直接返回
            // 该任务，而不是落入创建重复任务的逻辑，
            // 否则会破坏会话签名。
            if task.status == constant::STATUS_INITIALIZED {
                return Ok(PrecheckResult {
                    exists: false,
                    object_id: Some(task.object_id),
                    storage_path: None,
                    task_id: Some(task.uuid),
                    uploaded_chunks: vec![],
                    chunk_size: task.chunk_size as u64,
                });
            }

            // 对于"uploading"/"merging"任务，在允许续传前验证暂存目录
            // 仍然存在。GC 可能已清理了过期/失败任务的暂存文件，
            // 或者手动清理可能已删除它们。如果暂存目录不存在，
            // 则落入创建新任务的逻辑，而不是返回指向
            // 缺失分块的 task_id。
            let task_uuid = Uuid::parse_str(&task.uuid)
                .map_err(|_| AppError::Internal("invalid task uuid in db".into()))?;
            let staging_dir = path::get_chunk_staging_dir(&task_uuid);
            if !staging_dir.exists() {
                tracing::warn!(
                    task_id = %task.uuid,
                    "staging directory missing for resume task — creating new task"
                );
                true
            } else {
                let bitmap = task.parse_bitmap();
                let total = task.chunk_count as u32;
                let mut uploaded = Vec::new();
                for i in 0..total {
                    let word = i as usize / 64;
                    let bit = i % 64;
                    if word < bitmap.len() && (bitmap[word] & (1u64 << bit)) != 0 {
                        uploaded.push(i);
                    }
                }
                return Ok(PrecheckResult {
                    exists: false,
                    object_id: Some(task.object_id),
                    storage_path: None,
                    task_id: Some(task.uuid),
                    uploaded_chunks: uploaded,
                    chunk_size: task.chunk_size as u64,
                });
            }
        }
        _ => true,
    };

    if should_create_new {
        let object_id = Uuid::new_v4();
        let chunk_count = file_size.div_ceil(chunk_size) as u32;

        // 动态过期时间：根据文件大小缩放（最小 72h，每 10GB +24h）
        let size_gb = file_size as f64 / constant::GB_DIVISOR;
        let expiration_hours = (constant::MIN_UPLOAD_EXPIRATION_HOURS as f64
            + (size_gb / 10.0).ceil() * constant::EXPIRATION_SCALE_HOURS_PER_10GB as f64)
            as i64;

        let task = dao::create_upload_task_with_expiration(
            db,
            object_id,
            file_md5,
            file_size as i64,
            chunk_size as i64,
            chunk_count as i32,
            user_id,
            expiration_hours,
        )
        .await?;
        return Ok(PrecheckResult {
            exists: false,
            object_id: Some(object_id.to_string()),
            storage_path: None,
            task_id: Some(task.uuid),
            uploaded_chunks: vec![],
            chunk_size,
        });
    }

    // 不可达——以上所有分支都已返回
    Err(AppError::Internal("precheck: unexpected state".into()))
}

/// 将所有已上传的分块合并到最终的对象文件中。
/// 通过 `spawn_blocking` 将重度 I/O 卸载到阻塞线程。
/// 在 MD5 验证后从临时路径原子重命名到最终路径。
/// 开始时将状态设置为"merging"；失败时设置为"failed"。
///
/// 合并失败时保留暂存文件，以便客户端无需重新上传分块即可重试合并。
///   GC 在过期时进行清理。
/// 使用上传期间存储的分块 MD5 sidecar 文件，避免仅为了计算 MD5
///   而重新读取所有分块。
pub async fn merge(
    db: &DatabaseConnection,
    user_id: u64,
    file_name: &str,
    file_md5: &str,
    file_size: u64,
    content_type: Option<&str>,
    task_id: Uuid,
) -> Result<MergeResult, AppError> {
    let task = dao::find_upload_task(db, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("upload task not found".into()))?;

    if task.user_id != user_id {
        return Err(AppError::Forbidden(
            "upload task does not belong to user".into(),
        ));
    }

    // 验证所有分块是否已上传——先检查内存缓存，再检查数据库位图
    let key = task_id.to_string();
    let bitmap = if let Some(entry) = super::chunk_svc::bitmap_cache().get(&key) {
        entry.value().read().await.words.clone()
    } else {
        task.parse_bitmap()
    };

    let total = task.chunk_count as u32;
    for i in 0..total {
        let word = i as usize / constant::BITMAP_BITS_PER_WORD;
        let bit = i % constant::BITMAP_BITS_PER_WORD as u32;
        if word >= bitmap.len() || (bitmap[word] & (1u64 << bit)) == 0 {
            return Err(AppError::UploadIncomplete(
                (0..total)
                    .filter(|&j| {
                        let w = j as usize / constant::BITMAP_BITS_PER_WORD;
                        let b = j % constant::BITMAP_BITS_PER_WORD as u32;
                        w >= bitmap.len() || (bitmap[w] & (1u64 << b)) == 0
                    })
                    .count() as u32,
            ));
        }
    }

    let object_id = Uuid::parse_str(&task.object_id)
        .map_err(|_| AppError::Internal("invalid object_id in task".into()))?;

    let extension = path::get_extension(file_name);
    let now = chrono::Utc::now();
    let ext_str = extension.as_deref().unwrap_or("");
    let output_path = path::get_object_storage_path(
        &object_id,
        user_id,
        now.year(),
        now.month(),
        now.day(),
        ext_str,
    );

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::StorageError(format!("create object dir: {}", e)))?;
    }

    // 在 spawn_blocking 中执行最终磁盘空间预检，避免阻塞 tokio 运行时
    let _available =
        tokio::task::spawn_blocking(move || validate::check_disk_space_for_merge(file_size))
            .await
            .map_err(|e| AppError::Internal(format!("disk check panicked: {}", e)))??;

    // 将任务标记为"merging"，以便客户端知道正在进行中
    let _ = dao::update_upload_status(db, task_id, TaskStatus::Merging.as_str()).await;

    // 将整个合并操作卸载到阻塞线程，以避免
    // 在大文件同步 I/O 上饿死 tokio 异步运行时。
    let task_id_clone = task_id;
    let output_path_clone = output_path.clone();

    // 在生成阻塞任务前获取合并信号量。限制并发合并，
    // 防止同时合并多个大文件（100GB+）时线程池耗尽。
    // 使用超时防止无限等待。
    let sem = merge_semaphore();
    let _permit = tokio::time::timeout(
        std::time::Duration::from_secs(constant::REQUEST_TIMEOUT_SECS),
        sem.acquire(),
    )
    .await
    .map_err(|_| AppError::Internal("merge semaphore acquire timed out".into()))?
    .map_err(|_| AppError::Internal("merge semaphore closed".into()))?;

    let merge_result =
        tokio::task::spawn_blocking(move || do_merge_io(&task_id_clone, total, &output_path_clone))
            .await
            .map_err(|e| AppError::Internal(format!("merge task panicked: {}", e)))?;

    let (computed_md5, total_written) = match merge_result {
        Ok(v) => v,
        Err(e) => {
            // 合并失败时不要删除暂存文件。保留它们以便客户端
            // 无需重新上传分块即可重试合并。仅标记为失败；
            // GC 会在过期后清理。
            let _ = dao::update_upload_status(db, task_id, TaskStatus::Failed.as_str()).await;
            return Err(e);
        }
    };

    let temp_path = output_path.with_extension(constant::TEMP_FILE_EXTENSION);

    if computed_md5 != file_md5 {
        // 哈希不匹配——仅清理临时文件，不清除暂存分块（#5）
        let _ = std::fs::remove_file(&temp_path);
        let _ = dao::update_upload_status(db, task_id, TaskStatus::Failed.as_str()).await;
        return Err(AppError::HashMismatch {
            expected: file_md5.to_string(),
            actual: computed_md5,
        });
    }

    // Atomic rename from temp to final path
    if let Err(e) = tokio::fs::rename(&temp_path, &output_path).await {
        // 重命名失败——仅清理临时文件，不清除暂存分块（#5）
        let _ = std::fs::remove_file(&temp_path);
        let _ = dao::update_upload_status(db, task_id, TaskStatus::Failed.as_str()).await;
        return Err(AppError::StorageError(format!("rename merged: {}", e)));
    }

    // 重命名后 fsync 父目录，确保重命名是持久的。
    // 如果没有这个，即使文件数据已 fsync，崩溃也可能丢失目录条目
    // ——文件在重启后会显示为丢失。
    if let Some(parent) = output_path.parent()
        && let Err(e) = sync_directory(parent)
    {
        tracing::warn!(path = %parent.display(), error = %e, "fsync parent dir after rename failed");
    }

    // 如果未提供，从扩展名检测 content_type
    let effective_content_type: Option<String> = content_type
        .map(|s| s.to_string())
        .or_else(|| detect_mime_from_extension(file_name));

    // 如果文件是图片，检测图片尺寸
    let (image_width, image_height, image_type_str) = if let Some(ref ct) = effective_content_type {
        if ct.starts_with("image/") {
            let path = output_path.clone();
            tokio::task::spawn_blocking(move || detect_image_dims(&path))
                .await
                .map_err(|e| AppError::Internal(format!("image detect panicked: {}", e)))?
                .unwrap_or((0, 0, String::new()))
        } else {
            (0i64, 0i64, String::new())
        }
    } else {
        (0i64, 0i64, String::new())
    };

    // 将数据库操作包装在事务中
    let txn = db.begin().await?;
    let uuid_str = object_id.to_string();
    let storage_path_str = output_path.to_string_lossy().to_string();
    let result = objects::Entity::insert(objects::ActiveModel {
        uuid: Set(uuid_str.clone()),
        name: Set(file_name.to_string()),
        size: Set(total_written as i64),
        md5: Set(file_md5.to_string()),
        content_type: Set(effective_content_type.clone()),
        extension: Set(extension.clone()),
        bucket: Set(constant::DEFAULT_BUCKET.to_string()),
        storage_path: Set(storage_path_str.clone()),
        image_width: Set(image_width),
        image_height: Set(image_height),
        image_type: Set(image_type_str.clone()),
        status: Set(ObjectStatus::Active.as_str().to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(&txn)
    .await?;

    let object_internal_id = result.last_insert_id;

    // 创建用户-对象关联
    dao::insert_user_object(&txn, user_id, object_internal_id).await?;

    // 将任务标记为已完成
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

    // 成功时从位图缓存中移除任务
    super::chunk_svc::bitmap_cache().remove(&task_id.to_string());

    // 清理暂存目录
    let staging_dir = path::get_chunk_staging_dir(&task_id);
    if staging_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
    }

    Ok(MergeResult {
        object_id: uuid_str.clone(),
        storage_path: storage_path_str.clone(),
        size: total_written,
        md5: file_md5.to_string(),
    })
}

/// 合并的纯同步 I/O 部分：一次遍历完成副本分块+计算 MD5。
/// 在阻塞线程上运行，因此不会阻塞 tokio 运行时。
///
/// 分块 MD5 从 sidecar 文件（上传期间存储）读取，而不是
/// 重新读取所有分块数据。文件级 MD5 计算方式：
///   file_md5 = MD5(chunk_md5_hex[0] || chunk_md5_hex[1] || ...)
/// 这消除了零拷贝合并路径中的双重读取，大致将大文件的
/// 磁盘 I/O 减少了一半。
fn do_merge_io(
    task_id: &Uuid,
    total: u32,
    output_path: &std::path::Path,
) -> Result<(String, u64), AppError> {
    use std::io::{Read, Seek, Write};

    let temp_path = output_path.with_extension(constant::TEMP_FILE_EXTENSION);
    let output_file = std::fs::File::create(&temp_path)
        .map_err(|e| AppError::StorageError(format!("create output: {}", e)))?;

    let mut hasher = Md5::new();
    let mut total_written: u64 = 0;

    // BufWriter 仅用于回退路径；零拷贝直接写入原始 File。
    let mut buf_writer =
        std::io::BufWriter::with_capacity(constant::MERGE_BUF_WRITER_CAPACITY, output_file);

    // 用于回退/sidecar 缺失读取路径的可复用缓冲区——避免每次迭代分配。
    let mut buffer = vec![0u8; constant::CHUNK_STREAM_BUFFER_SIZE];

    for i in 0..total {
        let chunk_path = path::get_chunk_staging_path(task_id, i);
        let chunk_size = match std::fs::metadata(&chunk_path) {
            Ok(m) => m.len() as usize,
            Err(_) => return Err(AppError::ChunkNotFound(format!("chunk {} not found", i))),
        };

        // 从 sidecar 文件读取分块 MD5，用于整个文件的 MD5 计算。
        // Sidecar 文件在上传期间写入（参见 write_chunk_md5_sidecar）。
        // 这避免在零拷贝路径中仅为了 MD5 而重新读取分块字节。
        let sidecar_path = chunk_path.with_extension(CHUNK_MD5_SIDECAR_EXTENSION);
        let sidecar_exists = sidecar_path.as_path().exists();

        if sidecar_exists {
            let sidecar_md5 = std::fs::read_to_string(&sidecar_path).unwrap_or_default();
            let sidecar_md5 = sidecar_md5.trim();
            // 使用 sidecar MD5 十六进制字符串作为全文件哈希器的输入。
            // 这与 CLI 的 Merkle 根计算方式一致：
            //   file_md5 = MD5(chunk_md5[0] || chunk_md5[1] || ...)
            if !sidecar_md5.is_empty() {
                hasher.update(sidecar_md5.as_bytes());
            }
        } else {
            // Sidecar 缺失——读取分块数据计算 MD5，并保持文件句柄
            // 打开供回退拷贝路径使用，避免重新打开。
            let mut chunk_file = std::fs::File::open(&chunk_path)
                .map_err(|_| AppError::ChunkNotFound(format!("chunk {} not found", i)))?;
            loop {
                let n = chunk_file
                    .read(&mut buffer)
                    .map_err(|e| AppError::StorageError(format!("read chunk {}: {}", i, e)))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
        }

        // 在零拷贝前刷新 BufWriter，确保原始 fd 位置正确
        buf_writer
            .flush()
            .map_err(|e| AppError::StorageError(format!("flush before zero-copy: {}", e)))?;

        // 获取内部文件，在缓冲区刷新后寻道到当前 total_written 位置
        let inner = buf_writer.get_mut();
        inner
            .seek(std::io::SeekFrom::Start(total_written))
            .map_err(|e| AppError::StorageError(format!("seek for zero-copy: {}", e)))?;

        // 仅在 sidecar 存在时尝试零拷贝（MD5 已从 sidecar 获取）。
        // 当 sidecar 缺失时，跳过零拷贝以避免
        // 第三次打开分块文件（已在上方为 MD5 计算打开过）。
        let zero_copy_ok = if sidecar_exists {
            copy_file_range_merge(&chunk_path, inner, chunk_size)
        } else {
            false
        };

        if !zero_copy_ok {
            // 回退：通过 BufWriter 的缓冲读写。
            // MD5 已在上方处理（来自 sidecar 或分块读取），
            // 因此这里只拷贝数据。
            let mut chunk_file = std::fs::File::open(&chunk_path)
                .map_err(|_| AppError::ChunkNotFound(format!("chunk {} not found", i)))?;
            loop {
                let n = chunk_file
                    .read(&mut buffer)
                    .map_err(|e| AppError::StorageError(format!("read chunk {}: {}", i, e)))?;
                if n == 0 {
                    break;
                }
                buf_writer
                    .write_all(&buffer[..n])
                    .map_err(|e| AppError::StorageError(format!("write merged: {}", e)))?;
                total_written += n as u64;
            }
        } else {
            // 零拷贝路径：数据已通过 copy_file_range 写入。
            // MD5 已从上面的 sidecar 获取——无需重新读取。
            total_written += chunk_size as u64;
        }
    }

    buf_writer
        .flush()
        .map_err(|e| AppError::StorageError(format!("flush output: {}", e)))?;

    // fsync 输出文件，确保在重命名前数据在磁盘上是持久的。
    // 没有 fsync，重命名后的崩溃/断电可能导致最终文件
    // 数据不完整（元数据可能在数据之前提交）。
    let inner = buf_writer.get_ref();
    inner
        .sync_all()
        .map_err(|e| AppError::StorageError(format!("fsync output: {}", e)))?;

    let computed_md5 = hex::encode(hasher.finalize());
    Ok((computed_md5, total_written))
}

/// 尝试使用 Linux `copy_file_range` 系统调用进行零拷贝合并。
/// 通过重试处理 EAGAIN（资源暂时不可用）和 EINTR（中断），
/// 这对大文件合并至关重要，因为内核可能返回部分副本或被信号中断。
#[cfg(target_os = "linux")]
fn copy_file_range_merge(
    chunk_path: &std::path::Path,
    output: &std::fs::File,
    _chunk_size: usize,
) -> bool {
    use std::os::unix::io::AsRawFd;

    let src = match std::fs::File::open(chunk_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut remaining = _chunk_size;
    loop {
        let copied = unsafe {
            libc::copy_file_range(
                src.as_raw_fd(),
                std::ptr::null_mut(),
                output.as_raw_fd(),
                std::ptr::null_mut(),
                remaining,
                0,
            )
        };
        if copied > 0 {
            remaining -= copied as usize;
            if remaining == 0 {
                break;
            }
            continue;
        }
        if copied == 0 {
            // EOF on source — unexpected if chunk_size > 0, but not an error
            break;
        }
        // copied < 0：错误
        let err = std::io::Error::last_os_error();
        let raw = err.raw_os_error();
        if raw == Some(libc::EINTR) || raw == Some(libc::EAGAIN) {
            // 对瞬时错误进行重试
            continue;
        }
        return false; // real I/O error
    }
    remaining == 0
}

#[cfg(not(target_os = "linux"))]
fn copy_file_range_merge(
    _chunk_path: &std::path::Path,
    _output: &std::fs::File,
    _chunk_size: usize,
) -> bool {
    false
}

/// 写入分块 MD5 sidecar 文件，用于合并时逐分块完整性验证。
/// 在每个分块上传成功后调用。
pub fn write_chunk_md5_sidecar(
    task_id: &Uuid,
    chunk_index: u32,
    md5_hex: &str,
) -> std::io::Result<()> {
    let chunk_path = path::get_chunk_staging_path(task_id, chunk_index);
    let md5_path = chunk_path.with_extension(CHUNK_MD5_SIDECAR_EXTENSION);
    std::fs::write(&md5_path, md5_hex)
}

/// fsync 一个目录，确保目录条目（如重命名）是持久的。
/// 通过路径打开目录并在 fd 上调用 fsync。
#[cfg(target_os = "linux")]
fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let dir = std::fs::File::open(path)?;
    let fd = dir.as_raw_fd();
    let ret = unsafe { libc::fsync(fd) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn sync_directory(_path: &std::path::Path) -> std::io::Result<()> {
    // 目录上的 fsync 不是普遍支持的；在非 Linux 上跳过
    Ok(())
}

/// 从文件扩展名检测 MIME 类型。如果未知则返回 None。
/// 使用 `mime_guess` crate 获取全面且维护良好的 MIME 映射。
fn detect_mime_from_extension(file_name: &str) -> Option<String> {
    let mime = mime_guess::from_path(file_name).first_or_octet_stream();
    if mime.essence_str() == "application/octet-stream" {
        return None;
    }
    Some(mime.to_string())
}

/// 通过读取魔数从最终文件检测图片尺寸和类型。
/// 支持 JPEG、PNG、GIF、WebP、BMP。失败时返回默认值。
fn detect_image_dims(path: &std::path::Path) -> Result<(i64, i64, String), AppError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| AppError::Internal(format!("open file for image detect: {}", e)))?;

    // 读取足够所有常见图片头部的数据（JPEG SOF 可能在 EXIF 之后更深的位置）
    let mut buf = vec![0u8; 4096];
    let n = file
        .read(&mut buf)
        .map_err(|e| AppError::Internal(format!("read file for image detect: {}", e)))?;
    let header = &buf[..n];

    // JPEG：以 FF D8 FF 开头
    if n >= 3 && header[0] == 0xFF && header[1] == 0xD8 && header[2] == 0xFF {
        let mut pos = 2;
        while pos + 9 < n {
            if header[pos] == 0xFF && matches!(header[pos + 1], 0xC0..=0xC2) {
                let height = u16::from_be_bytes([header[pos + 5], header[pos + 6]]);
                let width = u16::from_be_bytes([header[pos + 7], header[pos + 8]]);
                return Ok((width as i64, height as i64, "jpeg".into()));
            }
            pos += 1;
        }
        return Ok((0, 0, "jpeg".into())); // 已知为 jpeg 但未在头部窗口中找到尺寸
    }

    // PNG：魔数 89 50 4E 47 0D 0A 1A 0A，IHDR 在偏移 16 处
    if n >= 24 && header[0] == 0x89 && header[1] == b'P' && header[2] == b'N' && header[3] == b'G' {
        let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
        let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
        return Ok((width as i64, height as i64, "png".into()));
    }

    // GIF：魔数 "GIF87a" 或 "GIF89a"，尺寸在偏移 6 处（小端）
    if n >= 10
        && (header[0] == b'G'
            && header[1] == b'I'
            && header[2] == b'F'
            && (header[3] == b'8' && (header[4] == b'7' || header[4] == b'9') && header[5] == b'a'))
    {
        let width = u16::from_le_bytes([header[6], header[7]]);
        let height = u16::from_le_bytes([header[8], header[9]]);
        return Ok((width as i64, height as i64, "gif".into()));
    }

    // BMP：魔数 "BM"，尺寸在偏移 18 处（小端）
    if n >= 26 && header[0] == b'B' && header[1] == b'M' {
        let width = u32::from_le_bytes([header[18], header[19], header[20], header[21]]);
        let height = u32::from_le_bytes([header[22], header[23], header[24], header[25]]);
        return Ok((width as i64, height as i64, "bmp".into()));
    }

    // WebP：RIFF + 大小 + WEBP
    if n >= 30
        && header[0] == b'R'
        && header[1] == b'I'
        && header[2] == b'F'
        && header[3] == b'F'
        && header[8] == b'W'
        && header[9] == b'E'
        && header[10] == b'B'
        && header[11] == b'P'
    {
        let fourcc = &header[12..16];
        if fourcc == b"VP8 " && n >= 30 {
            // VP8 关键帧：宽/高在偏移 26 处（小端，16 像素对齐）
            let raw = u16::from_le_bytes([header[26], header[27]]);
            let width = (raw & 0x3FFF) as u32;
            let raw = u16::from_le_bytes([header[28], header[29]]);
            let height = (raw & 0x3FFF) as u32;
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        } else if fourcc == b"VP8L" && n >= 25 {
            // VP8L 无损：宽/高打包在偏移 21 处的 4 个字节中
            let bits = u32::from_le_bytes([header[21], header[22], header[23], header[24]]);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        } else if fourcc == b"VP8X" && n >= 30 {
            // VP8X 扩展：3 字节宽（小端），3 字节高，在偏移 24 处
            let width = u24_le(&header[24..27]);
            let height = u24_le(&header[27..30]);
            if width > 0 && height > 0 {
                return Ok((width as i64, height as i64, "webp".into()));
            }
        }
        return Ok((0, 0, "webp".into()));
    }

    // 未知格式——返回默认值
    Ok((0, 0, String::new()))
}

/// 从字节切片解析 24 位小端值（必须至少 3 字节）。
fn u24_le(bytes: &[u8]) -> u32 {
    bytes[0] as u32 | (bytes[1] as u32) << 8 | (bytes[2] as u32) << 16
}
