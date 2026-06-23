# buckets 部署指南

## 一、环境要求

### 硬件要求

| CPU | 内存 | 磁盘 | 场景 |
|-----|------|------|------|
| 2 核 | 4GB | 20GB+ | 开发测试、个人使用 |
| 4 核 | 8GB | 500GB+ | 小团队文件共享 |
| 8 核 | 16GB | 2TB+ | 大规模使用 |

磁盘建议使用 SSD。分片暂存目录（staging）建议与对象存储目录放在同一磁盘，避免合并时的跨盘文件拷贝。合并操作需要约 2x 文件大小的临时磁盘空间。

### 软件要求

| 组件 | 版本要求 | 说明 |
|------|----------|------|
| Rust | 1.85+ | 编译环境（Rust 2024 edition） |
| MySQL | 8.0+ | 元数据存储（可选），需要 InnoDB 引擎 |
| SQLite | 3.x（内嵌） | 默认数据库，开箱即用，无需单独安装 |
| 操作系统 | Linux / macOS | Windows 通过 WSL2 也可运行 |

> **注意**：当前版本不依赖 Redis，所有会话缓存和速率限制数据通过 DashMap 内存实现。

---

## 二、快速开始

### 2.1 克隆与编译

```bash
git clone <repo-url> buckets
cd buckets

# 编译全部组件（release 模式，优化体积和性能）
cargo build --release

# 或单独编译特定组件
cargo build --release -p buckets-srv       # 仅服务端
cargo build --release -p buckets-cli       # 仅客户端
```

编译产物位于 `target/release/`：
- `buckets-srv` — 服务端二进制
- `buckets-cli` — 命令行客户端二进制

首次编译会下载所有依赖，耗时 3-10 分钟。后续增量编译更快。

### 2.2 初始化数据库

默认使用 SQLite，开箱即用无需任何数据库服务器。如需 MySQL，先创建数据库：

```bash
# SQLite（默认）— 无需额外操作，migration 自动创建 buckets.db
# MySQL — 需先创建数据库
mysql -u root -p -e "CREATE DATABASE IF NOT EXISTS buckets CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
```

运行迁移（自动建表 + 创建 seed admin 用户）：

```bash
cargo run --release -p migration
```

迁移完成后表结构如下（两种后端相同）：

| 表名 | 说明 |
|------|------|
| `users` | 系统用户 |
| `objects` | 文件对象元数据 |
| `user_objects` | 用户-对象关联（多对多） |
| `upload_tasks` | 上传任务与进度 |

> 迁移使用 sea-orm-migration 框架，DDL 定义在 `migration/src/m20220101_000001_create_tables.rs`。种子管理员密码通过 `ADMIN_PASSWORD` 环境变量配置（默认 `buckets`）。

服务端首次启动时会：
1. 自动连接数据库并验证连接
2. 确保数据存储目录存在（objects、staging、cache）

### 2.3 配置环境变量

在项目根目录创建 `.env` 文件（可参考 `.env.example`）：

```bash
cp .env.example .env
```

**最小配置**（默认 SQLite，无需修改）：

```bash
DATABASE_URL="sqlite:buckets.db"
```

如使用 MySQL，改为：

```bash
DATABASE_URL="mysql://root:root@localhost:3306/buckets"
```

**完整配置项**：

```bash
# 服务器
HOST=0.0.0.0              # 监听地址（默认 0.0.0.0）
PORT=8080                 # 监听端口（默认 8080）

# 数据库
DATABASE_URL="sqlite:buckets.db"
DB_MAX_CONN=20            # 连接池大小（默认 20，最小 2，SQLite 下仅使用 1 个连接）

# CORS
CORS_ALLOWED_ORIGINS=     # 逗号分隔，空=允许所有

# 超级管理员
SUPER_ADMIN_IDS=1         # 逗号分隔的用户 ID 列表

# 种子用户密码（migration 时使用）
ADMIN_PASSWORD=buckets  # 默认管理员密码，migration 自动创建 seed 用户

# 速率限制
RATE_LIMIT_ENABLED=true
RATE_LIMIT_RPS=2.0        # Token Bucket 每秒请求数
RATE_LIMIT_BURST=10.0     # Token Bucket 突发容量
RATE_LIMIT_MAX_CONCURRENT=5   # 每用户最大并发上传数
RATE_LIMIT_DAILY_QUOTA=50     # 每用户每日上传配额

# 分片上传
MAX_CHUNK_SIZE=268435456  # 最大分片大小（默认 256 MiB）

# 日志
RUST_LOG=info
```

网关启动时自动读取 `.env` 文件并加载为环境变量。

`export` 方式依然可用，且优先级高于 `.env` 文件。同时使用两者时，已存在的环境变量不会被 `.env` 覆盖。

### 2.4 启动服务端

```bash
# 在项目根目录执行
./target/release/buckets-srv
```

启动日志示例：
```
buckets-srv starting...
 INFO buckets_srv: Connecting to database...
 INFO buckets_srv: Starting server on 0.0.0.0:8080
```

如果启用了速率限制，还会输出：
```
 INFO buckets_srv: Upload rate limiting enabled rps=2 burst=10 concurrent=5 daily_quota=50
```

服务将在 `http://0.0.0.0:8080` 开始接受请求。

### 2.5 上传文件

使用 CLI 客户端上传文件：

```bash
# 使用默认 seed 用户上传
./target/release/buckets-cli \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    upload ./large-file.mp4

# 指定并发数和分片大小
./target/release/buckets-cli \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    upload ./large-file.mp4 \
    --parallel 8 \
    --chunk-size 4

# 断点续传模式
./target/release/buckets-cli \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    upload ./large-file.mp4 --resume

# 查看上传状态
./target/release/buckets-cli \
    --server http://127.0.0.1:8080 \
    --email admin@buckets.local \
    --password buckets \
    status <task_id>

# 保存凭证（后续命令无需重复输入 server/email/password）
./target/release/buckets-cli \
    --server http://127.0.0.1:8080 \
    login --email admin@buckets.local
# 输入密码后凭证保存到 ~/.buckets/credentials.json

# 交互模式
./target/release/buckets-cli
> upload
> status <task_id>
> resume
```

---

## 三、Docker 部署

### 3.1 一键部署（Docker Compose）

`deploy/docker-compose.yaml` 一键启动 MySQL + 应用（前后端一体）：

```bash
cd deploy
docker compose up -d
```

这会启动：
- **MySQL 8.0**：端口 3306，root 密码 `root`，自动创建 `buckets` 数据库
- **app**：Rust 后端 + Vue 前端，监听 8080 端口，数据持久化到 `./docker/mysql/`

访问 `http://localhost:8080` 即可打开管理后台（默认账号 `admin@buckets.local` / `buckets`）。

### 3.2 构建镜像

```bash
# 在项目根目录
docker build -t buckets:latest .
```

Dockerfile 使用三段式构建：
1. **builder-rust**：`rust:1.89-slim` 编译 release 版后端
2. **builder-web**：`node:20-alpine` 构建 Vue 前端
3. **runtime**：`debian:bookworm-slim`，仅包含二进制和前端静态文件

### 3.3 运行服务端容器

```bash
# 创建数据目录
mkdir -p /data/buckets/{objects,staging,cache}

# SQLite（默认，无需外部数据库）
docker run -d \
    --name buckets-app \
    --restart unless-stopped \
    -p 8080:8080 \
    -e DATABASE_URL="sqlite:/home/buckets/data/buckets.db" \
    -e SUPER_ADMIN_IDS=1 \
    -e RUST_LOG=info \
    -v /data/buckets:/home/buckets/data \
    buckets:latest

# MySQL（需先准备 MySQL 实例）
docker run -d \
    --name buckets-app \
    --restart unless-stopped \
    -p 8080:8080 \
    -e DATABASE_URL="mysql://root:root@mysql-host:3306/buckets" \
    -e SUPER_ADMIN_IDS=1 \
    -e RUST_LOG=info \
    -v /data/buckets/objects:/home/buckets/data/objects \
    -v /data/buckets/staging:/home/buckets/data/staging \
    -v /data/buckets/cache:/home/buckets/data/cache \
    buckets:latest

# 查看日志
docker logs -f buckets-app
```

访问 `http://localhost:8080` 即可打开管理后台。

---

## 四、环境变量配置

### 完整配置项

| 变量 | 默认值 | 必填 | 说明 |
|------|--------|------|------|
| `HOST` | `0.0.0.0` | 否 | 监听地址，生产建议 `127.0.0.1` + Nginx |
| `PORT` | `8080` | 否 | 监听端口 |
| `DATABASE_URL` | `sqlite:buckets.db` | **是** | 数据库连接串，`sqlite:buckets.db`（SQLite）或 `mysql://user:pass@host:port/db`（MySQL） |
| `DB_MAX_CONN` | `20` | 否 | 数据库连接池最大连接数（最小 2） |
| `CORS_ALLOWED_ORIGINS` | (空=允许所有) | 否 | 允许的来源域名，逗号分隔 |
| `SUPER_ADMIN_IDS` | (空) | 否 | 超级管理员用户 ID，逗号分隔 |
| `ADMIN_PASSWORD` | `buckets` | 否 | 种子管理员密码，migration 时使用，argon2 哈希存储 |

| `RATE_LIMIT_ENABLED` | `true` | 否 | 是否启用速率限制 |
| `RATE_LIMIT_RPS` | `2.0` | 否 | Token Bucket 每秒请求数 |
| `RATE_LIMIT_BURST` | `10.0` | 否 | Token Bucket 突发容量 |
| `RATE_LIMIT_MAX_CONCURRENT` | `5` | 否 | 每用户最大并发上传数 |
| `RATE_LIMIT_DAILY_QUOTA` | `50` | 否 | 每用户每日上传配额 |
| `MAX_CHUNK_SIZE` | `268435456` (256MB) | 否 | 服务端允许的最大分片大小（字节） |
| `RUST_LOG` | `info` | 否 | 日志级别: `error`/`warn`/`info`/`debug`/`trace` |
| `CHUNK_UPLOAD_TIMEOUT_SECS` | `1800` | 否 | CLI 分片上传超时（秒），仅 CLI 使用 |

### 数据库连接串格式

```
SQLite: sqlite:buckets.db                    # 相对路径，文件位于工作目录
        sqlite:///data/buckets/buckets.db    # 绝对路径

MySQL:  mysql://[user][:password]@[host][:port]/[database][?params]

示例:
sqlite:buckets.db
mysql://root:root@localhost:3306/buckets
mysql://admin:secret@192.168.1.100:3306/buckets?charset=utf8mb4
```

### 配置优先级

```
环境变量（export） > .env 文件 > 默认值
```

`.env` 文件不会覆盖已存在的环境变量。无其他配置文件，所有配置通过环境变量注入。

---

## 五、日志与监控

### 日志格式

```
{timestamp} {level} [{trace_id}] {target}: {message}
```

示例：
```
2024-01-01T00:00:00.123456Z  INFO [550e8400-e29b-41d4-a716-446655440000] buckets_srv::middleware::logger: POST /api/v1/upload/precheck 200 15ms
```

### 日志级别

通过 `RUST_LOG` 控制：

```bash
export RUST_LOG=info                          # 默认，生产推荐
export RUST_LOG=debug                         # 开发调试
export RUST_LOG=buckets_srv=debug           # 仅 srv crate 的 debug 日志
export RUST_LOG=warn                          # 仅警告和错误
```

### 健康检查

```bash
curl http://localhost:8080/health
```

响应示例：
```json
{
    "status": "ok",
    "db_ok": true,
    "disk_available_bytes": 107374182400,
    "disk_usage_percent": 45.2
}
```

- `status`: `"ok"` 或 `"degraded"`（数据库不可用或磁盘使用率 >90%）
- `db_ok`: 数据库连接是否正常
- `disk_available_bytes`: 可用磁盘空间（字节）
- `disk_usage_percent`: 磁盘使用率百分比

### 链路追踪

所有请求支持 `X-Trace-Id`：

1. CLI 自动为每次上传会话生成 trace_id（UUID v4）
2. 每个 HTTP 请求携带 `X-Trace-Id: {uuid}` 头
3. 服务端回写同值 `X-Trace-Id` 响应头
4. 服务端日志以 `[{trace_id}]` 为前缀

**排查流程**：
```
1. CLI 报错: [trace_id: xxx] upload failed: ...
2. 服务端 grep: grep "xxx" /var/log/buckets.log
3. 查看该 trace_id 的所有日志行，定位具体错误
   ├── 无匹配 → 请求未到达服务器 → 客户端/网络问题
   └── 有匹配 → 查看该请求的完整调用链 → 定位具体环节
```

---

## 六、生产部署建议

### 安全配置

1. **数据库**：使用独立用户，限制权限仅为 `SELECT/INSERT/UPDATE/DELETE` 三张表
2. **监听地址**：`HOST=127.0.0.1` 配合反向代理（Nginx）暴露，避免直接暴露服务端口
3. **HTTPS**：推荐使用 Nginx 反向代理 + Let's Encrypt TLS 证书
4. **密码安全**：seed 管理员密码通过 `ADMIN_PASSWORD` 环境变量配置（默认 `buckets`），migration 时自动 argon2 哈希存储，生产环境请修改该值
5. **速率限制**：根据实际需求调整 `RATE_LIMIT_RPS`、`RATE_LIMIT_MAX_CONCURRENT`、`RATE_LIMIT_DAILY_QUOTA`
6. **CORS**：设置 `CORS_ALLOWED_ORIGINS` 为明确的前端域名，避免使用通配符
7. **文件类型校验**：生产环境建议在 `buckets-common/src/utils/validate.rs` 中将 `STRICT_EXTENSION_CHECK` 设为 `true`
8. **防火墙**：仅开放 443（HTTPS），8080 端口仅对 Nginx 可见

### 反向代理配置（Nginx）

由于后端同时提供 API 和前端静态文件，Nginx 只需透传所有请求即可：

```nginx
server {
    listen 443 ssl http2;
    server_name buckets.example.com;

    ssl_certificate /etc/letsencrypt/live/example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/example.com/privkey.pem;

    client_max_body_size 0;     # 不限（由应用层 MAX_CHUNK_SIZE 控制）
    proxy_buffering off;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        proxy_connect_timeout 60s;
        proxy_send_timeout 600s;
        proxy_read_timeout 600s;
    }
}
```

### 性能调优

| 参数 | 推荐值 | 说明 |
|------|--------|------|
| `DB_MAX_CONN` | 20-50 | 连接池大小，按并发请求数调整 |
| 分片大小 | 4-8MB | 默认 8MB，网络差可调小，带宽充足可调大（最大 256MB） |
| 并发数 | 4-8 | CLI `--parallel` 参数，按带宽调整，最大 16 |
| `MAX_CHUNK_SIZE` | 256MB | 服务端最大分片限制 |
| MySQL `innodb_buffer_pool_size` | 物理内存 70% | InnoDB 缓存（仅 MySQL） |
| MySQL `max_allowed_packet` | 64MB | 允许大分片传输（仅 MySQL） |

### 存储空间规划

```
data/
├── objects/     # 最终存储空间 = 所有文件大小总和
├── staging/     # 临时空间 = 最大并发上传文件大小 × 并发数
└── cache/       # 少量缓存文件
```

合并时需要约 2x 文件大小的临时空间（staging 分片 + 合并中的对象文件）。

### 数据备份

```bash
# SQLite 数据库备份
sqlite3 buckets.db ".backup buckets_$(date +%Y%m%d).db"

# MySQL 数据库备份
mysqldump -u root -p buckets > buckets_$(date +%Y%m%d).sql

# 对象文件备份（使用 rsync）
rsync -avz --progress /data/buckets/objects/ backup-server:/backup/objects/

# 分片暂存目录（上传中数据，非必须备份）
rsync -avz /data/buckets/staging/ backup-server:/backup/staging/
```

### 优雅关闭

服务端支持 SIGTERM/SIGINT 信号优雅关闭：

```bash
# 发送信号
kill -TERM $(pidof buckets-srv)

# 或使用 Ctrl+C

# 日志输出:
#  INFO buckets_srv: Received Ctrl+C, initiating graceful shutdown...
```

关闭时会：
1. 停止接受新请求
2. 等待进行中的请求完成
3. 取消后台任务（gc_clean, ref_check）
4. 关闭数据库连接池

### systemd 服务配置

```ini
# /etc/systemd/system/buckets-srv.service
[Unit]
Description=buckets Server Service
After=network.target
# SQLite: 无需数据库服务
# MySQL:  取消下面注释
# After=network.target mysql.service
# Wants=mysql.service

[Service]
Type=simple
User=buckets
Group=buckets
WorkingDirectory=/opt/buckets
EnvironmentFile=/opt/buckets/.env
ExecStart=/opt/buckets/buckets-srv
ExecStop=/bin/kill -TERM $MAINPID
Restart=on-failure
RestartSec=10

# 安全加固
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/data/buckets

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable buckets-srv
sudo systemctl start buckets-srv
sudo systemctl status buckets-srv
```

### 故障排查

**Q: 服务启动失败，提示数据库连接失败**
```
检查: DATABASE_URL 是否正确
检查: SQLite — buckets.db 所在目录是否可写
检查: MySQL — 是否运行: systemctl status mysql
检查: MySQL — 网络连通性: telnet mysql-host 3306
检查: MySQL — 数据库用户权限
```

**Q: 上传到 99% 卡住**
```
检查: 是否有分片上传失败
运行: buckets-cli status <task_id>
查看: missing_chunks 列表
重试: buckets-cli upload ./file --resume
```

**Q: 合并失败，code 409**
```
可能原因: 分片未上传完整 → 重新 precheck 确认 missing_chunks
可能原因: 合并后 MD5 不匹配（传输损坏）→ 重新上传
排查: 查看服务端日志 trace_id 对应的错误信息
```

**Q: 合并失败，code 500，磁盘空间不足**
```
检查: df -h 查看磁盘空间
清理: 清理过期暂存文件或等待 gc_clean 自动清理
扩容: 增加磁盘空间或迁移数据目录
```

**Q: 上传速度慢**
```
尝试: 增大 --parallel（如 8、16）
尝试: 调整 --chunk-size 为 4MB 或 16MB
检查: 网络带宽是否饱和
检查: 服务端磁盘 IO 是否成为瓶颈（iostat）
```

**Q: 速率限制触发，返回 403**
```
提示: "rate limit exceeded" / "concurrent upload limit" / "daily quota exceeded"
解决: 等待限流窗口过去，或调整 RATE_LIMIT_* 环境变量
关闭: 设置 RATE_LIMIT_ENABLED=false（不推荐生产环境）
```

**Q: 签名过期，返回 401**
```
原因: 上传会话超过 activity_timeout（基础 1h，大文件动态延长）
解决: 重新执行上传流程（STS → precheck → upload）
```

---

## 七、CLI 配置

### 凭证存储

CLI 支持本地凭证存储，避免每次输入密码和服务器地址：

```bash
# 登录（保存 server + 凭证）
buckets-cli --server http://127.0.0.1:8080 login --email admin@buckets.local
# 提示输入密码，保存到 ~/.buckets/credentials.json

# 多服务器
buckets-cli --server https://other:9090 login --email admin@other.local

# 列出所有服务器
buckets-cli list

# 切换默认
buckets-cli use http://127.0.0.1:8080

# 之后直接使用（无需 --server --email --password）
buckets-cli upload ./file.mp4
buckets-cli status <task_id>

# 登出（删除指定 server 的凭证）
buckets-cli --server http://127.0.0.1:8080 logout
```

凭证文件权限为 `0600`（仅 owner 可读写），格式如下：

```json
{
  "default": "http://127.0.0.1:8080",
  "auths": {
    "http://127.0.0.1:8080": "base64(email:password)",
    "https://other-host:9090": "base64(email:password)"
  }
}
```

支持多服务器共存，`login` 只写入当前 server，`logout` 只删除指定 server，`use` 切换默认。

### 环境变量配置

CLI 也支持环境变量：

```bash
export buckets_SERVER_URL=http://192.168.1.100:8080
buckets-cli upload ./file.mp4
```

### CLI 命令参考

| 命令 | 说明 |
|------|------|
| `login` | 交互式登录，保存凭证 |
| `logout` | 删除本地凭证 |
| `upload <file>` | 上传文件 |
| `status <task_id>` | 查询上传进度 |
| `resume` | 续传最近的上传任务 |
| (无命令) | 交互模式 |
