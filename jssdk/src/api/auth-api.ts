import { AuthClient } from '../auth-client'
import { LoginResult } from './types'

export class AuthApi {
  constructor(private client: AuthClient) {}

  setToken(token: string): void {
    this.client.setToken(token)
  }

  async login(email: string, password: string): Promise<LoginResult> {
    return this.client.requestPublic<LoginResult>('/api/v1/auth/login', { email, password })
  }

  async refreshToken(refreshToken: string): Promise<LoginResult> {
    return this.client.requestPublic<LoginResult>('/api/v1/auth/refresh', { refresh_token: refreshToken })
  }

  async logout(): Promise<void> {
    await this.client.request<void>('POST', '/api/v1/auth/logout')
  }

  async verify(): Promise<{ user_id: number }> {
    return this.client.request<{ user_id: number }>('POST', '/api/v1/auth/verify')
  }
}
