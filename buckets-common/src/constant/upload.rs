//! 上传/分块/合并/会话/位图常量。

// ============================================================================
// 上传状态值
// ============================================================================

pub const STATUS_INITIALIZED: &str = "initialized";
pub const STATUS_UPLOADING: &str = "uploading";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_EXPIRED: &str = "expired";
pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_MERGING: &str = "merging";
pub const CHUNK_STATUS_ALREADY_EXISTS: &str = "already_exists";
pub const CHUNK_STATUS_UPLOADED: &str = "uploaded";

// ============================================================================
// 分块大小和缓冲区
// ============================================================================

pub const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024;
pub const DEFAULT_MAX_CHUNK_SIZE: u64 = 256 * 1024 * 1024;
pub const CHUNK_STREAM_BUFFER_SIZE: usize = 65536;
pub const MERGE_BUF_WRITER_CAPACITY: usize = 1024 * 1024;
pub const TEMP_FILE_EXTENSION: &str = "tmp";

// ============================================================================
// 位图
// ============================================================================

pub const BITMAP_BITS_PER_WORD: usize = 64;
pub const BITMAP_FLUSH_INTERVAL_SECS: u64 = 5;
pub const BITMAP_CACHE_CLEANUP_INTERVAL_SECS: u64 = 300;
pub const BITMAP_CACHE_MAX_ENTRIES: usize = 500;

// ============================================================================
// 合并
// ============================================================================

pub const MAX_CONCURRENT_MERGES: usize = 4;

// ============================================================================
// 文件大小
// ============================================================================

pub const GB_DIVISOR: f64 = 1024.0 * 1024.0 * 1024.0;

/// 文件大小的软限制——超过时记录警告（1 TiB）。
pub const FILE_SIZE_SOFT_LIMIT_WARN: u64 = 1024 * 1024 * 1024 * 1024u64;

// ============================================================================
// 会话超时
// ============================================================================

pub const SESSION_ACTIVITY_TIMEOUT_SECS: i64 = 3600;
pub const MAX_SESSION_ACTIVITY_TIMEOUT_SECS: i64 = 172800;

// ============================================================================
// 上传过期时间缩放
// ============================================================================

pub const MIN_UPLOAD_EXPIRATION_HOURS: i64 = 72;
pub const EXPIRATION_SCALE_HOURS_PER_10GB: i64 = 24;
pub const LIVENESS_SCALE_SECS_PER_10GB: i64 = 3600;
