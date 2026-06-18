//! 使用 Argon2id（内存硬 KDF）进行密码哈希。
//!
//! 使用加盐的、抗暴力破解的算法替代纯 SHA256，
//! 适用于存储用户凭据。

use crate::error::AppError;
use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use std::sync::OnceLock;

/// Argon2 内存开销参数：64 MiB。
const ARGON2_M_COST: u32 = 65536;
/// Argon2 时间开销（迭代次数）：3 轮。
const ARGON2_T_COST: u32 = 3;
/// Argon2 并行度：4 条流水线。
const ARGON2_P_COST: u32 = 4;

/// 缓存的 Argon2 实例——参数固定，因此只创建一次。
static ARGON2: OnceLock<Argon2<'static>> = OnceLock::new();

fn argon2() -> &'static Argon2<'static> {
    ARGON2.get_or_init(|| {
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
                .expect("argon2 params are valid"),
        )
    })
}

/// 使用 Argon2id 哈希密码（内存硬、加盐、抗暴力破解）。
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("argon2 hash: {}", e)))?;
    Ok(hash.to_string())
}

/// 验证密码是否与 Argon2 PHC 哈希字符串匹配。
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("parse password hash: {}", e)))?;
    Ok(argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}
