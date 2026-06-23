//! # buckets-common
//!
//! buckets 私有 OSS（对象存储服务）的共享库。
//!
//! 提供：
//! - **error**: 统一的 `AppError` 类型，包含 HTTP 状态码映射
//! - **model**: 数据库实体（`User`、`ObjectMeta`、`UploadTask`）和 API DTO
//! - **utils**: 加密签名、哈希、路径辅助、文件验证、密码哈希

pub mod constant;
pub mod db;
pub mod error;
pub mod model;
pub mod utils;

#[cfg(test)]
mod tests;
