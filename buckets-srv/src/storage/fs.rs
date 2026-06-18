//! 文件系统存储初始化。
//!
//! 确保启动时所需的目录结构已存在。

use buckets_common::constant;
use buckets_common::error::AppError;

/// 创建存储、暂存和缓存目录（如果它们不存在）。
pub async fn ensure_directories() -> Result<(), AppError> {
    let dirs = [
        constant::storage_dir(),
        constant::staging_dir(),
        constant::CACHE_DIR.to_string(),
    ];
    for dir in &dirs {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| AppError::StorageError(format!("create dir {}: {}", dir, e)))?;
    }
    Ok(())
}
