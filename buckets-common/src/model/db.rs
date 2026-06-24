//! 数据库实体模型——通过 sea-orm 直接映射到 MySQL 表行。
//!
//! 每个表都有自己的实体模块和 `DeriveEntityModel`。`Model`
//! 结构体被重新导出为 `User`、`ObjectMeta`、`UploadTask` 以向后兼容现有服务代码。
//!
//! 状态字段在 MySQL/MariaDB 中存储为 VARCHAR。我们使用 `String` 作为
//! 数据库列类型，并提供枚举转换辅助函数以保证类型安全。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

// ============================================================================
// 用户实体
// ============================================================================

pub mod users {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub email: String,
        #[serde(skip_serializing)]
        pub password: String,
        pub secret_key: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ============================================================================
// 对象实体
// ============================================================================

pub mod objects {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "objects")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub uuid: String,
        pub name: String,
        pub size: i64,
        pub md5: String,
        pub content_type: Option<String>,
        pub extension: Option<String>,
        pub bucket: String,
        pub storage_path: String,
        #[sea_orm(default_value = "chunked")]
        pub upload_method: String,
        pub image_width: i64,
        pub image_height: i64,
        pub image_type: String,
        #[serde(skip)]
        pub status: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ============================================================================
// 用户-对象关联实体（复合主键）
// ============================================================================

pub mod user_objects {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "user_objects")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub user_id: i64,
        #[sea_orm(primary_key)]
        pub object_id: i64,
        pub created_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ============================================================================
// 上传任务实体
// ============================================================================

pub mod upload_tasks {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "upload_tasks")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub uuid: String,
        pub object_id: String,
        pub file_md5: String,
        pub file_size: i64,
        pub chunk_size: i64,
        pub chunk_count: i64,
        pub user_id: i64,
        #[serde(skip)]
        pub status: String,
        pub uploaded_bitmap: String,
        #[sea_orm(default_value = "chunked")]
        pub upload_method: String,
        #[sea_orm(default_value = 0)]
        pub current_offset: i64,
        #[sea_orm(default_value = false)]
        pub is_deferred: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
        pub expires_at: DateTimeUtc,
        pub last_activity_at: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ============================================================================
// 向后兼容的重新导出
// ============================================================================

pub use objects::Model as ObjectMeta;
pub use upload_tasks::Model as UploadTask;
pub use users::Model as User;

// ============================================================================
// 状态枚举辅助函数（保持不变）
// ============================================================================

/// 类型安全的上传任务状态枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Initialized,
    Uploading,
    Merging,
    Completed,
    Failed,
    Expired,
}

impl TaskStatus {
    /// 返回 snake_case 字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Initialized => "initialized",
            TaskStatus::Uploading => "uploading",
            TaskStatus::Merging => "merging",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Expired => "expired",
        }
    }

    /// 如果此状态表示终止状态则返回 true。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Expired
        )
    }

    /// 从数据库字符串值解析。
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "initialized" => TaskStatus::Initialized,
            "uploading" => TaskStatus::Uploading,
            "merging" => TaskStatus::Merging,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "expired" => TaskStatus::Expired,
            _ => TaskStatus::Initialized,
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| e.to_string())
    }
}

/// 围绕 TaskStatus 的新类型包装，与 sea-orm VARCHAR 列兼容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub struct TaskStatusColumn(pub TaskStatus);

impl From<TaskStatus> for TaskStatusColumn {
    fn from(v: TaskStatus) -> Self {
        TaskStatusColumn(v)
    }
}

impl From<TaskStatusColumn> for String {
    fn from(v: TaskStatusColumn) -> Self {
        v.0.as_str().to_string()
    }
}

impl From<String> for TaskStatusColumn {
    fn from(s: String) -> Self {
        TaskStatusColumn(TaskStatus::from_db_str(&s))
    }
}

impl std::fmt::Display for TaskStatusColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// 类型安全的对象生命周期状态枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStatus {
    Active,
    Deleted,
    Archived,
}

impl ObjectStatus {
    /// 返回 snake_case 字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectStatus::Active => "active",
            ObjectStatus::Deleted => "deleted",
            ObjectStatus::Archived => "archived",
        }
    }

    /// 从数据库字符串值解析。
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "active" => ObjectStatus::Active,
            "deleted" => ObjectStatus::Deleted,
            "archived" => ObjectStatus::Archived,
            _ => ObjectStatus::Active,
        }
    }
}

impl std::fmt::Display for ObjectStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ObjectStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| e.to_string())
    }
}

/// 围绕 ObjectStatus 的新类型包装，与 sea-orm VARCHAR 列兼容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub struct ObjectStatusColumn(pub ObjectStatus);

impl From<ObjectStatus> for ObjectStatusColumn {
    fn from(v: ObjectStatus) -> Self {
        ObjectStatusColumn(v)
    }
}

impl From<ObjectStatusColumn> for String {
    fn from(v: ObjectStatusColumn) -> Self {
        v.0.as_str().to_string()
    }
}

impl From<String> for ObjectStatusColumn {
    fn from(s: String) -> Self {
        ObjectStatusColumn(ObjectStatus::from_db_str(&s))
    }
}

impl std::fmt::Display for ObjectStatusColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ============================================================================
// UploadTask 辅助函数（保持不变）
// ============================================================================

impl UploadTask {
    /// 将状态作为 TaskStatus 枚举获取。
    pub fn status_enum(&self) -> TaskStatus {
        TaskStatus::from_db_str(&self.status)
    }

    /// 将已上传分块的 JSON 位图解析为 `Vec<u64>`。
    /// 每个位表示一个分块是否已上传。
    /// 如果位图为空或无效，返回零填充的向量。
    pub fn parse_bitmap(&self) -> Vec<u64> {
        if self.uploaded_bitmap.is_empty() || self.uploaded_bitmap == "[]" {
            let word_count =
                (self.chunk_count as usize).div_ceil(crate::constant::BITMAP_BITS_PER_WORD);
            return vec![0u64; word_count];
        }
        serde_json::from_str(&self.uploaded_bitmap).unwrap_or_else(|_| {
            let word_count =
                (self.chunk_count as usize).div_ceil(crate::constant::BITMAP_BITS_PER_WORD);
            vec![0u64; word_count]
        })
    }
}

// ============================================================================
// ObjectMeta 辅助函数（保持不变）
// ============================================================================

impl ObjectMeta {
    /// 将状态作为 ObjectStatus 枚举获取。
    pub fn status_enum(&self) -> ObjectStatus {
        ObjectStatus::from_db_str(&self.status)
    }
}
