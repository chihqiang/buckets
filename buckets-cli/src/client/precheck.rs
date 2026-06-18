//! 预检和 STS API 客户端函数。

use anyhow::Result;
use buckets_common::constant;

/// STS 返回的会话签名信息（整个上传使用一个签名）。
pub struct SessionInfo {
    pub task_id: String,
    pub session_signature: String,
    pub session_timestamp: i64,
    pub session_salt: String,
}

/// 为新上传请求 STS 令牌（会话级别签名）。
pub async fn get_sts_token(
    client: &reqwest::Client,
    server_url: &str,
    file_name: &str,
    file_size: u64,
    file_md5: &str,
    chunk_size: u64,
) -> Result<SessionInfo> {
    let url = format!("{}{}/upload/sts", server_url, constant::API_BASE_PATH);
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "file_name": file_name,
            "file_size": file_size,
            "file_md5": file_md5,
            "chunk_size": chunk_size,
        }))
        .send()
        .await
        .map_err(|e| super::map_network_error(e, server_url))?;

    let status = resp.status();
    let resp_text = resp.text().await?;
    tracing::debug!(url = %url, status = %status, body_len = resp_text.len(), "STS response");
    let body: serde_json::Value = serde_json::from_str(&resp_text).map_err(|_| {
        anyhow::anyhow!(
            "STS response parse error (status={}, body={})",
            status,
            &resp_text[..resp_text.len().min(200)]
        )
    })?;
    let code = body["code"].as_u64().unwrap_or(0);
    if code != 200 {
        anyhow::bail!(
            "STS failed: {}",
            body["message"].as_str().unwrap_or("unknown error")
        );
    }
    let data = &body["data"];
    let task_id = data["task_id"].as_str().unwrap_or("").to_string();
    let session_signature = data["session_signature"].as_str().unwrap_or("").to_string();
    let session_timestamp = data["session_timestamp"].as_i64().unwrap_or(0);
    let session_salt = data["session_salt"].as_str().unwrap_or("").to_string();
    if task_id.is_empty() {
        anyhow::bail!("STS returned empty task_id");
    }
    Ok(SessionInfo {
        task_id,
        session_signature,
        session_timestamp,
        session_salt,
    })
}

/// 预检 API 调用的结果。
pub struct PrecheckResult {
    pub exists: bool,
    pub storage_path: Option<String>,
    pub uploaded_chunks: Vec<u32>,
}

/// 预检文件：去重检查和断点续传支持。
pub async fn precheck_file(
    client: &reqwest::Client,
    server_url: &str,
    file_name: &str,
    file_size: u64,
    file_md5: &str,
    chunk_size: u64,
) -> Result<PrecheckResult> {
    let resp = client
        .post(format!(
            "{}{}/upload/precheck",
            server_url,
            constant::API_BASE_PATH
        ))
        .json(&serde_json::json!({
            "file_name": file_name,
            "file_size": file_size,
            "file_md5": file_md5,
            "chunk_size": chunk_size,
        }))
        .send()
        .await
        .map_err(|e| super::map_network_error(e, server_url))?;

    let body: serde_json::Value = resp.json().await?;
    let exists = body["data"]["exists"].as_bool().unwrap_or(false);
    let storage_path = body["data"]["storage_path"].as_str().map(|s| s.to_string());
    let chunks: Vec<u32> = body["data"]["uploaded_chunks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u32))
                .collect()
        })
        .unwrap_or_default();

    Ok(PrecheckResult {
        exists,
        storage_path,
        uploaded_chunks: chunks,
    })
}
