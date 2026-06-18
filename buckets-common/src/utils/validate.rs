//! 文件上传的输入验证辅助函数。
//!
//! 涵盖文件扩展名过滤、文件大小检查、对象所有权
//! 验证以及磁盘空间预检。

use crate::error::AppError;

// ============================================================================
// 内联常量（从已删除的 constant.rs 迁移）
// ============================================================================

/// 当为 `false` 时，完全跳过扩展名验证。
/// 可通过 `STRICT_EXTENSION_CHECK=true` 环境变量覆盖。
fn strict_extension_check() -> bool {
    std::env::var("STRICT_EXTENSION_CHECK")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// 启用 `STRICT_EXTENSION_CHECK` 时允许的文件扩展名。
const ALLOWED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "mp4", "mkv", "avi", "mov", "wmv", "flv", "mp3",
    "wav", "flac", "aac", "ogg", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "csv",
    "json", "xml", "yaml", "toml", "zip", "tar", "gz", "bz2", "7z", "rar", "iso", "img", "bin",
    "dat", "raw", "log", "tmp", "bak",
];

/// 启用 `STRICT_EXTENSION_CHECK` 时阻止的文件扩展名。
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "bat", "cmd", "com", "msi", "scr", "js", "vbs", "ps1", "dll", "sys", "drv", "ocx",
];

/// 磁盘空间余量乘数（百分比 / 100）。
const DISK_SPACE_HEADROOM_RATIO: u64 = 110;
const DISK_SPACE_HEADROOM_DIVISOR: u64 = 100;
/// 合并磁盘空间乘数（2 倍文件大小 + 暂存和输出的余量）。
const MERGE_DISK_SPACE_MULTIPLIER: u64 = 2;

/// 根据允许/阻止列表验证文件名的扩展名。
/// 当 `STRICT_EXTENSION_CHECK` 为 false 时，所有扩展名都通过。
pub fn validate_file_extension(filename: &str) -> Result<(), AppError> {
    // 当 STRICT_EXTENSION_CHECK 环境变量为 false（默认）时，允许所有文件类型（私有存储场景）
    if !strict_extension_check() {
        return Ok(());
    }

    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext {
        Some(ref e) if BLOCKED_EXTENSIONS.contains(&e.as_str()) => Err(AppError::InvalidFileType(
            format!("file extension '{}' is blocked", e),
        )),
        Some(ref e) if !ALLOWED_EXTENSIONS.contains(&e.as_str()) => Err(AppError::InvalidFileType(
            format!("file extension '{}' is not allowed", e),
        )),
        Some(_) => Ok(()),
        None => Err(AppError::InvalidFileType("file has no extension".into())),
    }
}

/// 检查可用磁盘空间是否满足给定的字节需求。
/// 增加 10% 余量。适用于一般情况。
pub fn check_disk_space(required: u64) -> Result<u64, AppError> {
    let Some(available) = available_disk_space() else {
        return Ok(u64::MAX); // 不支持的平台跳过检查
    };
    // 要求至少 10% 的余量
    let needed = required.saturating_mul(DISK_SPACE_HEADROOM_RATIO) / DISK_SPACE_HEADROOM_DIVISOR;
    if available < needed {
        return Err(AppError::StorageError(format!(
            "insufficient disk space: need {} bytes, available {} bytes",
            needed, available
        )));
    }
    Ok(available)
}

/// 在写入单个分块之前检查磁盘空间（临时暂存数据）。
/// 分块相对于总文件大小较小，因此只需要分块大小 + 10%。
pub fn check_disk_space_for_chunk(chunk_size: u64) -> Result<u64, AppError> {
    check_disk_space(chunk_size)
}

/// 检查错误是否为磁盘已满（ENOSPC）错误。
/// 如果磁盘已满，返回用户友好的错误消息。
#[cfg(target_os = "linux")]
pub fn check_enospc(err: &std::io::Error) -> Option<AppError> {
    if err.raw_os_error() == Some(28) {
        // ENOSPC = 28
        Some(AppError::StorageError(
            "disk full: no space left on device".into(),
        ))
    } else {
        None
    }
}

#[cfg(not(target_os = "linux"))]
pub fn check_enospc(err: &std::io::Error) -> Option<AppError> {
    let _ = err;
    None
}

/// 合并前检查磁盘空间：输出文件为完整文件大小。
/// 需要 file_size（合并输出）+ 10% 余量。
/// 注意：此时分块暂存数据仍然存在，但将在合并后删除，
/// 因此保守要求 2 倍 file_size + 10%。
pub fn check_disk_space_for_merge(file_size: u64) -> Result<u64, AppError> {
    let Some(available) = available_disk_space() else {
        return Ok(u64::MAX); // 不支持的平台跳过检查
    };
    // 输出文件 + 现有暂存分块 ≈ 2 倍 file_size，加上 10% 余量
    let needed = file_size
        .saturating_mul(MERGE_DISK_SPACE_MULTIPLIER)
        .saturating_mul(DISK_SPACE_HEADROOM_RATIO)
        / DISK_SPACE_HEADROOM_DIVISOR;
    if available < needed {
        return Err(AppError::StorageError(format!(
            "insufficient disk space for merge: need {} bytes (output + staging), available {} bytes",
            needed, available
        )));
    }
    Ok(available)
}

/// 返回当前目录所在文件系统的可用磁盘空间字节数。
pub fn available_disk_space() -> Option<u64> {
    disk_space_info_raw().map(|(available, _total, _pct)| available)
}

/// 返回（可用字节数，使用百分比）用于健康检查。
pub fn disk_space_info() -> (Option<u64>, Option<f64>) {
    disk_space_info_raw()
        .map(|(available, _total, pct)| (Some(available), Some(pct)))
        .unwrap_or((None, None))
}

/// 内部函数：返回（可用字节数，总字节数，使用百分比）。
fn disk_space_info_raw() -> Option<(u64, u64, f64)> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        use std::os::unix::fs::MetadataExt;

        let info = fs::metadata(".").ok()?;
        let dev = info.dev();
        let mounts = fs::read_to_string("/proc/self/mountinfo").ok()?;
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 10 {
                let mount_point = parts[4];
                let mount_meta = fs::metadata(mount_point).ok()?;
                if mount_meta.dev() == dev {
                    let mut statbuf: std::mem::MaybeUninit<libc::statvfs> =
                        std::mem::MaybeUninit::zeroed();
                    let ret = unsafe {
                        libc::statvfs(
                            mount_point.as_ptr() as *const libc::c_char,
                            statbuf.as_mut_ptr(),
                        )
                    };
                    if ret != 0 {
                        // statvfs 在挂载点失败（可能发生在 overlay 环境下）。
                        // 回退到直接尝试当前工作目录。
                        continue;
                    }
                    let stat = unsafe { statbuf.assume_init() };
                    let total = stat.f_blocks * stat.f_bsize;
                    let available = stat.f_bavail * stat.f_bsize;
                    let used = total.saturating_sub(available);
                    let pct = if total > 0 {
                        (used as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    return Some((available, total, pct));
                }
            }
        }
        // 回退：如果没有匹配的 mountinfo 条目（例如 overlay/容器中
        // 设备和 mountinfo 的设备号不同），对当前工作目录执行 statvfs。
        let cwd = std::env::current_dir().ok()?;
        let mut statbuf: std::mem::MaybeUninit<libc::statvfs> = std::mem::MaybeUninit::zeroed();
        let ret = unsafe {
            libc::statvfs(
                cwd.to_str()?.as_ptr() as *const libc::c_char,
                statbuf.as_mut_ptr(),
            )
        };
        if ret != 0 {
            return None;
        }
        let stat = unsafe { statbuf.assume_init() };
        let total = stat.f_blocks * stat.f_bsize;
        let available = stat.f_bavail * stat.f_bsize;
        let used = total.saturating_sub(available);
        let pct = if total > 0 {
            (used as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        Some((available, total, pct))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
