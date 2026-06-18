import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getApi } from './api'
import type { ObjectItem } from '../sdk/api'

export const useObjectsStore = defineStore('objects', () => {
  const objects = ref<ObjectItem[]>([])
  const total = ref(0)
  const loading = ref(false)
  const api = getApi()

  async function fetchList(page = 1, pageSize = 20) {
    loading.value = true
    try {
      const res = await api.getObjectList(page, pageSize)
      objects.value = res.items
      total.value = res.total
    } finally {
      loading.value = false
    }
  }

  async function remove(id: number) {
    await api.deleteObject(id)
  }

  return { objects, total, loading, fetchList, remove }
})
