# buckets 管理后台前端

基于 Vue 3 + TypeScript + Vite 的 Web 管理后台。

## 技术栈

- **框架**: Vue 3 (Composition API + `<script setup>`)
- **构建**: Vite 8
- **样式**: Tailwind CSS 4
- **路由**: Vue Router 4
- **状态管理**: Pinia 3
- **SDK**: `@chihqiang/buckets`（本地 `file:../jssdk` 依赖）
- **类型检查**: TypeScript 6 + vue-tsc 3

## 目录结构

```
src/
├── stores/            # Pinia 状态管理
│   ├── api.ts         # BucketsClient 单例 + login/logout（含 localStorage 管理）
│   ├── auth.ts        # token 响应式状态、isSuperAdmin
│   ├── objects.ts     # 对象列表、分页、删除
│   └── users.ts       # 用户 CRUD、重置密钥
├── router/
│   └── index.ts       # 路由表 + 导航守卫（未登录 → /login，非管理员 → /objects）
├── views/
│   ├── Login.vue      # 登录页（邮箱 + 密码）
│   ├── ObjectList.vue # 对象列表（分页 + 删除 + 三种上传模式）
│   └── UserList.vue   # 用户管理（新建/编辑/删除/重置密钥，仅管理员）
├── components/
│   └── Layout.vue     # 顶栏导航 + 退出按钮 + RouterView
├── App.vue            # 根组件
├── main.ts            # 入口：createApp + Pinia + Router
└── style.css          # @import "tailwindcss"
```

## 开发

```bash
cd ../jssdk && npm install && npm run build && cd ../web
npm install
npm run dev     # 开发服务器（Vite dev proxy /api → :8080）
npm run build   # 生产构建
npm run preview # 预览生产构建
```

## 构建说明

在项目根目录执行 `cargo build --release -p buckets-srv` 时，Dockerfile 会先构建 jssdk 和 web（前端 dist 嵌入 Rust 二进制）。开发时使用 Vite 代理实现热更新。
