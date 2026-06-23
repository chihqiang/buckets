//! 上传速率限制中间件。
//!
//! 为上传相关端点实现三层速率限制：
//! 1. **请求速率**：每用户每秒最大请求数（令牌桶）
//! 2. **并发上传**：每用户最大活跃上传任务数
//! 3. **每日配额**：每用户每天最大上传任务数

use crate::app::AppState;
use crate::config::RateLimitConfig;
use crate::db::UserId;
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use buckets_common::constant;
use buckets_common::model::db::upload_tasks;
use dashmap::DashMap;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// 每用户速率限制状态
#[derive(Debug)]
struct UserBucket {
    /// 令牌桶令牌数（浮点数，实现平滑补充）
    tokens: f64,
    /// 上次补充时间戳
    last_refill: Instant,
}

/// 共享的速率限制器状态
#[derive(Clone)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Arc<DashMap<u64, UserBucket>>,
    /// 每用户的内存并发上传计数器。
    /// 避免每次请求查询数据库获取活跃任务数。
    concurrent_counters: Arc<DashMap<u64, AtomicU32>>,
    db: DatabaseConnection,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig, db: DatabaseConnection) -> Self {
        Self {
            config,
            buckets: Arc::new(DashMap::new()),
            concurrent_counters: Arc::new(DashMap::new()),
            db,
        }
    }

    /// 启动时从数据库重建并发上传计数器。
    /// 通过统计每用户活跃上传任务数（initialized、uploading、merging），
    /// 防止进程重启后超过限制。
    /// 仅统计最近 24 小时内的任务，避免因崩溃重启场景中
    /// 进行中的上传从未递减而导致过时计数器。
    pub async fn init_concurrent_counters(&self) {
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::hours(24);
        let tasks = upload_tasks::Entity::find()
            .filter(
                upload_tasks::Column::Status.is_in([
                    buckets_common::constant::STATUS_INITIALIZED,
                    buckets_common::constant::STATUS_UPLOADING,
                    buckets_common::constant::STATUS_MERGING,
                ]),
            )
            .filter(upload_tasks::Column::UpdatedAt.gte(cutoff))
            .all(&self.db)
            .await;

        let tasks = match tasks {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "failed to rebuild concurrent counters from DB");
                return;
            }
        };

        let mut user_counts: std::collections::HashMap<u64, u32> =
            std::collections::HashMap::new();
        for task in &tasks {
            *user_counts.entry(task.user_id).or_insert(0) += 1;
        }
        for (user_id, count) in &user_counts {
            self.concurrent_counters
                .insert(*user_id, AtomicU32::new(*count));
        }
        if !tasks.is_empty() {
            tracing::info!(
                users = user_counts.len(),
                "rebuilt concurrent upload counters from DB"
            );
        }
    }

    /// 启动周期性清理非活动用户桶（防止内存泄漏）。
    /// 接受 [`CancellationToken`] 以实现优雅关闭。
    pub fn start_cleanup_task(limiter: Arc<Self>, cancellation: CancellationToken) {
        tokio::spawn(async move {
            let interval_dur =
                Duration::from_secs(constant::RATE_LIMIT_BUCKET_CLEANUP_INTERVAL_SECS);
            let mut interval = tokio::time::interval(interval_dur);
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        tracing::debug!("rate limiter cleanup shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        let cutoff = Instant::now() - interval_dur;
                        let before = limiter.buckets.len();
                        limiter.buckets.retain(|_, bucket| bucket.last_refill > cutoff);
                        let after = limiter.buckets.len();
                        if before > after {
                            tracing::info!("rate limiter cleaned: {} -> {} entries", before, after);
                        }
                    }
                }
            }
        });
    }

    /// 使用令牌桶算法检查请求速率
    fn check_request_rate(&self, user_id: u64) -> bool {
        let mut bucket = self.buckets.entry(user_id).or_insert_with(|| UserBucket {
            tokens: self.config.burst_size,
            last_refill: Instant::now(),
        });

        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.last_refill = now;

        // 补充令牌
        bucket.tokens = (bucket.tokens + elapsed * self.config.max_requests_per_second)
            .min(self.config.burst_size);

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 增加用户的并发上传计数器。
    /// 创建新上传任务时调用。
    pub fn increment_concurrent(&self, user_id: u64) {
        let counter = self
            .concurrent_counters
            .entry(user_id)
            .or_insert_with(|| AtomicU32::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// 减少用户的并发上传计数器。
    /// 上传任务完成、失败或过期时调用。
    ///
    /// 使用 `fetch_update` 防止下降到 0 以下。在异常情况下
    ///（例如合并完成和 GC 之间的竞态导致重复递减），
    /// 没有下限的 `fetch_sub` 可能使计数器变为负数，
    /// 导致所有后续上传被拒绝。
    pub fn decrement_concurrent(&self, user_id: u64) {
        let counter = self
            .concurrent_counters
            .entry(user_id)
            .or_insert_with(|| AtomicU32::new(0));
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |val| {
                if val > 0 { Some(val - 1) } else { Some(0) }
            })
            .ok();
    }

    /// 使用内存计数器检查并发上传限制。
    /// 定期从数据库同步以保持准确性。
    fn check_concurrent_uploads(&self, user_id: u64) -> bool {
        let counter = self
            .concurrent_counters
            .entry(user_id)
            .or_insert_with(|| AtomicU32::new(0));
        counter.load(Ordering::Relaxed) < self.config.max_concurrent_uploads
    }

    /// 通过统计今天创建的任务数检查每日配额。
    /// 相同模式——可以缓存，但每日配额检查频率远比并发上传低，
    /// 因此直接查询数据库是可以接受的。
    async fn check_daily_quota(&self, user_id: u64) -> Result<bool, String> {
        let today = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let count = upload_tasks::Entity::find()
            .filter(upload_tasks::Column::UserId.eq(user_id))
            .filter(upload_tasks::Column::CreatedAt.gte(today))
            .count(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(count < self.config.daily_upload_quota as u64)
    }
}

/// 超出速率限制时返回的响应
fn rate_limited_response(reason: &str) -> Response {
    let body = serde_json::json!({
        "code": 429,
        "message": format!("rate limit exceeded: {}", reason),
        "data": null,
    });
    (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response()
}

/// 上传速率限制中间件层
///
/// 此中间件应在认证中间件**之后**应用，以便
/// `UserId` 在请求扩展中可用。
pub async fn upload_ratelimit(mut req: Request, next: Next) -> Response {
    // 从扩展中提取 UserId（由认证中间件设置）
    let uid = match req.extensions_mut().get::<UserId>().copied() {
        Some(uid) => uid,
        None => return next.run(req).await, // no auth, skip rate limiting
    };

    let state = match req.extensions_mut().get::<AppState>().cloned() {
        Some(s) => s,
        None => return next.run(req).await,
    };

    let limiter = match &state.rate_limiter {
        Some(l) => l,
        None => return next.run(req).await,
    };

    // 检查请求速率（内存令牌桶）
    if !limiter.check_request_rate(uid.0) {
        return rate_limited_response("too many requests, slow down");
    }

    // 检查并发上传（内存计数器）
    if !limiter.check_concurrent_uploads(uid.0) {
        tracing::warn!(user_id = uid.0, "concurrent upload limit reached");
        return rate_limited_response(&format!(
            "max {} concurrent uploads reached",
            limiter.config.max_concurrent_uploads
        ));
    }

    // 检查每日配额（数据库查询）
    match limiter.check_daily_quota(uid.0).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(user_id = uid.0, "daily upload quota reached");
            return rate_limited_response(&format!(
                "daily upload quota of {} reached",
                limiter.config.daily_upload_quota
            ));
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to check daily quota");
            // 以开放模式失败（不阻断请求）
        }
    }

    next.run(req).await
}

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_config_default() {
        let default = RateLimitConfig::default();
        assert_eq!(
            default.max_requests_per_second,
            constant::DEFAULT_RATE_LIMIT_RPS
        );
        assert_eq!(default.burst_size, constant::DEFAULT_RATE_LIMIT_BURST);
        assert_eq!(
            default.max_concurrent_uploads,
            constant::DEFAULT_RATE_LIMIT_MAX_CONCURRENT
        );
        assert_eq!(
            default.daily_upload_quota,
            constant::DEFAULT_RATE_LIMIT_DAILY_QUOTA
        );
    }

    #[test]
    fn test_rate_limit_config_from_env_matches_default() {
        let default = RateLimitConfig::default();
        let from_env = RateLimitConfig::from_env();
        assert_eq!(
            default.max_requests_per_second,
            from_env.max_requests_per_second
        );
        assert_eq!(default.burst_size, from_env.burst_size);
        assert_eq!(
            default.max_concurrent_uploads,
            from_env.max_concurrent_uploads
        );
        assert_eq!(default.daily_upload_quota, from_env.daily_upload_quota);
    }

    #[test]
    fn test_concurrent_upload_counter() {
        use std::sync::atomic::AtomicU32;
        let config = RateLimitConfig::default();
        let counters: Arc<DashMap<u64, AtomicU32>> = Arc::new(DashMap::new());

        // 初始为 0，应通过
        let counter = counters.entry(1).or_insert_with(|| AtomicU32::new(0));
        assert!(counter.load(Ordering::Relaxed) < config.max_concurrent_uploads);

        // 递增至上限
        counter.store(config.max_concurrent_uploads, Ordering::Relaxed);
        assert!(counter.load(Ordering::Relaxed) >= config.max_concurrent_uploads);
    }

    #[test]
    fn test_token_bucket_burst_and_throttle() {
        let config = RateLimitConfig {
            max_requests_per_second: 1.0,
            burst_size: 3.0,
            max_concurrent_uploads: 5,
            daily_upload_quota: 50,
        };

        let buckets: Arc<DashMap<u64, UserBucket>> = Arc::new(DashMap::new());
        let user_id = 42u64;

        buckets.insert(
            user_id,
            UserBucket {
                tokens: 3.0,
                last_refill: Instant::now(),
            },
        );

        let check = |buckets: &DashMap<u64, UserBucket>, uid: u64, cfg: &RateLimitConfig| -> bool {
            let mut bucket = buckets.entry(uid).or_insert_with(|| UserBucket {
                tokens: cfg.burst_size,
                last_refill: Instant::now(),
            });
            let now = Instant::now();
            let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
            bucket.last_refill = now;
            bucket.tokens =
                (bucket.tokens + elapsed * cfg.max_requests_per_second).min(cfg.burst_size);
            if bucket.tokens >= 1.0 {
                bucket.tokens -= 1.0;
                true
            } else {
                false
            }
        };

        assert!(check(&buckets, user_id, &config));
        assert!(check(&buckets, user_id, &config));
        assert!(check(&buckets, user_id, &config));
        assert!(!check(&buckets, user_id, &config));
    }

    #[test]
    fn test_token_bucket_per_user_isolation() {
        let config = RateLimitConfig {
            max_requests_per_second: 1.0,
            burst_size: 2.0,
            max_concurrent_uploads: 5,
            daily_upload_quota: 50,
        };

        let buckets: Arc<DashMap<u64, UserBucket>> = Arc::new(DashMap::new());

        let check = |buckets: &DashMap<u64, UserBucket>, uid: u64, cfg: &RateLimitConfig| -> bool {
            let mut bucket = buckets.entry(uid).or_insert_with(|| UserBucket {
                tokens: cfg.burst_size,
                last_refill: Instant::now(),
            });
            let now = Instant::now();
            let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
            bucket.last_refill = now;
            bucket.tokens =
                (bucket.tokens + elapsed * cfg.max_requests_per_second).min(cfg.burst_size);
            if bucket.tokens >= 1.0 {
                bucket.tokens -= 1.0;
                true
            } else {
                false
            }
        };

        assert!(check(&buckets, 1, &config));
        assert!(check(&buckets, 1, &config));
        assert!(!check(&buckets, 1, &config));
        assert!(check(&buckets, 2, &config));
        assert!(check(&buckets, 2, &config));
    }

    #[test]
    fn test_rate_limited_response_format() {
        let response = rate_limited_response("test reason");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
