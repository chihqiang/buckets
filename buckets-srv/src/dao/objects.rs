use chrono::{DateTime, Utc};
use buckets_common::error::AppError;
use buckets_common::model::db::{ObjectMeta, objects, user_objects};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Order,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

/// 通过 MD5 + file_size 查找对象，用于全局去重（不限特定用户）。
pub async fn find_object_by_md5(
    db: &DatabaseConnection,
    md5: &str,
    bucket: &str,
    file_size: i64,
) -> Result<Option<ObjectMeta>, AppError> {
    objects::Entity::find()
        .filter(objects::Column::Md5.eq(md5))
        .filter(objects::Column::Size.eq(file_size))
        .filter(objects::Column::Bucket.eq(bucket))
        .filter(objects::Column::Status.eq("active"))
        .one(db)
        .await
        .map_err(Into::into)
}

/// 通过 UUID 查找对象。
pub async fn find_object_by_uuid(
    db: &DatabaseConnection,
    uuid: &str,
) -> Result<Option<ObjectMeta>, AppError> {
    objects::Entity::find()
        .filter(objects::Column::Uuid.eq(uuid))
        .one(db)
        .await
        .map_err(Into::into)
}

/// 将对象标记为已删除（软删除——将状态设置为'deleted'）。
pub async fn soft_delete_object(
    db: &DatabaseConnection,
    uuid: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    objects::Entity::update_many()
        .filter(objects::Column::Uuid.eq(uuid))
        .set(objects::ActiveModel {
            status: Set("deleted".to_string()),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await?;
    Ok(())
}

/// 通过对象的 UUID 检查用户是否与该对象关联（所有者检查）。
pub async fn check_user_owns_object_by_uuid(
    db: &DatabaseConnection,
    user_id: i64,
    uuid: &str,
) -> Result<bool, AppError> {
    let obj = objects::Entity::find()
        .filter(objects::Column::Uuid.eq(uuid))
        .one(db)
        .await?;
    match obj {
        Some(o) => {
            let count = user_objects::Entity::find()
                .filter(user_objects::Column::UserId.eq(user_id))
                .filter(user_objects::Column::ObjectId.eq(o.id))
                .count(db)
                .await?;
            Ok(count > 0)
        }
        None => Ok(false),
    }
}

/// 通过对象的内部 ID 插入用户-对象关联（一个文件可以属于多个用户）。
/// 如果关联已存在，返回 Ok(()) 不报错。
pub async fn insert_user_object<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    object_id: i64,
) -> Result<(), AppError> {
    let exists = user_objects::Entity::find()
        .filter(user_objects::Column::UserId.eq(user_id))
        .filter(user_objects::Column::ObjectId.eq(object_id))
        .one(db)
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    let now = Utc::now();
    user_objects::ActiveModel {
        user_id: Set(user_id),
        object_id: Set(object_id),
        created_at: Set(now),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// 通过对象的 UUID 移除用户-对象关联。如果是最后一个所有者则返回 true。
pub async fn remove_user_object_by_uuid(
    db: &DatabaseConnection,
    user_id: i64,
    uuid: &str,
) -> Result<bool, AppError> {
    let obj = objects::Entity::find()
        .filter(objects::Column::Uuid.eq(uuid))
        .one(db)
        .await?;
    match obj {
        Some(o) => {
            user_objects::Entity::delete_many()
                .filter(user_objects::Column::UserId.eq(user_id))
                .filter(user_objects::Column::ObjectId.eq(o.id))
                .exec(db)
                .await?;
            let count = user_objects::Entity::find()
                .filter(user_objects::Column::ObjectId.eq(o.id))
                .count(db)
                .await?;
            Ok(count == 0)
        }
        None => Ok(false),
    }
}

/// 通过对象的自增 ID 移除用户-对象关联。返回是否实际删除了记录。
pub async fn remove_user_object_by_id(
    db: &DatabaseConnection,
    user_id: i64,
    object_id: i64,
) -> Result<bool, AppError> {
    let result = user_objects::Entity::delete_many()
        .filter(user_objects::Column::UserId.eq(user_id))
        .filter(user_objects::Column::ObjectId.eq(object_id))
        .exec(db)
        .await?;
    Ok(result.rows_affected > 0)
}
use sea_orm::sea_query::Query;

/// 文件列表查询返回的行。
#[derive(Debug)]
pub struct ObjectRow {
    pub id: i64,
    pub uuid: String,
    pub name: String,
    pub size: i64,
    pub md5: String,
    pub content_type: Option<String>,
    pub extension: Option<String>,
    pub bucket: String,
    pub storage_path: Option<String>,
    pub upload_method: Option<String>,
    pub image_width: i64,
    pub image_height: i64,
    pub image_type: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<ObjectMeta> for ObjectRow {
    fn from(o: ObjectMeta) -> Self {
        Self {
            id: o.id,
            uuid: o.uuid,
            name: o.name,
            size: o.size,
            md5: o.md5,
            content_type: o.content_type,
            extension: o.extension,
            bucket: o.bucket,
            storage_path: Some(o.storage_path),
            upload_method: Some(o.upload_method),
            image_width: o.image_width,
            image_height: o.image_height,
            image_type: o.image_type,
            status: o.status,
            created_at: o.created_at,
            updated_at: o.updated_at,
        }
    }
}

/// 列出特定用户拥有的文件（分页）。
pub async fn list_objects_by_user(
    db: &DatabaseConnection,
    user_id: i64,
    page: u64,
    page_size: u64,
) -> Result<(Vec<ObjectRow>, i64), AppError> {
    let offset = (page - 1) * page_size;

    let mut subq = Query::select();
    subq.column(user_objects::Column::ObjectId)
        .from(user_objects::Entity)
        .and_where(user_objects::Column::UserId.eq(user_id));
    let subquery = subq.clone();

    let total = objects::Entity::find()
        .filter(objects::Column::Id.in_subquery(subquery.clone()))
        .count(db)
        .await? as i64;

    let rows = objects::Entity::find()
        .filter(objects::Column::Id.in_subquery(subquery))
        .order_by(objects::Column::CreatedAt, Order::Desc)
        .offset(offset)
        .limit(page_size)
        .all(db)
        .await?;

    let objects: Vec<ObjectRow> = rows.into_iter().map(|o| o.into()).collect();

    Ok((objects, total))
}

/// 列出所有用户的所有对象（仅超级管理员，分页）。
pub async fn list_all_objects(
    db: &DatabaseConnection,
    page: u64,
    page_size: u64,
) -> Result<(Vec<ObjectRow>, i64), AppError> {
    let offset = (page - 1) * page_size;

    let mut subq = Query::select();
    subq.column(user_objects::Column::ObjectId)
        .from(user_objects::Entity);
    let subquery = subq.clone();

    let total = objects::Entity::find()
        .filter(objects::Column::Id.in_subquery(subquery))
        .count(db)
        .await? as i64;

    let rows = objects::Entity::find()
        .filter(objects::Column::Id.in_subquery(subq.clone()))
        .order_by(objects::Column::CreatedAt, Order::Desc)
        .offset(offset)
        .limit(page_size)
        .all(db)
        .await?;

    let objects: Vec<ObjectRow> = rows.into_iter().map(|o| o.into()).collect();

    Ok((objects, total))
}
