//! 从环境变量加载的应用配置。
//!
//! 所有字段都有适合开发的合理默认值。生产部署
//! 应显式设置 `DATABASE_URL` 和 CORS 源。

use buckets_common::constant;

/// 上传速率限制的配置。
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 每用户每秒最大上传请求数（突发+持续）
    pub max_requests_per_second: f64,
    /// 最大突发大小（令牌桶容量）
    pub burst_size: f64,
    /// 每用户最大并发活跃上传数
    pub max_concurrent_uploads: u32,
    /// 每用户每天最大新建上传任务数
    pub daily_upload_quota: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_second: constant::DEFAULT_RATE_LIMIT_RPS,
            burst_size: constant::DEFAULT_RATE_LIMIT_BURST,
            max_concurrent_uploads: constant::DEFAULT_RATE_LIMIT_MAX_CONCURRENT,
            daily_upload_quota: constant::DEFAULT_RATE_LIMIT_DAILY_QUOTA,
        }
    }
}

impl RateLimitConfig {
    pub fn from_env() -> Self {
        fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
            std::env::var(key)
                .unwrap_or_default()
                .parse()
                .unwrap_or(default)
        }
        Self {
            max_requests_per_second: env_parse(
                constant::ENV_RATE_LIMIT_RPS,
                constant::DEFAULT_RATE_LIMIT_RPS,
            ),
            burst_size: env_parse(
                constant::ENV_RATE_LIMIT_BURST,
                constant::DEFAULT_RATE_LIMIT_BURST,
            ),
            max_concurrent_uploads: env_parse(
                constant::ENV_RATE_LIMIT_MAX_CONCURRENT,
                constant::DEFAULT_RATE_LIMIT_MAX_CONCURRENT,
            ),
            daily_upload_quota: env_parse(
                constant::ENV_RATE_LIMIT_DAILY_QUOTA,
                constant::DEFAULT_RATE_LIMIT_DAILY_QUOTA,
            ),
        }
    }
}

/// 服务器的中央配置结构体。
#[derive(Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    #[cfg_attr(debug_assertions, allow(dead_code))]
    database_url: String,
    pub db_max_conn: u32,
    /// 可配置的 CORS 源（逗号分隔，为空=开发环境允许所有）
    pub cors_allowed_origins: Vec<String>,
    /// 具有超级管理员权限的用户 ID 集合（Web 管理）
    pub super_admin_ids: Vec<u64>,
    /// 速率限制配置（网关）
    pub rate_limit: RateLimitConfig,
    /// 是否启用速率限制（网关）
    pub rate_limit_enabled: bool,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database_url", &"[redacted]")
            .field("db_max_conn", &self.db_max_conn)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("super_admin_ids", &self.super_admin_ids)
            .field("rate_limit", &self.rate_limit)
            .field("rate_limit_enabled", &self.rate_limit_enabled)
            .finish()
    }
}

impl AppConfig {
    /// 返回数据库 URL（仅 crate 内部可访问）。
    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }
}

impl AppConfig {
    /// 从环境变量构建配置，带有回退默认值。
    pub fn from_env() -> anyhow::Result<Self> {
        fn env_str(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }

        fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
            env_str(key, "").parse().unwrap_or(default)
        }
        let cors_origins = std::env::var(constant::ENV_CORS_ALLOWED_ORIGINS).unwrap_or_default();
        let cors_allowed_origins = if cors_origins.is_empty() {
            Vec::new() // empty means allow all (dev mode)
        } else {
            cors_origins
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        let super_admin_ids: Vec<u64> = std::env::var(constant::ENV_SUPER_ADMIN_IDS)
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect();

        let host = env_str(constant::ENV_HOST, constant::DEFAULT_HOST);
        let port = env_parse(constant::ENV_PORT, constant::DEFAULT_PORT);
        let database_url = env_str(constant::ENV_DATABASE_URL, constant::DEFAULT_DATABASE_URL);
        let db_max_conn = env_parse(constant::ENV_DB_MAX_CONN, constant::DEFAULT_DB_MAX_CONN);

        // --- 配置验证（启动时快速失败）---
        validate_port(port)?;
        validate_database_url(&database_url)?;
        validate_db_max_conn(db_max_conn)?;
        validate_cors_origins(&cors_allowed_origins)?;

        Ok(AppConfig {
            host,
            port,
            database_url,
            db_max_conn,
            cors_allowed_origins,
            super_admin_ids,
            rate_limit: RateLimitConfig::from_env(),
            rate_limit_enabled: env_parse(
                constant::ENV_RATE_LIMIT_ENABLED,
                constant::DEFAULT_RATE_LIMIT_ENABLED,
            ),
        })
    }
}

/// 验证端口在有效范围内（1..=65535）。
fn validate_port(port: u16) -> anyhow::Result<()> {
    if port == 0 {
        anyhow::bail!("invalid PORT={}: port must be between 1 and 65535", port);
    }
    Ok(())
}

/// 验证 DATABASE_URL 具有可识别的 MySQL 方案。
fn validate_database_url(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("mysql://") && !url.starts_with("mysqlx://") {
        anyhow::bail!(
            "DATABASE_URL must start with 'mysql://' or 'mysqlx://', got: {}...",
            &url[..url.len().min(20)]
        );
    }
    Ok(())
}

/// 验证 db_max_conn 在合理范围内。
fn validate_db_max_conn(max_conn: u32) -> anyhow::Result<()> {
    if max_conn == 0 {
        anyhow::bail!("DB_MAX_CONN must be >= 1, got 0");
    }
    if max_conn > 1000 {
        tracing::warn!(
            "DB_MAX_CONN={} is very high, consider a lower value",
            max_conn
        );
    }
    Ok(())
}

/// 验证 CORS 源格式（每个条目应为 URL 格式）。
fn validate_cors_origins(origins: &[String]) -> anyhow::Result<()> {
    for origin in origins {
        if origin.is_empty() {
            anyhow::bail!("CORS_ALLOWED_ORIGINS contains an empty entry");
        }
        if !origin.starts_with("http://") && !origin.starts_with("https://") {
            anyhow::bail!(
                "CORS_ALLOWED_ORIGINS entry '{}' does not start with 'http://' or 'https://'",
                origin
            );
        }
    }
    Ok(())
}
