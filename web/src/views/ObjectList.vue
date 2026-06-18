<script setup lang="ts">
import { onMounted, ref, computed } from 'vue'
import { useObjectsStore } from '../stores/objects'
import { useAuthStore } from '../stores/auth'
import { Client, ChunkUploader } from '../sdk'
import { useDialog } from '../composables/useDialog'

const store = useObjectsStore()
const auth = useAuthStore()
const { confirm } = useDialog()
const page = ref(1)
const pageSize = 20

const uploading = ref(false)
const uploadProgress = ref(0)
const uploadStatus = ref<'idle' | 'computing' | 'uploading' | 'merging' | 'completed' | 'error'>('idle')
const uploadError = ref('')

const statusText = computed(() => {
  switch (uploadStatus.value) {
    case 'computing': return '正在计算文件哈希...'
    case 'uploading': return `正在上传 ${uploadProgress.value}%`
    case 'merging': return '正在合并文件...'
    case 'completed': return '上传完成'
    case 'error': return uploadError.value || '上传失败'
    default: return ''
  }
})

onMounted(() => store.fetchList(page.value, pageSize))

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return (bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0) + ' ' + units[i]
}

async function handleDelete(id: number) {
  if (!(await confirm('确定要删除此文件吗？'))) return
  await store.remove(id)
  await store.fetchList(page.value, pageSize)
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleString('zh-CN')
}

async function goPage(p: number) {
  page.value = p
  await store.fetchList(p, pageSize)
}

async function handleUpload() {
  const input = document.createElement('input')
  input.type = 'file'
  input.onchange = async () => {
    const file = input.files?.[0]
    if (!file) return

    const client = new Client({ baseUrl: '', token: auth.token })
    const uploader = new ChunkUploader(client)
    uploading.value = true
    uploadProgress.value = 0
    uploadStatus.value = 'computing'
    uploadError.value = ''

    try {
      await uploader.upload(file, {
        onProgress: (p) => {
          uploadProgress.value = p.percent
          uploadStatus.value = 'uploading'
        },
      })
      uploadStatus.value = 'completed'
      await store.fetchList(page.value, pageSize)
    } catch (err: any) {
      uploadStatus.value = 'error'
      uploadError.value = err.message || String(err)
    } finally {
      uploading.value = false
    }
  }
  input.click()
}
</script>

<template>
  <div>
    <div class="flex items-center justify-between mb-4">
      <h2 class="text-lg font-semibold text-gray-800">文件管理</h2>
      <button
        @click="handleUpload"
        :disabled="uploading"
        class="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-lg hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        {{ uploading ? '上传中...' : '上传文件' }}
      </button>
    </div>

    <div
      v-if="uploading || uploadStatus === 'completed' || uploadStatus === 'error'"
      class="mb-4 p-3 rounded-lg border text-sm"
      :class="{
        'bg-blue-50 border-blue-200 text-blue-700': uploadStatus === 'computing' || uploadStatus === 'uploading' || uploadStatus === 'merging',
        'bg-green-50 border-green-200 text-green-700': uploadStatus === 'completed',
        'bg-red-50 border-red-200 text-red-700': uploadStatus === 'error',
      }"
    >
      <div class="flex items-center gap-2">
        <span v-if="uploadStatus === 'computing' || uploadStatus === 'uploading' || uploadStatus === 'merging'" class="inline-block w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
        <span>{{ statusText }}</span>
      </div>
      <div v-if="uploadStatus === 'uploading'" class="mt-2 w-full bg-blue-200 rounded-full h-2">
        <div class="bg-blue-600 h-2 rounded-full transition-all duration-300" :style="{ width: uploadProgress + '%' }" />
      </div>
    </div>

    <div class="bg-white rounded-lg border border-gray-200">
      <div class="overflow-x-auto">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-gray-200 bg-gray-50">
              <th class="text-left px-4 py-3 font-medium text-gray-600">ID</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">UUID</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">文件名</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">存储路径</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">大小</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">类型</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">状态</th>
              <th class="text-left px-4 py-3 font-medium text-gray-600">上传时间</th>
              <th class="text-right px-4 py-3 font-medium text-gray-600">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="store.loading" class="border-b border-gray-100">
              <td colspan="9" class="px-4 py-8 text-center text-gray-400">加载中...</td>
            </tr>
            <tr v-else-if="store.objects.length === 0" class="border-b border-gray-100">
              <td colspan="9" class="px-4 py-8 text-center text-gray-400">暂无文件</td>
            </tr>
            <tr v-for="f in store.objects" :key="f.id" class="border-b border-gray-100 hover:bg-gray-50">
              <td class="px-4 py-3 text-gray-600 text-xs">{{ f.id }}</td>
              <td class="px-4 py-3 text-gray-400 text-xs max-w-xs truncate" :title="f.uuid">{{ f.uuid }}</td>
              <td class="px-4 py-3 text-gray-800 max-w-xs truncate" :title="f.name">{{ f.name }}</td>
              <td class="px-4 py-3 text-gray-400 text-xs max-w-xs truncate" :title="f.storage_path ?? ''">{{ f.storage_path ?? '-' }}</td>
              <td class="px-4 py-3 text-gray-600">{{ formatSize(f.size) }}</td>
              <td class="px-4 py-3 text-gray-600">
                {{ f.content_type || '-' }}
                <span v-if="f.image_width > 0" class="ml-1 text-xs text-gray-400">
                  ({{ f.image_width }}x{{ f.image_height }}, {{ f.image_type }})
                </span>
              </td>
              <td class="px-4 py-3">
                <span
                  class="inline-block px-2 py-0.5 rounded-full text-xs font-medium"
                  :class="f.status === 'active' ? 'bg-green-100 text-green-700' : 'bg-gray-100 text-gray-600'"
                >
                  {{ f.status === 'active' ? '正常' : f.status }}
                </span>
              </td>
              <td class="px-4 py-3 text-gray-600 text-xs">{{ formatDate(f.created_at) }}</td>
              <td class="px-4 py-3 text-right">
                <button
                  @click="handleDelete(f.id)"
                  class="text-red-600 hover:text-red-800 text-sm"
                >
                  删除
                </button>
              </td>
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
