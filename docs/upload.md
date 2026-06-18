# 文件上传链路

本文档详细描述 buckets 文件上传的完整链路，从 CLI 客户端到服务端各阶段的设计与实现。

---

## 目录

1. [总览](#总览)
2. [数据库模型](#数据库模型)
3. [API 路由与中间件](#api-路由与中间件)
4. [阶段一：STS — 获取上传会话凭证](#阶段一sts--获取上传会话凭证)
5. [阶段二：Precheck — 去重与断点续传](#阶段二precheck--去重与断点续传)
6. [阶段三：Chunk Upload — 分块上传](#阶段三chunk-upload--分块上传)
7. [阶段四：Merge — 异步合并](#阶段四merge--异步合并)
8. [阶段五：Merge Status — 轮询合并进度](#阶段五merge-status--轮询合并进度)
9. [客户端上传流程](#客户端上传流程)
10. [后台任务与缓存](#后台任务与缓存)
11. [签名机制](#签名机制)
12. [错误处理与重试](#错误处理与重试)

---

## 总览

```
CLI 客户端 (buckets-cli)                      服务端 (buckets-srv)
─────────────────────────────────               ──────────────────────────

  ① POST /api/v1/upload/sts     ──────────►  sts.rs → auth_svc::issue_sts
     (获取会话签名)                              ├─ 生成 object_id (UUID v4)
                                                ├─ 计算 chunk_count
                                                ├─ 动态过期时间
                                                ├─ 创建 upload_tasks 记录
                                                └─ 生成 session 级 HMAC 签名

  ② POST /api/v1/upload/precheck ──────────►  precheck.rs → file_svc::precheck
     (去重/断点续传检查)                         ├─ MD5 全局去重 → 秒传
                                                ├─ 断点续传 → 返回已上传 chunks
                                                └─ 新建 upload_tasks 记录

  ③ POST /api/v1/upload/chunk/upload-binary ─►  chunk.rs → chunk_svc::upload_chunk_binary_stream
     (并行分块上传，带 session 签名)              ├─ 查 task 并校验归属
                                                ├─ 验证 session_signature (secret_key 缓存)
                                                ├─ 会话活性检查 (动态超时)
                                                ├─ 流式写盘 (body stream → tmp file)
                                                ├─ MD5 校验
                                                ├─ 原子 rename (tmp → 最终路径)
                                                └─ 更新内存 bitmap (批量 flush DB)

  ④ POST /api/v1/upload/merge    ──────────►  merge.rs → file_svc::merge
     (触发异步合并)                              ├─ 202 Accepted 立即返回
                                                ├─ tokio::spawn 后台合并
                                                ├─ 全量 chunk 校验 (bitmap)
                                                ├─ spawn_blocking I/O (零拷贝)
                                                ├─ 文件级 MD5 验证
                                                 ├─ 原子 rename → buckets 表
                                                └─ 清理 staging 目录

  ⑤ GET  /api/v1/upload/merge/status ──────►  merge.rs → merge_status
     (轮询合并进度)                              └─ 返回 status + storage_path
```

### 状态机

```
initialized ──► uploading ──► merging ──► completed
    │                            │
    └────────── expired          └────── failed
```

---

## 数据库模型

### `upload_tasks` 表

```sql
CREATE TABLE upload_tasks (
    id               BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    uuid             VARCHAR(36)  NOT NULL,
    object_id        VARCHAR(36)  NOT NULL,
    file_md5         VARCHAR(32)  NOT NULL,
    file_size        BIGINT       NOT NULL,
    chunk_size       INT          NOT NULL,
    chunk_count      INT          NOT NULL,
    user_id          BIGINT UNSIGNED NOT NULL,
    status           VARCHAR(32)  NOT NULL DEFAULT 'initialized',
    uploaded_bitmap  JSON         NOT NULL DEFAULT ('[]'),
    last_activity_at BIGINT       NULL,
    created_at       TIMESTAMP(6) NOT NULL,
    updated_at       TIMESTAMP(6) NOT NULL,
    expires_at       TIMESTAMP(6) NOT NULL,
    UNIQUE INDEX idx_upload_tasks_uuid (uuid),
    INDEX idx_upload_tasks_md5 (file_md5),
    INDEX idx_upload_tasks_user (user_id),
    INDEX idx_upload_tasks_expires (expires_at),
    INDEX idx_upload_tasks_status_expires (status, expires_at),
    CONSTRAINT fk_upload_tasks_user FOREIGN KEY (user_id) REFERENCES users(id)
);
```

### `UploadTask` 结构体

```rust
pub struct UploadTask {
    pub id: u64,
    pub uuid: String,
    pub object_id: String,
    pub file_md5: String,
    pub file_size: i64,
    pub chunk_size: i64,
    pub chunk_count: i64,
    pub user_id: u64,
    pub status: String,            // "initialized" | "uploading" | "merging" | "completed" | "failed" | "expired"
    pub uploaded_bitmap: String,   // JSON array of u64 words
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_activity_at: Option<i64>,
}
```

### Bitmap 编码

每个 chunk 的上传状态用一个 bit 表示：
- `Vec<u64>` 作为 JSON 存储在 `uploaded_bitmap` 字段
- 每个 u64 管理 64 个 chunk
- bit = 1 表示已上传，0 表示未上传
- word_index = chunk_index / 64, bit = chunk_index % 64

```rust
pub fn parse_bitmap(&self) -> Vec<u64> {
    if self.uploaded_bitmap.is_empty() || self.uploaded_bitmap == "[]" {
        let word_count = (self.chunk_count as usize).div_ceil(64);
        return vec![0u64; word_count];
    }
    serde_json::from_str(&self.uploaded_bitmap).unwrap_or_else(|_| {
        let word_count = (self.chunk_count as usize).div_ceil(64);
        vec![0u64; word_count]
    })
}
```

### `objects` 表

合并完成后写入的最终对象表：

```sql
CREATE TABLE objects (
    id            BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    uuid          VARCHAR(36)  NOT NULL,
    name          VARCHAR(1024) NOT NULL,
    size          BIGINT       NOT NULL,
    md5           VARCHAR(64)  NOT NULL,
    content_type  VARCHAR(256),
    extension     VARCHAR(64),
    bucket        VARCHAR(256) NOT NULL DEFAULT 'default',
    storage_path  VARCHAR(1024),
    image_width   INT          NOT NULL DEFAULT 0,
    image_height  INT          NOT NULL DEFAULT 0,
    image_type    VARCHAR(32)  NOT NULL DEFAULT '',
    status        VARCHAR(32)  NOT NULL DEFAULT 'active',
    created_at    TIMESTAMP(6) NOT NULL,
    updated_at    TIMESTAMP(6) NOT NULL,
    UNIQUE INDEX idx_objects_uuid (uuid),
    INDEX idx_objects_md5 (md5)
);
```

### `user_objects` 关联表

多对多关系，支持同一文件被多个用户引用：

```sql
CREATE TABLE user_objects (
    id         BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    user_id    BIGINT UNSIGNED NOT NULL,
    object_id  BIGINT UNSIGNED NOT NULL,
    created_at TIMESTAMP(6) NOT NULL,
    UNIQUE INDEX idx_user_objects (user_id, object_id)
);
```

---

## API 路由与中间件

### 路由定义 (`buckets-srv/src/api/mod.rs`)

上传路由挂载在 `/api/v1` 下：

| 方法 | 路径 | Handler | 说明 |
|------|------|---------|------|
| `POST` | `/upload/sts` | `sts::get_sts_token` | 获取 STS 会话令牌 |
| `POST` | `/upload/precheck` | `precheck::precheck_file` | 上传前预检 |
| `POST` | `/upload/chunk/upload-binary` | `chunk::upload_chunk_binary` | 二进制分块上传 (流式) |
| `POST` | `/upload/chunk/status` | `chunk::chunk_status` | 查询分块上传进度 |
| `POST` | `/upload/merge` | `merge::merge_chunks` | 触发异步合并 |
| `GET` | `/upload/merge/status` | `merge::merge_status` | 轮询合并状态 |

### 上传路由中间件层

```rust
let upload_routes = Router::new()
    .route("/upload/sts", post(sts::get_sts_token))
    // ...
    // 上传速率限制
    .layer(mw::from_fn(ratelimit::upload_ratelimit))
    // Body 大小限制 (默认 256 MiB + overhead)
    .layer(DefaultBodyLimit::max(max_body));
```

### 三层速率限制 (`ratelimit::upload_ratelimit`)

1. **Token Bucket 请求限流** — 基于用户 ID 的令牌桶算法，限制每秒请求数
2. **并发上传计数** — 跟踪活跃的 `upload_tasks` 数量，超出限制则拒绝
3. **每日配额** — 查询今日创建的 `upload_tasks` 数量，超出日配额则拒绝

### 全局中间件

所有请求经过（`app.rs` 中定义的顺序）：
1. CORS
2. Tracing
3. Compression
4. Request Logger
5. Request Timeout (来自 `constant::REQUEST_TIMEOUT_SECS`)
6. Extensions Layer (注入 `AuthCache`, `SecretKeyCache`, `DatabaseConnection`, `AppConfig`, `JwtFailLimiter`, `BasicAuthFailLimiter`)
7. Auth Middleware (JWT Bearer / Basic Auth)

---

## 阶段一：STS — 获取上传会话凭证

### 端点

```
POST /api/v1/upload/sts
Authorization: Bearer <token>  (或 Basic base64(email:password))
Content-Type: application/json
```

### 请求

```json
{
    "file_name": "example.mp4",
    "file_size": 1073741824,
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "chunk_size": 8388608
}
```

### 响应

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "object_key": "1/2026/06/14/660e8400-e29b-41d4-a716-446655440001.mp4",
        "session_signature": "a1b2c3d4e5f6...",
        "session_timestamp": 1718313600,
        "session_salt": "770e8400-e29b-41d4-a716-446655440002"
    }
}
```

### 服务端处理流程 (`auth_svc::issue_sts`)

```
1. 校验 file_size > 0
2. 生成 object_id = Uuid::new_v4()
3. 计算 chunk_count = file_size.div_ceil(chunk_size)
4. 动态过期时间:
   size_gb = file_size / (1024³)
   expiration_hours = 72 + ceil(size_gb / 10) * 24
   → 最小 72 小时，每 10GB 增加 24 小时
5. 创建 upload_tasks 记录 (status = "initialized")
6. 生成 object_key = "{user_id}/{YYYY}/{MM}/{DD}/{object_id}.{ext}"
7. 从 DB 获取用户 secret_key
8. 生成 session 级 HMAC-SHA256 签名:
   message = "session:{user_id}:{task_id}:{file_md5}:{chunk_size}:{timestamp}:{salt}"
   signature = HMAC-SHA256(secret_key, message)
9. 返回 StsResult
```

### 关键设计

- **一个 session 签名覆盖所有 chunk** — 不需要每个 chunk 单独签名
- **签名有效期 2 小时** — 由 `verify_session_timestamp()` 校验
- **salt 随机生成** — 每次 STS 调用的签名不同，防止重放

---

## 阶段二：Precheck — 去重与断点续传

### 端点

```
POST /api/v1/upload/precheck
Authorization: Bearer <token>
Content-Type: application/json
```

### 请求

```json
{
    "file_name": "example.mp4",
    "file_size": 1073741824,
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "chunk_size": 8388608
}
```

### 响应 — 秒传（文件已存在）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "exists": true,
        "object_id": "660e8400-...",
        "storage_path": "/data/objects/1/2026/06/14/660e8400-....mp4",
        "task_id": null,
        "uploaded_chunks": [],
        "chunk_size": 8388608
    }
}
```

### 响应 — 断点续传

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "exists": false,
        "object_id": "770e8400-...",
        "storage_path": null,
        "task_id": "880e8400-...",
        "uploaded_chunks": [0, 1, 2, 5, 8],
        "chunk_size": 8388608
    }
}
```

### 响应 — 新上传

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "exists": false,
        "object_id": "990e8400-...",
        "storage_path": null,
        "task_id": "aa0e8400-...",
        "uploaded_chunks": [],
        "chunk_size": 8388608
    }
}
```

### 服务端处理流程 (`file_svc::precheck`)

```
1. 校验文件扩展名 (validate_file_extension)
2. 磁盘空间检查 (spawn_blocking 避免阻塞 runtime)
3. 软限制告警 (超过 1 TiB 记录 warn 日志)
4. 全局去重:
   SELECT * FROM objects WHERE md5 = ? AND bucket = ? AND status = 'active' AND size = ?
    → 找到 → 插入 user_objects 关联 → 返回 exists=true (秒传)
5. 断点续传检查:
   SELECT * FROM upload_tasks
   WHERE file_md5 = ? AND user_id = ? AND chunk_size = ?
   AND status NOT IN ('completed', 'expired', 'failed')
   ORDER BY created_at DESC LIMIT 1
   → 找到 → 校验 staging 目录存在 → 解析 bitmap → 返回已上传 chunks
   → staging 目录不存在 → 回退到新建任务
6. 新建任务:
   - 生成 object_id
   - 动态过期时间 (同 STS 逻辑)
   - INSERT INTO upload_tasks
   → 返回空 uploaded_chunks
```

### 三条分支决策树

```
precheck(file_md5, file_size, user_id, chunk_size, file_name)
│
├─ objects 表有相同 MD5 + size?
│  └─ YES → 秒传 (exists=true, object_id=已有, 关联 user_objects)
│
├─ upload_tasks 表有同用户同 MD5 且 chunk_size 匹配?
│  ├─ YES → staging 目录存在?
│  │  └─ YES → 断点续传 (task_id=已有, uploaded_chunks=已上传列表)
│  │  └─ NO  → 新建任务
│  └─ NO  → 新建任务
│
└─ 新建 upload_tasks 记录 (status=initialized)
```

---

## 阶段三：Chunk Upload — 分块上传

### 端点

```
POST /api/v1/upload/chunk/upload-binary?task_id=<uuid>&chunk_index=0&chunk_md5=<hex>
Authorization: Bearer <token>  (或 Basic)
X-Session-Signature: <session_signature>
X-Session-Timestamp: <unix_timestamp>
X-Session-Salt: <random_uuid>
Content-Type: application/octet-stream
Body: <raw chunk bytes>
```

> 注意：session 签名参数通过 HTTP Headers 传递，而非 URL 参数，避免 URL 长度限制和日志泄露。

### 响应

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "chunk_index": 0,
        "status": "uploaded",
        "md5": "a1b2c3d4e5f6..."
    }
}
```

### 服务端处理流程 (`chunk_svc::upload_chunk_binary_stream`)

```
1. 查 task
   dao::find_upload_task(db, task_id)
   → 不存在 → 404
   → task.user_id != user_id → 403

2. 获取 secret_key (带缓存)
   先从 SecretKeyCache (DashMap, TTL=30min) 查
   → miss → dao::get_user_secret_key(db, user_id) → 回填缓存
   → hit  → 直接用缓存值

3. 验证 session 签名
   构造 SessionSignInput { user_id, task_id, file_md5, chunk_size, timestamp, salt }
   crypto::verify_session_signature(secret_key, &input, &signature)
   → 失败 → 401 SignatureInvalid
   verify_session_timestamp(timestamp)  // 2小时有效期

4. 会话活性检查
   skip if status == "initialized" (第一个 chunk)
   elapsed = now - last_activity_at
   timeout = 3600 + ceil(size_gb/10) * 3600, capped at 172800 (48h)
   → 超时 → 标记 expired → 400 "upload session expired"

5. 幂等检查
   如果 chunk 文件已存在于 staging 目录:
   → 更新内存 bitmap → 返回 "already_exists"

6. 流式写盘
   stream = body.into_data_stream()
   tmp_path = chunk_path.with_extension("tmp")
   while let Some(data) = stream.next():
       write_all(&data)
       累计 total_bytes
       → total_bytes > chunk_size → 删除 tmp → 400 "chunk size exceeds limit"
       → ENOSPC → 返回 StorageError (磁盘满)
   file.flush()  // 确保数据落盘

7. MD5 校验 (从磁盘文件读取，不占内存)
   compute_file_md5_sync(&tmp_path)  // 64KB buffer 流式计算
   → computed_md5 != chunk_md5 → 删除 tmp → 400 HashMismatch

8. 原子 rename
   tokio::fs::rename(&tmp_path, &chunk_path)
   → 失败 → 清理 tmp → StorageError

9. 写 chunk MD5 sidecar 文件
   write_chunk_md5_sidecar(task_id, chunk_index, &computed_md5)
   → 供 merge 阶段使用，避免重新读取 chunk 计算 MD5

10. 更新内存 bitmap
    set_bit_in_memory(&bitmap, chunk_index, task_id)
    → words[word_index] |= 1 << bit
    → dirty = true (标记需要 flush)

11. 批量记录 last_activity_at
    pending_activity.insert(task_id)  // 由 flush 任务批量更新 DB

12. 首次上传标记
    if status == "initialized" → update status to "uploading"
```

### 内存 Bitmap 缓存

为避免每次 chunk 上传都写 DB，使用全局内存 bitmap 缓存：

```rust
// 全局 DashMap，key = task_id
static BITMAP_CACHE: OnceLock<DashMap<String, Arc<RwLock<TaskBitmap>>>>;

struct TaskBitmap {
    words: Vec<u64>,     // bitmap 数据
    chunk_count: u32,    // 总 chunk 数
    dirty: bool,         // 是否有未 flush 的变更
}
```

**flush 机制**：
- 后台定时任务每 5 秒触发 `flush_dirty_bitmaps()`
- 使用 `DIRTY_TASK_IDS` HashSet 跟踪需要 flush 的 task，避免全量遍历 DashMap
- 同时批量更新 `last_activity_at`（多个 task 合并为一条 SQL）
- 所有 chunk 上传完毕的 task 从缓存中移除

**缓存淘汰**：
- 后台任务每 5 分钟检查缓存大小
- 超过 `BITMAP_CACHE_MAX_ENTRIES` (500) 时，驱逐 clean (非 dirty) 条目

### Secret Key 缓存

同一用户连续上传多个 chunk，每次都查 DB 获取 `secret_key` 是浪费。使用 `SecretKeyCache`：

```rust
pub type SecretKeyCache = Arc<DashMap<String, SecretKeyCacheEntry>>;

struct SecretKeyCacheEntry {
    secret_key: String,
    expires_at: Instant,  // TTL = 30 分钟
}
```

- 第一个 chunk 查 DB → 缓存
- 后续 chunk 直接从内存命中
- 后台定期清理过期条目

---

## 阶段四：Merge — 异步合并

### 端点

```
POST /api/v1/upload/merge
Authorization: Bearer <token>
Content-Type: application/json
```

### 请求

```json
{
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "file_name": "example.mp4",
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "file_size": 1073741824,
    "content_type": "video/mp4"
}
```

### 响应 (202 Accepted)

```json
{
    "code": 202,
    "message": "merge accepted",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "message": "merge started, poll /upload/merge/status for completion"
    }
}
```

### 服务端处理流程 (`file_svc::merge`)

```
1. 前置校验
   - 查 task → 校验 user_id 归属
   - 防止重复合并 (status == "merging" || "completed" → 400)
   - 全量 chunk 校验:
     优先从内存 bitmap 缓存读取
     回退到 DB uploaded_bitmap
     遍历所有 chunk_index，检查每个 bit
     → 有缺失 → 400 UploadIncomplete(missing_count)

2. 标记 "merging"
   update_upload_status(task_id, "merging")

3. 磁盘空间检查 (spawn_blocking)

4. 获取合并信号量
   MAX_CONCURRENT_MERGES = 4
   超时等待 = REQUEST_TIMEOUT_SECS

5. spawn_blocking 执行 I/O 合并 (do_merge_io)
   在独立线程上运行，避免阻塞 tokio runtime

6. do_merge_io 详细流程:
   for i in 0..chunk_count:
       a. 读取 chunk MD5 sidecar 文件
          → 有: hasher.update(md5_hex.as_bytes())  // 零拷贝路径
          → 无: 读取 chunk 数据计算 MD5           // 兼容旧数据
       b. 零拷贝合并 (Linux copy_file_range)
          → 成功: total_written += chunk_size
          → 失败: fallback 到 BufWriter 读写
   flush + fsync 输出文件
   返回 (computed_md5, total_written)

7. MD5 校验
   computed_md5 != file_md5 → 删除 temp → 标记 failed → 保留 staging 文件供重试

8. 原子 rename (temp → final path)

9. fsync 父目录 (确保 rename 持久化)

10. MIME 类型检测 (从扩展名推断)

11. 图像尺寸检测 (spawn_blocking 读取 magic bytes)

12. DB 事务写入:
    BEGIN
    INSERT INTO objects (...)
    INSERT INTO user_objects (user_id, object_id)
    UPDATE upload_tasks SET status = 'completed'
    COMMIT

13. 清理
    - 从 bitmap 缓存中移除 task
    - 删除 staging 目录
```

### 零拷贝合并 (Linux)

```rust
// 使用 copy_file_range syscall 在 kernel 空间直接复制数据
// 避免用户态/内核态数据拷贝，大幅减少 I/O 开销
unsafe {
    libc::copy_file_range(
        src_fd, ptr::null_mut(),
        dst_fd, ptr::null_mut(),
        remaining, 0,
    )
}
```

### 文件级 MD5 计算

文件 MD5 不是对整个文件内容计算，而是对 chunk MD5 的拼接计算（Merkle 树根）：

```
file_md5 = MD5(chunk_md5_hex[0] || chunk_md5_hex[1] || ... || chunk_md5_hex[N-1])
```

这允许 merge 阶段从 sidecar 文件读取 chunk MD5 而不重新读取 chunk 数据，配合零拷贝路径将磁盘 I/O 减半。

### 合并失败处理

- **不删除 staging 文件** — 保留供重试，GC 在过期后统一清理
- 标记 status = "failed"
- 客户端可以重新调用 merge

---

## 阶段五：Merge Status — 轮询合并进度

### 端点

```
GET /api/v1/upload/merge/status?task_id=<uuid>
Authorization: Bearer <token>
```

### 响应

**合并中**：
```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-...",
        "status": "merging",
        "storage_path": null
    }
}
```

**合并完成**：
```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-...",
        "status": "completed",
        "storage_path": "/data/objects/1/2026/06/14/660e8400-....mp4"
    }
}
```

**合并失败**：
```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-...",
        "status": "failed",
        "storage_path": null
    }
}
```

### 客户端轮询策略

```rust
// 指数退避轮询
let mut delay = Duration::from_secs(1);
let mut attempts = 0;
loop {
    let resp = poll_merge_status().await?;
    match resp.status {
        "completed" => return resp.storage_path,
        "failed" => bail!("merge failed"),
        "merging" => {
            attempts += 1;
            delay = min(delay * 2, MAX_POLL_INTERVAL);
            sleep(delay).await;
        }
        _ => bail!("unexpected status"),
    }
}
```

---

## 客户端上传流程

### CLI 客户端 (`buckets-cli`)

```
upload_file(file_path, object_name, chunk_size_mb, parallel, resume)
│
├─ 1. 文件信息
│     file_size = metadata.len()
│     chunk_size = chunk_size_mb * 8MiB
│     chunk_count = file_size.div_ceil(chunk_size)
│
├─ 2. 预计算 MD5 (spawn_blocking)
│     逐 chunk 读取文件 → 计算每个 chunk 的 MD5
│     file_md5 = MD5(concat of all chunk_md5_hex strings)
│     → 一次读取完成所有 chunk MD5 计算
│
├─ 3. STS (get_sts_token)
│     POST /upload/sts
│     → 获取 session_signature + task_id
│
├─ 4. Precheck (precheck_file)
│     POST /upload/precheck
│     → exists=true → 秒传完成
│     → 获取 uploaded_chunks 列表
│
├─ 5. 并行 Chunk 上传
│     missing = [0..chunk_count] - uploaded_chunks
│     semaphore = Semaphore::new(parallel)
│     for idx in missing:
│         tokio::spawn:
│           ├─ 计算 chunk MD5 (streaming, 64KB buffer)
│           ├─ open_chunk_stream(file, offset, chunk_size)
│           ├─ upload_chunk_streaming()
│           │   POST /upload/chunk/upload-binary
│           │   Headers: X-Session-Signature, X-Session-Timestamp, X-Session-Salt
│           │   Body: raw bytes stream (ReaderStream)
│           │   Retry: 最多 3 次，指数退避 + 随机 jitter
│           └─ progress.inc()
│
├─ 6. 触发 Merge
│     POST /upload/merge
│     → 202 Accepted
│
└─ 7. 轮询 Merge Status
      GET /upload/merge/status
      指数退避轮询 → completed → 返回 object URL
```

### 并行上传控制

```rust
let semaphore = Arc::new(Semaphore::new(parallel));  // 默认 4 并发
for idx in missing_indices {
    let permit = semaphore.clone().acquire_owned().await?;
    tokio::spawn(async move {
        let _permit = permit;  // 持有到 chunk 上传完成
        // ... upload chunk ...
    });
}
```

### 断点续传

- 客户端本地缓存上次上传的 task 信息（文件路径、chunk_size）
- `resume` 命令读取缓存，重新执行 precheck 获取已上传 chunks
- 只上传缺失的 chunks

---

## 后台任务与缓存

### 全局缓存架构

```
┌──────────────────────────────────────────────────────┐
│                    请求处理                           │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │
│  │ AuthCache    │  │ SecretKeyCache│  │ BitmapCache│ │
│  │ (DashMap)    │  │ (DashMap)    │  │ (DashMap)  │ │
│  │ email:pw→uid │  │ uid→secret   │  │ task→bitmap│ │
│  │ TTL: 30min   │  │ TTL: 30min   │  │ dirty flag │ │
│  └──────────────┘  └──────────────┘  └────────────┘ │
│                                              │        │
│                                    PENDING_ACTIVITY   │
│                                    DIRTY_TASK_IDS     │
└──────────────────────────────────────────────────────┘
                        │
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
     Auth Cache    Secret Key     Bitmap Flush
     Cleaner       Cache Cleaner  Task (5s)
     (30min)       (30min)        + Cache Cleanup
                                   (5min, cap=500)
```

### Bitmap Flush 任务

```rust
pub fn start_bitmap_flush_task(db, cancellation) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    flush_dirty_bitmaps(&db).await;  // 最终 flush
                    break;
                }
                _ = interval.tick() => {
                    flush_dirty_bitmaps(&db).await;
                }
            }
        }
    });
}
```

**批量 flush 策略**：
1. 收集 `PENDING_ACTIVITY` → 一条 SQL 批量更新多个 task 的 `last_activity_at`
2. 收集 `DIRTY_TASK_IDS` → 只遍历 dirty 的 task（避免全量 DashMap 遍历）
3. 每个 dirty task → `UPDATE upload_tasks SET uploaded_bitmap = ?`
4. 全部上传完毕的 task → 从缓存移除

### 优雅关闭

所有后台任务通过 `CancellationToken` 绑定到主进程生命周期：
- `start_bitmap_flush_task` — 收到 cancel 后执行最终 flush
- `start_bitmap_cache_cleanup` — 收到 cancel 后退出
- `start_auth_cache_cleaner` — 收到 cancel 后退出
- `start_secret_key_cache_cleaner` — 收到 cancel 后退出

---

## 签名机制

### Session 签名 (HMAC-SHA256)

用于授权整个上传会话中的所有 chunk 上传。

**生成** (`crypto::generate_session_signature`)：
```rust
message = "session:{user_id}:{task_id}:{file_md5}:{chunk_size}:{timestamp}:{salt}"
signature = hex(HMAC-SHA256(secret_key, message))
```

**验证** (`crypto::verify_session_signature`)：
```rust
expected = generate_session_signature(secret_key, input)
return expected == signature
```

**时间戳验证** (`crypto::verify_session_timestamp`)：
```rust
if |now - timestamp| > 7200 {  // 2 小时
    return SignatureExpired
}
```

### JWT Auth Token

用于 API 认证（非上传签名）：

- **Access Token**: HS256，用 `users.secret_key` 签名，7 天有效
  ```json
  header: { "alg": "HS256", "kid": "user_id" }
  payload: { "sub": user_id, "iat": ..., "exp": ..., "jti": "uuid" }
  ```
- **Refresh Token**: HS256，用全局 `REFRESH_TOKEN_KEY` 签名，7 天有效

### 密钥体系

```
┌─────────────────────────────────────────────┐
│              users.secret_key                │
│         (per-user, 64-char hex)              │
│                                              │
│   ├── JWT Access Token 签名 (HS256)         │
│   ├── Session 签名 (HMAC-SHA256)            │
│   └── 默认值: DEFAULT_SECRET_KEY             │
│       (可通过 RESET SECRET KEY 重置)          │
├─────────────────────────────────────────────┤
│           REFRESH_TOKEN_KEY                   │
│         (global server key)                  │
│                                              │
│   └── JWT Refresh Token 签名 (HS256)         │
└─────────────────────────────────────────────┘
```

---

## 错误处理与重试

### 服务端错误映射

| 场景 | HTTP Status | AppError |
|------|-------------|----------|
| Task 不存在 | 404 | `NotFound` |
| Task 不属于用户 | 403 | `Forbidden` |
| Session 签名无效 | 401 | `SignatureInvalid` |
| Session 签名过期 | 401 | `SignatureExpired` |
| Chunk MD5 不匹配 | 400 | `HashMismatch` |
| Chunk 大小超限 | 400 | `BadRequest` |
| 磁盘空间不足 | 507 | `StorageError(ENOSPC)` |
| 上传未完成 | 400 | `UploadIncomplete` |
| 合并中/已完成 | 400 | `BadRequest` |
| 会话超时 | 400 | `BadRequest(expired)` |

### 客户端重试策略

```rust
const CHUNK_UPLOAD_MAX_RETRIES: u32 = 3;
const CHUNK_UPLOAD_RETRY_BACKOFF_BASE_SECS: u64 = 1;

for attempt in 0..MAX_RETRIES {
    match upload_chunk().await {
        Ok(_) => break,
        Err(_) if attempt + 1 < MAX_RETRIES => {
            // 指数退避 + 随机 jitter
            let backoff = 1 << attempt;  // 1s, 2s, 4s
            let jitter = random() % 500ms;
            sleep(backoff + jitter).await;
        }
        Err(e) => return Err(e),
    }
}
```

---

## 关键常量汇总

| 常量 | 值 | 说明 |
|------|-----|------|
| `DEFAULT_CHUNK_SIZE` | 8 MiB | 默认分块大小 |
| `DEFAULT_MAX_CHUNK_SIZE` | 256 MiB | 最大分块大小 |
| `CHUNK_STREAM_BUFFER_SIZE` | 64 KiB | 流式 I/O buffer |
| `BITMAP_FLUSH_INTERVAL_SECS` | 5 | bitmap flush 间隔 |
| `BITMAP_CACHE_MAX_ENTRIES` | 500 | bitmap 缓存上限 |
| `MAX_CONCURRENT_MERGES` | 4 | 并发合并上限 |
| `SESSION_ACTIVITY_TIMEOUT_SECS` | 3600 (1h) | 会话活性超时 |
| `MAX_SESSION_ACTIVITY_TIMEOUT_SECS` | 172800 (48h) | 最大会话超时 |
| `MIN_UPLOAD_EXPIRATION_HOURS` | 72 | 最小上传过期时间 |
| `EXPIRATION_SCALE_HOURS_PER_10GB` | 24 | 每 10GB 增加的过期时间 |
| `AUTH_CACHE_TTL_SECS` | 1800 (30min) | 认证/secret key 缓存 TTL |
| `REQUEST_TIMEOUT_SECS` | 600 (10min) | 请求超时 |
| `FILE_SIZE_SOFT_LIMIT_WARN` | 1 TiB | 文件大小软限制 |

---

## 文件结构索引

| 文件 | 职责 |
|------|------|
| `buckets-srv/src/api/mod.rs` | 路由定义 |
| `buckets-srv/src/api/sts.rs` | STS / Object 端点 |
| `buckets-srv/src/api/precheck.rs` | Precheck 端点 |
| `buckets-srv/src/api/chunk.rs` | Chunk 上传 / 状态端点 |
| `buckets-srv/src/api/merge.rs` | Merge / Merge Status 端点 |
| `buckets-srv/src/service/auth_svc.rs` | STS 签发逻辑 |
| `buckets-srv/src/service/chunk_svc.rs` | Chunk 上传 + Bitmap 缓存 |
| `buckets-srv/src/service/file_svc.rs` | Precheck + Merge 逻辑 |
| `buckets-srv/src/dao/` | 数据库访问层（objects, tasks, users） |
| `buckets-srv/src/db.rs` | DB Connection + Auth 工具 |
| `buckets-srv/src/middleware/auth.rs` | Auth 中间件 + SecretKeyCache |
| `buckets-srv/src/middleware/ratelimit.rs` | 上传限流 |
| `buckets-srv/src/app.rs` | 路由组装 + AppState |
| `buckets-srv/src/main.rs` | 入口 + 后台任务启动 |
| `buckets-common/src/model/api.rs` | API DTOs |
| `buckets-common/src/model/db.rs` | DB 实体 |
| `buckets-common/src/utils/crypto.rs` | 签名 + JWT |
| `buckets-common/src/constant/upload.rs` | 上传常量 |
| `buckets-common/src/constant/auth.rs` | 认证常量 |
| `buckets-common/src/constant/storage.rs` | 存储常量 |
| `buckets-common/src/constant/http.rs` | HTTP 头常量 |
| `buckets-common/src/constant/task.rs` | 后台任务常量 |
| `buckets-cli/src/client/mod.rs` | CLI 上传编排 |
| `buckets-cli/src/client/chunk.rs` | CLI chunk 上传 |
| `buckets-cli/src/client/precheck.rs` | CLI STS + Precheck |
| `buckets-cli/src/client/merge.rs` | CLI Merge + 轮询 |
