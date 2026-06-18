//! 按功能域组织的配置常量。
//!
//! | 模块 | 域 |
//! |--------|--------|
//! | [`auth`] | 认证、令牌、密钥、凭据 |
//! | [`upload`] | 分块大小、上传状态、位图、合并、会话、文件大小 |
//! | [`http`] | HTTP 头部、MIME、API 路径、超时、限流、服务器绑定、CORS |
//! | [`storage`] | 存储/暂存/缓存目录、存储桶默认值 |
//! | [`db`] | 数据库连接池配置 |
//! | [`task`] | 后台任务间隔和 GC 批处理参数 |
//! | [`cli`] | CLI HTTP 客户端、重试、合并轮询、配置路径 |

pub mod auth;
pub mod cli;
pub mod db;
pub mod http;
pub mod storage;
pub mod task;
pub mod upload;

// 重新导出所有常量，以保持现有的 `use buckets_common::constant::*` 正常工作。
pub use auth::*;
pub use cli::*;
pub use db::*;
pub use http::*;
pub use storage::*;
pub use task::*;
pub use upload::*;
