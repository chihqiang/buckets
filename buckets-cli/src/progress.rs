//! 使用 `indicatif` 的上传操作进度条。

use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 带分块计数器的线程安全上传进度条。
#[derive(Clone)]
pub struct UploadProgress {
    bar: ProgressBar,
    done: Arc<AtomicU64>,
}

impl UploadProgress {
    /// 为给定的总分块数创建新的进度条。
    pub fn new(total_chunks: u64) -> Self {
        let bar = ProgressBar::new(total_chunks);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
                .expect("template string is valid")
                .progress_chars("#>-"),
        );
        UploadProgress {
            bar,
            done: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 将进度条增加一个分块。
    pub fn inc(&self) {
        self.done.fetch_add(1, Ordering::SeqCst);
        self.bar.inc(1);
    }

    /// 设置在进度条旁边显示的状态消息。
    pub fn set_message(&self, msg: &str) {
        self.bar.set_message(msg.to_string());
    }

    /// 将进度条标记为完成。
    pub fn finish(&self) {
        self.bar.finish_with_message("Complete");
    }
}
