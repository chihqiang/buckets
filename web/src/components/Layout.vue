<script setup lang="ts">
import { useAuthStore } from '../stores/auth'
import { useRouter } from 'vue-router'

const auth = useAuthStore()
const router = useRouter()

async function handleLogout() {
  await auth.logout()
  router.push('/login')
}
</script>

<template>
  <div class="min-h-screen flex flex-col bg-gray-100">
    <header class="h-14 bg-white border-b border-gray-200 flex items-center px-4 shrink-0">
      <span class="font-semibold text-lg text-gray-800 mr-8">buckets</span>
      <nav class="flex items-center gap-1 flex-1">
        <RouterLink
          to="/objects"
          class="px-3 py-1.5 rounded-lg text-sm text-gray-600 hover:bg-gray-100 hover:text-gray-900"
          active-class="bg-blue-50 text-blue-700 font-medium"
        >
          文件管理
        </RouterLink>
        <RouterLink
          v-if="auth.isSuperAdmin"
          to="/users"
          class="px-3 py-1.5 rounded-lg text-sm text-gray-600 hover:bg-gray-100 hover:text-gray-900"
          active-class="bg-blue-50 text-blue-700 font-medium"
        >
          用户管理
        </RouterLink>
      </nav>
      <button
        @click="handleLogout"
        class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm text-gray-600 hover:bg-gray-100 hover:text-red-600"
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"/></svg>
        退出
      </button>
    </header>
    <main class="flex-1 overflow-auto p-6">
      <RouterView />
    </main>
  </div>
</template>
