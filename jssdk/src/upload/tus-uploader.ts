import { AuthClient } from '../auth-client'
import { ClientError } from '../errors'
import { readFileSlice } from './read-file-slice'
import { TusUploadOptions, TusUploadResult, TusCapabilities } from './types'

const DEFAULT_CHUNK_SIZE = 6 * 1024 * 1024

export class TusUploader {
  constructor(private client: AuthClient) {}

  async getCapabilities(): Promise<TusCapabilities> {
    const res = await this.client.requestPublicRaw('OPTIONS', '/api/v1/upload/tus')
    const version = res.headers.get('Tus-Version') || '1.0.0'
    const extensions = (res.headers.get('Tus-Extension') || '')
      .split(',').map(s => s.trim()).filter(Boolean)
    const maxSize = Number(res.headers.get('Tus-Max-Size')) || Infinity
    return { version, extensions, maxSize }
  }

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
      const pairs = Object.entries(metadata).map(([key, value]) => `${key} ${btoa(unescape(encodeURIComponent(value)))}`)
      headers['Upload-Metadata'] = pairs.join(',')
    }

    const res = await this.client.requestRaw('POST', '/api/v1/upload/tus', { headers })
    const location = res.headers.get('Location') || ''
    const taskId = location.split('/').filter(Boolean).pop() || ''
    if (!taskId) {
      throw new ClientError('tus create: missing task_id in Location header', 500)
    }
    return taskId
  }

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
      throw new ClientError('tus append: missing Upload-Offset in response', 500)
    }
    return newOffset
  }

  async terminate(taskId: string): Promise<void> {
    await this.client.requestRaw('DELETE', `/api/v1/upload/tus/${taskId}`, {
      headers: { 'Tus-Resumable': '1.0.0' },
    })
  }

  async upload(file: File, options?: TusUploadOptions): Promise<TusUploadResult> {
    const signal = options?.signal
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const chunkSize = options?.chunkSize || DEFAULT_CHUNK_SIZE
    const deferLength = options?.deferLength || false

    const metadata: Record<string, string> = {
      filename: file.name,
      ...options?.metadata,
    }
    if (file.type) {
      metadata.content_type = file.type
    }

    const taskId = await this.create(file.size, metadata, deferLength)
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    let offset = await this.queryCurrentOffset(taskId)

    let bytesUploaded = offset
    while (offset < file.size) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

      const end = Math.min(offset + chunkSize, file.size)
      const data = await readFileSlice(file, offset, end)
      const isFinalChunk = end >= file.size
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

  private async queryCurrentOffset(taskId: string): Promise<number> {
    try {
      const { offset } = await this.offset(taskId)
      return offset
    } catch (err) {
      if (err instanceof ClientError && err.status === 404) {
        return 0
      }
      throw err
    }
  }
}
