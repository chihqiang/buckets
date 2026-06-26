//! STS 令牌签发处理器。

use crate::app::AppState;
use crate::db::UserId;
use crate::service::auth_svc;
use axum::extract::{Extension, State};
use axum::Json;
use buckets_common::error::AppError;
use buckets_common::model::api::{ApiResponse, StsRequest, StsResult, api_ok};

/// 为新上传会话签发 STS 令牌。
pub async fn get_sts_token(
    state: State<AppState>,
    Extension(uid): Extension<UserId>,
    Json(req): Json<StsRequest>,
) -> Result<Json<ApiResponse<StsResult>>, AppError> {
    let result = auth_svc::issue_sts(&state.db, uid.0, &req).await?;
    Ok(api_ok(result))
}
