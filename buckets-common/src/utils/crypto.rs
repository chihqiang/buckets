//! 会话级别 HMAC 签名，用于分块上传授权。
//!
//! 认证令牌使用标准 JWT（HS256），在头部包含 `kid`（密钥 ID）
//! 以便在验证期间路由到每个用户的密钥。
//! JWT 令牌的生成/验证在此处，同时支持访问令牌和刷新令牌。

use crate::constant;
use crate::error::AppError;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// 内联常量
// ============================================================================

/// 会话级别签名有效期：2 小时。
const SESSION_SIGNATURE_EXPIRATION_SECS: i64 = 7200;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// 会话签名（用于分块上传授权）
// ============================================================================

/// 会话级别签名的输入（覆盖整个上传，不是每个分块）。
#[derive(Debug, Clone)]
pub struct SessionSignInput {
    pub user_id: i64,
    pub task_id: String,
    pub file_md5: String,
    pub chunk_size: u64,
    pub timestamp: i64,
    pub salt: String,
}

/// 为整个上传会话生成会话级别签名。
pub fn generate_session_signature(
    secret_key: &str,
    input: &SessionSignInput,
) -> Result<String, AppError> {
    let message = format!(
        "session:{}:{}:{}:{}:{}:{}",
        input.user_id, input.task_id, input.file_md5, input.chunk_size, input.timestamp, input.salt
    );
    let mut mac = HmacSha256::new_from_slice(secret_key.as_bytes())
        .map_err(|_| AppError::Internal("HMAC key error".into()))?;
    mac.update(message.as_bytes());
    let result = mac.finalize();
    Ok(hex::encode(result.into_bytes()))
}

/// 验证会话级别签名。
pub fn verify_session_signature(
    secret_key: &str,
    input: &SessionSignInput,
    signature: &str,
) -> Result<bool, AppError> {
    let expected = generate_session_signature(secret_key, input)?;
    Ok(expected == signature)
}

/// 验证会话签名时间戳。
pub fn verify_session_timestamp(timestamp: i64) -> Result<(), AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Internal("time error".into()))?
        .as_secs() as i64;

    if (now - timestamp).abs() > SESSION_SIGNATURE_EXPIRATION_SECS {
        return Err(AppError::SignatureExpired);
    }
    Ok(())
}

// ============================================================================
// JWT 认证令牌
// ============================================================================

/// 访问令牌的 JWT 声明。
#[derive(Debug, Serialize, Deserialize)]
pub struct AccessClaims {
    /// 用户 ID
    pub sub: i64,
    /// 签发时间（Unix 时间戳）
    pub iat: usize,
    /// 过期时间（Unix 时间戳）
    pub exp: usize,
    /// 唯一令牌 ID
    pub jti: String,
}

/// 刷新令牌的 JWT 声明。
#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshClaims {
    /// 用户 ID
    pub sub: i64,
    /// 签发时间（Unix 时间戳）
    pub iat: usize,
    /// 过期时间（Unix 时间戳）
    pub exp: usize,
    /// 唯一令牌 ID
    pub jti: String,
}

/// 生成使用用户的个人 secret_key 签名的 JWT 访问令牌（HS256）。
/// 返回（令牌，过期时间戳）。
///
/// 在 JWT 头部嵌入 `kid`（密钥 ID），以便验证器可以仅检查（未验证的）
/// 头部就路由到正确的用户密钥，而无需手动解码载荷。
pub fn generate_access_token(secret_key: &[u8], user_id: i64) -> Result<(String, i64), AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Internal("time error".into()))?
        .as_secs() as usize;
    let exp = now + constant::AUTH_TOKEN_TTL_SECS as usize;
    let jti = uuid::Uuid::new_v4().to_string();

    let claims = AccessClaims {
        sub: user_id,
        iat: now,
        exp,
        jti,
    };

    let header = jsonwebtoken::Header {
        kid: Some(user_id.to_string()),
        ..Default::default()
    };

    let token = jsonwebtoken::encode(
        &header,
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret_key),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok((token, exp as i64))
}

/// 生成使用全局服务器密钥签名的 JWT 刷新令牌（HS256）。
/// 返回（令牌，过期时间戳）。
pub fn generate_refresh_token(user_id: i64) -> Result<(String, i64), AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Internal("time error".into()))?
        .as_secs() as usize;
    let exp = now + constant::REFRESH_TOKEN_TTL_SECS as usize;
    let jti = uuid::Uuid::new_v4().to_string();

    let claims = RefreshClaims {
        sub: user_id,
        iat: now,
        exp,
        jti,
    };

    let key = refresh_token_key();
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(key.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))?;

    Ok((token, exp as i64))
}

/// 验证 JWT 访问令牌，有效时返回（user_id, jti）。
/// 访问令牌使用每个用户的密钥。
pub fn verify_access_token(secret_key: &[u8], token: &str) -> Option<(i64, String)> {
    let token_data = jsonwebtoken::decode::<AccessClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret_key),
        &jsonwebtoken::Validation::default(),
    )
    .ok()?;

    Some((token_data.claims.sub, token_data.claims.jti))
}

/// 验证 JWT 刷新令牌，有效时返回（user_id, jti）。
/// 刷新令牌使用全局服务器密钥。
pub fn verify_refresh_token(token: &str) -> Option<(i64, String)> {
    let key = refresh_token_key();
    let token_data = jsonwebtoken::decode::<RefreshClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(key.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .ok()?;

    Some((token_data.claims.sub, token_data.claims.jti))
}

/// 从环境变量或默认值获取全局刷新令牌签名密钥。
fn refresh_token_key() -> String {
    std::env::var(constant::ENV_REFRESH_TOKEN_KEY)
        .unwrap_or_else(|_| constant::DEFAULT_REFRESH_TOKEN_KEY.to_string())
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 从 `Authorization: Bearer <token>` 头部提取令牌值。
pub fn extract_bearer_token(header_value: &str) -> Option<&str> {
    header_value.strip_prefix("Bearer ")
}
