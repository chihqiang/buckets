import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getApi } from './api'
import type { User } from '../sdk/api'

export const useUsersStore = defineStore('users', () => {
  const users = ref<User[]>([])
  const total = ref(0)
  const loading = ref(false)
  const api = getApi()

  async function fetchList(page = 1, pageSize = 20) {
    loading.value = true
    try {
      const res = await api.getUserList(page, pageSize)
      users.value = res.items
      total.value = res.total
    } finally {
      loading.value = false
    }
  }

  async function create(email: string, password: string) {
    return api.createUser(email, password)
  }

  async function update(id: number, data: { email?: string; password?: string }) {
    return api.updateUser(id, data)
  }

  async function remove(id: number) {
    await api.deleteUser(id)
  }

  async function resetSecret(id: number) {
    await api.resetUserSecretKey(id)
  }

  return { users, total, loading, fetchList, create, update, remove, resetSecret }
})
