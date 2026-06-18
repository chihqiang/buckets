//! CLI 特定配置常量。

// ============================================================================
// 环境变量键名
// ============================================================================

pub const ENV_CHUNK_UPLOAD_TIMEOUT_SECS: &str = "CHUNK_UPLOAD_TIMEOUT_SECS";

// ============================================================================
// HTTP 客户端
// ============================================================================

pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 600;
pub const DEFAULT_HTTP_CONNECT_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_POOL_MAX_IDLE_PER_HOST: usize = 4;
pub const DEFAULT_CHUNK_UPLOAD_TIMEOUT_SECS: u64 = 1800;

// ============================================================================
// 上传
// ============================================================================

pub const DEFAULT_CHUNK_SIZE_MB: u64 = 8;
pub const DEFAULT_PARALLEL_UPLOADS: usize = 4;

// ============================================================================
// 分块上传重试
// ============================================================================

pub const CHUNK_UPLOAD_MAX_RETRIES: u32 = 3;
pub const CHUNK_UPLOAD_RETRY_BACKOFF_BASE_SECS: u64 = 1;
pub const CHUNK_UPLOAD_RETRY_MAX_JITTER_MS: u64 = 500;

// ============================================================================
// 异步合并轮询
// ============================================================================

pub const MERGE_POLL_INTERVAL_SECS: u64 = 2;
pub const MERGE_POLL_MAX_ATTEMPTS: u64 = 3600;
pub const MERGE_POLL_MAX_INTERVAL_SECS: u64 = 30;

// ============================================================================
// CLI 配置和缓存
// ============================================================================

pub const CREDENTIALS_FILE_MODE: u32 = 0o600;
pub const CLI_CONFIG_DIR: &str = ".buckets";
pub const CLI_CREDENTIALS_FILE: &str = "credentials.json";
pub const CLI_CACHE_SUBDIR: &str = "cache";
pub const CLI_CACHE_EXTENSION: &str = "json";
pub const CLI_CACHE_KEY_FILE_PATH: &str = "file_path";
pub const CLI_CACHE_KEY_OBJECT_NAME: &str = "bucket_name";
pub const CLI_CACHE_KEY_CHUNK_SIZE: &str = "chunk_size";

// ============================================================================
// 杂项
// ============================================================================

pub const UNKNOWN_FILE_NAME: &str = "unknown";
