//! 共享数据库工具——中间件、主程序和 Web/网关两层使用的连接创建和认证辅助函数。

use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::db::users;
use buckets_common::utils::password;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

// ============================================================================
// 连接创建
// ============================================================================

/// 通过 sea-orm 创建 MySQL 数据库连接池。
pub async fn create_pool(
    database_url: &str,
    _max_conn: u32,
) -> Result<DatabaseConnection, AppError> {
    sea_orm::Database::connect(database_url)
        .await
        .map_err(|e| AppError::DatabaseError(format!("connect: {}", e)))
}

// ============================================================================
// 共享类型
// ============================================================================

/// 从认证中间件提取的用户 ID。
#[derive(Debug, Clone, Copy)]
pub struct UserId(pub u64);

// ============================================================================
// 认证辅助函数
// ============================================================================

/// 包含 id 和 secret_key 的用户记录，用于认证。
pub struct UserWithKey {
    pub id: u64,
    pub secret_key: String,
}

/// 根据数据库验证用户凭据。成功时返回 `UserWithKey`。
pub async fn verify_user(
    db: &DatabaseConnection,
    email: &str,
    password: &str,
) -> Result<Option<UserWithKey>, AppError> {
    let user = users::Entity::find()
        .filter(users::Column::Email.eq(email))
        .one(db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "verify_user query error");
            AppError::DatabaseError(e.to_string())
        })?;

    match user {
        Some(u) => {
            if password::verify_password(password, &u.password)? {
                Ok(Some(UserWithKey {
                    id: u.id,
                    secret_key: u.secret_key.unwrap_or_default(),
                }))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// 获取用户的密钥以进行 HMAC 签名。
/// 如果存储的值为空，回退到 `DEFAULT_SECRET_KEY`。
pub async fn get_user_secret_key(
    db: &DatabaseConnection,
    user_id: u64,
) -> Result<String, AppError> {
    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;

    let sk = user.secret_key.unwrap_or_default();
    if sk.is_empty() {
        Ok(constant::DEFAULT_SECRET_KEY.to_string())
    } else {
        Ok(sk)
    }
}
