# buckets 架构设计文档

## 一、整体架构

buckets 是一个 Rust 实现的轻量高性能私有对象存储服务，支持客户端分片上传、秒传（全局 MD5 去重，多用户共享同一文件通过 user_objects 关联表）、断点续传、会话级 HMAC 预签名认证、三级速率限制。采用 Cargo Workspace 多包架构，分为三个独立子包编译运行。

### 1.1 设计目标

- **高性能**：基于 tokio 异步运行时 + axum HTTP 框架，全异步无阻塞
- **大文件支持**：分片并发上传，单文件最大取决于磁盘空间（分片大小 1-256MB 可配）
- **流式写入**：分片数据通过 axum Body stream 直接写入磁盘，避免将整个分片加载到内存
- **秒传**：服务端 MD5 全局去重，多用户共享同一文件通过 `user_objects` 关联表
- **断点续传**：u64 位图记录已上传分片，中断后自动跳过已完成分片
- **会话级签名**：STS 签发会话级 HMAC-SHA256 签名（2h 有效期），一次签名覆盖整个上传会话
- **异步合并**：分片合并改为后台异步执行 + 客户端轮询，避免 HTTP 长连接超时
- **零拷贝合并**：Linux 下使用 `copy_file_range` 零拷贝合并分片，配合 MD5 侧车文件减少 I/O
- **合并并发控制**：Semaphore 限制同时合并数，防止大文件并发合并耗尽线程池
- **链路追踪**：X-Trace-Id 贯穿请求全链路，快速定位故障
- **速率限制**：Token Bucket + 并发限制 + 每日配额，三层防护
- **磁盘预检**：上传前和合并前校验可用磁盘空间；健康检查返回磁盘使用率
- **数据持久性**：合并后 fsync 文件 + 目录，确保 rename 后数据不丢失
- **请求超时**：全局请求超时中间件（600s），防止挂起连接无限占用资源
- **可观测**：结构化日志 + tracing span，每条日志携带 trace_id；健康检查返回降级状态

### 1.2 Workspace 结构

```
buckets/                  # Workspace 根
├── .env.example             # 环境变量模板
├── jssdk/                   # JavaScript SDK（@chihqiang/buckets）
│   ├── src/
│   │   ├── buckets-client.ts  # 统一入口类
│   │   ├── http-client.ts     # 传输层（fetch + 超时 + 错误解析）
│   │   ├── auth-client.ts     # 认证包装
│   │   ├── api/               # 领域 API（auth, objects, users）
│   │   ├── upload/            # 上传器（direct, chunk, tus）
│   │   └── index.ts           # 统一导出
│   ├── vite.config.ts         # Vite lib 构建（ESM + CJS + UMD）
│   └── package.json           # 包名 @chihqiang/buckets
├── web/                     # 管理后台前端（Vue 3 + Tailwind CSS）
│   ├── src/
│   │   ├── stores/          # Pinia 状态管理（auth, objects, users）+ API 单例
│   │   ├── router/          # Vue Router（登录鉴权守卫）
│   │   ├── views/           # 页面：Login, ObjectList, UserList
│   │   └── components/      # Layout（导航栏 + 路由出口）
│   ├── vite.config.ts       # Vite 8 + @tailwindcss/vite 配置
│   └── package.json
├── buckets-common/   # 公共基础库（lib）
├── buckets-srv/      # 统一服务端（bin）：Gateway + Web 管理后台
├── buckets-cli/      # 命令行客户端（bin）
├── migration/           # 数据库迁移（sea-orm-migration）
├── deploy/                  # Docker Compose（开发测试用）
└── docs/                    # 文档
```

| 子包 | 类型 | 技术栈 | 职责 |
|------|------|--------|------|
| `buckets-common` | lib | serde, chrono, uuid, sea-orm, argon2, sha2, hmac, md-5, base64, thiserror, axum | 共享模型（API DTO + sea-orm 实体）、统一错误体系（AppError）、全局配置常量、工具函数（crypto/validate/path） |
| `buckets-srv` | bin | axum 0.8, tokio, sea-orm 1.x (MySQL+SQLite), tower-http, dotenvy, dashmap, jsonwebtoken | 统一 HTTP API 服务：Gateway（文件上传网关）+ Web（管理后台），统一认证中间件 |
| `buckets-cli` | bin | clap 4, reqwest 0.12, tokio, indicatif, rpassword | 文件预处理（MD5）、流式分片上传、进度展示、断点续传、凭证管理 |

### 1.3 依赖关系

```
buckets-cli ──► buckets-common
                        ▲
                        │
buckets-srv ─────────┘
```

`buckets-common` 作为共享基础库，提供模型定义、工具函数、常量和错误类型。`buckets-srv` 和 `buckets-cli` 各自独立编译。

## 二、Server 分层架构

Server 采用统一分层架构，Gateway 和 Web 管理后台共享同一套 middleware、API、Service 层。

```
src/
├── app.rs              # AppState + 路由组装 + 请求超时中间件 + 健康检查
├── main.rs             # 入口
├── config.rs           # 环境变量配置
├── db.rs               # 数据库连接（统一 create_pool，支持 SQLite/MySQL）+ UserId
├── dao/                # 数据库访问层
│   ├── mod.rs          # 模块导出 + 认证工具（verify_user, get_user_secret_key, 密钥管理）
│   ├── objects.rs      # 对象 CRUD（MD5 查找、软删除、用户-对象关联）
│   ├── tasks.rs        # 上传任务 CRUD + GC 批量过期
│   ├── users.rs        # 用户 CRUD（查询、创建、更新、删除、密钥管理）

├── api/                # HTTP handler 层
│   ├── mod.rs          # 路由组装
│   ├── auth.rs         # 登录/刷新/登出
│   ├── sts.rs          # STS 凭证签发 + 对象信息/删除
│   ├── direct.rs       # 直接上传（multipart/form-data）
│   ├── precheck.rs     # 文件预校验（秒传/续传）
│   ├── chunk.rs        # 分片上传 + 状态查询
│   ├── merge.rs        # 合并 + 合并状态轮询
│   ├── objects.rs      # Web 对象管理
│   └── users.rs        # Web 用户管理
├── middleware/          # 中间件
│   ├── mod.rs
│   ├── auth.rs         # 统一认证（JWT Bearer + Basic Auth）
│   ├── logger.rs       # 请求日志
│   ├── ratelimit.rs    # 速率限制（上传路由）
│   └── trace.rs        # TraceID 链路追踪
├── service/            # 业务逻辑层
│   ├── auth_svc.rs     # Token 签发/刷新/吊销
│   ├── chunk_svc.rs    # 分片管理 + 位图缓存
│   ├── file_svc.rs     # 文件元数据管理 + 预校验 + 合并 + 用户-对象关联
└── task/               # 后台定时任务
    ├── gc_clean.rs     # 过期任务清理
    └── ref_check.rs    # 引用计数清理
```

### 2.0 路由结构

```
/api/v1/              → 统一路由（认证中间件共享）
  /auth/login         → 登录
  /auth/refresh       → 刷新 Token
  /auth/logout        → 登出
  /auth/verify        → 验证凭据
  /upload/sts         → 获取 STS 凭证
  /upload/precheck    → 文件预校验
  /upload/direct      → 直接上传（multipart/form-data）
  /upload/chunk/*     → 分片上传
  /upload/merge       → 合并分片
  /object/{id}        → 对象操作（STS 端点）
  /users              → 用户管理（超级管理员）
  /objects            → 对象管理（Web 管理）

/health               → 健康检查（无需认证）
/ (fallback)          → 前端静态文件（ServeDir + SPA fallback 到 index.html）
```

所有路由共享统一的认证中间件，`/auth/login` 和 `/auth/refresh` 跳过认证，速率限制仅作用于上传路由（`/upload/*`）。

### 2.1 中间件执行顺序

```
请求到达
  │
  ▼
CORS 中间件 ──► 处理跨域头（支持 Any 或白名单模式）
  │
  ▼
Trace 中间件 ──► 提取/生成 X-Trace-Id → 注入 Extension + 创建 tracing span
  │
  ▼
Compression ──► gzip 响应压缩（tower-http）
  │
  ▼
Logger 中间件 ──► 记录 [{trace_id}] METHOD PATH STATUS DURATION
  │
  ▼
[Extensions 注入] ──► 将 AuthCache、SecretKeyCache、JwtFailLimiter、BasicAuthFailLimiter、DatabaseConnection、AppConfig 注入 Extension
  │
  ▼
[Auth 中间件] ──► 统一认证：JWT Bearer → Basic Auth 回退
  │               login/refresh 端点跳过认证
  │
  ▼
[Upload RateLimit] ──► Token Bucket + 并发数 + 每日配额检查（仅 /upload/* 路由）
  │
  ▼
API Handler ──► 解析参数 → 调用 Service → 返回 ApiResponse
```

各中间件职责明确、松耦合，通过 `axum::Extension` 和 `axum::extract::State` 传递上下文。
- CORS → Trace → Compression → Logger 为全局中间件
- Auth 中间件统一应用于所有 `/api/v1` 路由（`middleware/auth.rs`）
- RateLimit 中间件仅作用于上传路由（`middleware/ratelimit.rs`，`/upload/*` 路由）

### 2.2 响应格式

所有 API 响应统一使用 `ApiResponse<T>` 结构：

```json
{
    "code": 200,
    "message": "ok",
    "data": { ... }
}
```

- **`code`**: HTTP 语义的业务状态码（200 成功，4xx 客户端错误，5xx 服务端错误）
- **`message`**: 可读的状态描述
- **`data`**: 业务数据，成功时存在，失败时为 `null`

### 2.3 AppState 共享状态

```rust
pub struct AppState {
    pub db: DatabaseConnection,                    // 数据库连接（sea-orm）
    pub cfg: AppConfig,                            // 应用配置
    pub rate_limiter: Option<RateLimiter>,         // 速率限制器（可关闭）
    pub auth_cache: AuthCache,                     // Basic Auth 认证缓存（DashMap）
    pub secret_key_cache: SecretKeyCache,          // 用户密钥缓存（DashMap）
    pub jwt_fail_limiter: JwtFailLimiter,          // JWT 失败限流器（per-IP Token Bucket）
    pub basic_auth_fail_limiter: BasicAuthFailLimiter, // Basic Auth 失败限流器（防暴力破解）
}
```

## 三、核心流程详解

### 3.1 文件上传完整流程

```
CLI                                          Gateway
 │                                              │
 ├── 1. 计算文件 MD5 ──────────────────────►    │
 │    (本地流式 md5 hash)                        │
 │                                              │
 ├── 2. POST /api/v1/upload/sts ───────────►    │
 │    file_name, file_size, file_md5,           │ 生成 task_id + object_id
 │    chunk_size                                │ 签发 session_signature
 │    X-Trace-Id: uuid                          │
 │◄── {"task_id": "uuid",                    │
 │      "object_key": "1/file.mp4",             │
 │      "session_signature": "...",             │
 │      "session_timestamp": ...,               │
 │      "session_salt": "..."}                  │
 │                                              │
 ├── 3. POST /api/v1/upload/precheck ──────►    │
  │    file_md5, file_size, chunk_size           │ 查 buckets.md5 → 秒传
 │                                              │ 查 upload_tasks → 续传
 │◄── {"exists": false,                        │ 新建 upload_tasks → 新上传
 │      "task_id": "uuid",                   │
 │      "uploaded_chunks": [...]}              │
 │                                              │
    ├── 4. 并发上传分片（二进制流） ──────────►    │
    │    ┌─ chunk 0 ──► POST /api/v1/upload/      │ 会话签名验签
    │    │              chunk/upload-binary        │ 流式写入磁盘（64 KiB buf）
    │    │              ?task_id=...&              │ 从磁盘计算 MD5
    │    │              chunk_index=0&             │ MD5 校验
    │    │              chunk_md5=...&             │ 写暂存 → 更新内存位图
    │    │              session_signature=...&     │ 写入 MD5 侧车文件
    │    │              session_timestamp=...&     │ 批量更新 last_activity_at
    │    │              session_salt=...           │
    │    ├─ chunk 1 ──► (同上)                    │
    │    ├─ chunk 2 ──► (同上)                    │
    │    └─ ...                                    │
 │                                              │
 ├── 5. POST /api/v1/upload/chunk/status ──►   │
 │◄── {"is_complete": true,                     │
 │      "missing_chunks": []}                  │
 │                                              │
 ├── 6. POST /api/v1/upload/merge ────────►    │
 │    task_id, file_name, file_md5,            │ 完整性校验（位图遍历）
 │    file_size, content_type                  │ 磁盘空间预检
 │◄── {"task_id": "uuid",                      │ 接受合并，返回 accepted
 │      "message": "merge started"}            │
 │                                              │
 ├── 7. 后台异步合并 ──────────────────────►    │
 │    (tokio::spawn)                            │ 按顺序读取暂存分片
 │                                              │ 流式写入合并文件
 │                                              │ 边写边计算 MD5
 │                                              │ MD5 校验
  │                                              │ ORM 插入 objects 记录
  │                                              │ ORM 更新 upload_tasks status
 │                                              │ 清理暂存目录
 │                                              │
 ├── 8. 轮询合并状态 ─────────────────────►    │
 │    GET /api/v1/upload/merge/status          │
 │    ?task_id=uuid                             │
 │◄── {"status": "merging"/"completed"}        │
 │    轮询间隔 2s，最大 3600 次（2h）            │
 │◄── {"status": "completed",                   │
 │      "object_url": "/object/uuid"}           │
 │                                              │
```

#### 3.1.1 STS 凭证签发

用户调用 `/api/v1/upload/sts` 获取上传上下文：

1. 服务端生成 `task_id`（UUID v4，upload_tasks 主键）
2. 服务端生成 `object_id`（UUID v4，objects 业务标识）
3. 生成 `object_key`：格式 `{user_id}/{YYYYMMDD}/{object_id}.{ext}`
4. 计算 `session_signature` = HMAC-SHA256(`secret_key`, `"session:{user_id}:{task_id}:{file_md5}:{chunk_size}:{timestamp}:{salt}`)
5. 返回 STS 响应，包含会话级签名（有效期 2h，每次分片上传刷新 `last_activity_at`）

#### 3.1.2 文件预校验（秒传 + 断点续传）

核心的去重和续传逻辑：

```
precheck(file_md5, file_size, chunk_size, user_id)
  │
  ├── 文件扩展名校验（STRICT_EXTENSION_CHECK 开启时）
  │     └── 不在白名单或在黑名单 → 415 InvalidFileType
  │
  ├── ORM 查询 objects（按 md5 + bucket + status = 'active'）
  │     │
  │     ├── 存在 → ORM 插入 user_objects（用户-对象关联）
  │     │          返回 { exists: true, object_id, object_url }
  │     │          客户端直接完成，无需上传（秒传）
  │     │
  │     └── 不存在 → 继续检查 upload_tasks
  │
  └── ORM 查询 upload_tasks（按 file_md5 + user_id + status = initialized/uploading）
        │
        ├── 存在（status = initialized/uploading）
        │     → 解析 uploaded_bitmap，返回已上传分片列表
        │       客户端跳过这些分片（断点续传）
        │
        └── 不存在 → 新建 upload_tasks 记录
              → 根据文件大小动态计算 expires_at 和 activity_timeout
              → 返回空 uploaded_chunks 列表（全新上传）
```

**动态过期时间计算**：
- 基础过期：72h
- 每 10GB 文件增加 24h
- 基础测活窗口：1h
- 每 10GB 文件增加 1h

#### 3.1.3 分片上传（二进制流）

客户端将文件切分为固定大小的块（默认 8MB），并发上传。分片数据以 `application/octet-stream` 二进制流传输：

```
upload_chunk(task_id, chunk_index, chunk_md5, signature, timestamp, salt, binary_body)
  │
  ├── 会话签名校验（HMAC-SHA256）
  │     └── 校验失败 → 401 SignatureInvalid
  ├── 签名有效期校验（|now - timestamp| <= 2h 或 activity_timeout）
  │     └── 过期 → 401 SignatureExpired
  ├── 校验 task_id 对应的任务存在
  ├── 磁盘空间预检（spawn_blocking 中执行，避免阻塞 tokio runtime）
  ├── axum::body::Body 流式写入暂存文件（64 KiB buffer，不加载全部分片到内存）
  ├── 从磁盘文件计算 MD5（流式读取，64 KiB buffer）
  ├── 校验 MD5（chunk_md5 == computed_md5）
  │     └── 不匹配 → 409 HashMismatch
  ├── 校验分片大小不超过 chunk_size
  │     └── 超出 → 413 FileTooLarge
  ├── 原子 rename 到暂存目录: data/staging/{task_id}/chunk_{:06}
  ├── 写入 MD5 侧车文件: chunk_{:06}.md5（供合并时零拷贝路径使用）
  ├── 更新内存位图缓存（不直接写 DB）
  ├── 记录到 pending_activity 集合（批量更新 last_activity_at）
  └── 返回 { chunk_index, status: "uploaded"/"already_exists", md5 }
```

**内存位图缓存**：
- 上传进度位图缓存在 `DashMap<String, Arc<RwLock<TaskBitmap>>>` 中
- 每 5 秒批量刷新 dirty 位图到 DB，同时批量更新 `last_activity_at`
- 位图缓存有硬上限（500 条目），超出时淘汰干净条目
- 所有分片上传完成后自动从缓存移除

**位图机制**（详见 schema.md）：
- 使用 `Vec<u64>` 数组记录已上传分片
- 每个 bit 对应一个分片索引
- 写入时 `bitmap[word] |= 1u64 << bit`
- 查询时 `(bitmap[word] & (1u64 << bit)) != 0`
- 数据库中以 JSON 格式 TEXT 存储

#### 3.1.4 异步合并

合并改为异步后台执行，支持零拷贝和并发控制：

```
merge(task_id, file_name, file_md5, file_size, content_type)
  │
  ├── 检查所有分片是否存在（遍历位图）
  │     └── 缺失 → 返回 409 UploadIncomplete（含缺失数）
  │
  ├── 磁盘空间预检（spawn_blocking，需要 2x file_size 可用空间）
  │     └── 不足 → 500 StorageError
  │
  ├── 获取合并信号量（最多 4 个并发合并，防止线程池耗尽）
  ├── 更新 upload_tasks.status = 'merging'
  ├── 立即返回 { task_id, message: "merge started" }
  │
  └── spawn_blocking 后台执行:
        ├── 按 chunk_index 顺序处理每个分片
        ├── 尝试 Linux copy_file_range 零拷贝（无需用户态 buffer）
        │     ├── 成功 → 从侧车文件读取 MD5，更新 hasher（不重读分片数据）
        │     └── 失败 → BufWriter 流式读取 + 计算 MD5
        ├── 合并完成后 fsync 输出文件（确保数据持久化）
        ├── 校验 computed_md5 == file_md5
        │     ├── 匹配 → 原子 rename 到最终路径
        │     │         → fsync 父目录（确保 rename 持久化）
        │             │     → ORM 插入 objects + user_objects 记录
        │     │         → ORM 更新 status = 'completed'
        │     └── 不匹配 → status = 'failed'，仅清理合并文件（保留暂存分片供重试）
        ├── 清理暂存目录: rm -rf data/staging/{task_id}
        ├── 从位图缓存中移除
        └── 更新 upload_tasks status

客户端轮询:
  GET /api/v1/upload/merge/status?task_id=uuid
  → 返回 { status: "merging"/"completed"/"failed", object_url, error }
  轮询间隔 2s，指数退避（2s → 4s → 8s → ... → 30s cap），最大 3600 次
```

合并失败时保留暂存分片，客户端可重试合并而无需重新上传。GC 在过期后清理。

### 3.2 用户认证流程

buckets 使用统一认证中间件 (`middleware/auth.rs`)，支持两种认证方式：

- **JWT Bearer Token**（优先）：`Authorization: Bearer <jwt>` 请求头
- **Basic Auth**（回退）：`Authorization: Basic base64(email:password)` 请求头

#### JWT Token 认证流程（优先）

```
请求头 Authorization: Bearer <jwt_token>
  │
  ├── 解码 JWT header，确认算法为 HS256
  ├── 解码 payload（不验证签名），提取 sub（user_id）和 exp
  │
  ├── 检查 exp > now()
  │     └── 过期 → 401 token expired
  │
  ├── ORM 查询 users.secret_key（按 id）
  ├── 用 secret_key 重新验证 JWT 签名
  │     ├── 匹配 → 注入 Extension<UserId(user_id)>，继续处理
  │     └── 不匹配 → 401 invalid token
  │
  ▼
Handler 通过 Extension<UserId> 获取 user_id
```

#### Basic Auth 流程（回退）

Token 不存在时回退到 Basic Auth：

```
请求头 Authorization: Basic xxx
  │
  ├── 提取 "Basic " 后的 Base64 字符串
  ├── Base64 解码 → email:password
  ├── SHA256("email:password") → cache_key
  ├── 检查 AuthCache（DashMap 内存缓存，TTL 30min）
  │     ├── 命中且未过期 → 直接返回 UserId
  │     └── 未命中 → 查询数据库
  │
  ├── ORM 查询 users（按 email，返回 id + password）
  │     ├── 匹配 → argon2 验证密码
  │     │     ├── 验证通过 → 插入缓存，返回 Extension<UserId(id)>
  │     │     └── 验证失败 → 401 Unauthorized
  │     └── 用户不存在 → 401 Unauthorized
  │
  ▼
Handler 通过 Extension<UserId> 获取 user_id
```

#### Token 管理接口

统一 auth 端点（Gateway 和 Web 共用）：

| 接口 | 路径 | 认证 | 说明 |
|------|------|------|------|
| 登录 | `POST /api/v1/auth/login` | 无 | 邮箱+密码 → `{ token, refresh_token, expires_in, is_super_admin }` |
| 刷新 | `POST /api/v1/auth/refresh` | 无 | refresh_token → 新 token 对（旧 refresh_token 吊销） |
| 登出 | `POST /api/v1/auth/logout` | 必需 | 记录登出日志 |
| 验证 | `POST /api/v1/auth/verify` | 必需 | 验证凭据，返回 `{ user_id }` |

**Token 格式**：JWT HS256，header 包含 `kid`（用户 ID），payload 包含 `sub`（用户 ID）、`exp`（过期时间）、`jti`（唯一标识），签名密钥为 `users.secret_key`。

- Auth token: TTL=7天
- Refresh token: TTL=7天

注意：当前版本登出和刷新不会使旧 token 立即失效。Token 有过期时间限制（7 天），安全性不受影响。

注意：
- 服务端存储的密码为 argon2 哈希
- 首次启动时自动将 placeholder 密码替换为 argon2 哈希
- Basic Auth 缓存使用 DashMap，后台定时清理过期条目
- `login`、`refresh`、`health` 端点无需认证

### 3.3 HMAC 签名机制

buckets 使用 HMAC-SHA256 实现两类签名：

#### 会话签名（分片上传）

分片上传使用会话级 HMAC-SHA256 签名（替代逐分片签名）：

```
STS 签发时:
  签名输入 = { user_id, task_id, file_md5, chunk_size, session_timestamp, session_salt }
   密钥 = `users.secret_key`（64 字符 hex，数据库预设）
  message = "session:{user_id}:{task_id}:{file_md5}:{chunk_size}:{timestamp}:{salt}"
  signature = HMAC-SHA256(secret_key, message)

分片上传验证:
  1. 提取请求中的 signature, timestamp, salt
  2. 检查 task 的 last_activity_at 是否在有效期内
  3. 用相同密钥和输入重新计算期望签名
  4. 比对 signature == expected
  5. 更新 last_activity_at = now()（刷新会话）
```

#### Token 签名（Web 认证）

Token 认证使用 JWT HS256：

```
Token 生成:
   密钥 = users.secret_key（64 字符 hex，数据库预设）
  header = { alg: "HS256", kid: user_id }
  payload = { sub: user_id, exp: expires_at, jti: uuid }
  token = JWT HS256 encode(header, payload, secret_key)

Token 验证:
  1. 解码 JWT header（不验证签名），确认 alg = HS256
  2. 从 header.kid 提取 user_id
  3. 查询 users.secret_key（by user_id，优先走 SecretKeyCache）
  4. 用 secret_key 验证 JWT 签名
  5. 检查 exp > now()
  6. 失败请求受 JwtFailLimiter（per-IP Token Bucket）限流保护
```

两种签名共用同一 `users.secret_key` 作为 HMAC 密钥。

### 3.4 TraceID 链路追踪

每次请求的处理流程：

```
CLI: Client::new() 时生成 trace_id（UUID v4）
   │
   ├── 设置默认请求头 X-Trace-Id: {trace_id}
   │
   └── 每个 HTTP 请求自动携带该头

Server: trace 中间件
   │
   ├── 提取 X-Trace-Id（没有则生成新 UUID）
   ├── 注入 Extension<TraceId>
   ├── 创建 tracing span: info_span!("request", trace_id)
   ├── 进入 span 执行后续处理
   └── 响应头回写 X-Trace-Id: {trace_id}

Logger: 记录 [{trace_id}] METHOD PATH STATUS DURATION_ms

排查:
  CLI 报错 → 日志中有 trace_id → grep {trace_id} 服务端日志
    ├── 服务端无此 trace_id → 请求未到达服务器 → 客户端/网络问题
    └── 服务端有此 trace_id → 查看该请求完整调用链 → 定位具体环节
```

### 3.5 速率限制机制

三级限流（可通过 `RATE_LIMIT_ENABLED=false` 全局关闭）：

**第一级 — Token Bucket（请求速率）**：
- 每用户独立 token bucket
- 默认 2 req/s，突发容量 10
- DashMap 内存存储，后台定时清理过期 bucket

**第二级 — 并发限制**：
- 查询 upload_tasks 中 `status IN ('initialized', 'uploading')` 的活跃任务数
- 默认最大 5 个并发

**第三级 — 每日配额**：
- 查询今日创建的 upload_tasks 数量
- 默认每日 50 次

### 3.6 后台任务

后台任务位于 `task/`，通过 `tokio::spawn` 启动，使用 `CancellationToken` 支持优雅关闭：

**gc_clean**（30 分钟间隔）：
```
ORM 查询过期 upload_tasks（expires_at < now, status 不在 completed/expired 中），限 100 条
  │
  └── 分批处理（每批 100 条，最多 1000 条/次，批次间间隔 500ms）:
        ├── ORM 更新 status = 'expired'
        ├── 删除 data/staging/{task_id}/ 目录
        ├── 删除 data/cache/{task_id}/ 目录
        └──（保留 upload_tasks 记录用于审计）
```
批次间暂停和总上限防止 I/O 风暴影响活跃上传。

**ref_check**（2 小时间隔）：
```
ORM 查询 objects（status = 'deleted'），在 Rust 侧过滤无 user_objects 关联的记录
  │
  └── 对每个待清理对象（无所有者）:
        ├── 删除对应的物理文件: data/objects/{path}
        ├── ORM 删除 objects 记录
        └──（user_objects 关联已在 DELETE 时清理）
```

**auth_cache_cleaner**（持续）：
```
周期性扫描 AuthCache，删除过期条目
```

## 四、Web 管理后台架构

```
web/                        # Vue 3 + TypeScript + Tailwind CSS
├── src/
│   ├── main.ts             # 入口：createApp + Pinia + Router
│   ├── App.vue             # 根组件（RouterView）
│   ├── style.css           # @import "tailwindcss"
│   ├── stores/             # Pinia 状态管理 + API 单例
│   │   ├── api.ts          # BucketsClient 单例 + login/logout（含 localStorage 管理）
│   │   ├── auth.ts         # token 响应式状态、isSuperAdmin
│   │   ├── objects.ts      # 对象列表、分页、删除
│   │   └── users.ts        # 用户 CRUD、重置密钥
│   ├── router/
│   │   └── index.ts        # 路由表 + beforeGuard（未登录 → /login，非管理员 → /objects）
│   ├── views/
│   │   ├── Login.vue       # 登录页（邮箱 + 密码）
│   │   ├── ObjectList.vue  # 对象列表（分页 + 删除 + 三种上传模式）
│   │   └── UserList.vue    # 用户管理（新建/编辑/删除/重置密钥，仅管理员）
│   └── components/
│       └── Layout.vue      # 顶栏导航 + 退出按钮 + RouterView
├── vite.config.ts          # Vite 8 + @tailwindcss/vite（Tailwind CSS 4）插件 + /api 代理
└── package.json
```

### 4.1 数据流

```
页面组件 → Pinia Store → BucketsClient（@chihqiang/buckets）→ HTTP → 后端
```

- `@chihqiang/buckets` 包提供 `BucketsClient` 统一入口，封装了传输层和认证
- `stores/api.ts` 持有 `BucketsClient` 单例并提供 `login`/`logout` 便捷函数（含 localStorage 管理）
- 页面组件只与 store 交互，不直接调用 `BucketsClient`

### 4.2 权限控制

- 登录后服务端返回 `is_super_admin`，前端存入 localStorage
- 侧边栏「用户管理」仅对 `isSuperAdmin === true` 显示
- 路由守卫拦截 `/users`，非管理员自动跳转 `/objects`
- 后端同样校验，API 返回 401/403 时由各业务代码处理（不再由全局拦截器处理）

## 五、CLI 客户端架构

```
main.rs ─── cli.rs ─── client/
  │                     ├── mod.rs      上传主流程（Client 结构体）
  │                     ├── precheck.rs STS + 秒传预检
  │                     ├── chunk.rs    流式分片上传
  │                     └── merge.rs    合并请求 + 轮询
  │
  ├── config.rs      凭证本地存储（Map<server_url, base64(email:password)>，~/.buckets/credentials.json）
  ├── local.rs       本地文件读取 + 分片切分 + 断点缓存
  └── progress.rs    进度条（indicatif MultiProgress）
```

**上传并发控制**：

使用 `tokio::sync::Semaphore` 控制并发分片数，默认 4 并发，最大 16：

```rust
let semaphore = Arc::new(Semaphore::new(parallel));
// 每个分片任务 acquire_owned() → 获取许可后执行上传
```

**重试策略**：

分片上传失败自动重试 3 次，间隔指数退避（1s, 2s, 4s）+ 随机抖动（0-500ms）：

```rust
for attempt in 0..CHUNK_UPLOAD_MAX_RETRIES {
    match upload_chunk(...).await {
        Ok(_) => break,
        Err(e) if attempt < MAX_RETRIES - 1 => {
            let backoff = Duration::from_secs(1 << attempt);
            let jitter = rand::thread_rng().gen_range(0..MAX_JITTER_MS);
            sleep(backoff + Duration::from_millis(jitter));
        }
        Err(e) => bail!("chunk {idx} failed: {e}"),
    }
}
```

**流式分片上传**：

分片数据以 `application/octet-stream` 二进制流传输，支持流式读取本地文件、计算分片 MD5、Base64 编码后发送。使用 `tokio_util::io::ReaderStream` 包装文件读取器。

**进度展示**：

使用 `indicatif::MultiProgress` 管理：
- 总进度条（文件级）
- 每个活跃分片独立进度条
- 上传速度（Bytes/s）
- 预计剩余时间

## 六、错误处理体系

```
AppError（buckets-common）
  │
  ├── BadRequest(String)           400 参数错误
  ├── Unauthorized                 401 认证失败
  ├── Forbidden(String)            403 权限不足
  ├── NotFound(String)             404 资源不存在
  ├── Conflict(String)             409 冲突
  ├── Internal(String)             500 内部错误（敏感信息脱敏返回）
  ├── ChunkAlreadyExists           409 分片已存在（幂等）
  ├── ChunkNotFound(String)        404 分片文件丢失
  ├── UploadIncomplete(u32)        409 分片未传完（含缺失数）
  ├── HashMismatch {expected, actual}  409 MD5 不匹配
  ├── SignatureExpired             401 签名过期
  ├── SignatureInvalid             401 签名无效
  ├── FileTooLarge(String)         413 文件/分片过大
  ├── InvalidFileType(String)      415 不支持的文件类型
  ├── StorageError(String)         500 存储层错误（脱敏）
  └── DatabaseError(String)        500 数据库错误（脱敏）
```

每个错误实现了 `status_code()` 方法返回对应 HTTP 状态码。`StorageError`、`DatabaseError`、`Internal` 三种内部错误会：
1. 通过 `tracing::error!` 记录完整错误详情
2. 向客户端返回脱敏后的通用消息

Handler 统一转换为 `ApiResponse`：

```rust
match service_fn(...).await {
    Ok(result) => Json(ApiResponse { code: 200, message: "ok", data: Some(result) }),
    Err(e) => {
        // AppError 实现了 IntoResponse，自动处理脱敏和格式转换
        e.into_response()
    }
}
```

## 七、技术选型

| 组件 | 技术 | 版本 | 用途 |
|------|------|------|------|
| 运行时 | tokio | 1.x | 异步 IO + 任务调度 |
| HTTP 框架 | axum | 0.8 | 路由 + 中间件 + 提取器 |
| 数据库驱动 | sea-orm | 1.x | 异步 ORM，代码生成，迁移管理 |
| 序列化 | serde + serde_json | 1.x | 模型序列化 |
| 哈希 | md-5, sha2, sha256 | 0.10 | 文件校验 + 签名 + 密码 |
| 密码哈希 | argon2 | 0.5 | 用户密码安全存储 |
| 密码学 | hmac | 0.12 | 会话签名 |
| JWT | jsonwebtoken | 9 | Web Token 签发与验证 |
| 编码 | base64 | 0.22 | 分片数据编码 |
| CLI 框架 | clap | 4.x | 命令行参数解析 |
| HTTP 客户端 | reqwest | 0.12 | CLI HTTP 客户端 |
| 进度条 | indicatif | 0.17 | CLI 进度展示 |
| 日志 | tracing + tracing-subscriber | 0.1/0.3 | 结构化日志 |
| 环境变量 | dotenvy | 0.15 | .env 文件加载 |
| 缓存 | dashmap | 6 | 并发内存缓存（auth cache + secret key cache + bitmap cache） |
| CORS/压缩 | tower-http | 0.6 | 跨域 + gzip 压缩 |
| Body 工具 | http-body-util | 0.1 | 请求体缓冲读取 |
| 优雅关闭 | tokio-util CancellationToken | 0.7 | 信号处理 |

### 设计原则

1. **分层分离**：Middleware / API / Service / DAO 职责清晰，每层只做分内事
2. **中间件认证**：认证统一在中间件完成，handler/service 不再重复校验
3. **幂等设计**：分片上传、合并等操作支持重复调用
4. **完全异步**：全栈 tokio 异步，无阻塞调用
5. **配置即环境变量**：无配置文件，所有配置通过环境变量注入（dotenvy 加载 .env）
6. **内部错误脱敏**：Storage/Database/Internal 错误只向客户端返回通用消息
7. **异步合并**：耗时合并操作后台执行 + 客户端轮询，避免 HTTP 超时
8. **优雅关闭**：SIGTERM/SIGINT 触发 CancellationToken，等待进行中的请求完成
