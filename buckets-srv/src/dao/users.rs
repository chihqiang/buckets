use chrono::{DateTime, Utc};
use buckets_common::error::AppError;
use buckets_common::model::db::{users, user_objects, upload_tasks};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, Order, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};

/// 用户列表查询返回的行（不含 secret_key）。
#[derive(Debug)]
pub struct UserRow {
    pub id: i64,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 分页列出用户。
pub async fn list_users(
    db: &DatabaseConnection,
    page: u64,
    page_size: u64,
) -> Result<(Vec<UserRow>, i64), AppError> {
    let offset = (page - 1) * page_size;

    let total = users::Entity::find().count(db).await?;

    let users = users::Entity::find()
        .order_by(users::Column::Id, Order::Asc)
        .offset(offset)
        .limit(page_size)
        .all(db)
        .await?;

    let rows = users
        .into_iter()
        .map(|u| UserRow {
            id: u.id,
            email: u.email,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .collect();

    Ok((rows, total as i64))
}

/// 通过 ID 获取单个用户。
pub async fn get_user(
    db: &DatabaseConnection,
    user_id: i64,
) -> Result<Option<UserRow>, AppError> {
    let user = users::Entity::find_by_id(user_id).one(db).await?;
    Ok(user.map(|u| UserRow {
        id: u.id,
        email: u.email,
        created_at: u.created_at,
        updated_at: u.updated_at,
    }))
}

/// 创建新用户。返回新用户的 ID。
pub async fn create_user(
    db: &DatabaseConnection,
    email: &str,
    password_hash: &str,
    secret_key: &str,
) -> Result<i64, AppError> {
    let now = Utc::now();
    let result = users::Entity::insert(users::ActiveModel {
        email: Set(email.to_string()),
        password: Set(password_hash.to_string()),
        secret_key: Set(Some(secret_key.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .exec(db)
    .await?;

    Ok(result.last_insert_id)
}

/// 更新用户邮箱和/或密码。
pub async fn update_user(
    db: &DatabaseConnection,
    user_id: i64,
    email: Option<&str>,
    password_hash: Option<&str>,
) -> Result<bool, AppError> {
    if email.is_none() && password_hash.is_none() {
        return Ok(true);
    }

    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;

    let mut active: users::ActiveModel = user.into();
    let now = Utc::now();
    if let Some(e) = email {
        active.email = Set(e.to_string());
    }
    if let Some(p) = password_hash {
        active.password = Set(p.to_string());
    }
    active.updated_at = Set(now);

    active.update(db).await?;
    Ok(true)
}

/// 通过 ID 删除用户，级联删除关联的 user_objects 和 upload_tasks。
pub async fn delete_user(
    db: &DatabaseConnection,
    user_id: i64,
) -> Result<bool, AppError> {
    let txn = db.begin().await?;

    user_objects::Entity::delete_many()
        .filter(user_objects::Column::UserId.eq(user_id))
        .exec(&txn)
        .await?;

    upload_tasks::Entity::delete_many()
        .filter(upload_tasks::Column::UserId.eq(user_id))
        .exec(&txn)
        .await?;

    let result = users::Entity::delete_by_id(user_id).exec(&txn).await?;

    txn.commit().await?;
    Ok(result.rows_affected > 0)
}

/// 将用户的 secret_key 重置为新的随机值。
pub async fn reset_user_secret_key(
    db: &DatabaseConnection,
    user_id: i64,
    new_secret_key: &str,
) -> Result<bool, AppError> {
    let now = Utc::now();
    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))?;

    let mut active: users::ActiveModel = user.into();
    active.secret_key = Set(Some(new_secret_key.to_string()));
    active.updated_at = Set(now);

    active.update(db).await?;
    Ok(true)
}

/// 生成随机的 32 字节十六进制密钥。
pub fn generate_secret_key() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}
