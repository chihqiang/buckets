import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/login',
      name: 'Login',
      component: () => import('../views/Login.vue'),
    },
    {
      path: '/',
      component: () => import('../components/Layout.vue'),
      redirect: '/objects',
      children: [
        {
          path: 'objects',
          name: 'ObjectList',
          component: () => import('../views/ObjectList.vue'),
        },
        {
          path: 'users',
          name: 'UserList',
          component: () => import('../views/UserList.vue'),
        },
      ],
    },
  ],
})

router.beforeEach((to) => {
  const token = localStorage.getItem('token')
  if (to.name !== 'Login' && !token) {
    return { name: 'Login' }
  }
  if (to.name === 'UserList') {
    const auth = useAuthStore()
    if (!auth.isSuperAdmin) {
      return { name: 'ObjectList' }
    }
  }
})

export default router
