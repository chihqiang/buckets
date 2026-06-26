import { HttpClient } from './http-client'

export class AuthClient {
  private token = ''

  constructor(
    private http: HttpClient,
    initialToken?: string,
  ) {
    if (initialToken) this.token = initialToken
  }

  setToken(token: string): void {
    this.token = token
  }

  getToken(): string {
    return this.token
  }

  private authHeaders(): Record<string, string> {
    return this.token ? { Authorization: `Bearer ${this.token}` } : {}
  }

  async request<T>(method: string, path: string, body?: Record<string, unknown>): Promise<T> {
    return this.http.request<T>(method, path, this.authHeaders(), body)
  }

  async uploadBinary<T>(path: string, data: ArrayBuffer | Blob | FormData, extraHeaders: Record<string, string>): Promise<T> {
    return this.http.uploadBinary<T>(path, data, { ...this.authHeaders(), ...extraHeaders })
  }

  async requestRaw(method: string, path: string, options?: {
    headers?: Record<string, string>
    body?: BodyInit | null
    signal?: AbortSignal
  }): Promise<Response> {
    const authHeaders = this.authHeaders()
    const mergedHeaders = options?.headers
      ? { ...authHeaders, ...options.headers }
      : authHeaders
    return this.http.requestRaw(method, path, { ...options, headers: mergedHeaders })
  }

  async requestPublic<T>(path: string, body: Record<string, unknown>): Promise<T> {
    return this.http.requestPublic<T>(path, body)
  }

  async requestPublicRaw(method: string, path: string): Promise<Response> {
    return this.http.requestPublicRaw(method, path)
  }

  async requestBlob(method: string, path: string): Promise<Blob> {
    return this.http.requestBlob(method, path, this.authHeaders())
  }

  getBaseUrl(): string {
    return this.http.getBaseUrl()
  }
}
