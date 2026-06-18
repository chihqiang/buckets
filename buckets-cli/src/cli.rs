//! 使用 clap 的 CLI 参数定义。

use clap::{Parser, Subcommand};
use buckets_common::constant;

/// 顶层 CLI 结构体，包含全局选项和可选的子命令。
#[derive(Parser)]
#[command(name = "buckets-cli")]
#[command(about = "buckets CLI - private OSS file upload client", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(
        global = true,
        short = 's',
        long = "server",
        default_value = constant::DEFAULT_SERVER_URL
    )]
    pub server_url: String,

    #[arg(global = true, long = "email")]
    pub email: Option<String>,

    #[arg(global = true, long = "password")]
    pub password: Option<String>,
}

/// 可用的子命令。
#[derive(Subcommand)]
pub enum Commands {
    /// 保存凭据供后续命令使用
    Login {
        email: Option<String>,
        #[arg(long = "password")]
        password: Option<String>,
    },
    /// 移除当前服务器的已保存凭据
    Logout,
    /// 切换默认服务器（需要先登录）
    Use { server_url: String },
    /// 列出所有已保存的服务器
    List,
    /// 上传文件
    Upload {
        file: String,
        #[arg(short = 'n', long = "name")]
        name: Option<String>,
        #[arg(
            short = 'c',
            long = "chunk-size",
            default_value_t = constant::DEFAULT_CHUNK_SIZE_MB
        )]
        chunk_size: u64,
        #[arg(
            short = 'p',
            long = "parallel",
            default_value_t = constant::DEFAULT_PARALLEL_UPLOADS
        )]
        parallel: usize,
        #[arg(short = 'r', long = "resume")]
        resume: bool,
    },
    /// 检查上传状态
    Status { task_id: String },
    /// 恢复中断的上传
    Resume,
    /// 生成 argon2 密码哈希
    HashPassword {
        /// 要哈希的密码（省略则以交互方式提示）
        password: Option<String>,
    },
}
