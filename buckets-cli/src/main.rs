//! # buckets-cli
//!
//! buckets 私有 OSS 的命令行客户端。
//!
//! 支持交互模式和子命令：
//! - `login` / `logout` / `use` / `list`：凭据管理
//! - `upload`：带并行传输和断点续传的分块文件上传
//! - `status`：检查上传进度
//! - `resume`：从本地缓存恢复中断的上传

mod cli;
mod client;
mod config;
mod local;
mod progress;

use clap::Parser;
use cli::Cli;
use buckets_common::constant;
use std::io::Write;
use tracing_subscriber::EnvFilter;

/// 解析有效的服务器 URL：CLI --server > 已保存的默认值 > 内置默认值。
fn resolve_server(cli: &Cli) -> String {
    // 如果用户显式传递了 --server，则使用它
    if cli.server_url != constant::DEFAULT_SERVER_URL {
        return cli.server_url.clone();
    }
    // 否则使用已保存的默认值
    config::get_default_server().unwrap_or_else(|| constant::DEFAULT_SERVER_URL.to_string())
}

/// 从 CLI 参数或已保存的配置文件中解析凭据。
/// 返回 (server_url, email, password)。
fn resolve_credentials(cli: &Cli) -> anyhow::Result<(String, String, String)> {
    let server = resolve_server(cli);

    // 如果显式给出了 --email/--password，则使用它们
    if let (Some(email), Some(password)) = (&cli.email, &cli.password) {
        return Ok((server, email.clone(), password.clone()));
    }
    if cli.email.is_some() && cli.password.is_none() {
        anyhow::bail!("--password is required when --email is provided");
    }
    // 否则从已保存的凭据中加载
    if let Some(creds) = config::load_credentials(&server) {
        return Ok((server, creds.email, creds.password));
    }
    anyhow::bail!(
        "not logged in for {server}. run `buckets-cli --server {server} login <email>` first, or pass --email/--password"
    );
}

/// 从 stdin 读取一行，带提示信息。
fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or_default();
    input.trim().to_string()
}

/// 提取的通用交互式登录流程
async fn interactive_login(server_url: &str) -> anyhow::Result<client::Client> {
    let email = loop {
        let email = read_line("Email: ");
        if !email.is_empty() {
            break email;
        }
    };
    let password = loop {
        let password = rpassword::prompt_password("Password: ")?;
        if !password.is_empty() {
            break password;
        }
    };

    let mgr = client::Client::new(server_url, &email, &password);
    mgr.verify_credentials().await?;
    println!("Login successful");
    Ok(mgr)
}

/// 提取的通用交互式命令分发
async fn handle_interactive_command(mgr: &client::Client, action: &str) -> anyhow::Result<()> {
    match action.to_lowercase().as_str() {
        "upload" => {
            let file = read_line("File path: ");
            let bucket_url = mgr
                .upload_file(
                    &file,
                    None,
                    constant::DEFAULT_CHUNK_SIZE_MB,
                    constant::DEFAULT_PARALLEL_UPLOADS,
                    false,
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!("[trace_id: {}] upload failed: {e:#}", mgr.trace_id)
                })?;
            println!("Upload complete: {bucket_url}");
        }
        "status" => {
            let task_id = read_line("Task ID: ");
            mgr.print_status(&task_id).await?;
        }
        "resume" => {
            mgr.resume_upload().await?;
        }
        other => {
            anyhow::bail!("unknown command: {other}");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let result = match &cli.command {
        Some(command) => handle_subcommand(command, &cli).await,
        None => handle_interactive_mode(&cli).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

/// 分发解析后的子命令。
async fn handle_subcommand(command: &cli::Commands, cli: &Cli) -> anyhow::Result<()> {
    match command {
        cli::Commands::Login { email, password } => {
            let email = email.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
                loop {
                    let input = read_line("Email: ");
                    if !input.is_empty() {
                        break input;
                    }
                }
            });
            let password = password
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    loop {
                        let pw = rpassword::prompt_password("Password: ").unwrap_or_default();
                        if !pw.is_empty() {
                            break pw;
                        }
                    }
                });
            config::save_credentials(&cli.server_url, &email, &password)?;
            println!("Logged in as {email} ({})", cli.server_url);
        }
        cli::Commands::Logout => {
            let existed = config::remove_credentials(&cli.server_url)?;
            if existed {
                println!("Logged out from {}", cli.server_url);
            } else {
                println!("No saved credentials for {}", cli.server_url);
            }
        }
        cli::Commands::Use { server_url } => {
            config::set_default_server(server_url)?;
            println!("Default server switched to: {server_url}");
        }
        cli::Commands::List => {
            let servers = config::list_servers();
            if servers.is_empty() {
                println!("No saved servers.");
            } else {
                println!("Saved servers:");
                for (url, is_default) in &servers {
                    if *is_default {
                        println!("* {url} (default)");
                    } else {
                        println!("  {url}");
                    }
                }
            }
        }
        cli::Commands::Upload {
            file,
            name,
            chunk_size,
            parallel,
            resume,
        } => {
            let (server, email, password) = resolve_credentials(cli)?;
            let mgr = client::Client::new(&server, &email, &password);

            let bucket_url = mgr
                .upload_file(file, name.clone(), *chunk_size, *parallel, *resume)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("[trace_id: {}] upload failed: {e:#}", mgr.trace_id)
                })?;

            println!("Upload complete: {}", bucket_url);
        }
        cli::Commands::Status { task_id } => {
            let (server, email, password) = resolve_credentials(cli)?;
            let mgr = client::Client::new(&server, &email, &password);
            mgr.print_status(task_id).await?;
        }
        cli::Commands::Resume => {
            let (server, email, password) = resolve_credentials(cli)?;
            let mgr = client::Client::new(&server, &email, &password);
            mgr.resume_upload().await?;
        }
        cli::Commands::HashPassword { password } => {
            let password = password
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    loop {
                        let pw = rpassword::prompt_password("Password: ").unwrap_or_default();
                        if !pw.is_empty() {
                            break pw;
                        }
                        let confirm =
                            rpassword::prompt_password("Confirm password: ").unwrap_or_default();
                        if pw == confirm {
                            break pw;
                        }
                        eprintln!("Passwords do not match, try again.");
                    }
                });
            let hash = buckets_common::utils::password::hash_password(&password)
                .map_err(|e| anyhow::anyhow!("hash password failed: {e}"))?;
            println!("{hash}");
        }
    }
    Ok(())
}

/// 交互模式：提示输入服务器 URL、登录，然后选择操作。
async fn handle_interactive_mode(cli: &Cli) -> anyhow::Result<()> {
    let default_server = resolve_server(cli);
    let server = read_line(&format!("Server URL [{default_server}]: "));
    let server = if server.is_empty() {
        &default_server
    } else {
        &server
    };

    let mgr = interactive_login(server).await?;

    println!("\nAvailable: upload, status, resume");
    let action = read_line("Command: ");
    handle_interactive_command(&mgr, &action).await
}
