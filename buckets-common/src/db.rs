//! 共享数据库连接池创建，支持 MySQL 和 SQLite。

use std::time::Duration;

use sea_orm::{ConnectOptions, DatabaseConnection};

use crate::error::AppError;

/// 创建数据库连接池，支持 MySQL 和 SQLite。
pub async fn create_pool(database_url: &str, max_conn: u32) -> Result<DatabaseConnection, AppError> {
    let mut opt = ConnectOptions::new(database_url.to_owned());
    opt.max_connections(max_conn.max(1))
        .connect_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(600))
        .acquire_timeout(Duration::from_secs(10));

    if database_url.starts_with("sqlite:") {
        opt.max_connections(1);
        opt.map_sqlx_sqlite_opts(|opts| {
            opts.create_if_missing(true)
                .journal_mode(sea_orm::sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(Duration::from_secs(30))
                .synchronous(sea_orm::sqlx::sqlite::SqliteSynchronous::Normal)
        });
    }

    sea_orm::Database::connect(opt)
        .await
        .map_err(|e| AppError::DatabaseError(format!("connect: {}", e)))
}
