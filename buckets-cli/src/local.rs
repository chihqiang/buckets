//! 用于分块读取和流式传输的本地文件 I/O 辅助函数。

use std::io::SeekFrom;
use std::path::Path;

/// 打开文件，返回一个从 `offset` 开始精确读取 `size` 字节的 AsyncRead。
/// 用于流式上传，避免将整个分块加载到内存中。
pub async fn open_chunk_stream(
    path: &Path,
    offset: u64,
    size: u64,
) -> Result<tokio::io::Take<tokio::fs::File>, std::io::Error> {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncSeekExt;

    let mut file = tokio::fs::File::open(path).await?;
    file.seek(SeekFrom::Start(offset)).await?;
    Ok(file.take(size))
}
