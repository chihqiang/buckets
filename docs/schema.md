# buckets 数据库表结构设计

## 概述

buckets 使用 MySQL 8.0+ 作为元数据存储，共 4 张表：`users`（用户）、`objects`（对象）、`user_objects`（用户-对象关联）、`upload_tasks`（上传任务）。所有 timestamp 字段使用 `TIMESTAMP(6)` 以支持微秒精度。DDL 管理使用 sea-orm-migration 迁移框架（`migration/` crate）。此外还有三个内存缓存：`AuthCache`（Basic Auth 缓存）、`SecretKeyCache`（用户密钥缓存）、`BitmapCache`（上传进度位图缓存）。

## 0. users — 用户表

存储系统用户信息。自增主键，email 唯一索引。

```sql
CREATE TABLE users (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    email           VARCHAR(256) NOT NULL COMMENT '邮箱',
    password        VARCHAR(256) NOT NULL COMMENT '密码(哈希)',
    secret_key      VARCHAR(128) DEFAULT NULL COMMENT 'HMAC签名密钥',
    created_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE INDEX idx_users_email (email)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 字段说明

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT UNSIGNED | PK, AUTO_INCREMENT | 自增用户 ID，其他表通过此 ID 引用 |
| email | VARCHAR(256) | NOT NULL, UNIQUE | 邮箱地址，登录凭证 |
| password | VARCHAR(256) | NOT NULL | argon2 哈希后的密码 |
| secret_key | VARCHAR(128) | NULLABLE | HMAC-SHA256 签名密钥，64 字符 hex，migration 时自动生成（DEFAULT_SECRET_KEY 或后续重置） |
| created_at | TIMESTAMP(6) | NOT NULL, DEFAULT NOW | 创建时间 |
| updated_at | TIMESTAMP(6) | NOT NULL, ON UPDATE | 最后更新时间，自动更新 |

### 种子用户

由 migration 在首次运行时自动创建默认管理员（通过 `sea-orm-migration` 的 `insert()` 操作）：

| email | password (raw) | password (stored) | secret_key |
|-------|---------------|-------------------|------------|
| admin@buckets.local | buckets | argon2 哈希（由 `ADMIN_PASSWORD` 环境变量指定，默认 `buckets`） | 自动生成（DEFAULT_SECRET_KEY） |

创建逻辑（migration `m20220101_000001_create_tables.rs`）：
1. 读取 `ADMIN_PASSWORD` 环境变量（默认 `"buckets"`）
2. 调用 `hash_password()` 计算 argon2 哈希
3. `INSERT INTO users` 写入 email、argon2 密码、默认 secret_key

### 查询模式

```sql
-- 登录验证（auth middleware）
SELECT id, password FROM users WHERE email = ?;

-- 用户信息查询
SELECT id, email, created_at, updated_at FROM users WHERE id = ?;
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| idx_users_email | email | UNIQUE | 登录查询 + 防止重复注册 |

---

## 1. objects — 对象表

存储已上传完成的文件信息。自增主键，`uuid` 作为业务标识，MD5 索引用于秒传去重。用户与对象的关联通过 `user_objects` 表管理。

```sql
CREATE TABLE objects (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    uuid            VARCHAR(36) NOT NULL COMMENT 'UUID 业务标识',
    name            VARCHAR(1024) NOT NULL COMMENT '原始文件名',
    size            BIGINT NOT NULL COMMENT '文件大小(字节)',
    md5             VARCHAR(64) NOT NULL COMMENT '文件MD5(去重依据)',
    content_type    VARCHAR(256) COMMENT 'MIME类型',
    extension       VARCHAR(64) COMMENT '文件扩展名',
    bucket          VARCHAR(256) NOT NULL DEFAULT 'default' COMMENT '存储桶',
    storage_path    VARCHAR(1024) COMMENT '物理存储路径',
    image_width     INT NOT NULL DEFAULT 0 COMMENT '图片宽度',
    image_height    INT NOT NULL DEFAULT 0 COMMENT '图片高度',
    image_type      VARCHAR(32) NOT NULL DEFAULT '' COMMENT '图片类型',
    status          VARCHAR(32) NOT NULL DEFAULT 'active' COMMENT '状态: active/deleted',
    created_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE INDEX idx_objects_uuid (uuid),
    INDEX idx_objects_md5 (md5)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 字段说明

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT UNSIGNED | PK, AUTO_INCREMENT | 自增主键，内部引用用 |
| uuid | VARCHAR(36) | NOT NULL, UNIQUE | UUID v4，业务标识，对外暴露 |
| name | VARCHAR(1024) | NOT NULL | 原始上传文件名 |
| size | BIGINT | NOT NULL | 文件大小（字节），支持最大 9EB |
| md5 | VARCHAR(64) | NOT NULL | 文件 MD5 哈希，全局去重依据 |
| content_type | VARCHAR(256) | NULLABLE | MIME 类型，如 `video/mp4` |
| extension | VARCHAR(64) | NULLABLE | 文件扩展名，如 `mp4` |
| bucket | VARCHAR(256) | NOT NULL, DEFAULT 'default' | 存储桶名称，用于多租户隔离 |
| storage_path | VARCHAR(1024) | NULLABLE | 物理文件存储的相对路径 |
| image_width | INT | NOT NULL, DEFAULT 0 | 图片宽度（像素） |
| image_height | INT | NOT NULL, DEFAULT 0 | 图片高度（像素） |
| image_type | VARCHAR(32) | NOT NULL, DEFAULT '' | 图片类型/格式 |
| status | VARCHAR(32) | NOT NULL, DEFAULT 'active' | 状态: `active` / `deleted` |
| created_at | TIMESTAMP(6) | NOT NULL | 创建时间 |
| updated_at | TIMESTAMP(6) | NOT NULL | 最后更新时间 |

### 状态流转

```
active ──► DELETE 请求（最后一位所有者）──► deleted ──► ref_check GC ──► 物理文件删除
  │
  └──（其他用户秒传）──► user_objects 新增关联记录
```

- **active**: 正常可用状态
- **deleted**: 最后一位所有者发起 DELETE 后的软删除状态，业务不可见，等待后台 GC

### 秒传机制

```
 用户 A 上传文件 → objects 表插入记录 + user_objects 插入 (A, object_id)

用户 B 上传相同文件（相同 MD5）：
  └── precheck 查询：
SELECT * FROM objects WHERE md5 = ? AND bucket = ? AND status = 'active' AND size = ?
        ├── 存在 → user_objects 插入 (B, object_id)
        │          直接返回对象信息（秒传）
        └── 不存在 → 创建新上传任务
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键查询 |
| idx_objects_uuid | uuid | UNIQUE | UUID 业务标识查询 |
| idx_objects_md5 | md5 | INDEX | 秒传去重查询 |

### 查询模式

```sql
-- 秒传检测
SELECT * FROM objects WHERE md5 = ? AND bucket = ? AND status = 'active' AND size = ?;

-- 对象详情
SELECT * FROM objects WHERE uuid = ?;

-- GC 查询待清理对象（无所有者的已删除对象）
SELECT o.id, o.storage_path FROM objects o
WHERE o.status = 'deleted'
AND NOT EXISTS (SELECT 1 FROM user_objects uo WHERE uo.object_id = o.id);

-- 用户文件列表
SELECT o.uuid, o.name, o.size, o.md5, o.created_at
FROM objects o
INNER JOIN user_objects uo ON o.id = uo.object_id
WHERE uo.user_id = ? AND o.status = 'active';
```

---

## 2. user_objects — 用户-对象关联表

实现多对多关系：一个文件可被多个用户拥有（秒传场景）。自增主键，`(user_id, object_id)` 唯一约束。

```sql
CREATE TABLE user_objects (
    id          BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    user_id     BIGINT UNSIGNED NOT NULL COMMENT '用户ID',
    object_id   BIGINT UNSIGNED NOT NULL COMMENT '对象ID',
    created_at  TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    UNIQUE INDEX idx_user_objects (user_id, object_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 字段说明

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT UNSIGNED | PK, AUTO_INCREMENT | 自增主键 |
| user_id | BIGINT UNSIGNED | NOT NULL | 用户 ID |
| object_id | BIGINT UNSIGNED | NOT NULL | 对象 ID（关联 objects.id） |
| created_at | TIMESTAMP(6) | NOT NULL | 关联创建时间 |

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键 |
| idx_user_objects | (user_id, object_id) | UNIQUE | 唯一约束，防止重复关联 |

### 查询模式

```sql
-- 检查用户是否拥有某对象
SELECT 1 FROM user_objects WHERE user_id = ? AND object_id = ?;

-- 获取对象的所有所有者
SELECT user_id FROM user_objects WHERE object_id = ?;

-- 删除用户-对象关联
DELETE FROM user_objects WHERE user_id = ? AND object_id = ?;
```

---

## 3. upload_tasks — 上传任务表

存储文件上传过程中的任务状态和分片进度。支持断点续传和位图查询。

```sql
CREATE TABLE upload_tasks (
    id              BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    uuid            VARCHAR(36) NOT NULL COMMENT 'UUID 业务标识',
    object_id       VARCHAR(36) NOT NULL COMMENT '关联对象UUID',
    file_md5        VARCHAR(64) NOT NULL COMMENT '文件MD5',
    file_size       BIGINT NOT NULL COMMENT '文件大小',
    chunk_size      BIGINT NOT NULL COMMENT '分片大小',
    chunk_count     INT NOT NULL COMMENT '总分片数',
    user_id         BIGINT UNSIGNED NOT NULL COMMENT '上传用户ID',
    status          VARCHAR(32) NOT NULL DEFAULT 'initialized' COMMENT '状态',
    uploaded_bitmap TEXT NOT NULL COMMENT '已上传分片位图(JSON Vec<u64>)',
    last_activity_at BIGINT DEFAULT NULL COMMENT '最后活跃时间戳(Unix秒)',
    created_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    expires_at      TIMESTAMP(6) NOT NULL COMMENT '过期时间(动态计算)',
    UNIQUE INDEX idx_upload_tasks_uuid (uuid),
    INDEX idx_upload_tasks_md5 (file_md5),
    INDEX idx_upload_tasks_user (user_id),
    INDEX idx_upload_tasks_expires (expires_at),
    INDEX idx_upload_tasks_status_expires (status, expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 字段说明

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT UNSIGNED | PK, AUTO_INCREMENT | 自增主键 |
| uuid | VARCHAR(36) | NOT NULL, UNIQUE | UUID v4，上传任务标识，对外暴露 |
| object_id | VARCHAR(36) | NOT NULL | 关联的对象 UUID |
| file_md5 | VARCHAR(64) | NOT NULL | 文件 MD5，用于续传时查询已有任务 |
| file_size | BIGINT | NOT NULL | 文件大小（字节） |
| chunk_size | BIGINT | NOT NULL | 分片大小（字节），默认 8MB |
| chunk_count | INT | NOT NULL | 总分片数 |
| user_id | BIGINT UNSIGNED | NOT NULL | 上传用户 ID |
| status | VARCHAR(32) | NOT NULL | 状态: `initialized` / `uploading` / `merging` / `completed` / `failed` / `expired` |
| uploaded_bitmap | TEXT | NOT NULL | 已上传分片位图，JSON 格式 `Vec<u64>` |
| last_activity_at | BIGINT | NULLABLE | 最后活跃时间戳（Unix 秒），用于会话测活 |
| created_at | TIMESTAMP(6) | NOT NULL | 创建时间 |
| updated_at | TIMESTAMP(6) | NOT NULL | 自动更新时间 |
| expires_at | TIMESTAMP(6) | NOT NULL | 过期时间，动态计算 |

### 状态流转

```
initialized ──► 首个分片上传 ──► uploading ──► merge 请求 ──► merging
                  │                    │              │
                  │                    │              ├── MD5 成功 ──► completed
                  │                    │              └── MD5 失败 ──► failed
                  │                    │
                  │                    └── 会话超时 ──► expired（GC 清理）
                  │
                  └── 72h+ 无活动 ──► expired（GC 清理）
```

### 动态过期时间计算

```
基础过期: 72h
每 10GB 文件增加: 24h

基础测活窗口: 1h
每 10GB 文件增加: 1h
最大测活窗口: 48h
```

每次分片上传成功后刷新 `last_activity_at`，延长会话有效期。

### 位图机制详解

`uploaded_bitmap` 使用 JSON 格式存储 `Vec<u64>` 数组，每个 bit 代表一个分片是否已上传。

#### 数据结构

```text
文件总分片数: 128
位图数组大小: (128 + 63) / 64 = 2

u64[0] = 0b0000...0111  → 分片 0,1,2 已上传
u64[1] = 0b0000...1000  → 分片 64 已上传

JSON 存储: "[7, 8]"   // 注意 JSON 存的是十进制数
```

#### 位操作

```rust
// 设置分片 index 为已上传
let word = index as usize / 64;
let bit = index % 64;
bitmap[word] |= 1u64 << bit;

// 查询分片 index 是否已上传
let word = index as usize / 64;
let bit = index % 64;
let uploaded = (bitmap[word] & (1u64 << bit)) != 0;
```

#### 查询已上传分片列表

```rust
let mut uploaded = Vec::new();
for i in 0..chunk_count {
    let word = i as usize / 64;
    let bit = i % 64;
    if word < bitmap.len() && (bitmap[word] & (1u64 << bit)) != 0 {
        uploaded.push(i);
    }
}
```

#### 缺失分片列表

```rust
let mut missing = Vec::new();
for i in 0..chunk_count {
    let word = i as usize / 64;
    let bit = i % 64;
    if word >= bitmap.len() || (bitmap[word] & (1u64 << bit)) == 0 {
        missing.push(i);
    }
}
```

#### 初始化与空位图

```rust
// 空位图（所有分片未上传）
let word_count = (chunk_count as usize + 63) / 64;
let bitmap = vec![0u64; word_count];
// JSON: "[0, 0, ...]"
```

### 原子更新

位图更新使用 MySQL `UPDATE ... WHERE` 原子操作，防止并发写入冲突：

```sql
UPDATE upload_tasks
SET uploaded_bitmap = ?,
    status = CASE WHEN status = 'initialized' THEN 'uploading' ELSE status END,
    last_activity_at = ?
WHERE id = ?;
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键查询 |
| idx_upload_tasks_md5 | file_md5 | INDEX | 断点续传时查找已有任务 |
| idx_upload_tasks_user | user_id | INDEX | 按用户查询上传任务 + 速率限制并发检查 |
| idx_upload_tasks_expires | expires_at | INDEX | GC 清理过期任务 |

### 查询模式

```sql
-- 续传查询
SELECT * FROM upload_tasks
WHERE file_md5 = ? AND user_id = ?
AND status IN ('initialized', 'uploading');

-- 查询上传任务详情
SELECT * FROM upload_tasks WHERE id = ?;

-- 原子更新位图
UPDATE upload_tasks
SET uploaded_bitmap = ?, status = 'uploading', last_activity_at = ?
WHERE id = ?;

-- 更新合并状态
UPDATE upload_tasks SET status = 'merging' WHERE id = ?;
UPDATE upload_tasks SET status = 'completed' WHERE id = ?;

-- GC 查询过期任务
SELECT * FROM upload_tasks
WHERE expires_at < NOW()
AND status NOT IN ('completed', 'expired');

-- 速率限制：并发检查
SELECT COUNT(*) FROM upload_tasks
WHERE user_id = ? AND status IN ('initialized', 'uploading');

-- 速率限制：每日配额检查
SELECT COUNT(*) FROM upload_tasks
WHERE user_id = ? AND created_at >= CURDATE();
```

---

## 4. 外键约束

当前版本未在数据库层面设置外键约束。关联关系（如 `user_objects.object_id` → `objects.id`）由应用层逻辑保证，由 sea-orm 实体定义。设计原因是外键约束在大规模写操作场景下可能带来性能开销和锁竞争。业务层通过 DAO 保证引用完整性。

---

## 5. 性能建议

### 大表索引维护

- `idx_objects_md5` 和 `idx_upload_tasks_md5` 是高频查询索引（秒传/续传）
- `idx_upload_tasks_expires` 和 `idx_upload_tasks_status_expires` 是 GC 任务的核心索引，确保定期清理不阻塞
- `idx_upload_tasks_user` 同时服务于续传查询和速率限制并发检查
- `idx_user_objects` 用于查询用户-对象关联和 GC 清理

### 分页查询

```sql
-- 用户文件列表（通过 user_objects 关联表）
SELECT o.uuid, o.name, o.size, o.md5, o.created_at
FROM objects o
INNER JOIN user_objects uo ON o.id = uo.object_id
WHERE uo.user_id = ? AND o.status = 'active'
ORDER BY o.created_at DESC
LIMIT 20 OFFSET 0;
```

### 数据生命周期

- `upload_tasks` 在 `completed` 或 `expired` 后可考虑归档或删除
- `objects` 的 `deleted` 记录保留用于审计，可按时间定期清理
- 过期上传任务的暂存文件由 `gc_clean` 后台任务清理（30 分钟间隔）
- 已删除对象的物理文件由 `ref_check` 后台任务清理（2 小时间隔）

### MySQL 配置建议

```ini
# my.cnf
[mysqld]
innodb_buffer_pool_size = 70%_of_RAM
max_allowed_packet = 64M
innodb_log_file_size = 512M
innodb_flush_log_at_trx_commit = 2
```

### 存储类型说明

- `uploaded_bitmap` 使用 `TEXT`（最大 64KB），支持超大型文件（如 100 万分片 = 约 8MB JSON，TEXT 容量足够）
- `storage_path` 使用 `VARCHAR(1024)`，存储相对路径足够
- 所有 timestamp 使用 `TIMESTAMP(6)` 微秒精度，支持精确时间比较

---

## 6. 内存数据结构

### AuthCache（Basic Auth 认证缓存）

- **类型**：`DashMap<String, AuthCacheEntry>`
- **Key**：`SHA256("email:password")`（hex 编码）
- **Value**：`{ user_id: i64, expires_at: Instant }`
- **TTL**：30 分钟
- **清理**：后台定时器每 30 分钟扫描并删除过期条目

### SecretKeyCache（用户密钥缓存）

- **类型**：`DashMap<String, SecretKeyCacheEntry>`
- **Key**：用户 ID 字符串
- **Value**：`{ secret_key: String, expires_at: Instant }`
- **TTL**：30 分钟
- **作用**：缓存用户 HMAC 签名密钥，避免每个 chunk 上传都查 DB
- **清理**：后台定时器每 30 分钟扫描并删除过期条目

### BitmapCache（上传进度位图缓存）

- **类型**：`DashMap<String, Arc<RwLock<TaskBitmap>>>`
- **Key**：task_id
- **Value**：`{ words: Vec<u64>, chunk_count: u32, dirty: bool }`
- **缓冲**：每 5 秒批量刷新 dirty 位图到 DB
- **上限**：500 条目，超出时淘汰干净条目
