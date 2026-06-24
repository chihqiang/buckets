import { BucketsClient } from '@chihqiang/buckets'
import type { LoginResult } from '@chihqiang/buckets'

let _client: BucketsClient | null = null

export function getApi(): BucketsClient {
  if (!_client) {
    const token = localStorage.getItem('token') ?? ''
    _client = new BucketsClient({ baseUrl: '', initialToken: token })
  }
  return _client
}

export async function login(email: string, password: string): Promise<LoginResult> {
  const client = getApi()
  const data = await client.auth.login(email, password)
  client.setToken(data.token)
  localStorage.setItem('token', data.token)
  localStorage.setItem('refresh_token', data.refresh_token)
  localStorage.setItem('is_super_admin', String(data.is_super_admin))
  return data
}

export async function logout(): Promise<void> {
  const client = getApi()
  try {
    await client.auth.logout()
  } catch {
    // Ignore API errors; clear local state regardless
  }
  client.setToken('')
  localStorage.removeItem('token')
  localStorage.removeItem('refresh_token')
  localStorage.removeItem('is_super_admin')
}
