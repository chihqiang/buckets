//! HTTP 层常量：头部、MIME、API、超时、限流、服务器绑定、CORS。

// ============================================================================
// 头部
// ============================================================================

pub const HEADER_TRACE_ID: &str = "x-trace-id";
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_SESSION_SIGNATURE: &str = "x-session-signature";
pub const HEADER_SESSION_TIMESTAMP: &str = "x-session-timestamp";
pub const HEADER_SESSION_SALT: &str = "x-session-salt";

// ============================================================================
// MIME 类型
// ============================================================================

pub const MIME_OCTET_STREAM: &str = "application/octet-stream";

// ============================================================================
// 认证方案
// ============================================================================

pub const AUTH_SCHEME_BASIC: &str = "Basic ";
pub const AUTH_SCHEME_BEARER: &str = "Bearer ";

// ============================================================================
// API 基础路径
// ============================================================================

pub const API_BASE_PATH: &str = "/api/v1";

// ============================================================================
// 超时和限制
// ============================================================================

pub const REQUEST_TIMEOUT_SECS: u64 = 600;
pub const BODY_LIMIT_OVERHEAD: u64 = 1024;

// ============================================================================
// 速率限制——环境变量键名
// ============================================================================

pub const ENV_RATE_LIMIT_ENABLED: &str = "RATE_LIMIT_ENABLED";
pub const ENV_RATE_LIMIT_RPS: &str = "RATE_LIMIT_RPS";
pub const ENV_RATE_LIMIT_BURST: &str = "RATE_LIMIT_BURST";
pub const ENV_RATE_LIMIT_MAX_CONCURRENT: &str = "RATE_LIMIT_MAX_CONCURRENT";
pub const ENV_RATE_LIMIT_DAILY_QUOTA: &str = "RATE_LIMIT_DAILY_QUOTA";

// ============================================================================
// 速率限制——默认值
// ============================================================================

pub const DEFAULT_RATE_LIMIT_ENABLED: bool = true;
pub const DEFAULT_RATE_LIMIT_RPS: f64 = 2.0;
pub const DEFAULT_RATE_LIMIT_BURST: f64 = 10.0;
pub const DEFAULT_RATE_LIMIT_MAX_CONCURRENT: u32 = 5;
pub const DEFAULT_RATE_LIMIT_DAILY_QUOTA: u32 = 50;

// ============================================================================
// 速率限制——清理
// ============================================================================

pub const RATE_LIMIT_BUCKET_CLEANUP_INTERVAL_SECS: u64 = 3600;

// ============================================================================
// 服务器绑定
// ============================================================================

pub const ENV_HOST: &str = "HOST";
pub const ENV_PORT: &str = "PORT";
pub const DEFAULT_HOST: &str = "0.0.0.0";
pub const DEFAULT_PORT: u16 = 8080;

// ============================================================================
// 跨域资源共享
// ============================================================================

pub const ENV_CORS_ALLOWED_ORIGINS: &str = "CORS_ALLOWED_ORIGINS";

// ============================================================================
// 杂项服务器设置
// ============================================================================

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";
pub const ENV_MAX_CHUNK_SIZE: &str = "MAX_CHUNK_SIZE";
pub const ENV_SUPER_ADMIN_IDS: &str = "SUPER_ADMIN_IDS";
pub const ENV_HOME: &str = "HOME";
