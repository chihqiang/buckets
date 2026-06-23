# buckets API 接口文档

## 基础信息

### 请求地址

```
Base URL: http://{host}:{port}/api/v1
Content-Type: application/json（JSON 接口）
Content-Type: application/octet-stream（分片上传接口）
```

### 认证方式

所有接口使用统一认证中间件，支持两种方式：

| 认证方式 | 用途 | 格式 |
|---------|------|------|
| JWT Bearer Token | Web 前端、API 调用 | `Authorization: Bearer <jwt_token>` |
| HTTP Basic Auth | CLI 客户端 | `Authorization: Basic base64(email:password)` |

`/health` 和 auth 接口（login/refresh）除外，无需认证。

#### 方式一：JWT Bearer Token（推荐用于 Web 前端和 API 调用）

```
Authorization: Bearer eyJhbGciOiJIUzI1NiJ9...
```

Token 通过 `/api/v1/auth/login` 获取，JWT HS256 签名，有效期 7 天，支持刷新。Bearer Token 优先于 Basic Auth，无效时直接返回 401，不会回退。

#### 方式二：HTTP Basic Auth（CLI 客户端使用）

```
Authorization: Basic base64("email:password")
```

示例：
```
# email = admin@buckets.local
# password = buckets
# base64("admin@buckets.local:buckets") = YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0

Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0
```

服务端收到后通过 argon2 验证密码。`user_id` 不由客户端传入，由服务端从认证信息中提取。认证结果缓存 30 分钟（DashMap 内存缓存）。

### TraceID 链路追踪

所有请求支持 `X-Trace-Id` 请求头用于链路追踪：

```
X-Trace-Id: 550e8400-e29b-41d4-a716-446655440000
```

- CLI 客户端自动生成 trace_id（UUID v4）并随每个请求发送
- 服务端在响应头中回写相同的 `X-Trace-Id`
- 服务端每条请求日志带有 `[{trace_id}]` 前缀
- 排查问题时用 trace_id 在服务端日志中 `grep` 即可定位完整请求链路

### 通用响应格式

**成功响应**：
```json
{
    "code": 200,
    "message": "ok",
    "data": { ... }
}
```

**错误响应**：
```json
{
    "code": 400,
    "message": "具体错误描述",
    "data": null
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| code | u32 | 状态码（同 HTTP status） |
| message | String | 状态描述 |
| data | T / null | 业务数据 |

### 通用错误码

| code | 含义 | 说明 |
|------|------|------|
| 200 | 成功 | 请求处理完成 |
| 400 | 参数错误 | 缺少必填字段、格式错误等 |
| 401 | 认证/签名失败 | Basic Auth 无效、签名过期/无效 |
| 403 | 权限不足 | 速率限制触发（请求频率/并发/每日配额） |
| 404 | 资源不存在 | task_id / object_id 未找到 |
| 409 | 冲突 | MD5 不匹配、分片不完整、分片已存在 |
| 413 | 文件过大 | 分片/文件超过大小限制 |
| 415 | 文件类型不支持 | 扩展名不在白名单（严格模式） |
| 500 | 服务端错误 | 内部错误、数据库/存储异常（错误详情脱敏） |

---

## 1. 登录（获取 Token）

```
POST /api/v1/auth/login
```

**Auth**: 不需要

### Request

```json
{
    "email": "admin@buckets.local",
    "password": "buckets"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| email | String | 是 | 用户邮箱 |
| password | String | 是 | 明文密码 |

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "token": "eyJhbGciOiJIUzI1NiJ9...",
        "refresh_token": "eyJhbGciOiJIUzI1NiJ9...",
        "expires_in": 604800,
        "is_super_admin": true
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| token | String | JWT HS256 访问令牌，有效期 7 天 |
| refresh_token | String | JWT HS256 刷新令牌，用于获取新 token |
| expires_in | i64 | Token 有效期（秒），默认 604800（7 天） |
| is_super_admin | bool | 是否为超级管理员 |

---

## 2. 刷新 Token

```
POST /api/v1/auth/refresh
```

**Auth**: 不需要

### Request

```json
{
    "refresh_token": "eyJhbGciOiJIUzI1NiJ9..."
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| refresh_token | String | 是 | 登录时获取的 refresh_token |

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "token": "<new_token>",
        "refresh_token": "<new_refresh_token>",
        "expires_in": 604800,
        "is_super_admin": true
    }
}
```

返回结构与登录接口相同，均为新的 token 对。旧 refresh_token 立即吊销。

---

## 3. 登出（吊销 Token）

```
POST /api/v1/auth/logout
```

**Auth**: 必需（需携带有效的 Bearer Token 或 Basic Auth）

### Request

无需请求体。服务端从 `Authorization` header 提取 token 并加入黑名单。

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": null
}
```

---

## 4. 验证凭据

```
POST /api/v1/auth/verify
```

**Auth**: 必需

验证当前凭据是否有效，返回认证用户 ID。

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "user_id": 1
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| user_id | u64 | 认证用户 ID |

---

## 5. 获取 STS 凭证

获取上传会话凭证和会话级 HMAC 签名。每个上传任务开始前先调用此接口。

```
POST /api/v1/upload/sts
```

**Auth**: 必需

### Request

```json
{
    "file_name": "example.mp4",
    "file_size": 1073741824,
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "chunk_size": 8388608
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| file_name | String | 是 | 原始文件名 |
| file_size | u64 | 是 | 文件大小（字节） |
| file_md5 | String | 是 | 文件 MD5 哈希 |
| chunk_size | u64 | 否 | 分片大小（字节），默认 8MB（8388608） |

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "object_key": "1/example.mp4",
        "session_signature": "a1b2c3d4e5f6...",
        "session_timestamp": 1700000000,
        "session_salt": "550e8400-e29b-41d4-a716-446655440000"
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| task_id | String | UUID，上传会话标识 |
| object_key | String | 格式 `{user_id}/{file_name}`，存储逻辑路径 |
| session_signature | String | 会话级 HMAC-SHA256 签名（hex），有效期 2h |
| session_timestamp | i64 | 签名时间戳（Unix 秒） |
| session_salt | String | 签名随机盐值（UUID） |

**签名算法**：
```
message = "session:{user_id}:{task_id}:{file_md5}:{chunk_size}:{timestamp}:{salt}"
secret_key = users.secret_key（64 字符 hex，数据库预设）
signature = HMAC-SHA256(secret_key, message)
```

---

## 6. 文件预校验（秒传 + 断点续传）

上传前检查文件状态。支持：
- **秒传**：服务端已有相同 MD5 的文件，直接返回对象信息
- **续传**：存在进行中的上传任务，返回已上传分片列表
- **新上传**：创建新的上传任务，返回空分片列表

```
POST /api/v1/upload/precheck
```

**Auth**: 必需

### Request

```json
{
    "file_name": "example.mp4",
    "file_size": 1073741824,
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "chunk_size": 8388608
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| file_name | String | 是 | 原始文件名 |
| file_size | u64 | 是 | 文件大小（字节） |
| file_md5 | String | 是 | 文件 MD5 哈希 |
| chunk_size | u64 | 否 | 分片大小，默认 8MB |

### Response（秒传命中）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "exists": true,
        "object_id": "550e8400-e29b-41d4-a716-446655440000",
        "storage_path": "data/objects/ab/cd/550e8400-...",
        "task_id": null,
        "uploaded_chunks": [],
        "chunk_size": 8388608
    }
}
```

客户端收到 `exists: true` 后可直接完成，无需上传。

### Response（需上传 / 续传）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "exists": false,
        "object_id": "550e8400-e29b-41d4-a716-446655440000",
        "storage_path": null,
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "uploaded_chunks": [0, 1, 2, 5, 6],
        "chunk_size": 8388608
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| exists | bool | `true`=秒传，`false`=需要上传 |
| object_id | String / null | 对象 UUID |
| storage_path | String / null | 对象物理存储路径 |
| task_id | String / null | 上传任务 UUID |
| uploaded_chunks | [u32] | 已上传的分片索引列表（续传时使用） |
| chunk_size | u64 | 分片大小（字节），默认 8MB = 8388608 |

**客户端续传逻辑**：
1. 将 `uploaded_chunks` 中的索引标记为已完成
2. 仅上传缺失的分片
3. 上传完成后检查状态，确认全部完成

---

## 7. 上传分片（二进制流）

上传单个分片。数据以 `application/octet-stream` 二进制流传输。分片数据通过流式写入磁盘，避免将整个分片加载到内存。

**Session 签名参数通过自定义 HTTP Header 传递**，避免 URL 长度限制及访问日志/浏览器历史泄露：

```
POST /api/v1/upload/chunk/upload-binary?task_id=<uuid>&chunk_index=<n>&chunk_md5=<md5>
```

**Auth**: 必需

### Query Parameters

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| task_id | UUID | 是 | 上传任务 ID |
| chunk_index | u32 | 是 | 分片序号（从 0 开始） |
| chunk_md5 | String | 是 | 该分片的 MD5 哈希 |

### Request Headers

| Header | 类型 | 必填 | 说明 |
|------|------|------|------|
| X-Session-Signature | String | 是 | 会话级 HMAC-SHA256 签名 |
| X-Session-Timestamp | i64 | 是 | 签名时间戳 |
| X-Session-Salt | String | 是 | 签名随机盐值 |

### Request Body

```
Content-Type: application/octet-stream

<binary chunk data>
```

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "chunk_index": 3,
        "status": "uploaded",
        "md5": "d41d8cd98f00b204e9800998ecf8427e"
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| chunk_index | u32 | 已上传的分片索引 |
| status | String | `uploaded`=首次上传, `already_exists`=已存在（幂等） |
| md5 | String | 分片 MD5 确认 |

### 上传流程说明

1. 客户端计算分片数据的 MD5
2. 通过 STS 接口获取会话签名
3. 将分片二进制数据作为 request body 发送
4. 服务端校验：
   - 会话签名有效性（HMAC-SHA256）
   - 签名是否在有效期内
   - task_id 对应的上传任务存在
   - 分片 MD5 校验
   - 分片大小不超过任务设定的 chunk_size
5. 写入暂存目录 `data/staging/{task_id}/chunk_{:06}`
6. 原子更新上传任务位图（JSON `Vec<u64>`）
7. 刷新 `last_activity_at`（会话测活）

---

## 8. 查询分片状态

查询上传任务的总体进度，包括已上传数量和缺失分片列表。

```
POST /api/v1/upload/chunk/status
```

**Auth**: 必需

### Request

```json
{
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| task_id | UUID | 是 | 上传任务 ID |

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "chunk_count": 128,
        "uploaded_count": 64,
        "missing_chunks": [64, 65, 66, 67, 68],
        "is_complete": false
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| task_id | String | 上传任务 ID |
| chunk_count | i64 | 总分片数 |
| uploaded_count | u32 | 已上传分片数 |
| missing_chunks | [u32] | 缺失的分片索引列表 |
| is_complete | bool | 是否所有分片已上传 |

---

## 9. 合并分片（异步）

所有分片上传完成后，发起合并请求。合并改为**异步后台执行**，接口立即返回，客户端需轮询合并状态。

```
POST /api/v1/upload/merge
```

**Auth**: 必需

### Request

```json
{
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "file_name": "example.mp4",
    "file_md5": "d41d8cd98f00b204e9800998ecf8427e",
    "file_size": 1073741824,
    "content_type": "video/mp4"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| task_id | UUID | 是 | 上传任务 ID |
| file_name | String | 是 | 最终文件名 |
| file_md5 | String | 是 | 文件 MD5（合并后校验） |
| file_size | u64 | 是 | 文件大小 |
| content_type | String | 否 | MIME 类型 |

### Response（合并已接受，202）

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

| 字段 | 类型 | 说明 |
|------|------|------|
| task_id | String | 上传任务 ID |
| message | String | "merge started" |

### 合并后台流程

```
1. 检查所有分片是否存在（遍历位图）
   └── 存在缺失 → 返回 409 UploadIncomplete
2. 磁盘空间预检（需要 2x file_size 可用空间）
   └── 不足 → 返回 500 StorageError
3. 更新 upload_tasks.status = 'merging'
4. 立即返回 { task_id, message: "merge started" }
5. tokio::spawn 后台执行:
   ├── 按 chunk_index 顺序读取暂存分片
   ├── BufWriter(1MB) 流式写入合并文件: data/objects/{obj_id_prefix}/{obj_id}
   ├── 边写边计算 MD5
   ├── 校验 computed_md5 == file_md5
     │   ├── 匹配 → ORM 插入 objects + user_objects → status = 'completed'
   │   └── 不匹配 → status = 'failed'，清理合并文件
   ├── 清理暂存目录: rm -rf data/staging/{task_id}
   └── 更新 upload_tasks status
```

---

## 10. 查询合并状态

轮询合并进度。

```
GET /api/v1/upload/merge/status?task_id=<uuid>
```

**Auth**: 必需

### Query Parameters

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| task_id | UUID | 是 | 上传任务 ID |

### Response（合并中）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "status": "merging",
        "storage_path": null
    }
}
```

### Response（合并完成）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "status": "completed",
        "storage_path": "data/objects/ab/cd/550e8400-..."
    }
}
```

### Response（合并失败）

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "task_id": "550e8400-e29b-41d4-a716-446655440000",
        "status": "failed",
        "storage_path": null,
        "error": "merge failed"
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| task_id | String | 上传任务 ID |
| status | String | `merging`=进行中, `completed`=成功, `failed`=失败 |
| storage_path | String / null | 完成时返回对象物理存储路径 |
| error | String / null | 失败时的错误描述，其他状态为 null |

**轮询建议**：
- 轮询间隔：2 秒
- 最大轮询次数：3600 次（2 小时超时）

---

## 11. 获取对象信息

查询已上传完成的对象元数据。

```
GET /api/v1/object/{object_id}
```

**Auth**: 必需

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "id": 1,
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "name": "example.mp4",
        "size": 1073741824,
        "md5": "d41d8cd98f00b204e9800998ecf8427e",
        "content_type": "video/mp4",
        "extension": "mp4",
        "bucket": "default",
        "storage_path": "data/objects/1/2026/06/14/550e8400-....mp4",
        "image_width": 0,
        "image_height": 0,
        "image_type": "",
        "status": "active",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| id | u64 | 自增主键 |
| uuid | String | 对象 UUID（业务标识） |
| name | String | 原始文件名 |
| size | i64 | 文件大小（字节） |
| md5 | String | 文件 MD5 |
| content_type | String / null | MIME 类型 |
| extension | String / null | 文件扩展名 |
| bucket | String | 存储桶（默认 "default"） |
| storage_path | String | 物理存储相对路径 |
| status | String | `active`=正常, `deleted`=已删除, `archived`=已归档 |
| created_at | DateTime | 创建时间 |
| updated_at | DateTime | 最后更新时间 |

---

## 12. 删除对象

软删除对象（标记 `status = 'deleted'`）。

```
DELETE /api/v1/object/{object_id}
```

**Auth**: 必需

### Response

```json
{
    "code": 200,
    "message": "deleted",
    "data": null
}
```

注意：实际物理文件由后台 `ref_check` 任务异步清理，不阻塞请求。

---

## 13. 健康检查

查询服务健康状态（无需认证）。

```
GET /health
```

**Auth**: 不需要

### Response

```json
{
    "status": "ok",
    "db_ok": true,
    "disk_available_bytes": 107374182400,
    "disk_usage_percent": 45.2
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| status | String | `ok`=正常, `degraded`=降级（数据库不可用或磁盘使用率 >90%） |
| db_ok | bool | 数据库连接是否正常 |
| disk_available_bytes | u64 / null | 可用磁盘空间（字节），非 Linux 环境为 null |
| disk_usage_percent | f64 / null | 磁盘使用率百分比，非 Linux 环境为 null |

---

## 14. Web 管理后台 API

以下接口位于 `/api/v1/` 下，使用 Bearer Token 认证：

```
Authorization: Bearer <auth_token>
```

Token 通过 `POST /api/v1/auth/login` 获取。

### 13.1 用户列表

```
GET /api/v1/users?page=1&page_size=20
```

**Auth**: 必需（超级管理员）

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "items": [
            {
                "id": 1,
                "email": "admin@buckets.local",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            }
        ],
        "total": 1,
        "page": 1,
        "page_size": 20
    }
}
```

### 13.2 创建用户

```
POST /api/v1/users
```

**Auth**: 必需（超级管理员）

### Request

```json
{
    "email": "user@example.com",
    "password": "secure_password"
}
```

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "id": 2,
        "email": "user@example.com",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    }
}
```

### 13.3 获取用户

```
GET /api/v1/users/{id}
```

**Auth**: 必需（超级管理员）

### 13.4 更新用户

```
PUT /api/v1/users/{id}
```

**Auth**: 必需（超级管理员）

### Request

```json
{
    "email": "newemail@example.com",
    "password": "new_password"
}
```

`email` 和 `password` 均为可选，只更新提供的字段。

### 13.5 删除用户

```
DELETE /api/v1/users/{id}
```

**Auth**: 必需（超级管理员）

### 13.6 重置用户密钥

```
POST /api/v1/users/{id}/reset-secret-key
```

**Auth**: 必需（超级管理员）

重置用户的 `secret_key`，用于 HMAC 签名和 Token 签发。重置后该用户所有现有 Token 立即失效。

### 13.7 对象列表

```
GET /api/v1/objects?page=1&page_size=20&user_id=1
```

**Auth**: 必需

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| page | u64 | 否 | 页码，默认 1 |
| page_size | u64 | 否 | 每页条数，默认 20，最大 100 |
| user_id | i64 | 否 | 按用户筛选（仅超级管理员可用） |

- 超级管理员：可查看所有用户的文件，可通过 `user_id` 筛选
- 普通用户：仅查看自己的文件

### Response

```json
{
    "code": 200,
    "message": "ok",
    "data": {
        "items": [
            {
                "id": 1,
                "uuid": "550e8400-e29b-41d4-a716-446655440000",
                "name": "example.mp4",
                "size": 1073741824,
                "md5": "d41d8cd98f00b204e9800998ecf8427e",
                "content_type": "video/mp4",
                "extension": "mp4",
                "bucket": "default",
                "storage_path": "data/objects/1/2026/06/14/550e8400-....mp4",
                "image_width": 0,
                "image_height": 0,
                "image_type": "",
                "status": "active",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            }
        ],
        "total": 1,
        "page": 1,
        "page_size": 20
    }
}
```

### 13.8 删除对象

```
DELETE /api/v1/objects/{uuid}
```

**Auth**: 必需

移除当前用户与文件的关联。若为最后一个所有者，文件同时软删除。物理文件由后台 `ref_check` 任务异步清理。

---

## 完整上传流程示例

### 使用 JWT Bearer Token 认证（Web 端推荐）

```bash
# 0. 登录获取 token
$ curl -H "Content-Type: application/json" \
  -d '{"email":"admin@buckets.local","password":"buckets"}' \
  http://localhost:8080/api/v1/auth/login

# 返回: {"code":200,"data":{"token":"<TOKEN>","refresh_token":"<REFRESH>","expires_in":604800,"is_super_admin":true}}

# 1. 获取 STS（Bearer Token 认证）
$ curl -H "Content-Type: application/json" \
  -H "Authorization: Bearer <TOKEN>" \
  -H "X-Trace-Id: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"file_name":"large-file.mp4","file_size":1073741824,"file_md5":"d41d8cd98f00b204e9800998ecf8427e"}' \
  http://localhost:8080/api/v1/upload/sts

# 2. 预校验
$ curl -H "Content-Type: application/json" \
  -H "Authorization: Bearer <TOKEN>" \
  -d '{"file_name":"large-file.mp4","file_size":1073741824,"file_md5":"d41d8cd98f00b204e9800998ecf8427e"}' \
  http://localhost:8080/api/v1/upload/precheck

# 3. 上传分片（二进制流）
$ curl -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/octet-stream" \
  -H "X-Session-Signature: <sig>" \
  -H "X-Session-Timestamp: <ts>" \
  -H "X-Session-Salt: <salt>" \
  --data-binary @/tmp/chunk_0 \
  "http://localhost:8080/api/v1/upload/chunk/upload-binary?task_id=<task_id>&chunk_index=0&chunk_md5=<md5>"

# 后续步骤同上...
```

### 使用 Basic Auth（CLI 端）

```bash
# 0. 计算文件 MD5（CLI 本地计算）
$ md5sum large-file.mp4
d41d8cd98f00b204e9800998ecf8427e

# 1. 获取 STS
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  -H "Content-Type: application/json" \
  -H "X-Trace-Id: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"file_name":"large-file.mp4","file_size":1073741824,"file_md5":"d41d8cd98f00b204e9800998ecf8427e"}' \
  http://localhost:8080/api/v1/upload/sts

# 2. 预校验
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  -H "Content-Type: application/json" \
  -d '{"file_name":"large-file.mp4","file_size":1073741824,"file_md5":"d41d8cd98f00b204e9800998ecf8427e"}' \
  http://localhost:8080/api/v1/upload/precheck

# 3. 上传分片（二进制流，每个分片独立请求）
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  -H "Content-Type: application/octet-stream" \
  -H "X-Session-Signature: <sig>" \
  -H "X-Session-Timestamp: <ts>" \
  -H "X-Session-Salt: <salt>" \
  --data-binary @/tmp/chunk_0 \
  "http://localhost:8080/api/v1/upload/chunk/upload-binary?task_id=<task_id>&chunk_index=0&chunk_md5=<md5>"

# 4. 查询状态
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  -H "Content-Type: application/json" \
  -d '{"task_id":"<task_id>"}' \
  http://localhost:8080/api/v1/upload/chunk/status

# 5. 合并
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  -H "Content-Type: application/json" \
  -d '{"task_id":"<task_id>","file_name":"large-file.mp4","file_md5":"d41d8cd98f00b204e9800998ecf8427e","file_size":1073741824}' \
  http://localhost:8080/api/v1/upload/merge

# 6. 轮询合并状态
$ curl -H "Authorization: Basic YWRtaW5AcnVzdGJ1Y2tldC5sb2NhbDpydXN0YnVja2V0" \
  "http://localhost:8080/api/v1/upload/merge/status?task_id=<task_id>"

# 7. 健康检查（无需认证）
$ curl http://localhost:8080/health
```

### Web 管理后台示例

```bash
# 0. 登录获取 token
$ curl -H "Content-Type: application/json" \
  -d '{"email":"admin@buckets.local","password":"buckets"}' \
  http://localhost:8080/api/v1/auth/login

# 1. 列出用户（Bearer Token 认证）
$ curl -H "Authorization: Bearer <TOKEN>" \
  http://localhost:8080/api/v1/users?page=1&page_size=20

# 2. 创建用户
$ curl -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"email":"newuser@example.com","password":"secure123"}' \
  http://localhost:8080/api/v1/users

# 3. 列出文件
$ curl -H "Authorization: Bearer <TOKEN>" \
  http://localhost:8080/api/v1/objects?page=1&page_size=20

# 4. 删除文件
$ curl -H "Authorization: Bearer <TOKEN>" \
  -X DELETE \
  http://localhost:8080/api/v1/objects/550e8400-e29b-41d4-a716-446655440000
```
