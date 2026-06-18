//! GC（垃圾回收）后台任务。
//!
//! 定期扫描过期的上传任务，在单个事务中将其标记为过期，
//! 并从磁盘清理其暂存文件。
//!
//! 分批处理过期任务，避免在处理大量过期任务时阻塞。

use buckets_common::constant;
use buckets_common::error::AppError;
use sea_orm::DatabaseConnection;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 启动 GC 清理后台任务。定期运行并通过 `CancellationToken` 监听优雅关闭。
pub async fn start_gc_cleaner(db: DatabaseConnection, cancellation: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            buckets_common::constant::CLEANUP_INTERVAL_SECS,
        ));
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::info!("GC cleaner shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = cleanup_expired_tasks(&db).await {
                        tracing::error!("GC cleanup error: {}", e);
                    }
                }
            }
        }
    });
}

/// 原子性地标记过期任务并移除其暂存目录和缓存文件。
/// 按 GC_BATCH_SIZE 分批处理，避免大事务和长阻塞期。
/// 在批次之间添加休眠以限制 I/O 速率，避免清理大量过期任务时产生磁盘风暴。
async fn cleanup_expired_tasks(db: &DatabaseConnection) -> Result<(), AppError> {
    let mut total_cleaned: u32 = 0;
    loop {
        // 在标记前获取过期任务 ID，然后清理暂存文件
        let expired_ids =
            crate::dao::expire_and_list_tasks_batch(db, constant::GC_BATCH_SIZE).await?;

        if expired_ids.is_empty() {
            break;
        }

        tracing::info!("GC: expired {} upload tasks (batch)", expired_ids.len());

        // 清理过期任务的物理暂存文件和缓存
        for id_str in &expired_ids {
            if let Ok(task_id) = uuid::Uuid::parse_str(id_str) {
                // 清理位图缓存条目
                crate::service::chunk_svc::bitmap_cache().remove(id_str);

                // 清理暂存目录（分片）
                let staging_dir = buckets_common::utils::path::get_chunk_staging_dir(&task_id);
                if staging_dir.exists()
                    && let Err(e) = tokio::fs::remove_dir_all(&staging_dir).await
                {
                    tracing::warn!(
                        "GC: failed to remove staging dir {}: {}",
                        staging_dir.display(),
                        e
                    );
                }
                // 清理缓存目录（非分片，遗留路径）
                let cache_dir =
                    std::path::PathBuf::from(buckets_common::constant::CACHE_DIR).join(id_str);
                if cache_dir.exists()
                    && let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await
                {
                    tracing::warn!(
                        "GC: failed to remove cache dir {}: {}",
                        cache_dir.display(),
                        e
                    );
                }
            }
        }

        total_cleaned += expired_ids.len() as u32;

        // 限制批次之间的磁盘 I/O 速率，防止可能影响活跃上传的 I/O 风暴。
        tokio::time::sleep(Duration::from_millis(constant::GC_BATCH_PAUSE_MS)).await;

        // 如果少于批次大小，则完成
        if (expired_ids.len() as u32) < constant::GC_BATCH_SIZE {
            break;
        }

        // 限制每次 GC 运行的总清理次数，避免无限制的 I/O
        if total_cleaned >= constant::GC_MAX_CLEANUP_PER_RUN {
            tracing::info!(
                "GC: reached max cleanup limit ({}), deferring remaining",
                total_cleaned
            );
            break;
        }
    }

    if total_cleaned > 0 {
        tracing::info!("GC: total cleaned {} tasks this run", total_cleaned);
    }

    Ok(())
}
