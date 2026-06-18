//! 分块上传 API 客户端函数——流式上传。
//!
//! 从文件句柄流式传输分块数据，避免将整个分块加载到内存中。

use anyhow::Result;
use buckets_common::constant;

/// 使用文件的流式传输上传分块——避免将整个分块加载到内存中。
/// 文件句柄应已寻道到正确的偏移位置。
/// 使用会话级别签名（一次上传中的所有分块使用一个签名）。
/// 会话签名参数通过自定义 HTTP 头部传递，以避免
/// URL 长度限制和在访问日志/浏览器历史中暴露。
#[allow(clippy::too_many_arguments)]
pub async fn upload_chunk_streaming(
    client: &reqwest::Client,
    server_url: &str,
    task_id: &str,
    chunk_index: u32,
    chunk_md5: &str,
    file: tokio::io::Take<tokio::fs::File>,
    _chunk_size: u64,
    session_signature: &str,
    session_timestamp: i64,
    session_salt: &str,
) -> Result<()> {
    use tokio_util::io::ReaderStream;

    let url = format!(
        "{}{}/upload/chunk/upload-binary?task_id={}&chunk_index={}&chunk_md5={}",
        server_url,
        constant::API_BASE_PATH,
        task_id,
        chunk_index,
        chunk_md5,
    );

    // 从文件流式传输——由于 Take 包装器，ReaderStream 精确读取 chunk_size 字节
    let stream = ReaderStream::with_capacity(file, constant::CHUNK_STREAM_BUFFER_SIZE);
    let body = reqwest::Body::wrap_stream(stream);

    let resp = client
        .post(&url)
        .header(constant::HEADER_CONTENT_TYPE, constant::MIME_OCTET_STREAM)
        .header(constant::HEADER_SESSION_SIGNATURE, session_signature)
        .header(
            constant::HEADER_SESSION_TIMESTAMP,
            session_timestamp.to_string(),
        )
        .header(constant::HEADER_SESSION_SALT, session_salt)
        .body(body)
        .send()
        .await
        .map_err(|e| super::map_network_error(e, server_url))?;

    if resp.status().is_success() || resp.status().as_u16() == 409 {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("upload chunk failed: {}", body);
    }
}
