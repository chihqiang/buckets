# buckets

轻量高性能私有对象存储，专为大文件分片上传场景设计，支持秒传（MD5 去重）、断点续传、会话级预签名认证、Token + Basic Auth 双模认证、流式分片上传、速率限制。

## 核心特性

- **分片上传**：大文件切分为可配置大小（默认 8MB）的分片，支持并发上传（默认 4 并发，最大 16）
- **流式写入**：分片数据流式写入磁盘，避免将整个分片加载到内存，杜绝 OOM 风险
- **秒传**：服务端 MD5 + 文件大小全局去重，相同文件秒级完成，无需重复上传
- **断点续传**：基于位图（bitmap）记录已上传分片，中断后自动跳过已完成分片，仅上传缺失部分
- **会话级 HMAC 签名**：STS 接口签发会话级签名（有效期 2h），一个签名覆盖整个上传会话，无需逐分片签发
- **异步合并**：分片合并改为异步后台执行，客户端轮询等待结果，避免 HTTP 长连接超时；合并时使用零拷贝（copy_file_range）和 MD5 侧车文件优化磁盘 I/O
- **双模认证**：Token 认证（Web 端）与 Basic Auth 认证（CLI 端），自动选择；Token 有效期 7 天，支持刷新
- **TraceID 链路追踪**：全链路 UUID 追踪 ID，贯穿 CLI 客户端和服务端，快速定位故障
- **速率限制**：三级限流——Token Bucket 请求速率（默认 2 req/s）、最大并发上传数（默认 5）、每日上传配额（默认 50）
- **内存位图缓存**：上传进度位图缓存在内存中，批量刷新到 DB，减少数据库压力
- **磁盘空间预检**：上传前校验可用磁盘空间，合并时要求 2x 文件大小；磁盘使用率 >90% 时健康检查返回降级状态
- **文件类型校验**：可选严格扩展名白名单/黑名单校验（默认关闭）
- **请求超时**：全局请求超时（默认 600s），防止挂起连接无限占用资源
- **优雅关闭**：SIGTERM/SIGINT 信号触发优雅关闭，等待进行中的请求完成
- **单机部署**：所有文件存储在本机磁盘，无需分布式存储

## 项目结构

```bash
buckets/
├── Cargo.toml                # Workspace 配置
├── Cargo.lock
├── .env.example              # 环境变量模板
├── buckets-common/       # 公共基础库（lib）
│   ├── src/
│   │   ├── error.rs      # 统一错误类型 AppError
│   │   ├── model/        # 共享数据模型（API DTO）+ sea-orm 实体
│   │   └── utils/        # 工具函数（crypto, path, validate）
│   └── Cargo.toml
├── buckets-srv/         # 服务端（bin）
│   ├── src/
│   │   ├── main.rs       # 入口，启动流程
│   │   ├── app.rs        # Router 组装，AppState，健康检查
│   │   ├── config.rs     # 配置加载（环境变量）
│   │   ├── db.rs         # 数据库连接、认证工具
│   │   ├── dao/          # 数据库访问层（objects, tasks, users）
│   │   ├── api/          # HTTP handler 层（auth, sts, chunk, merge, precheck, objects, users）
│   │   ├── service/      # 业务逻辑层（auth_svc, chunk_svc, file_svc）
│   │   ├── middleware/   # 中间件（auth, logger, ratelimit, trace）
│   │   └── task/         # 后台任务（gc_clean, ref_check, bitmap_flush）
│   └── Cargo.toml
├── migration/               # 数据库迁移（sea-orm-migration）
│   ├── src/
│   │   ├── lib.rs         # Migrator 定义
│   │   ├── m20220101_000001_create_tables.rs  # 建表 + seed 用户
│   └── Cargo.toml
├── buckets-cli/         # 命令行客户端（bin）
│   ├── src/
│   │   ├── main.rs       # 入口，交互式/命令模式
│   │   ├── cli.rs        # clap 参数定义
│   │   ├── config.rs     # 凭证本地存储
│   │   ├── local.rs      # 本地文件读取（流式 MD5）
│   │   ├── progress.rs   # 进度条（indicatif）
│   │   └── client/       # HTTP 客户端（STS, precheck, chunk, merge）
│   └── Cargo.toml
├── web/                      # 管理后台前端（Vue 3 + Tailwind CSS）
│   ├── src/
│   │   ├── sdk/              # HTTP 客户端（Client）+ API 封装（Api 类）
│   │   ├── stores/           # Pinia 状态管理 + 登录/登出逻辑
│   │   ├── views/            # 页面组件（Login, ObjectList, UserList）
│   │   ├── components/       # 通用组件（Layout）
│   │   ├── router/           # Vue Router 配置
│   │   ├── App.vue
│   │   └── main.ts
│   ├── index.html
│   ├── vite.config.ts
│   └── package.json
├── deploy/
│   ├── docker-compose.yaml   # 一键部署（MySQL + 应用）
│   └── nginx.conf            # Nginx 反向代理参考（独立部署用）
└── docs/                     # 技术文档
    ├── architecture.md       # 架构设计
    ├── api.md                # API 接口文档
    ├── schema.md             # 数据库表结构
    ├── upload.md             # 文件上传链路详解
    └── deploy.md             # 部署指南
```

## 子包说明

| 子包 | 类型 | 技术栈 | 职责 |
|------|------|--------|------|
| `buckets-common` | lib | serde, chrono, uuid, sha2, hmac, md-5, base64, argon2, axum, sea-orm, thiserror | 共享模型、统一错误体系、sea-orm 实体、全局常量、工具函数（crypto/validate/path） |
| `buckets-srv` | bin | axum 0.8, tokio, sea-orm 1.x, tower-http, dotenvy, dashmap, jsonwebtoken | HTTP API 服务、JWT + Basic Auth 双模认证、分片管理、异步合并、流式写入、GC 清理、速率限制、**前端静态文件分发** |
| `buckets-cli` | bin | clap 4, reqwest 0.12, tokio, indicatif, rpassword | 文件预处理、流式分片上传、进度展示、断点续传、凭证管理 |

### 依赖关系

```
buckets-cli ──► buckets-common
                        ▲
                        │
buckets-srv ─────────┘
```

## 快速开始

```bash
# 1. 配置环境变量
cp .env.example .env
# 编辑 .env，修改 DATABASE_URL、ADMIN_PASSWORD 等配置

# 2. 初始化数据库（自动建表 + 创建 admin 用户）
cargo run --release -p migration

# 3. 启动服务端
cargo run --release -p buckets-srv

# 4. 构建并访问管理后台（可选）
cd web && npm install && npm run build && cd ..
# 访问 http://localhost:8080 即可打开管理后台
# 默认账号：admin@buckets.local / buckets

# 5. 上传文件（CLI）
cargo run --release -p buckets-cli -- \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    upload ./large-file.mp4

# 6. 查看上传状态
cargo run --release -p buckets-cli -- \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    status <task_id>

# 7. 断点续传
cargo run --release -p buckets-cli -- \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    upload ./large-file.mp4 --resume
```

Seed 用户 `admin@buckets.local` / `buckets` 由 migration 在首次运行时自动创建（密码使用 argon2 哈希，通过 `ADMIN_PASSWORD` 环境变量配置，默认 `buckets`）。

## Web 管理后台

管理后台提供基于浏览器的文件管理和用户管理功能，由 Rust 后端直接 serve 静态文件。

| 页面 | 路径 | 权限 | 功能 |
|------|------|------|------|
| 登录 | `/login` | 无需认证 | 邮箱 + 密码登录 |
| 对象管理 | `/objects` | 所有登录用户 | 对象列表、删除 |
| 用户管理 | `/users` | 超级管理员 | 用户 CRUD、重置密钥 |

开发模式支持热更新：

```bash
cd web && npm run dev
# 前台：http://localhost:5173（Vite proxy /api → :8080）
# 也可访问后端直 serv：http://localhost:8080（需先 npm run build）
```

## CLI 命令详解

```bash
# 登录（保存到 ~/.buckets/credentials.json，类似 Docker config.json 格式）
buckets-cli --server http://127.0.0.1:8080 login --email admin@buckets.local --password buckets
buckets-cli --server https://other:9090 login --email admin@other.local  # 多服务器共存

# 列出所有已保存的服务器
buckets-cli list

# 切换默认服务器
buckets-cli use http://127.0.0.1:8080

# 登出（删除当前 server 的凭证，不影响其他 server）
buckets-cli --server http://127.0.0.1:8080 logout

# 上传文件
buckets-cli upload ./file.mp4                    # 默认参数
buckets-cli upload ./file.mp4 --name custom-name # 自定义对象名
buckets-cli upload ./file.mp4 --parallel 8       # 8 并发上传
buckets-cli upload ./file.mp4 --chunk-size 4     # 4MB 分片
buckets-cli upload ./file.mp4 --resume           # 断点续传模式

# 查询上传状态
buckets-cli status <task_id>

# 交互模式（无子命令时进入）
buckets-cli
> upload
> status <task_id>
> resume
```

## 分片上传配置

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--chunk-size` | 8 MB | 分片大小（MB），范围 1-256 |
| `--parallel` | 4 | 并发上传数，最大 16 |
| `--resume` | false | 启用断点续传模式 |

客户端自动重试机制：分片上传失败自动重试 3 次，间隔指数退避（1s → 2s → 4s）+ 随机抖动（0-500ms）。

## 认证机制

buckets 支持两种认证方式，自动按优先级选择：

### 方式一：JWT Token 认证（推荐用于 Web 前端）

通过 `Authorization: Bearer <jwt_token>` 请求头携带 Token，服务端通过 JWT HS256 验证：

```bash
Authorization: Bearer eyJhbGciOiJIUzI1NiJ9...
```

Token 通过登录接口获取，有效期 7 天，支持刷新和黑名单吊销。JWT header 包含 `kid`（用户 ID），payload 包含 `sub`（用户 ID）、`exp`（过期时间）、`jti`（唯一标识）。

**Token 管理接口**：

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/v1/auth/login` | 登录，返回 `{ token, refresh_token, expires_in, is_super_admin }` |
| POST | `/api/v1/auth/refresh` | 刷新 Token，返回新 token 对（旧 refresh_token 失效） |
| POST | `/api/v1/auth/logout` | 吊销 Token（加入黑名单，从 `Authorization` header 提取） |

### 方式二：Basic Auth（CLI 客户端使用）

```bash
Authorization: Basic base64("email:password")
```

- 客户端发送原始密码
- 服务端收到后 argon2 验证密码
- 服务端存储的密码为 argon2 哈希（首次启动自动升级 placeholder）
- 认证成功缓存 30 分钟（DashMap 内存缓存），减少 DB 压力
- 认证失败按 IP 限流（Token Bucket，突发 10 次），防止暴力破解密码

### 认证优先级

统一认证中间件按以下优先级处理：
1. `Authorization: Bearer <jwt>` — JWT Token 认证（优先）
2. `Authorization: Basic base64(email:password)` — Basic Auth 认证（CLI 回退）

Bearer Token 无效时不会回退到 Basic Auth，直接返回 401。

## 秒传机制

```bash
用户 A 上传文件 → objects 表插入 { md5: "abc...", ref_count: 1 }
用户 B 上传相同 MD5 文件：
  → precheck 查询 objects.md5
  → 命中 → 直接返回对象信息，秒传完成
```

## 速率限制

默认启用三级限流（可通过 `RATE_LIMIT_ENABLED=false` 关闭）：

| 限制类型 | 默认值 | 环境变量 | 说明 |
|----------|--------|----------|------|
| 请求速率 | 2 req/s | `RATE_LIMIT_RPS` | Token Bucket 算法，突发 10 |
| 最大并发 | 5 个 | `RATE_LIMIT_MAX_CONCURRENT` | 同时进行中的上传任务数 |
| 每日配额 | 50 次 | `RATE_LIMIT_DAILY_QUOTA` | 每日上传请求次数限制 |

## 后台任务

| 任务 | 间隔 | 说明 |
|------|------|------|
| `gc_clean` | 30 分钟 | 清理过期 upload_tasks 及其暂存文件；分批处理 + 限流，防止 I/O 风暴 |
| `ref_check` | 2 小时 | 扫描已删除对象的引用计数，清理无引用的物理文件 |
| `auth_cache_cleaner` | 30 分钟 | 清理过期的 Basic Auth 认证缓存条目 |
| `secret_key_cache_cleaner` | 30 分钟 | 清理过期的密钥缓存条目 |
| `bitmap_flush` | 5 秒 | 将内存中的位图缓存批量刷新到 DB |
| `bitmap_cache_cleanup` | 5 分钟 | 清理过期的位图缓存条目，超过上限时强制淘汰 |

## 存储结构

```bash
data/
├── objects/          # 已合并完成的对象文件（按用户/日期分片路径）
│   └── <user_id>/<YYYY>/<MM>/<DD>/<uuid>.<ext>
├── staging/          # 分片暂存目录（上传中）
│   └── <task_id>/
│       ├── chunk_000000
│       ├── chunk_000001.md5      # MD5 侧车文件（优化合并 I/O）
│       └── ...
└── cache/            # 上传会话缓存
```

## 编译

```bash
# 环境要求：Rust 1.85+（2024 edition）

cargo build --release                         # 全部编译
cargo build --release -p buckets-srv       # 仅服务端
cargo build --release -p buckets-cli       # 仅客户端

# 运行测试
cargo test --release

# 运行特定包测试
cargo test -p buckets-common
```

## 技术栈

| 组件 | 技术 | 用途 |
|------|------|------|

### 后端

| 组件 | 技术 | 用途 |
|------|------|------|
| 运行时 | tokio 1.x | 异步 IO + 任务调度 |
| HTTP 框架 | axum 0.8 | 路由 + 中间件 + 提取器 |
| 数据库驱动 | sea-orm 1.x | 异步 ORM，代码生成，迁移管理 |
| 序列化 | serde + serde_json 1.x | 模型序列化 |
| 哈希 | md-5, sha2 0.10 | 文件校验 + 密码哈希 |
| 密码学 | hmac 0.12 | 会话签名 |
| 密码哈希 | argon2 0.5 | 用户密码安全存储 |
| JWT | jsonwebtoken 9 | Web Token 签发与验证 |
| 编码 | base64 0.22 | 分片数据传输 |
| CLI 框架 | clap 4.x | 命令行参数 |
| HTTP 客户端 | reqwest 0.12 | CLI 上传 |
| 进度条 | indicatif 0.17 | CLI 进度展示 |
| 日志 | tracing + tracing-subscriber 0.1/0.3 | 结构化日志 |
| 环境变量 | dotenvy 0.15 | .env 文件加载 |
| 缓存 | dashmap 6 | 并发内存缓存（auth cache + secret key cache + bitmap cache） |
| CORS | tower-http 0.6 | 跨域支持、静态文件 serve |
| Body 工具 | http-body-util 0.1 | 请求体缓冲读取 |

### 前端（管理后台）

| 组件 | 技术 | 用途 |
|------|------|------|
| 框架 | Vue 3 (Composition API) | UI 层 |
| 构建工具 | Vite 8 | 开发服务器 + 生产构建 |
| 样式 | Tailwind CSS 4 | 原子化 CSS |
| 路由 | Vue Router 4 | SPA 路由 |
| 状态管理 | Pinia 3 | 全局状态（auth, objects, users） |
| HTTP | Axios 1.x | 请求发送 + 拦截器 |
| 类型检查 | TypeScript 6 + vue-tsc 3 | 类型安全 |

## 文档

- [架构设计](docs/architecture.md) — 整体架构、分层设计、核心流程详解
- [API 接口](docs/api.md) — 完整 API 接口文档（含 Token 认证）
- [数据库表结构](docs/schema.md) — 4 张表详细设计，位图机制
- [上传链路详解](docs/upload.md) — 从 CLI 到服务端的分片上传完整链路
- [部署指南](docs/deploy.md) — 环境要求、Docker、生产部署建议

## License

详见 [LICENSE](LICENSE)
