import { Api } from '../sdk/api'
import type { LoginResult } from '../sdk/api'
import { Client } from '../sdk/client'

let _api: Api | null = null

export function getApi(): Api {
  if (!_api) {
    const token = localStorage.getItem('token') ?? ''
    _api = new Api(new Client({ baseUrl: '', token }))
  }
  return _api
}

/** 登录，保存 token 到 localStorage，返回登录结果 */
export async function login(email: string, password: string): Promise<LoginResult> {
  const api = getApi()
  const data = await api.login(email, password)
  api.setToken(data.token)
  localStorage.setItem('token', data.token)
  localStorage.setItem('refresh_token', data.refresh_token)
  localStorage.setItem('is_super_admin', String(data.is_super_admin))
  return data
}

/** 退出登录，清除 localStorage */
export async function logout(): Promise<void> {
  const api = getApi()
  try {
    await api.logout()
  } catch {
    // Ignore API errors; clear local state regardless
  }
  api.setToken('')
  localStorage.removeItem('token')
  localStorage.removeItem('refresh_token')
  localStorage.removeItem('is_super_admin')
}
