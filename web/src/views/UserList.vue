<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useUsersStore } from '../stores/users'
import type { User } from '@chihqiang/buckets'
import { useDialog } from '../composables/useDialog'

const store = useUsersStore()
const page = ref(1)
const pageSize = 20

const showCreate = ref(false)
const createEmail = ref('')
const createPassword = ref('')

const editing = ref<number | null>(null)
const editEmail = ref('')
const editPassword = ref('')

const listError = ref('')
const dialog = useDialog()

onMounted(() => store.fetchList(page.value, pageSize).catch((e: any) => {
  listError.value = e.message || '加载失败'
}))

async function handleDelete(id: number) {
  if (!(await dialog.confirm('确定要删除此用户吗？'))) return
  try {
    await store.remove(id)
    await store.fetchList(page.value, pageSize)
  } catch (e: any) {
    dialog.error(e.message || '删除失败')
  }
}

async function handleCreate() {
  if (!createEmail.value || !createPassword.value) return
  try {
    await store.create(createEmail.value, createPassword.value)
    createEmail.value = ''
    createPassword.value = ''
    showCreate.value = false
    await store.fetchList(page.value, pageSize)
  } catch (e: any) {
    dialog.error(e.message || '创建用户失败')
  }
}

function startEdit(u: User) {
  editing.value = u.id
  editEmail.value = u.email
  editPassword.value = ''
}

async function handleUpdate(id: number) {
  const payload: { email?: string; password?: string } = {}
  if (editEmail.value) payload.email = editEmail.value
  if (editPassword.value) payload.password = editPassword.value
  if (!payload.email && !payload.password) {
    editing.value = null
    return
  }
  try {
    await store.update(id, payload)
    editing.value = null
    await store.fetchList(page.value, pageSize)
  } catch (e: any) {
    dialog.error(e.message || '更新用户失败')
  }
}

async function handleResetSecret(id: number) {
  if (!(await dialog.confirm('确定要重置此用户的密钥吗？该用户所有现有 Token 将立即失效。'))) return
  try {
    await store.resetSecret(id)
  } catch (e: any) {
    dialog.error(e.message || '重置密钥失败')
  }
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleString('zh-CN')
}

async function goPage(p: number) {
  page.value = p
  try {
    await store.fetchList(p, pageSize)
  } catch (e: any) {
    dialog.error(e.message || '加载失败')
  }
}
</script>

<template>
  <div>
    <div class="flex items-center justify-between mb-4">
      <h2 class="text-lg font-semibold text-gray-800">用户管理</h2>
      <button
        @click="showCreate = !showCreate"
        class="px-3 py-1.5 text-sm bg-blue-600 text-white rounded-lg hover:bg-blue-700"
      >
        {{ showCreate ? '取消' : '新建用户' }}
      </button>
    </div>

    <div v-if="listError" class="mb-4 p-3 rounded-lg border text-sm bg-red-50 border-red-200 text-red-700">
      <span>{{ listError }}</span>
      <button @click="listError = ''" class="ml-2 text-red-500 hover:text-red-700">×</button>
    </div>

    <div v-if="showCreate" class="mb-4 p-4 bg-white rounded-lg border border-gray-200">
      <h3 class="text-sm font-medium text-gray-700 mb-3">新建用户</h3>
      <div class="flex gap-3">
        <input
          v-model="createEmail"
          type="email"
          placeholder="邮箱"
          class="flex-1 px-3 py-1.5 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <input
          v-model="createPassword"
          type="password"
          placeholder="密码"
          class="flex-1 px-3 py-1.5 border border-gray-300 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
        />
        <button
          @click="handleCreate"
          class="px-4 py-1.5 text-sm bg-green-600 text-white rounded-lg hover:bg-green-700"
        >
          创建
        </button>
      </div>
    </div>

    <div class="bg-white rounded-lg border border-gray-200">
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-gray-200 bg-gray-50">
              <th class="text-left px-4 py-3 font-medium text-gray-600">ID</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">邮箱</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">创建时间</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">更新时间</th>
              <th class="text-right px-4 py-3 font-medium text-gray-600">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="store.loading" class="border-b border-gray-100">
              <td colspan="5" class="px-4 py-8 text-center text-gray-400">加载中...</td>
            </tr>
            <tr v-else-if="store.users.length === 0" class="border-b border-gray-100">
              <td colspan="5" class="px-4 py-8 text-center text-gray-400">暂无用户</td>
            </tr>
            <tr v-for="u in store.users" :key="u.id" class="border-b border-gray-100 hover:bg-gray-50">
              <td class="px-4 py-3 text-gray-800">{{ u.id }}</td>
              <td v-if="editing === u.id" class="px-4 py-3">
                <div class="flex gap-2">
                  <input
                    v-model="editEmail"
                    type="email"
                    class="w-40 px-2 py-1 border border-gray-300 rounded text-sm"
                  />
                  <input
                    v-model="editPassword"
                    type="password"
                    placeholder="新密码(可选)"
                    class="w-36 px-2 py-1 border border-gray-300 rounded text-sm"
                  />
                  <button @click="handleUpdate(u.id)" class="text-green-600 hover:text-green-800 text-sm">保存</button>
                  <button @click="editing = null" class="text-gray-400 hover:text-gray-600 text-sm">取消</button>
                </div>
              </td>
              <td v-else class="px-4 py-3 text-gray-600">{{ u.email }}</td>
              <td class="px-4 py-3 text-gray-600 text-xs">{{ formatDate(u.created_at) }}</td>
              <td class="px-4 py-3 text-gray-600 text-xs">{{ formatDate(u.updated_at) }}</td>
              <td v-if="editing !== u.id" class="px-4 py-3 text-right space-x-2">
                <button @click="startEdit(u)" class="text-blue-600 hover:text-blue-800 text-sm">编辑</button>
                <button @click="handleResetSecret(u.id)" class="text-orange-600 hover:text-orange-800 text-sm">重置密钥</button>
                <button @click="handleDelete(u.id)" class="text-red-600 hover:text-red-800 text-sm">删除</button>
              </td>
              <td v-else class="px-4 py-3"></td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="store.total > pageSize" class="flex items-center justify-between px-4 py-3 border-t border-gray-200">
        <span class="text-sm text-gray-500">共 {{ store.total }} 条</span>
        <div class="flex gap-1">
          <button
            :disabled="page <= 1"
            @click="goPage(page - 1)"
            class="px-3 py-1 text-sm border border-gray-200 rounded hover:bg-gray-50 disabled:opacity-40"
          >
            上一页
          </button>
          <button
            :disabled="page * pageSize >= store.total"
            @click="goPage(page + 1)"
            class="px-3 py-1 text-sm border border-gray-200 rounded hover:bg-gray-50 disabled:opacity-40"
          >
            下一页
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
