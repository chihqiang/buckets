# @chihqiang/buckets

Buckets 服务端 JavaScript SDK，支持文件上传、认证、对象管理等 API。

## 安装

```bash
npm install @chihqiang/buckets
```

## CDN

```html
<script src="https://unpkg.com/@chihqiang/buckets/dist/index.umd.js"></script>
<script>
  const client = new BucketsSdk.BucketsClient({ baseUrl: 'https://example.com' })
</script>
```

## 快速开始

```ts
import { BucketsClient } from '@chihqiang/buckets'

const client = new BucketsClient({
  baseUrl: 'https://your-buckets-server.com',
  timeout: 30000,
})

// 登录
const { token } = await client.auth.login('user@example.com', 'password')
client.setToken(token)

// 对象管理
const objects = await client.objects.list(1, 20)
await client.objects.delete(1)

// 用户管理
const users = await client.users.list()
await client.users.create('new@example.com', 'pass123')
await client.users.update(1, { email: 'updated@example.com' })
await client.users.delete(2)
await client.users.resetSecretKey(1)

// 上传文件
const file = fileInput.files[0]
await client.chunk.upload(file, {
  onProgress: (p) => console.log(`${p.percent}%`),
})
await client.tus.upload(file, { deferLength: true })
```

## API

### BucketsClient

统一入口，组合所有子模块。

```ts
const client = new BucketsClient({
  baseUrl: string       // 必填，服务端地址
  timeout?: number      // 请求超时（毫秒）
  initialToken?: string // 初始 token
})

client.setToken(token: string)
client.getToken(): string
```

### client.auth - 认证

| 方法 | 说明 |
|------|------|
| `login(email, password)` | 登录，返回 `{ token, refresh_token, expires_in, is_super_admin }` |
| `refreshToken(refreshToken)` | 刷新 token |
| `logout()` | 登出 |

### client.objects - 对象管理

| 方法 | 说明 |
|------|------|
| `list(page?, pageSize?)` | 获取对象列表，返回 `{ items, total, page, page_size }` |
| `delete(id)` | 删除对象 |

### client.users - 用户管理

| 方法 | 说明 |
|------|------|
| `list(page?, pageSize?)` | 获取用户列表 |
| `create(email, password)` | 创建用户 |
| `update(id, data)` | 更新用户（email / password） |
| `delete(id)` | 删除用户 |
| `resetSecretKey(id)` | 重置用户的 secret key |

### client.chunk - 分块上传

适合大文件，支持 MD5 校验、断点续传、并行上传。

```ts
await client.chunk.upload(file: File, options?: {
  chunkSize?: number      // 分块大小，默认 8MB
  parallel?: number       // 并行数，默认 3，最大 6
  onProgress?: (p) => void
  signal?: AbortSignal
}): Promise<{ bucketId, bucketUrl }>
```

### client.tus - TUS 可恢复上传

遵循 tus resumable upload protocol 1.0.0。

```ts
await client.tus.upload(file: File, options?: {
  chunkSize?: number      // 分块大小，默认 6MB
  deferLength?: boolean   // 是否延迟设置文件大小
  metadata?: Record<string, string>
  onProgress?: (p) => void
  signal?: AbortSignal
}): Promise<{ objectId, storagePath }>
```

## 低阶 API

也可以直接使用底层类：

```ts
import { HttpClient, AuthClient, AuthApi, ObjectsApi, UsersApi } from '@chihqiang/buckets'

const http = new HttpClient({ baseUrl: 'https://example.com', timeout: 30000 })
const authClient = new AuthClient(http)
authClient.setToken('xxx')

const authApi = new AuthApi(authClient)
const objectsApi = new ObjectsApi(authClient)
```

## 自定义错误

```ts
import { ClientError } from '@chihqiang/buckets'

try {
  await client.auth.login('bad@email.com', 'wrong')
} catch (err) {
  if (err instanceof ClientError) {
    console.error(err.status, err.traceId, err.message)
  }
}
```
