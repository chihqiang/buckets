<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView, useRouter } from 'vue-router'
import Dialog from './components/Dialog.vue'
import { verifyToken } from './stores/api'
import { useAuthStore } from './stores/auth'

const auth = useAuthStore()
const router = useRouter()

onMounted(async () => {
  if (auth.isLoggedIn) {
    const valid = await verifyToken()
    if (!valid) {
      auth.isLoggedIn = false
      auth.isSuperAdmin = false
      auth.token = ''
      router.push('/login')
    }
  }
})
</script>

<template>
  <Dialog />
  <RouterView />
</template>
