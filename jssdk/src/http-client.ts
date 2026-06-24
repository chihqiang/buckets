import { ClientError } from './errors'

export interface HttpClientOptions {
  baseUrl: string
  timeout?: number
}

interface ApiResponse<T> {
  code: number
  message: string
  data: T
}

export class HttpClient {
  constructor(private options: HttpClientOptions) {}

  private get baseUrl(): string {
    return this.options.baseUrl
  }

  private get timeout(): number | undefined {
    return this.options.timeout
  }

  private async fetchWithTimeout(url: string, init: RequestInit): Promise<Response> {
    const timeout = this.timeout
    if (!timeout) return fetch(url, init)

    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), timeout)
    try {
      const res = await fetch(url, { ...init, signal: controller.signal })
      return res
    } finally {
      clearTimeout(timer)
    }
  }

  private async parseResponse<T>(res: Response): Promise<T> {
    const traceId = res.headers.get('x-trace-id') || '-'

    if (!res.ok) {
      const text = await res.text()
      const message = extractMessage(text, res.statusText)
      throw new ClientError(`[${traceId}] ${message}`, res.status, traceId)
    }

    const json: ApiResponse<T> = await res.json()
    if (json.code < 200 || json.code >= 300) {
      throw new ClientError(`[${traceId}] ${json.message}`, json.code, traceId)
    }
    return json.data
  }

  async request<T>(method: string, path: string, extraHeaders?: Record<string, string>, body?: Record<string, unknown>): Promise<T> {
    const url = `${this.baseUrl}${path}`
    const headers: Record<string, string> = { ...extraHeaders }
    const init: RequestInit = { method }

    if (body) {
      headers['Content-Type'] = 'application/json'
      init.body = JSON.stringify(body)
    }

    init.headers = headers
    const res = await this.fetchWithTimeout(url, init)
    return this.parseResponse<T>(res)
  }

  async uploadBinary<T>(path: string, data: ArrayBuffer | Blob, extraHeaders: Record<string, string>): Promise<T> {
    const url = `${this.baseUrl}${path}`
    const headers: Record<string, string> = {
      'Content-Type': 'application/octet-stream',
      ...extraHeaders,
    }

    const res = await this.fetchWithTimeout(url, {
      method: 'POST',
      headers,
      body: data,
    })

    return this.parseResponse<T>(res)
  }

  async requestRaw(method: string, path: string, options?: {
    headers?: Record<string, string>
    body?: BodyInit | null
  }): Promise<Response> {
    const url = `${this.baseUrl}${path}`
    const headers: Record<string, string> = { ...options?.headers }
    const init: RequestInit = { method, headers }
    if (options?.body !== undefined) {
      init.body = options.body
    }

    const res = await this.fetchWithTimeout(url, init)
    if (!res.ok) {
      const traceId = res.headers.get('x-trace-id') || '-'
      const text = await res.text()
      const message = extractMessage(text, res.statusText)
      throw new ClientError(`[${traceId}] ${message}`, res.status, traceId)
    }
    return res
  }

  async requestPublic<T>(path: string, body: Record<string, unknown>): Promise<T> {
    const url = `${this.baseUrl}${path}`
    const res = await this.fetchWithTimeout(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    return this.parseResponse<T>(res)
  }

  async requestPublicRaw(method: string, path: string): Promise<Response> {
    const url = `${this.baseUrl}${path}`
    return this.fetchWithTimeout(url, { method })
  }
}

function extractMessage(text: string, fallback: string): string {
  try {
    return JSON.parse(text).message || fallback
  } catch {
    return text || fallback
  }
}
