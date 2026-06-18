//! 合并 API 客户端函数——带轮询的异步合并。
//!
//! 合并轮询使用指数退避，减少长时间运行合并期间的服务器压力。

use anyhow::Result;
use buckets_common::constant;
use std::time::Duration;

/// 请求服务器开始合并所有已上传的分块（异步，返回 202 Accepted）。
pub async fn merge_chunks(
    client: &reqwest::Client,
    server_url: &str,
    task_id: &str,
    file_name: &str,
    file_md5: &str,
    file_size: u64,
) -> Result<()> {
    let resp = client
        .post(format!(
            "{}{}/upload/merge",
            server_url,
            constant::API_BASE_PATH
        ))
        .json(&serde_json::json!({
            "task_id": task_id,
            "file_name": file_name,
            "file_md5": file_md5,
            "file_size": file_size,
        }))
        .send()
        .await
        .map_err(|e| super::map_network_error(e, server_url))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("merge request failed: {}", body);
    }

    Ok(())
}

/// 轮询合并状态直到完成或失败。成功时返回对象 URL。
/// 使用指数退避（2s → 4s → 8s → ... → 30s 上限）减少大文件长时间合并期间的服务器压力。
pub async fn poll_merge_status(
    client: &reqwest::Client,
    server_url: &str,
    task_id: &str,
) -> Result<String> {
    let mut poll_interval = Duration::from_secs(constant::MERGE_POLL_INTERVAL_SECS);
    let max_interval = Duration::from_secs(constant::MERGE_POLL_MAX_INTERVAL_SECS);

    for _ in 0..constant::MERGE_POLL_MAX_ATTEMPTS {
        tokio::time::sleep(poll_interval).await;

        let resp = client
            .get(format!(
                "{}{}/upload/merge/status?task_id={}",
                server_url,
                constant::API_BASE_PATH,
                task_id,
            ))
            .send()
            .await
            .map_err(|e| super::map_network_error(e, server_url))?;

        let body: serde_json::Value = resp.json().await?;
        let status = body["data"]["status"].as_str().unwrap_or("unknown");

        match status {
            constant::STATUS_COMPLETED => {
                let url = body["data"]["storage_path"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if url.is_empty() {
                    anyhow::bail!("merge completed but no storage_path returned");
                }
                return Ok(url);
            }
            constant::STATUS_FAILED => {
                let err = body["data"]["error"].as_str().unwrap_or("unknown error");
                anyhow::bail!("merge failed: {}", err);
            }
            constant::STATUS_MERGING => {
                // 仍在进行中，增加轮询间隔
                poll_interval = (poll_interval * 2).min(max_interval);
            }
            _ => {
                tracing::debug!("merge status: {}", status);
            }
        }
    }

    anyhow::bail!(
        "merge timed out after {} attempts",
        constant::MERGE_POLL_MAX_ATTEMPTS
    );
}
