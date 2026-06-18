//! 请求日志记录中间件。
//!
//! 记录每个请求的方法、路径、状态码和持续时间。
//! 对 5xx/4xx 响应分别使用 error/warn 级别。
//! trace_id 来自父级 `request` span（trace_layer）。

use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;
use tracing::{error, info, warn};

/// 记录请求方法、路径、状态和耗时的 Axum 中间件。
/// 5xx 响应记录为 ERROR 级别，4xx 为 WARN，其他为 INFO。
pub async fn request_logger(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let response = next.run(req).await;

    let duration = start.elapsed();
    let status = response.status().as_u16();

    match status {
        500..=599 => error!(
            method,
            path,
            status,
            ms = duration.as_millis(),
            "request failed"
        ),
        400..=499 => warn!(
            method,
            path,
            status,
            ms = duration.as_millis(),
            "request error"
        ),
        _ => info!(
            method,
            path,
            status,
            ms = duration.as_millis(),
            "request ok"
        ),
    }

    response
}
