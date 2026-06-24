import { Client } from './client'

/** 默认分块大小：6 MiB */
const DEFAULT_TUS_CHUNK_SIZE = 6 * 1024 * 1024

/** 上传进度回调参数 */
export interface TusUploadProgress {
  bytesUploaded: number
  totalBytes: number
  percent: number
}

/** 上传选项 */
export interface TusUploadOptions {
  /** 每片读取大小（字节），默认 6MB */
  chunkSize?: number
  /** 是否使用 Upload-Defer-Length 扩展 */
  deferLength?: boolean
  /** 进度回调 */
  onProgress?: (progress: TusUploadProgress) => void
  /** 取消信号 */
  signal?: AbortSignal
  /** 自定义 Upload-Metadata */
  metadata?: Record<string, string>
}

/** 上传结果 */
export interface TusUploadResult {
  objectId: string
  storagePath: string
}

/** 服务端能力 */
export interface TusCapabilities {
  version: string
  extensions: string[]
  maxSize: number
}

/**
 * Tus 可恢复上传器。
 *
 * 遵循 tus resumable upload protocol 1.0.0，实现：
 * - POST 创建上传资源（带 Upload-Length / Upload-Defer-Length）
 * - HEAD 查询进度
 * - PATCH 追加数据
 * - DELETE 终止上传
 * - OPTIONS 查询能力
 *
 * @example
 * ```ts
 * const uploader = new TusUploader(client)
 * const result = await uploader.upload(file)
 * ```
 */
export class TusUploader {
  private readonly client: Client

  constructor(client: Client) {
    this.client = client
  }

  /**
   * 查询服务端 tus 能力。
   * 公开端点，无需认证。
   */
  async getCapabilities(): Promise<TusCapabilities> {
    const res = await this.client.requestPublicRaw('OPTIONS', '/api/v1/upload/tus')
    const version = res.headers.get('Tus-Version') || '1.0.0'
    const extensions = (res.headers.get('Tus-Extension') || '').split(',').map(s => s.trim()).filter(Boolean)
    const maxSize = Number(res.headers.get('Tus-Max-Size')) || Infinity
    return { version, extensions, maxSize }
  }

  /**
   * 创建 tus 上传资源。
   *
   * @param fileSize - 文件大小（字节）。deferLength 时为 0
   * @param metadata - 元数据键值对（value 自动 base64 编码）
   * @param deferLength - 是否延迟设置文件大小
   * @returns 上传资源的 task_id
   */
  async create(fileSize: number, metadata?: Record<string, string>, deferLength = false): Promise<string> {
    const headers: Record<string, string> = {
      'Tus-Resumable': '1.0.0',
    }

    if (deferLength) {
      headers['Upload-Defer-Length'] = '1'
    } else {
      headers['Upload-Length'] = String(fileSize)
    }

    if (metadata && Object.keys(metadata).length > 0) {
      const pairs: string[] = []
      for (const [key, value] of Object.entries(metadata)) {
        const encoded = btoa(value)
        pairs.push(`${key} ${encoded}`)
      }
      headers['Upload-Metadata'] = pairs.join(',')
    }

    const res = await this.client.requestRaw('POST', '/api/v1/upload/tus', { headers })
    const location = res.headers.get('Location') || ''
    const taskId = location.split('/').pop() || ''
    if (!taskId) {
      throw new Error('tus create: missing task_id in Location header')
    }
    return taskId
  }

  /**
   * 查询上传进度。
   *
   * @param taskId - 上传任务 ID
   * @returns 当前偏移量和可选的文件总大小
   */
  async offset(taskId: string): Promise<{ offset: number; length?: number }> {
    const res = await this.client.requestRaw('HEAD', `/api/v1/upload/tus/${taskId}`, {
      headers: { 'Tus-Resumable': '1.0.0' },
    })
    const offset = Number(res.headers.get('Upload-Offset'))
    const lengthHeader = res.headers.get('Upload-Length')
    return {
      offset: isNaN(offset) ? 0 : offset,
      length: lengthHeader ? Number(lengthHeader) : undefined,
    }
  }

  /**
   * 追加数据到上传资源。
   *
   * @param taskId - 上传任务 ID
   * @param uploadOffset - 当前偏移量
   * @param data - 二进制数据
   * @param finalLength - 仅用于 Upload-Defer-Length：最终文件大小
   * @returns 新的偏移量
   */
  async append(
    taskId: string,
    uploadOffset: number,
    data: ArrayBuffer | Blob,
    finalLength?: number,
  ): Promise<number> {
    const headers: Record<string, string> = {
      'Tus-Resumable': '1.0.0',
      'Content-Type': 'application/offset+octet-stream',
      'Upload-Offset': String(uploadOffset),
    }

    if (finalLength !== undefined) {
      headers['Upload-Length'] = String(finalLength)
    }

    const res = await this.client.requestRaw('PATCH', `/api/v1/upload/tus/${taskId}`, {
      headers,
      body: data,
    })

    const newOffset = Number(res.headers.get('Upload-Offset'))
    if (isNaN(newOffset)) {
      throw new Error('tus append: missing Upload-Offset in response')
    }
    return newOffset
  }

  /**
   * 终止上传并清理服务端资源。
   */
  async terminate(taskId: string): Promise<void> {
    await this.client.requestRaw('DELETE', `/api/v1/upload/tus/${taskId}`, {
      headers: { 'Tus-Resumable': '1.0.0' },
    })
  }

  /**
   * 完整上传流程：分片读取文件 → 逐个 PATCH 上传。
   *
   * @param file - 要上传的文件
   * @param options - 上传选项
   */
  async upload(file: File, options?: TusUploadOptions): Promise<TusUploadResult> {
    const signal = options?.signal
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const chunkSize = options?.chunkSize || DEFAULT_TUS_CHUNK_SIZE
    const deferLength = options?.deferLength || false

    // 构建元数据
    const metadata: Record<string, string> = {
      filename: file.name,
      ...options?.metadata,
    }
    if (file.type) {
      metadata.content_type = file.type
    }

    // 创建上传资源
    const taskId = await this.create(file.size, metadata, deferLength)
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    // 支持断点续传：查询已上传的偏移量
    let offset = await this.queryCurrentOffset(taskId)

    // 分片读取并上传
    let bytesUploaded = offset
    while (offset < file.size) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

      const end = Math.min(offset + chunkSize, file.size)
      const data = await readFileSlice(file, offset, end)
      const isFinalChunk = end >= file.size

      // 最后一片：如果使用 deferLength，传入最终大小
      const finalLength = isFinalChunk && deferLength ? file.size : undefined

      offset = await this.append(taskId, offset, data, finalLength)
      bytesUploaded = offset

      options?.onProgress?.({
        bytesUploaded,
        totalBytes: file.size,
        percent: Math.round((bytesUploaded / file.size) * 100),
      })
    }

    return { objectId: taskId, storagePath: '' }
  }

  /** 查询当前已上传偏移量（用于断点续传） */
  private async queryCurrentOffset(taskId: string): Promise<number> {
    try {
      const { offset } = await this.offset(taskId)
      return offset
    } catch {
      return 0
    }
  }
}

/**
 * 通过 FileReader 读取文件切片。
 */
function readFileSlice(file: File, start: number, end: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as ArrayBuffer)
    reader.onerror = () => reject(reader.error)
    reader.readAsArrayBuffer(file.slice(start, end))
  })
}
