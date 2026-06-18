/** 客户端请求错误，包含 HTTP 状态码 */
export class ClientError extends Error {
  readonly status: number

  constructor(message: string, status: number) {
    super(message)
    this.name = 'ClientError'
    this.status = status
  }
}

/** 统一 API 响应包装 */
interface ApiResponse<T> {
  code: number
  message: string
  data: T
}

/** 客户端构造参数 */
export interface ClientOptions {
  baseUrl: string
  token: string
}

/** 封装 HTTP 请求，处理认证头、JSON 序列化和统一错误解析 */
export class Client {
  private options: ClientOptions

  constructor(options: ClientOptions) {
    this.options = { ...options }
  }

  get baseUrl(): string {
    return this.options.baseUrl
  }

  setToken(token: string): void {
    this.options.token = token
  }

  getToken(): string {
    return this.options.token
  }

  /**
   * 发送 JSON 请求
   * @param method - HTTP 方法
   * @param path - 完整 API 路径（含 /api/v1 前缀）
   * @param body - 请求体对象
   */
  async request<T>(method: string, path: string, body?: Record<string, unknown>): Promise<T> {
    const url = `${this.options.baseUrl}${path}`
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.options.token}`,
    }
    const init: RequestInit = { method }

    if (body) {
      headers['Content-Type'] = 'application/json'
      init.body = JSON.stringify(body)
    }

    init.headers = headers

    const res = await fetch(url, init)
    return this.parseResponse<T>(res)
  }

  /**
   * 上传二进制数据（用于分块上传）
   * @param path - 完整 API 路径（含 /api/v1 前缀）
   * @param data - 二进制数据
   * @param extraHeaders - 额外的请求头（会话签名等）
   */
  async uploadBinary<T>(
    path: string,
    data: ArrayBuffer | Blob,
    extraHeaders: Record<string, string>,
  ): Promise<T> {
    const url = `${this.options.baseUrl}${path}`
    const headers: Record<string, string> = {
      'Content-Type': 'application/octet-stream',
      Authorization: `Bearer ${this.options.token}`,
      ...extraHeaders,
    }

    const res = await fetch(url, {
      method: 'POST',
      headers,
      body: data,
    })

    return this.parseResponse<T>(res)
  }

  /** 发送无认证的 JSON POST 请求 */
  async requestPublic<T>(path: string, body: Record<string, unknown>): Promise<T> {
    const url = `${this.options.baseUrl}${path}`
    const res = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    return this.parseResponse<T>(res)
  }

  private async parseResponse<T>(res: Response): Promise<T> {
    const traceId = res.headers.get('x-trace-id') || '-'

    if (!res.ok) {
      const text = await res.text()
      let message: string
      try {
        message = JSON.parse(text).message || res.statusText
      } catch {
        message = text || res.statusText
      }
      throw new ClientError(`[${traceId}] ${message}`, res.status)
    }

    const json: ApiResponse<T> = await res.json()
    if (json.code < 200 || json.code >= 300) {
      throw new ClientError(`[${traceId}] ${json.message}`, json.code)
    }
    return json.data
  }
}
