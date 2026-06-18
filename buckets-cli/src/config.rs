//! CLI 客户端的凭据存储。
//!
//! 将每台服务器的凭据存储在 `~/.buckets/credentials.json` 中，
//! 具有严格的文件权限（Unix 上为 0600）。
//!
//! 注意：凭据以 `base64(email:password)` 格式存储，类似于 Docker 的
//! `config.json`。对于安全要求更高的生产环境，
//! 请考虑使用系统密钥环（例如 `keyring` crate）代替。
//!

use base64::Engine;
use buckets_common::constant;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 顶层凭据文件结构。
#[derive(Serialize, Deserialize, Default)]
struct CredentialFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(default)]
    auths: BTreeMap<String, String>,
}

/// 解码后的特定服务器凭据。
pub struct Credentials {
    pub email: String,
    pub password: String,
}

/// 返回配置目录：`~/.buckets`。
fn config_dir() -> PathBuf {
    let home = std::env::var(constant::ENV_HOME).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(constant::CLI_CONFIG_DIR)
}

/// 返回凭据 JSON 文件的路径。
fn credentials_path() -> PathBuf {
    config_dir().join(constant::CLI_CREDENTIALS_FILE)
}

/// 从磁盘加载凭据文件。
fn load_file() -> Option<CredentialFile> {
    let path = credentials_path();
    if !path.exists() {
        return None;
    }
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// 将凭据文件保存到磁盘，具有严格的权限（Unix 上为 0600）。
fn save_file(cf: &CredentialFile) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let data = serde_json::to_string_pretty(cf)?;
    let path = credentials_path();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&path, &data)?;
        let perms = std::fs::Permissions::from_mode(constant::CREDENTIALS_FILE_MODE);
        std::fs::set_permissions(&path, perms)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, &data)?;
    }
    Ok(())
}

/// 将 base64 编码的 "email:password" 字符串解码为 (email, password)。
fn decode_auth(encoded: &str) -> Option<Credentials> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let s = String::from_utf8(bytes).ok()?;
    let (email, password) = s.split_once(constant::CREDENTIAL_SEPARATOR)?;
    Some(Credentials {
        email: email.to_string(),
        password: password.to_string(),
    })
}

/// 将 "email:password" 编码为 base64。
fn encode_auth(email: &str, password: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!(
        "{}{}{}",
        email,
        constant::CREDENTIAL_SEPARATOR,
        password
    ))
}

// ---- 公共 API ----

/// 获取当前默认的服务器 URL。如果未设置则返回 `None`。
pub fn get_default_server() -> Option<String> {
    load_file()?.default
}

/// 加载指定服务器 URL 的已保存凭据。
/// 如果此服务器没有凭据则返回 `None`。
pub fn load_credentials(server_url: &str) -> Option<Credentials> {
    let cf = load_file()?;
    let encoded = cf.auths.get(server_url)?;
    decode_auth(encoded)
}

/// 保存指定服务器 URL 的凭据。
/// 与现有条目合并；如果是第一个则将此服务器设为默认。
pub fn save_credentials(server_url: &str, email: &str, password: &str) -> anyhow::Result<()> {
    let mut cf = load_file().unwrap_or_default();
    cf.auths
        .insert(server_url.to_string(), encode_auth(email, password));
    // 如果还没有默认值，则设为默认
    if cf.default.is_none() {
        cf.default = Some(server_url.to_string());
    }
    save_file(&cf)
}

/// 移除指定服务器的已保存凭据。
/// 如果被移除的服务器是默认服务器，则从剩余条目中自动选择新的默认值。
/// 如果条目存在且已移除则返回 true。
pub fn remove_credentials(server_url: &str) -> anyhow::Result<bool> {
    let mut cf = match load_file() {
        Some(c) => c,
        None => return Ok(false),
    };
    let existed = cf.auths.remove(server_url).is_some();
    if !existed {
        return Ok(false);
    }
    // 如果移除了默认值，选择第一个剩余条目作为新默认值
    if cf.default.as_deref() == Some(server_url) {
        cf.default = cf.auths.keys().next().cloned();
    }
    save_file(&cf)?;
    Ok(true)
}

/// 将默认服务器设置为给定的 URL。
/// 如果此服务器没有凭据则返回错误。
pub fn set_default_server(server_url: &str) -> anyhow::Result<()> {
    let mut cf = load_file().unwrap_or_default();
    if !cf.auths.contains_key(server_url) {
        anyhow::bail!(
            "no credentials found for {server_url}. run `buckets-cli --server {server_url} login` first."
        );
    }
    cf.default = Some(server_url.to_string());
    save_file(&cf)?;
    Ok(())
}

/// 列出所有已保存的服务器，并指示默认服务器。
pub fn list_servers() -> Vec<(String, bool)> {
    let cf = match load_file() {
        Some(c) => c,
        None => return vec![],
    };
    cf.auths
        .keys()
        .map(|url| {
            let is_default = cf.default.as_deref() == Some(url.as_str());
            (url.clone(), is_default)
        })
        .collect()
}
