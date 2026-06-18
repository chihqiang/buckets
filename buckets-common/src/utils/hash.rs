//! MD5 哈希工具，用于文件完整性验证。
//!
//! 提供异步文件 MD5 计算（带可选的进度回调）
//! 和内存中的分块 MD5 哈希。

use crate::constant;
use md5::{Digest, Md5};
use std::path::Path;
use tokio::io::AsyncReadExt;

/// 异步计算文件的 MD5 十六进制摘要（tokio）。
pub async fn compute_file_md5_async(path: &Path) -> Result<String, std::io::Error> {
    compute_file_md5_with_progress(path, |_, _| {}).await
}

/// 异步计算文件的 MD5 十六进制摘要，带进度回调。
/// 回调函数定期接收 (bytes_read, total_bytes)。
pub async fn compute_file_md5_with_progress<F>(
    path: &Path,
    progress: F,
) -> Result<String, std::io::Error>
where
    F: Fn(u64, u64),
{
    let file_size = tokio::fs::metadata(path).await?.len();
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Md5::new();
    let mut buffer = [0u8; constant::CHUNK_STREAM_BUFFER_SIZE];
    let mut total_read: u64 = 0;
    let mut last_report: u64 = 0;

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        total_read += bytes_read as u64;

        // 每 64 MiB 报告一次进度，避免过多的回调
        if total_read - last_report >= 64 * 1024 * 1024 {
            progress(total_read, file_size);
            last_report = total_read;
        }
    }
    // 最终报告
    if total_read > last_report {
        progress(total_read, file_size);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// 计算内存中字节切片（分块）的 MD5 十六进制摘要。
pub fn compute_chunk_md5(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// 验证内存中字节切片的 MD5 摘要是否与期望值匹配。
pub fn verify_chunk_md5(data: &[u8], expected_md5: &str) -> bool {
    let actual = compute_chunk_md5(data);
    actual == expected_md5
}
