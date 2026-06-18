#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::error::AppError;
    use crate::utils::crypto::*;
    use crate::utils::hash::*;
    use crate::utils::password::*;
    use crate::utils::validate::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // --- 哈希测试 ---

    #[test]
    fn test_compute_chunk_md5() {
        let data = b"hello world";
        let md5 = compute_chunk_md5(data);
        assert_eq!(md5.len(), 32); // MD5 十六进制始终为 32 个字符
    }

    #[test]
    fn test_verify_chunk_md5() {
        let data = b"test data";
        let md5 = compute_chunk_md5(data);
        assert!(verify_chunk_md5(data, &md5));
        assert!(!verify_chunk_md5(data, "00000000000000000000000000000000"));
    }

    #[tokio::test]
    async fn test_compute_file_md5_async() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"async test content").unwrap();
        tmp.flush().unwrap();

        let md5 = compute_file_md5_async(tmp.path()).await.unwrap();
        assert_eq!(md5.len(), 32);
    }

    // --- 验证测试 ---

    #[test]
    fn test_validate_file_extension_allowed() {
        assert!(validate_file_extension("test.jpg").is_ok());
        assert!(validate_file_extension("test.PNG").is_ok());
        assert!(validate_file_extension("test.json").is_ok());
    }

    #[test]
    fn test_validate_file_extension_blocked() {
        // STRICT_EXTENSION_CHECK 默认为 false，因此被阻止的扩展名也能通过
        assert!(validate_file_extension("test.exe").is_ok());
        assert!(validate_file_extension("test.sh").is_ok());
        assert!(validate_file_extension("test.bat").is_ok());
    }

    #[test]
    fn test_validate_file_extension_no_ext() {
        // STRICT_EXTENSION_CHECK 默认为 false，因此无扩展名也能通过
        assert!(validate_file_extension("noextension").is_ok());
    }

    // --- 加密测试 ---

    #[test]
    fn test_generate_and_verify_session_signature() {
        let secret = "test_secret_key";
        let input = SessionSignInput {
            user_id: 42,
            task_id: "test-task-id".to_string(),
            file_md5: "abcdef123456".to_string(),
            chunk_size: 8 * 1024 * 1024,
            timestamp: chrono::Utc::now().timestamp(),
            salt: "testsalt".to_string(),
        };
        let sig = generate_session_signature(secret, &input).unwrap();
        assert!(!sig.is_empty());
        assert!(verify_session_signature(secret, &input, &sig).unwrap());
    }

    #[test]
    fn test_verify_session_signature_wrong_key() {
        let secret = "test_secret_key";
        let input = SessionSignInput {
            user_id: 42,
            task_id: "test-task-id".to_string(),
            file_md5: "abcdef123456".to_string(),
            chunk_size: 8 * 1024 * 1024,
            timestamp: chrono::Utc::now().timestamp(),
            salt: "testsalt".to_string(),
        };
        let sig = generate_session_signature(secret, &input).unwrap();
        assert!(!verify_session_signature("wrong_key", &input, &sig).unwrap());
    }

    #[test]
    fn test_verify_session_timestamp_valid() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(verify_session_timestamp(now).is_ok());
    }

    #[test]
    fn test_verify_session_timestamp_expired() {
        assert!(verify_session_timestamp(0).is_err());
    }

    // --- 密码测试 ---

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("correct_password").unwrap();
        assert!(verify_password("correct_password", &hash).unwrap());
        assert!(!verify_password("wrong_password", &hash).unwrap());
    }

    #[test]
    fn test_password_hash_different_each_time() {
        let h1 = hash_password("same_password").unwrap();
        let h2 = hash_password("same_password").unwrap();
        // Argon2 使用随机盐，因此哈希值应不同
        assert_ne!(h1, h2);
    }

    // --- 错误测试 ---

    #[test]
    fn test_app_error_status_codes() {
        assert_eq!(AppError::BadRequest("x".into()).status_code(), 400);
        assert_eq!(AppError::Unauthorized.status_code(), 401);
        assert_eq!(AppError::Forbidden("x".into()).status_code(), 403);
        assert_eq!(AppError::NotFound("x".into()).status_code(), 404);
        assert_eq!(AppError::SignatureExpired.status_code(), 401);
        assert_eq!(AppError::FileTooLarge("x".into()).status_code(), 413);
        assert_eq!(
            AppError::HashMismatch {
                expected: "a".into(),
                actual: "b".into()
            }
            .status_code(),
            409
        );
        assert_eq!(AppError::StorageError("x".into()).status_code(), 500);
        assert_eq!(AppError::DatabaseError("x".into()).status_code(), 500);
    }
}
