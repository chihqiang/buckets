# buckets 数据库表结构设计

## 概述

buckets 支持 SQLite 和 MySQL 8.0+ 作为元数据存储，共 4 张表：`users`（用户）、`objects`（对象）、`user_objects`（用户-对象关联）、`upload_tasks`（上传任务）。所有 timestamp 字段使用微秒精度。DDL 管理使用 sea-orm-migration 迁移框架（`migration/` crate），同一套迁移代码同时兼容 SQLite 和 MySQL。此外还有三个内存缓存：`AuthCache`（Basic Auth 缓存）、`SecretKeyCache`（用户密钥缓存）、`BitmapCache`（上传进度位图缓存）。

## 0. users — 用户表

存储系统用户信息。自增主键，email 唯一索引。

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT | PK, AUTO_INCREMENT | 自增用户 ID |
| email | VARCHAR(256) | NOT NULL, UNIQUE | 邮箱地址，登录凭证 |
| password | VARCHAR(256) | NOT NULL | argon2 哈希后的密码 |
| secret_key | VARCHAR(128) | NULLABLE | HMAC-SHA256 签名密钥，64 字符 hex |
| created_at | TIMESTAMP | NOT NULL | 创建时间 |
| updated_at | TIMESTAMP | NOT NULL | 最后更新时间，自动更新 |

### 种子用户

由 migration 在首次运行时自动创建默认管理员（通过 `sea-orm-migration` 的 `insert()` 操作）：

| email | password (raw) | password (stored) | secret_key |
|-------|---------------|-------------------|------------|
| admin@buckets.local | buckets | argon2 哈希（由 `ADMIN_PASSWORD` 环境变量指定，默认 `buckets`） | 自动生成（DEFAULT_SECRET_KEY） |

创建逻辑（migration `m20220101_000001_create_tables.rs`）：
1. 读取 `ADMIN_PASSWORD` 环境变量（默认 `"buckets"`）
2. 调用 `hash_password()` 计算 argon2 哈希
3. ORM `insert()` 写入 email、argon2 密码、默认 secret_key

### 查询模式（ORM）

```rust
// 登录验证（auth middleware）
user::Entity::find()
    .select_only()
    .column(user::Column::Id)
    .column(user::Column::Password)
    .filter(user::Column::Email.eq(email))
    .one(db)
    .await?;

// 用户信息查询
user::Entity::find_by_id(id).one(db).await?;
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| idx_users_email | email | UNIQUE | 登录查询 + 防止重复注册 |

---

## 1. objects — 对象表

存储已上传完成的文件信息。自增主键，`uuid` 作为业务标识，MD5 索引用于秒传去重。用户与对象的关联通过 `user_objects` 表管理。

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT | PK, AUTO_INCREMENT | 自增主键 |
| uuid | VARCHAR(36) | NOT NULL, UNIQUE | UUID v4，业务标识 |
| name | VARCHAR(1024) | NOT NULL | 原始上传文件名 |
| size | BIGINT | NOT NULL | 文件大小（字节） |
| md5 | VARCHAR(64) | NOT NULL | 文件 MD5 哈希，全局去重依据 |
| content_type | VARCHAR(256) | NULLABLE | MIME 类型，如 `video/mp4` |
| extension | VARCHAR(64) | NULLABLE | 文件扩展名，如 `mp4` |
| bucket | VARCHAR(256) | NOT NULL, DEFAULT 'default' | 存储桶名称 |
| storage_path | VARCHAR(1024) | NULLABLE | 物理文件存储的相对路径 |
| image_width | INT | NOT NULL, DEFAULT 0 | 图片宽度（像素） |
| image_height | INT | NOT NULL, DEFAULT 0 | 图片高度（像素） |
| image_type | VARCHAR(32) | NOT NULL, DEFAULT '' | 图片类型/格式 |
| upload_method | VARCHAR(32) | NOT NULL, DEFAULT 'chunk' | 上传方式: `chunk` / `tus` |
| status | VARCHAR(32) | NOT NULL, DEFAULT 'active' | 状态: `active` / `deleted` |
| created_at | TIMESTAMP | NOT NULL | 创建时间 |
| updated_at | TIMESTAMP | NOT NULL | 最后更新时间 |

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
用户 A 上传 → ORM 插入 objects + user_objects 记录

用户 B 上传相同文件（相同 MD5）：
  └── precheck ORM 查询 objects（按 md5 + bucket + status = 'active' + size）
        ├── 存在 → ORM 插入 user_objects (B, object_id)
        │          直接返回对象信息（秒传）
        └── 不存在 → 创建新上传任务
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键查询 |
| idx_objects_uuid | uuid | UNIQUE | UUID 业务标识查询 |
| idx_objects_md5 | md5 | INDEX | 秒传去重查询 |

### 查询模式（ORM）

```rust
// 秒传检测
object::Entity::find()
    .filter(object::Column::Md5.eq(md5))
    .filter(object::Column::Bucket.eq(bucket))
    .filter(object::Column::Status.eq("active"))
    .filter(object::Column::Size.eq(size))
    .one(db)
    .await?;

// 对象详情
object::Entity::find()
    .filter(object::Column::Uuid.eq(uuid))
    .one(db)
    .await?;

// GC 查询待清理对象（无所有者的已删除对象）
object::Entity::find()
    .filter(object::Column::Status.eq("deleted"))
    .find_also_related(user_object::Entity)
    .all(db)
    .await?;
    // 在 Rust 侧过滤 user_object 为 None 的记录

// 用户文件列表
object::Entity::find()
    .inner_join(user_object::Entity)
    .filter(user_object::Column::UserId.eq(user_id))
    .filter(object::Column::Status.eq("active"))
    .order_by_desc(object::Column::CreatedAt)
    .all(db)
    .await?;
```

---

## 2. user_objects — 用户-对象关联表

实现多对多关系：一个文件可被多个用户拥有（秒传场景）。自增主键，`(user_id, object_id)` 唯一约束。

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT | PK, AUTO_INCREMENT | 自增主键 |
| user_id | BIGINT | NOT NULL | 用户 ID |
| object_id | BIGINT | NOT NULL | 对象 ID |
| created_at | TIMESTAMP | NOT NULL | 关联创建时间 |

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键 |
| idx_user_objects | (user_id, object_id) | UNIQUE | 唯一约束，防止重复关联 |

### 查询模式（ORM）

```rust
// 检查用户是否拥有某对象
user_object::Entity::find()
    .filter(user_object::Column::UserId.eq(user_id))
    .filter(user_object::Column::ObjectId.eq(object_id))
    .count(db)
    .await?;

// 获取对象的所有所有者
user_object::Entity::find()
    .filter(user_object::Column::ObjectId.eq(object_id))
    .all(db)
    .await?;

// 删除用户-对象关联
user_object::Entity::delete_many()
    .filter(user_object::Column::UserId.eq(user_id))
    .filter(user_object::Column::ObjectId.eq(object_id))
    .exec(db)
    .await?;
```

---

## 3. upload_tasks — 上传任务表

存储文件上传过程中的任务状态和分片进度。支持断点续传和位图查询。

| 字段 | 类型 | 约束 | 说明 |
|------|------|------|------|
| id | BIGINT | PK, AUTO_INCREMENT | 自增主键 |
| uuid | VARCHAR(36) | NOT NULL, UNIQUE | UUID v4，上传任务标识 |
| object_id | VARCHAR(36) | NOT NULL | 关联的对象 UUID |
| file_md5 | VARCHAR(64) | NOT NULL | 文件 MD5，用于续传时查询已有任务 |
| file_size | BIGINT | NOT NULL | 文件大小（字节） |
| chunk_size | BIGINT | NOT NULL | 分片大小（字节），默认 8MB |
| chunk_count | INT | NOT NULL | 总分片数 |
| user_id | BIGINT | NOT NULL | 上传用户 ID |
| status | VARCHAR(32) | NOT NULL | 状态: `initialized` / `uploading` / `merging` / `completed` / `failed` / `expired` |
| uploaded_bitmap | TEXT | NOT NULL | 已上传分片位图，JSON 格式 `Vec<u64>` |
| last_activity_at | BIGINT | NULLABLE | 最后活跃时间戳（Unix 秒），用于会话测活 |
| created_at | TIMESTAMP | NOT NULL | 创建时间 |
| updated_at | TIMESTAMP | NOT NULL | 自动更新时间 |
| expires_at | TIMESTAMP | NOT NULL | 过期时间，动态计算 |

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

位图更新使用 ORM 的 `update_many()` 原子操作，防止并发写入冲突：

```rust
upload_task::Entity::update_many()
    .col_expr(
        upload_task::Column::UploadedBitmap,
        upload_task::Column::UploadedBitmap.into_expr(),
    )
    .col_expr(
        upload_task::Column::Status,
        upload_task::Column::Status.into_expr(),
    )
    .filter(upload_task::Column::Id.eq(id))
    .exec(db)
    .await?;
```

### 索引说明

| 索引名 | 列 | 类型 | 用途 |
|--------|-----|------|------|
| PRIMARY | id | PK | 主键查询 |
| idx_upload_tasks_md5 | file_md5 | INDEX | 断点续传时查找已有任务 |
| idx_upload_tasks_user | user_id | INDEX | 按用户查询上传任务 + 速率限制并发检查 |
| idx_upload_tasks_expires | expires_at | INDEX | GC 清理过期任务 |

### 查询模式（ORM）

```rust
// 续传查询
upload_task::Entity::find()
    .filter(upload_task::Column::FileMd5.eq(md5))
    .filter(upload_task::Column::UserId.eq(user_id))
    .filter(upload_task::Column::Status.is_in(["initialized", "uploading"]))
    .one(db)
    .await?;

// 查询上传任务详情
upload_task::Entity::find_by_id(id).one(db).await?;

// 原子更新位图
upload_task::Entity::update_many()
    .set(upload_task::ActiveModel {
        uploaded_bitmap: Set(new_bitmap),
        status: Set("uploading".into()),
        last_activity_at: Set(Some(now)),
        ..Default::default()
    })
    .filter(upload_task::Column::Id.eq(id))
    .exec(db)
    .await?;

// 更新合并状态
upload_task::Entity::update_many()
    .set(upload_task::ActiveModel { status: Set("merging".into()), ..Default::default() })
    .filter(upload_task::Column::Id.eq(id))
    .exec(db)
    .await?;

upload_task::Entity::update_many()
    .set(upload_task::ActiveModel { status: Set("completed".into()), ..Default::default() })
    .filter(upload_task::Column::Id.eq(id))
    .exec(db)
    .await?;

// GC 查询过期任务
upload_task::Entity::find()
    .filter(upload_task::Column::ExpiresAt.lt(Utc::now()))
    .filter(upload_task::Column::Status.is_not_in(["completed", "expired"]))
    .all(db)
    .await?;

// 速率限制：并发检查
upload_task::Entity::find()
    .filter(upload_task::Column::UserId.eq(user_id))
    .filter(upload_task::Column::Status.is_in(["initialized", "uploading"]))
    .count(db)
    .await?;

// 速率限制：每日配额检查
upload_task::Entity::find()
    .filter(upload_task::Column::UserId.eq(user_id))
    .filter(upload_task::Column::CreatedAt.gte(today_start))
    .count(db)
    .await?;
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

```rust
// 用户文件列表（通过 user_objects 关联表）
object::Entity::find()
    .inner_join(user_object::Entity)
    .filter(user_object::Column::UserId.eq(user_id))
    .filter(object::Column::Status.eq("active"))
    .order_by_desc(object::Column::CreatedAt)
    .paginate(db, 20)
    .fetch_page(0)
    .await?;
```

### 数据生命周期

- `upload_tasks` 在 `completed` 或 `expired` 后可考虑归档或删除
- `objects` 的 `deleted` 记录保留用于审计，可按时间定期清理
- 过期上传任务的暂存文件由 `gc_clean` 后台任务清理（30 分钟间隔）
- 已删除对象的物理文件由 `ref_check` 后台任务清理（2 小时间隔）

### MySQL 配置建议（仅 MySQL 用户）

```ini
# my.cnf
[mysqld]
innodb_buffer_pool_size = 70%_of_RAM
max_allowed_packet = 64M
innodb_log_file_size = 512M
innodb_flush_log_at_trx_commit = 2
```

SQLite 用户无需额外配置。数据库文件默认创建在工作目录，WAL 模式自动启用以提升并发性能。

### 存储类型说明

- `uploaded_bitmap` 使用 `TEXT`，支持超大型文件（如 100 万分片 = 约 8MB JSON，容量足够）
- `storage_path` 使用 `VARCHAR(1024)`，存储相对路径足够
- 所有 timestamp 字段使用微秒精度，支持精确时间比较

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
