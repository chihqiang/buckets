//! 文件系统路径辅助函数，用于对象、暂存分块和缓存文件。

use std::path::PathBuf;
use uuid::Uuid;

use crate::constant;

/// 构建合并后对象文件的存储路径。
/// 格式：`data/objects/{userId}/{YYYYMMDD}/{uuid}.{ext}`
pub fn get_object_storage_path(
    object_id: &Uuid,
    user_id: u64,
    year: i32,
    month: u32,
    day: u32,
    ext: &str,
) -> PathBuf {
    PathBuf::from(constant::storage_dir())
        .join(user_id.to_string())
        .join(format!("{:04}{:02}{:02}", year, month, day))
        .join(format!(
            "{}{}",
            object_id,
            if ext.is_empty() {
                String::new()
            } else {
                format!(".{}", ext)
            }
        ))
}

/// 构建单个分块文件的暂存路径。
/// 格式：`data/staging/XX/YY/<task_id>/chunk_XXXXXX`
/// 其中 XX/YY 是任务 UUID 的前 4 位十六进制字符，用于分片。
pub fn get_chunk_staging_path(task_id: &Uuid, chunk_index: u32) -> PathBuf {
    let dir = get_chunk_staging_dir(task_id);
    dir.join(format!("chunk_{:06}", chunk_index))
}

/// 构建上传任务所有分块的暂存目录路径。
/// 格式：`data/staging/XX/YY/<task_id>/`
/// 其中 XX/YY 是任务 UUID 的前 4 位十六进制字符，用于分片。
pub fn get_chunk_staging_dir(task_id: &Uuid) -> PathBuf {
    let id_str = task_id.to_string();
    PathBuf::from(constant::staging_dir())
        .join(&id_str[0..2])
        .join(&id_str[2..4])
        .join(id_str)
}

/// 从文件名中提取小写文件扩展名。
pub fn get_extension(filename: &str) -> Option<String> {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}
