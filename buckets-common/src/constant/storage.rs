//! 存储路径、存储桶默认值和目录常量。

// ============================================================================
// 存储目录
// ============================================================================

pub fn storage_dir() -> String {
    std::env::var("STORAGE_DIR").unwrap_or_else(|_| "data/objects".into())
}

pub fn staging_dir() -> String {
    std::env::var("STAGING_DIR").unwrap_or_else(|_| "data/staging".into())
}

pub const CACHE_DIR: &str = "data/cache";

// ============================================================================
// 存储桶
// ============================================================================

pub const DEFAULT_BUCKET: &str = "default";
