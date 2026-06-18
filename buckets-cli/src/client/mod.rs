//! buckets 网关 API 的 HTTP 客户端。
//!
//! `Client` 结构管理认证头部、追踪 ID，并编排完整的上传流程：
//! STS → 预检 → 分块上传 → 合并。

pub mod chunk;
pub mod merge;
pub mod precheck;

use crate::local;
use crate::progress::UploadProgress;
use anyhow::Result;
use base64::Engine;
use buckets_common::constant;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// 将 reqwest 发送错误包装为用户友好的消息。
fn map_network_error(e: reqwest::Error, server_url: &str) -> anyhow::Error {
    if e.is_connect() {
        anyhow::anyhow!("cannot connect to {server_url} — is the server running?")
    } else if e.is_timeout() {
        anyhow::anyhow!("connection to {server_url} timed out")
    } else {
        anyhow::anyhow!("request failed: {e}")
    }
}

/// 包装 `reqwest::Client` 的 HTTP 客户端，包含认证头部和追踪 ID。
pub struct Client {
    pub server_url: String,
    pub trace_id: String,
    pub http: reqwest::Client,
    /// 存储认证头部值，用于创建分块上传客户端。
    auth_header_value: Option<reqwest::header::HeaderValue>,
}

impl Client {
    /// 创建带有 Basic 认证头部和唯一追踪 ID 的新客户端。
    pub fn new(server_url: &str, email: &str, password: &str) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            constant::HEADER_TRACE_ID,
            reqwest::header::HeaderValue::from_str(&trace_id).expect("UUID is valid header value"),
        );

        let credentials = base64::engine::general_purpose::STANDARD.encode(format!(
            "{}{}{}",
            email,
            constant::CREDENTIAL_SEPARATOR,
            password
        ));
        let auth_value = format!("{}{}", constant::AUTH_SCHEME_BASIC, credentials);
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth_value)
                .expect("base64 is valid header value"),
        );

        // 通用请求的默认超时时间（状态、预检等）
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(constant::DEFAULT_HTTP_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(
                constant::DEFAULT_HTTP_CONNECT_TIMEOUT_SECS,
            ))
            .pool_max_idle_per_host(constant::DEFAULT_POOL_MAX_IDLE_PER_HOST)
            .build()
            .expect("reqwest client build should not fail");

        tracing::info!("trace_id: {trace_id}");

        Client {
            server_url: server_url.to_string(),
            trace_id,
            http,
            auth_header_value: Some(auth_value.parse().expect("auth value is valid")),
        }
    }

    /// 创建具有更长超时时间的分块上传客户端。
    /// 慢速网络中上传大分块可能需要超过默认的 600 秒。
    pub fn chunk_upload_client(&self) -> reqwest::Client {
        let timeout_secs = std::env::var(constant::ENV_CHUNK_UPLOAD_TIMEOUT_SECS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(constant::DEFAULT_CHUNK_UPLOAD_TIMEOUT_SECS);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            constant::HEADER_TRACE_ID,
            reqwest::header::HeaderValue::from_str(&self.trace_id)
                .expect("UUID is valid header value"),
        );
        // 从主客户端复制认证头部
        if let Some(ref auth) = self.auth_header_value {
            headers.insert(reqwest::header::AUTHORIZATION, auth.clone());
        }

        reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(
                constant::DEFAULT_HTTP_CONNECT_TIMEOUT_SECS,
            ))
            .pool_max_idle_per_host(constant::DEFAULT_POOL_MAX_IDLE_PER_HOST)
            .build()
            .expect("reqwest chunk client build should not fail")
    }

    /// 使用分块传输上传文件：STS → 预检 → 并行分块上传 → 合并（异步）。
    pub async fn upload_file(
        &self,
        file_path: &str,
        bucket_name: Option<String>,
        chunk_size_mb: u64,
        parallel: usize,
        resume: bool,
    ) -> Result<String> {
        let path = Path::new(file_path);
        if !path.exists() {
            anyhow::bail!("file not found: {}", file_path);
        }

        let file_size = tokio::fs::metadata(path).await?.len();
        let chunk_size =
            chunk_size_mb * constant::DEFAULT_CHUNK_SIZE / constant::DEFAULT_CHUNK_SIZE_MB;
        let chunk_count = file_size.div_ceil(chunk_size) as u32;

        let name = bucket_name.unwrap_or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(constant::UNKNOWN_FILE_NAME)
                .to_string()
        });

        let progress = UploadProgress::new(chunk_count as u64);

        // 将文件 MD5 计算为分块 MD5 的 Merkle 树根。
        // 每个分块的 MD5 在上传期间计算。文件级 MD5 为：
        //   file_md5 = MD5(chunk_md5[0] || chunk_md5[1] || ...)
        // 其中每个 chunk_md5 是 32 字符的十六进制字符串。
        // 这避免将整个文件读取两次（一次在这里，一次在分块上传期间），
        // 并允许服务器在合并期间从 sidecar 文件验证文件 MD5，
        // 而无需在零拷贝合并路径中重新读取分块数据——消除双重磁盘 I/O。
        //
        // 首先通过读取文件一次计算所有分块 MD5，然后对其连接结果进行哈希。
        progress.set_message("Computing MD5...");
        let progress_md5 = progress.clone();
        let path_clone = path.to_path_buf();
        let chunk_md5s: Vec<String> = tokio::task::spawn_blocking(move || {
            use md5::Digest;
            use std::io::Read;
            let file = std::fs::File::open(&path_clone)?;
            let file_size = file.metadata()?.len();
            let mut reader = std::io::BufReader::with_capacity(65536, file);
            let mut md5s = Vec::new();
            let mut buf = [0u8; 65536];
            let mut offset: u64 = 0;
            for _ in 0..chunk_count {
                let remaining = file_size.saturating_sub(offset);
                let this_chunk = remaining.min(chunk_size);
                let mut hasher = md5::Md5::new();
                let mut read_in_chunk: u64 = 0;
                while read_in_chunk < this_chunk {
                    let to_read = (this_chunk - read_in_chunk).min(buf.len() as u64) as usize;
                    let n = reader.read(&mut buf[..to_read])?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                    read_in_chunk += n as u64;
                    offset += n as u64;
                }
                md5s.push(hex::encode(hasher.finalize()));
                // 每 64 MiB 报告一次进度
                if offset % (64 * 1024 * 1024) < read_in_chunk {
                    let pct = if file_size > 0 {
                        (offset as f64 / file_size as f64 * 100.0) as u32
                    } else {
                        0
                    };
                    progress_md5.set_message(&format!(
                        "Computing MD5... {}/{} MB ({}%)",
                        offset / (1024 * 1024),
                        file_size / (1024 * 1024),
                        pct
                    ));
                }
            }
            Ok::<Vec<String>, std::io::Error>(md5s)
        })
        .await??;

        // 计算 file_md5 = MD5（所有分块 MD5 十六进制字符串的连接）
        use md5::Digest;
        let mut file_hasher = md5::Md5::new();
        for md5_str in &chunk_md5s {
            file_hasher.update(md5_str.as_bytes());
        }
        let file_md5 = hex::encode(file_hasher.finalize());
        tracing::info!("File MD5 (Merkle root): {}", file_md5);

        progress.set_message("Getting STS...");
        let session = precheck::get_sts_token(
            &self.http,
            &self.server_url,
            &name,
            file_size,
            &file_md5,
            chunk_size,
        )
        .await?;
        let task_id = session.task_id;

        progress.set_message("Prechecking...");
        let precheck_result = precheck::precheck_file(
            &self.http,
            &self.server_url,
            &name,
            file_size,
            &file_md5,
            chunk_size,
        )
        .await?;

        if precheck_result.exists {
            progress.finish();
            let url = match &precheck_result.storage_path {
                Some(u) => format!("{}{}{}", self.server_url, constant::API_BASE_PATH, u),
                None => format!(
                    "{}{}/bucket/unknown",
                    self.server_url,
                    constant::API_BASE_PATH
                ),
            };
            return Ok(url);
        }

        let uploaded_set = precheck_result.uploaded_chunks;
        let mut uploaded: Vec<bool> = vec![false; chunk_count as usize];
        for &idx in &uploaded_set {
            if (idx as usize) < chunk_count as usize {
                uploaded[idx as usize] = true;
            }
        }

        if resume {
            let done = uploaded.iter().filter(|&&v| v).count();
            for _ in 0..done {
                progress.inc();
            }
        }

        // 获取所有需要上传的分块索引
        let missing_indices: Vec<u32> = (0..chunk_count)
            .filter(|&i| !uploaded[i as usize])
            .collect();

        if missing_indices.is_empty() {
            progress.set_message("All chunks already uploaded, triggering merge...");
        } else {
            // 对所有分块上传使用会话级别签名（无需批量签名）
            progress.set_message("Uploading chunks...");
            let session_sig = session.session_signature.clone();
            let session_ts = session.session_timestamp;
            let session_salt = session.session_salt.clone();

            let chunk_md5s = Arc::new(chunk_md5s);
            let semaphore = Arc::new(Semaphore::new(parallel));
            let path = Arc::new(path.to_path_buf());
            let progress = Arc::new(progress.clone());
            // 使用更长超时时间的客户端进行分块上传
            let http = Arc::new(self.chunk_upload_client());
            let server_url = Arc::new(self.server_url.clone());
            let task_id = Arc::new(task_id.clone());

            let mut handles = Vec::new();

            for idx in missing_indices {
                let permit = semaphore.clone().acquire_owned().await?;
                let http = http.clone();
                let server_url = server_url.clone();
                let path = path.clone();
                let progress = progress.clone();
                let tid = task_id.clone();
                let session_sig = session_sig.clone();
                let session_salt = session_salt.clone();

                let chunk_md5s = chunk_md5s.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = permit;

                    let md5 = chunk_md5s[idx as usize].clone();
                    // 使用预计算的 MD5，避免每个分块重新读取文件

                    // 使用会话签名（所有分块相同），带流式传输和抖动重试
                    for attempt in 0..constant::CHUNK_UPLOAD_MAX_RETRIES {
                        // 每次尝试重新打开文件流
                        let file =
                            local::open_chunk_stream(&path, idx as u64 * chunk_size, chunk_size)
                                .await?;

                        match chunk::upload_chunk_streaming(
                            &http,
                            &server_url,
                            &tid,
                            idx,
                            &md5,
                            file,
                            chunk_size,
                            &session_sig,
                            session_ts,
                            &session_salt,
                        )
                        .await
                        {
                            Ok(_) => {
                                progress.inc();
                                return Ok::<_, anyhow::Error>(());
                            }
                            Err(_e) if attempt + 1 < constant::CHUNK_UPLOAD_MAX_RETRIES => {
                                // 带随机抖动的指数退避
                                let backoff = Duration::from_secs(
                                    constant::CHUNK_UPLOAD_RETRY_BACKOFF_BASE_SECS << attempt,
                                );
                                let jitter = Duration::from_millis(
                                    rand::random::<u64>()
                                        % constant::CHUNK_UPLOAD_RETRY_MAX_JITTER_MS,
                                );
                                tokio::time::sleep(backoff + jitter).await;
                                continue;
                            }
                            Err(e) => anyhow::bail!(
                                "chunk {} failed after {} retries: {}",
                                idx,
                                constant::CHUNK_UPLOAD_MAX_RETRIES,
                                e
                            ),
                        }
                    }
                    Ok(())
                }));
            }

            for h in handles {
                h.await??;
            }
        }

        // 异步合并：POST 请求启动，然后轮询完成状态
        let progress_upload = progress.clone();
        progress_upload.set_message("Starting merge...");
        merge::merge_chunks(
            &self.http,
            &self.server_url,
            &task_id,
            &name,
            &file_md5,
            file_size,
        )
        .await?;

        progress_upload.set_message("Waiting for merge to complete...");
        let bucket_url = merge::poll_merge_status(&self.http, &self.server_url, &task_id).await?;

        progress_upload.finish();
        Ok(bucket_url)
    }

    /// 通过调用专用的 auth/verify 端点验证凭据。
    /// 此端点需要认证，使其成为验证凭据的干净方式。
    pub async fn verify_credentials(&self) -> Result<()> {
        let resp = match self
            .http
            .post(format!(
                "{}{}/auth/verify",
                self.server_url,
                constant::API_BASE_PATH
            ))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Err(map_network_error(e, &self.server_url)),
        };
        if resp.status().is_success() {
            Ok(())
        } else if resp.status().as_u16() == 401 {
            anyhow::bail!("invalid email or password");
        } else {
            anyhow::bail!("server returned {}", resp.status());
        }
    }

    /// 查询并美观打印指定任务 ID 的上传状态。
    pub async fn print_status(&self, task_id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!(
                "{}{}/upload/chunk/status",
                self.server_url,
                constant::API_BASE_PATH
            ))
            .json(&serde_json::json!({ "task_id": task_id }))
            .send()
            .await
            .map_err(|e| map_network_error(e, &self.server_url))?;

        let body: serde_json::Value = resp.json().await?;
        println!("{}", serde_json::to_string_pretty(&body)?);
        Ok(())
    }

    /// 从本地缓存恢复最近中断的上传。
    pub async fn resume_upload(&self) -> Result<()> {
        let home = std::env::var(constant::ENV_HOME).unwrap_or_else(|_| ".".into());
        let cache_dir = std::path::PathBuf::from(home)
            .join(constant::CLI_CONFIG_DIR)
            .join(constant::CLI_CACHE_SUBDIR);

        if !cache_dir.exists() {
            anyhow::bail!(
                "no cached upload found. Use 'upload' command with --resume flag instead."
            );
        }

        let mut entries: Vec<_> = std::fs::read_dir(&cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == constant::CLI_CACHE_EXTENSION)
                    .unwrap_or(false)
            })
            .collect();
        entries.sort_by_key(|b| std::cmp::Reverse(b.metadata().unwrap().modified().unwrap()));

        if entries.is_empty() {
            anyhow::bail!("no cached upload found");
        }

        let latest = &entries[0];
        let data = std::fs::read_to_string(latest.path())?;
        let cache: serde_json::Value = serde_json::from_str(&data)?;

        let file_path = cache[constant::CLI_CACHE_KEY_FILE_PATH]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid cache"))?;
        let bucket_name = cache[constant::CLI_CACHE_KEY_OBJECT_NAME]
            .as_str()
            .map(|s| s.to_string());
        let chunk_size = cache[constant::CLI_CACHE_KEY_CHUNK_SIZE]
            .as_u64()
            .unwrap_or(constant::DEFAULT_CHUNK_SIZE_MB);

        println!("Resuming upload of: {}", file_path);
        let bucket_url = self
            .upload_file(
                file_path,
                bucket_name,
                chunk_size,
                constant::DEFAULT_PARALLEL_UPLOADS,
                true,
            )
            .await
            .map_err(|e| anyhow::anyhow!("[trace_id: {}] resume failed: {e:#}", self.trace_id))?;
        println!("Upload complete: {bucket_url}");
        Ok(())
    }
}
