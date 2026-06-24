# Tus 可恢复上传

本文档描述 buckets **Tus 可恢复上传协议**（tus resumable upload protocol 1.0.0）的实现。与分片上传的区别见 [upload-chunked.md](upload-chunked.md)。

---

## 目录

1. [协议概述](#协议概述)
2. [数据库模型](#数据库模型)
3. [API 端点](#api-端点)
4. [上传流程](#上传流程)
5. [签名与认证](#签名与认证)
6. [文件存储与清理](#文件存储与清理)
7. [错误处理](#错误处理)
8. [常量汇总](#常量汇总)

---

## 协议概述

[Tus](https://tus.io) 是开放的、基于 HTTP 的可恢复文件上传协议（v1.0.0）。核心特点：

- **可恢复** — 上传中断后可从断点继续，不丢失已上传数据
- **简单** — 标准 HTTP 方法 + 自定义头部，无需额外库即可使用任何 HTTP 客户端
- **去中心化** — 客户端保存上传 URL，无需服务端额外存储会话状态

### 支持的扩展

| 扩展 | 说明 |
|------|------|
| **Creation** | 通过 POST 创建上传资源，返回 Location URL |
| **Termination** | 通过 DELETE 终止上传并清理 |
| **Expiration** | 上传资源有 TTL，到期自动清理 |

### Tus Vs. 分片上传

| 特性 | Tus 上传 | 分片上传 (Chunked) |
|------|----------|--------------------|
| 协议标准 | 遵循 tus 1.0.0 | 自定义协议 |
| 客户端要求 | 标准 HTTP PATCH + 头部 | 需要 STS 预签名、MD5 预计算 |
| MD5 校验 | 服务端上传完成后计算 | 客户端预提供每个分片 MD5 |
| 并发上传 | 不支持（单流顺序追加） | 支持多线程并行分片 |
| 断点续传 | 通过 Upload-Offset 头部自动支持 | 通过 bitmap + precheck |
| 零拷贝合并 | 不需要（直接写入最终文件） | chunk 合并需要零拷贝 |
| 适用场景 | 简单客户端、浏览器、curl | 高性能大文件并发上传 |

---

## 数据库模型

Tus 上传复用 `upload_tasks` 和 `objects` 表，通过 `upload_method` 字段区分。

### `upload_tasks` 表（tus 特有字段）

| 字段 | 类型 | tus 上传说明 |
|------|------|-------------|
| uuid | VARCHAR(36) (UNIQUE) | 任务 UUID，同时也是 tus 资源标识 |
| file_md5 | VARCHAR(64) | MD5 留空，上传完成后由服务端填充 |
| file_size | BIGINT | 文件总大小，由 `Upload-Length` 头部指定 |
| chunk_size | BIGINT | 始终为 0（tus 不需要分片） |
| chunk_count | BIGINT | 始终为 0（tus 不需要分片） |
| uploaded_bitmap | TEXT | 始终为 `[]`（tus 不使用 bitmap） |
| upload_method | VARCHAR(32) | `tus` |
| current_offset | BIGINT | 已上传数据量（字节），递增 |
| status | VARCHAR(32) | `initialized` / `uploading` / `completed` / `failed` / `expired` |

### `objects` 表（tus 特有字段）

| 字段 | 类型 | tus 说明 |
|------|------|----------|
| upload_method | VARCHAR(32) | `tus` |

---

## API 端点

### 概述

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| `OPTIONS` | `/api/v1/upload/tus` | **不需要** | 查询服务器支持的能力 |
| `POST` | `/api/v1/upload/tus` | 需要 | 创建上传资源 |
| `HEAD` | `/api/v1/upload/tus/{task_id}` | 需要 | 查询上传进度 |
| `PATCH` | `/api/v1/upload/tus/{task_id}` | 需要 | 上传数据 |
| `DELETE` | `/api/v1/upload/tus/{task_id}` | 需要 | 终止上传 |

所有 tus 响应**必须**包含 `Tus-Resumable: 1.0.0` 头部。

### OPTIONS — 能力查询

```
OPTIONS /api/v1/upload/tus
```

**Auth**: 不需要（此端点位于 auth 中间件之前）

**响应头部**：

| 头部 | 值 | 说明 |
|------|-----|------|
| Tus-Resumable | 1.0.0 | 必选，指示协议版本 |
| Tus-Version | 1.0.0 | 服务器支持的版本 |
| Tus-Extension | creation,termination,expiration | 支持的扩展 |
| Tus-Max-Size | 1099511627776 | 最大文件大小（1 TiB） |
| Cache-Control | no-store | 禁止缓存 |

**响应**: `204 No Content`，无 body。

### POST — 创建上传

```
POST /api/v1/upload/tus
Authorization: Bearer <token>  (或 Basic Auth)
Tus-Resumable: 1.0.0
Upload-Length: 1073741824
Upload-Metadata: filename ZXhhbXBsZS5tcDQ=,content_type dmlkZW8vbXA0
```

**Auth**: 需要

**请求头部**：

| 头部 | 必填 | 说明 |
|------|------|------|
| Tus-Resumable | 是 | `1.0.0` |
| Upload-Length | 是 | 文件总大小（字节），必须 > 0 |
| Upload-Metadata | 否 | 元数据，格式：`key <base64>[,key <base64>]` |

`Upload-Metadata` 支持的键：

| 键 | 说明 |
|------|------|
| `filename` | 原始文件名（base64 编码），如不存在使用 `object_id` |
| `content_type` | MIME 类型（base64 编码），服务端据此设置 Content-Type；如不存在则从扩展名推断 |

**响应头部**：

| 头部 | 值 | 说明 |
|------|-----|------|
| Tus-Resumable | 1.0.0 | |
| Location | `/api/v1/upload/tus/{task_id}` | 上传资源的 URL |

**响应**: `201 Created`

**服务端流程**：

```
1. 校验 Tus-Resumable 头部
2. 校验 Upload-Length > 0
3. 解析 Upload-Metadata（base64 解码 filename / content_type）
4. 生成 task_id（UUID v4）
5. 计算动态过期时间（同分片上传逻辑）
6. 创建 upload_tasks 记录:
   - upload_method = 'tus'
   - current_offset = 0
   - status = 'initialized'
   - file_md5 = ''（上传完成后填充）
   - chunk_size = 0, chunk_count = 0, uploaded_bitmap = '[]'
7. 创建暂存目录: data/staging/tus/{task_id}/
8. 将元数据持久化到 meta.json
9. 返回 Location 头部
```

### HEAD — 查询进度

```
HEAD /api/v1/upload/tus/{task_id}
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
```

**Auth**: 需要

**响应头部**：

| 头部 | 值 | 说明 |
|------|-----|------|
| Tus-Resumable | 1.0.0 | |
| Upload-Offset | 1048576 | 当前已上传的字节数 |
| Upload-Length | 1073741824 | 文件总大小 |
| Cache-Control | no-store | |

**响应**: `200 OK`，无 body。

**服务端流程**：

```
1. 解析 task_id → 查 upload_tasks
2. 校验 user_id 归属（403 如果不属于）
3. 检查 status（已过期/失败 → 404）
4. 返回 Upload-Offset + Upload-Length
```

### PATCH — 上传数据

```
PATCH /api/v1/upload/tus/{task_id}
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
Upload-Offset: 1048576
Content-Type: application/offset+octet-stream
Body: <binary data>
```

**Auth**: 需要

**请求头部**：

| 头部 | 必填 | 说明 |
|------|------|------|
| Tus-Resumable | 是 | `1.0.0` |
| Upload-Offset | 是 | 当前期望的偏移量，必须与服务端 `current_offset` 一致 |
| Content-Type | 是 | `application/offset+octet-stream` |

**响应头部**：

| 头部 | 值 | 说明 |
|------|-----|------|
| Tus-Resumable | 1.0.0 | |
| Upload-Offset | 2097152 | 新的偏移量 |
| Cache-Control | no-store | |

**响应**: `204 No Content`，无 body。

**服务端流程**：

```
1. 查 task → 校验 user_id 归属
2. 校验 Upload-Offset == task.current_offset
   → 不匹配 → 409 Conflict
3. 校验 task 未完成/未过期
4. 校验不超出 file_size
5. 将 body stream 追加到暂存文件 data/staging/tus/{task_id}/data
6. new_offset = expected_offset + bytes_written
7. 更新 DB:
   - 如果 status == 'initialized' → status = 'uploading'
   - current_offset = new_offset
8. 如果 new_offset >= file_size:
   → tokio::spawn 异步完成:
     a. spawn_blocking: 流式计算 MD5
     b. 读取 meta.json 获取 filename / content_type
     c. 计算最终存储路径
     d. 原子 rename 暂存文件到最终路径
     e. ORM 事务：insert objects + insert user_objects + update tasks status = 'completed'
     f. 清理暂存目录
```

**并发安全**：

PATCH 请求的并发安全性通过 `Upload-Offset` 校验实现。如果两个 PATCH 请求同时到达，只有一个能通过偏移量校验，另一个会收到 **409 Conflict**。

### DELETE — 终止上传

```
DELETE /api/v1/upload/tus/{task_id}
Authorization: Bearer <token>
Tus-Resumable: 1.0.0
```

**Auth**: 需要

**响应**: `204 No Content`

**服务端流程**：

```
1. 查 task → 校验 user_id 归属
2. 删除暂存目录 data/staging/tus/{task_id}/
3. 更新 task status = 'expired'
```

---

## 上传流程

### 完整示例 (curl)

```bash
# 0. 登录获取 token
TOKEN=$(curl -s -H "Content-Type: application/json" \
  -d '{"email":"admin@buckets.local","password":"buckets"}' \
  http://localhost:8080/api/v1/auth/login | jq -r '.data.token')

# 1. OPTIONS — 查询能力
curl -X OPTIONS http://localhost:8080/api/v1/upload/tus -i

# 2. POST — 创建上传（文件名 "large-file.mp4"）
#    filename 和 content_type 用 base64 编码
UPLOAD_URL=$(curl -s -i -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Tus-Resumable: 1.0.0" \
  -H "Upload-Length: 1073741824" \
  -H "Upload-Metadata: filename $(echo -n 'large-file.mp4' | base64),content_type $(echo -n 'video/mp4' | base64)" \
  http://localhost:8080/api/v1/upload/tus \
  | grep -i location | awk '{print $2}' | tr -d '\r')
# 提取 task_id
TASK_ID=$(echo $UPLOAD_URL | grep -oP '[^/]+$')

# 3. HEAD — 查询进度
curl -s -i -X HEAD \
  -H "Authorization: Bearer $TOKEN" \
  -H "Tus-Resumable: 1.0.0" \
  http://localhost:8080/api/v1/upload/tus/$TASK_ID \
  | grep -i upload-offset

# 4. PATCH — 上传数据（可多次调用，支持断点续传）
curl -s -i -X PATCH \
  -H "Authorization: Bearer $TOKEN" \
  -H "Tus-Resumable: 1.0.0" \
  -H "Upload-Offset: 0" \
  -H "Content-Type: application/offset+octet-stream" \
  --data-binary @/path/to/data.bin \
  http://localhost:8080/api/v1/upload/tus/$TASK_ID

# 5. DELETE — 终止上传（可选）
curl -s -i -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  -H "Tus-Resumable: 1.0.0" \
  http://localhost:8080/api/v1/upload/tus/$TASK_ID
```

### 断点续传流程

```
1. 客户端记录 task_id（存于本地文件或数据库）
2. 恢复时：
   a. HEAD /api/v1/upload/tus/{task_id}
      → 获取 Upload-Offset
   b. 从 offset 处读取文件剩余部分
   c. PATCH 请求带正确的 Upload-Offset
```

### 空文件上传

`Upload-Length: 0` 表示上传空文件。此时 POST 创建后上传立即完成：
- 服务端创建空的暂存文件
- 异步完成流程立即触发：MD5（空文件 MD5 = `d41d8cd98f00b204e9800998ecf8427e`）+ 建对象记录 + 移文件

---

## 签名与认证

### 认证方式

Tus 端点（OPTIONS 除外）使用与 buckets 其他 API 相同的统一认证：

| 方式 | 格式 | 适用场景 |
|------|------|----------|
| Bearer Token | `Authorization: Bearer <jwt>` | Web 前端、SDK |
| Basic Auth | `Authorization: Basic base64(email:password)` | CLI、curl |

### 无需认证的端点

`OPTIONS /api/v1/upload/tus` — 位于 auth 中间件之前，任何客户端均可查询。

---

## 文件存储与清理

### 暂存路径

```
data/staging/tus/{task_id}/
├── data          # 追加写入的上传数据
└── meta.json     # 上传元数据（文件名、MIME 类型、扩展名）
```

### 最终存储路径

```
data/objects/{user_id}/{YYYYMMDD}/{object_id}.{ext}
```

### 过期清理

- 后台 `ref_check` 任务定期扫描过期的 tus 上传任务
- 清理 `data/staging/tus/{task_id}/` 目录
- 标记任务 status = 'expired'

### 与分片上传暂存的隔离

Tus 暂存目录使用独立的子路径 `data/staging/tus/`，与分片上传的 `data/staging/` 隔离，确保：
- 分片 GC `gc_clean` 不会误清理 tus 暂存文件（路径不匹配）
- Tus 暂存的清理由 `ref_check` 负责
- 上传完成后立即清理暂存目录

---

## 错误处理

### Tus 特有错误

| 场景 | HTTP Status | 说明 |
|------|-------------|------|
| 缺少 Tus-Resumable 头部 | 400 | 必须携带 `Tus-Resumable: 1.0.0` |
| 缺少 Upload-Length | 400 | POST 时必填 |
| 缺少 Upload-Offset | 400 | PATCH 时必填 |
| Content-Type 不匹配 | 400 | PATCH 必须为 `application/offset+octet-stream` |
| Offset 不匹配 | 409 | 并发冲突，客户端应从 HEAD 获取最新 offset |
| 上传已完成 | 400 | 任务处于 terminal 状态 |
| 数据超限 | 400 | PATCH 数据超出剩余 file_size |
| 不存在 | 404 | task_id 无效或已过期 |
| 不属于当前用户 | 403 | 任务归属校验失败 |

### 错误响应格式

所有 tus 错误响应以 JSON body 返回（区别于标准 tus 协议，便于调试）：

```json
{
    "code": 409,
    "message": "offset mismatch: expected 1048576, got 0",
    "data": null
}
```

并且携带 tus 必需头部：
- `Tus-Resumable: 1.0.0`
- `Cache-Control: no-store`

### 异步完成失败处理

如果 PATCH 触发的异步完成流程失败（如 MD5 计算错误、磁盘 I/O 错误）：

1. 错误日志记录 trace
2. task status 标记为 `failed`
3. 暂存文件保留（可重新 PATCH 或重新创建）

---

## 常量汇总

| 常量 | 值 | 说明 |
|------|-----|------|
| `TUS_PROTOCOL_VERSION` | `1.0.0` | tus 协议版本 |
| `TUS_DEFAULT_MAX_SIZE` | 1099511627776 (1 TiB) | 最大文件大小 |
| `TUS_STAGING_SUBDIR` | `tus` | tus 暂存子目录名 |
| `HEADER_TUS_RESUMABLE` | `Tus-Resumable` | 协议版本头部 |
| `HEADER_TUS_VERSION` | `Tus-Version` | 服务端版本头部 |
| `HEADER_TUS_EXTENSION` | `Tus-Extension` | 扩展声明头部 |
| `HEADER_TUS_MAX_SIZE` | `Tus-Max-Size` | 最大大小头部 |
| `HEADER_UPLOAD_LENGTH` | `Upload-Length` | 文件大小头部 |
| `HEADER_UPLOAD_OFFSET` | `Upload-Offset` | 偏移量头部 |
| `HEADER_UPLOAD_METADATA` | `Upload-Metadata` | 元数据头部 |
| `HEADER_UPLOAD_DEFER_LENGTH` | `Upload-Defer-Length` | 延迟设置大小头部 |

---

## 文件结构索引

| 文件 | 职责 |
|------|------|
| `buckets-srv/src/api/tus.rs` | Tus 5 个端点处理器 + 响应包装 |
| `buckets-srv/src/api/mod.rs` | Tus 路由注册 |
| `buckets-srv/src/app.rs` | OPTIONS 路由（auth 外）+ CORS 允许 PATCH/tus 头部 |
| `buckets-srv/src/service/tus_svc.rs` | Tus 业务逻辑（创建、追加、完成） |
| `buckets-srv/src/dao/tasks.rs` | Tus 任务 DAO（创建、更新偏移量、设置 MD5） |
| `buckets-common/src/constant/upload.rs` | Tus 常量 |
| `buckets-common/src/model/db.rs` | `UploadTask` + `ObjectMeta` 实体（`upload_method`、`current_offset` 字段） |
