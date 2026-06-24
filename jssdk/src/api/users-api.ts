import { AuthClient } from '../auth-client'
import { User, PaginatedResponse } from './types'

export class UsersApi {
  constructor(private client: AuthClient) {}

  async list(page = 1, pageSize = 20): Promise<PaginatedResponse<User>> {
    const params = new URLSearchParams({ page: String(page), page_size: String(pageSize) })
    return this.client.request<PaginatedResponse<User>>('GET', `/api/v1/users?${params}`)
  }

  async create(email: string, password: string): Promise<User> {
    return this.client.request<User>('POST', '/api/v1/users', { email, password })
  }

  async update(id: number, data: { email?: string; password?: string }): Promise<User> {
    return this.client.request<User>('PUT', `/api/v1/users/${id}`, data)
  }

  async delete(id: number): Promise<void> {
    await this.client.request<void>('DELETE', `/api/v1/users/${id}`)
  }

  async resetSecretKey(id: number): Promise<void> {
    await this.client.request<void>('POST', `/api/v1/users/${id}/reset-secret-key`)
  }
}
