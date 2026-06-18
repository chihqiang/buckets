//! 引用计数检查后台任务。
//!
//! 定期扫描状态为 deleted 且在 `user_objects` 表中没有所有者的对象，
//! 然后移除其存储文件和数据库记录。

use buckets_common::constant;
use buckets_common::error::AppError;
use buckets_common::model::db::{objects, user_objects};
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// 启动引用计数检查后台任务。定期运行。
pub async fn start_ref_checker(db: DatabaseConnection, cancellation: CancellationToken) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(constant::REF_CHECK_INTERVAL_SECS));
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    tracing::info!("Ref checker shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = cleanup_orphan_objects(&db).await {
                        tracing::error!("Ref check error: {}", e);
                    }
                }
            }
        }
    });
}

/// 删除状态为 deleted 且没有所有者的对象。
async fn cleanup_orphan_objects(db: &DatabaseConnection) -> Result<(), AppError> {
    // 查找没有 user_objects 条目的已删除对象（通过 NOT IN 子查询的反连接）
    let mut sq = Query::select();
    sq.column(user_objects::Column::ObjectId).from(user_objects::Entity);
    let subquery = sq.clone();

    let orphans = objects::Entity::find()
        .filter(objects::Column::Status.eq("deleted"))
        .filter(objects::Column::Id.not_in_subquery(subquery))
        .all(db)
        .await?;

    let mut cleaned = 0u64;
    for obj in &orphans {
        tracing::info!("GC: cleaning orphan object {}", obj.uuid);
        let _ = tokio::fs::remove_file(&obj.storage_path).await;
        let _ = objects::Entity::delete_by_id(obj.id).exec(db).await;
        cleaned += 1;
    }

    if cleaned > 0 {
        tracing::info!("Ref check: cleaned {} orphan objects", cleaned);
    }

    Ok(())
}
