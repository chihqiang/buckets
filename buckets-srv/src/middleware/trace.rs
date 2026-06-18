//! 追踪 ID 中间件。
//!
//! 为每个请求提取或生成 `x-trace-id`，创建追踪 span，
//! 使子事件继承 trace_id，并在响应头部中回显。

use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use buckets_common::constant;

/// Axum 中间件：提取或生成追踪 ID，创建追踪 span，
/// 使所有子中间件和处理器事件继承 trace_id，
/// 并在响应 `x-trace-id` 头部中回显。
pub async fn trace_layer(req: Request, next: Next) -> Response {
    let trace_id = req
        .headers()
        .get(constant::HEADER_TRACE_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let span = tracing::info_span!("request", trace_id = %trace_id);
    let _guard = span.enter();

    let mut response = next.run(req).await;

    if let Ok(value) = HeaderValue::from_str(&trace_id) {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(constant::HEADER_TRACE_ID),
            value,
        );
    }

    response
}
