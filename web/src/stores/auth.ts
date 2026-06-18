import { defineStore } from 'pinia'
import { ref } from 'vue'
import { login as loginApi, logout as logoutApi } from './api'

export const useAuthStore = defineStore('auth', () => {
  const token = ref(localStorage.getItem('token') ?? '')
  const isLoggedIn = ref(!!token.value)
  const isSuperAdmin = ref(localStorage.getItem('is_super_admin') === 'true')

  async function login(email: string, password: string) {
    const data = await loginApi(email, password)
    token.value = data.token
    isSuperAdmin.value = data.is_super_admin
    isLoggedIn.value = true
  }

  async function logout() {
    await logoutApi()
    token.value = ''
    isSuperAdmin.value = false
    isLoggedIn.value = false
  }

  return { token, isLoggedIn, isSuperAdmin, login, logout }
})
