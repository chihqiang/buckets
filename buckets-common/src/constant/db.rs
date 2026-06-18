//! 数据库连接池和配置常量。

// ============================================================================
// 环境变量键名
// ============================================================================

pub const ENV_DATABASE_URL: &str = "DATABASE_URL";
pub const ENV_DB_MAX_CONN: &str = "DB_MAX_CONN";

// ============================================================================
// 默认值
// ============================================================================

pub const DEFAULT_DATABASE_URL: &str = "mysql://root:root@localhost:3306/buckets";
pub const DEFAULT_DB_MAX_CONN: u32 = 20;

// ============================================================================
// 连接池设置
// ============================================================================

pub const DB_POOL_MIN_CONNECTIONS: u32 = 2;
pub const DB_ACQUIRE_TIMEOUT_SECS: u64 = 30;
pub const DB_MAX_LIFETIME_SECS: u64 = 1800;
pub const DB_IDLE_TIMEOUT_SECS: u64 = 600;
