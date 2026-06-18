import { Client } from './client'

/** 登录 / 刷新 token 的响应 */
export interface LoginResult {
  token: string
  refresh_token: string
  expires_in: number
  is_super_admin: boolean
}

export interface ObjectItem {
  id: number
  uuid: string
  name: string
  size: number
  md5: string
  content_type: string | null
  extension: string | null
  bucket: string
  storage_path: string | null
  image_width: number
  image_height: number
  image_type: string
  status: string
  created_at: string
  updated_at: string
}

export interface User {
  id: number
  email: string
  created_at: string
  updated_at: string
}

export interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}

/** 封装所有 API 请求 */
export class Api {
  private client: Client

  constructor(client: Client) {
    this.client = client
  }

  setToken(token: string): void {
    this.client.setToken(token)
  }

  // ─── Auth ─────────────────────────────────────────────

  async login(email: string, password: string): Promise<LoginResult> {
    return this.client.requestPublic<LoginResult>('/api/v1/auth/login', { email, password })
  }

  async refreshToken(refreshToken: string): Promise<LoginResult> {
    return this.client.requestPublic<LoginResult>('/api/v1/auth/refresh', { refresh_token: refreshToken })
  }

  async logout(): Promise<void> {
    await this.client.request<void>('POST', '/api/v1/auth/logout')
  }

  // ─── Objects ──────────────────────────────────────────

  async getObjectList(page = 1, pageSize = 20): Promise<PaginatedResponse<ObjectItem>> {
    return this.client.request<PaginatedResponse<ObjectItem>>('GET', `/api/v1/objects?page=${page}&page_size=${pageSize}`)
  }

  async deleteObject(id: number): Promise<void> {
    await this.client.request<void>('DELETE', `/api/v1/objects/${id}`)
  }

  // ─── Users ────────────────────────────────────────────

  async getUserList(page = 1, pageSize = 20): Promise<PaginatedResponse<User>> {
    return this.client.request<PaginatedResponse<User>>('GET', `/api/v1/users?page=${page}&page_size=${pageSize}`)
  }

  async createUser(email: string, password: string): Promise<User> {
    return this.client.request<User>('POST', '/api/v1/users', { email, password })
  }

  async updateUser(id: number, data: { email?: string; password?: string }): Promise<User> {
    return this.client.request<User>('PUT', `/api/v1/users/${id}`, data)
  }

  async deleteUser(id: number): Promise<void> {
    await this.client.request<void>('DELETE', `/api/v1/users/${id}`)
  }

  async resetUserSecretKey(id: number): Promise<void> {
    await this.client.request<void>('POST', `/api/v1/users/${id}/reset-secret-key`)
  }
}
