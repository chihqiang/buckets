use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 用户表
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Users::Id)
                            .big_unsigned()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Users::Email).string_len(256).not_null())
                    .col(ColumnDef::new(Users::Password).string_len(256).not_null())
                    .col(ColumnDef::new(Users::SecretKey).string_len(128).null())
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        // 邮箱唯一索引
        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_users_email")
                    .table(Users::Table)
                    .col(Users::Email)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // 对象表
        manager
            .create_table(
                Table::create()
                    .table(Objects::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Objects::Id)
                            .big_unsigned()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Objects::Uuid).string_len(36).not_null())
                    .col(ColumnDef::new(Objects::Name).string_len(1024).not_null())
                    .col(ColumnDef::new(Objects::Size).big_integer().not_null())
                    .col(ColumnDef::new(Objects::Md5).string_len(64).not_null())
                    .col(ColumnDef::new(Objects::ContentType).string_len(256).null())
                    .col(ColumnDef::new(Objects::Extension).string_len(64).null())
                    .col(
                        ColumnDef::new(Objects::Bucket)
                            .string_len(256)
                            .not_null()
                            .default("default"),
                    )
                    .col(ColumnDef::new(Objects::StoragePath).text().not_null())
                    .col(
                        ColumnDef::new(Objects::ImageWidth)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Objects::ImageHeight)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Objects::ImageType)
                            .string_len(32)
                            .not_null()
                            .default(""),
                    )
                    .col(
                        ColumnDef::new(Objects::Status)
                            .string_len(32)
                            .not_null()
                            .default("active"),
                    )
                    .col(
                        ColumnDef::new(Objects::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Objects::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_objects_uuid")
                    .table(Objects::Table)
                    .col(Objects::Uuid)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_objects_md5")
                    .table(Objects::Table)
                    .col(Objects::Md5)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_objects_bucket")
                    .table(Objects::Table)
                    .col(Objects::Bucket)
                    .to_owned(),
            )
            .await?;

        // 用户-对象关联表
        manager
            .create_table(
                Table::create()
                    .table(UserObjects::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserObjects::UserId)
                            .big_unsigned()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserObjects::ObjectId)
                            .big_unsigned()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserObjects::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .primary_key(
                        IndexCreateStatement::new()
                            .col(UserObjects::UserId)
                            .col(UserObjects::ObjectId),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_user_objects_object")
                    .table(UserObjects::Table)
                    .col(UserObjects::ObjectId)
                    .to_owned(),
            )
            .await?;

        // 上传任务表
        manager
            .create_table(
                Table::create()
                    .table(UploadTasks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UploadTasks::Id)
                            .big_unsigned()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UploadTasks::Uuid).string_len(36).not_null())
                    .col(
                        ColumnDef::new(UploadTasks::ObjectId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::FileMd5)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::FileSize)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::ChunkSize)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::ChunkCount)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::UserId)
                            .big_unsigned()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::Status)
                            .string_len(32)
                            .not_null()
                            .default("initialized"),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::UploadedBitmap)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::LastActivityAt)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(UploadTasks::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_upload_tasks_uuid")
                    .table(UploadTasks::Table)
                    .col(UploadTasks::Uuid)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_upload_tasks_md5")
                    .table(UploadTasks::Table)
                    .col(UploadTasks::FileMd5)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_upload_tasks_user")
                    .table(UploadTasks::Table)
                    .col(UploadTasks::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_upload_tasks_expires")
                    .table(UploadTasks::Table)
                    .col(UploadTasks::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                IndexCreateStatement::new()
                    .name("idx_upload_tasks_status_expires")
                    .table(UploadTasks::Table)
                    .col(UploadTasks::Status)
                    .col(UploadTasks::ExpiresAt)
                    .to_owned(),
            )
            .await?;

        // 插入默认管理员用户
        // 密码从 ADMIN_PASSWORD 环境变量读取，默认回退为 "buckets"
        let admin_password =
            std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "buckets".into());
        let password_hash = buckets_common::utils::password::hash_password(&admin_password)
            .map_err(|e| DbErr::Custom(e.to_string()))?;

        let insert = Query::insert()
            .into_table(Users::Table)
            .columns([
                Users::Id,
                Users::Email,
                Users::Password,
                Users::SecretKey,
                Users::CreatedAt,
                Users::UpdatedAt,
            ])
            .values_panic([
                1u64.into(),
                "admin@buckets.local".into(),
                password_hash.into(),
                "d6b1f4e8a2c9f3e7b5d0a8c1e4f7b2a6c8d0e2f4a6b8c0d2e4f6a8b0c2d4e6f".into(),
                Expr::current_timestamp().into(),
                Expr::current_timestamp().into(),
            ])
            .to_owned();
        manager.exec_stmt(insert).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UploadTasks::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(UserObjects::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Objects::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Users::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    Password,
    SecretKey,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Objects {
    Table,
    Id,
    Uuid,
    Name,
    Size,
    Md5,
    ContentType,
    Extension,
    Bucket,
    StoragePath,
    ImageWidth,
    ImageHeight,
    ImageType,
    Status,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum UserObjects {
    Table,
    UserId,
    ObjectId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum UploadTasks {
    Table,
    Id,
    Uuid,
    ObjectId,
    FileMd5,
    FileSize,
    ChunkSize,
    ChunkCount,
    UserId,
    Status,
    UploadedBitmap,
    LastActivityAt,
    CreatedAt,
    UpdatedAt,
    ExpiresAt,
}
