//! 统一 DAO 层——用户 CRUD、对象查询、上传任务管理、
//! 文件查询和所有权检查。

pub mod objects;
pub mod tasks;
pub mod users;

pub use objects::*;
pub use tasks::*;
pub use users::*;

// 从 db.rs 重新导出共享的认证辅助函数
pub use crate::db::{get_user_secret_key, verify_user};
