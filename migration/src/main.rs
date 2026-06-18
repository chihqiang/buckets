use migration::Migrator;
use sea_orm_migration::prelude::*;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| buckets_common::constant::db::DEFAULT_DATABASE_URL.to_owned());
    let conn = sea_orm::Database::connect(&db_url)
        .await
        .expect("Failed to connect to database");
    Migrator::up(&conn, None).await.expect("Migration failed");
    println!("Migration completed successfully");
}
