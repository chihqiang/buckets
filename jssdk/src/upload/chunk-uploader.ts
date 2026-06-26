import { AuthClient } from '../auth-client'
import { ClientError } from '../errors'
import { computeChunkMd5s } from './hasher'
import { readFileSlice } from './read-file-slice'
import {
  ChunkPrecheckResult,
  ChunkStsOptions,
  ChunkStsResult,
  ChunkItemResult,
  ChunkUploadProgress,
  ChunkUploadOptions,
  ChunkUploadResult,
  ChunkSessionCredentials,
} from './types'

const MAX_PARALLEL = 6

export class ChunkUploader {
  constructor(private client: AuthClient) {}

  async sts(options: ChunkStsOptions): Promise<ChunkStsResult> {
    return this.client.request<ChunkStsResult>('POST', '/api/v1/upload/sts', { ...options })
  }

  async precheck(options: ChunkStsOptions): Promise<ChunkPrecheckResult> {
    return this.client.request<ChunkPrecheckResult>('POST', '/api/v1/upload/precheck', { ...options })
  }

  async merge(options: {
    task_id: string
    file_name: string
    file_md5: string
    file_size: number
    content_type?: string
  }): Promise<{ task_id: string; message: string }> {
    return this.client.request('POST', '/api/v1/upload/merge', { ...options })
  }

  async upload(file: File, options?: ChunkUploadOptions): Promise<ChunkUploadResult> {
    const signal = options?.signal
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const chunkSize = options?.chunkSize || 8 * 1024 * 1024
    const parallel = Math.min(options?.parallel || 3, MAX_PARALLEL)
    const totalChunks = Math.ceil(file.size / chunkSize)

    const [merkleMd5, chunkMd5s, rawFileMd5] = await computeChunkMd5s(file, chunkSize, totalChunks, signal)
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const stsResult = await this.sts({
      file_name: file.name,
      file_size: file.size,
      file_md5: rawFileMd5,
      chunk_size: chunkSize,
    })
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    // precheck 用原始文件 MD5 去重（与直传/TUS 一致）
    const precheckResult = await this.precheck({
      file_name: file.name,
      file_size: file.size,
      file_md5: rawFileMd5,
      chunk_size: chunkSize,
    })
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    if (precheckResult.exists) {
      return this.handleExists(precheckResult)
    }

    if (!precheckResult.task_id) {
      throw new ClientError('server returned exists=false without task_id', 500)
    }

    const session: ChunkSessionCredentials = {
      session_signature: stsResult.session_signature,
      session_timestamp: stsResult.session_timestamp,
      session_salt: stsResult.session_salt,
    }

    await this.uploadChunks(
      file, precheckResult.task_id, totalChunks, chunkSize, chunkMd5s,
      precheckResult.uploaded_chunks, session, parallel, signal,
      options?.onProgress,
    )

    await this.merge({
      task_id: precheckResult.task_id,
      file_name: file.name,
      file_md5: merkleMd5,
      file_size: file.size,
      content_type: file.type || undefined,
    })

    return this.pollMergeStatus(precheckResult.task_id)
  }

  private async sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms))
  }

  private handleExists(precheckResult: ChunkPrecheckResult): ChunkUploadResult {
    if (!precheckResult.bucket_id || !precheckResult.storage_path) {
      throw new ClientError('server returned exists=true without bucket_id or storage_path', 500)
    }
    return {
      bucketId: precheckResult.bucket_id,
      bucketUrl: precheckResult.storage_path,
    }
  }

  private async uploadChunks(
    file: File,
    taskId: string,
    totalChunks: number,
    chunkSize: number,
    chunkMd5s: string[],
    alreadyUploaded: number[],
    session: ChunkSessionCredentials,
    parallel: number,
    signal: AbortSignal | undefined,
    onProgress: ((progress: ChunkUploadProgress) => void) | undefined,
  ): Promise<void> {
    const uploadedSet = new Set(alreadyUploaded)
    const queue: number[] = []
    for (let i = 0; i < totalChunks; i++) {
      if (!uploadedSet.has(i)) queue.push(i)
    }
    if (queue.length === 0) return

    let completed = uploadedSet.size

    const slot = async () => {
      while (queue.length > 0) {
        if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
        const chunkIndex = queue.shift()!
        const start = chunkIndex * chunkSize
        const end = Math.min(start + chunkSize, file.size)
        const data = await readFileSlice(file, start, end)

        const params = new URLSearchParams({
          task_id: taskId,
          chunk_index: String(chunkIndex),
          chunk_md5: chunkMd5s[chunkIndex],
        })

        await this.client.uploadBinary<ChunkItemResult>(
          `/api/v1/upload/chunk/upload-binary?${params}`,
          data,
          {
            'X-Session-Signature': session.session_signature,
            'X-Session-Timestamp': String(session.session_timestamp),
            'X-Session-Salt': session.session_salt,
          },
        )

        completed++
        onProgress?.({
          totalChunks,
          uploadedChunks: completed,
          percent: Math.round((completed / totalChunks) * 100),
        })
      }
    }

    await Promise.all(
      Array.from({ length: Math.min(parallel, queue.length) }, () => slot()),
    )
  }

  private async pollMergeStatus(
    taskId: string,
    interval = 2000,
    maxAttempts = 3600,
  ): Promise<ChunkUploadResult> {
    for (let i = 0; i < maxAttempts; i++) {
      const params = new URLSearchParams({ task_id: taskId })
      const result = await this.client.request<{
        task_id: string
        status: string
        storage_path: string | null
        error?: string
      }>('GET', `/api/v1/upload/merge/status?${params}`)

      switch (result.status) {
        case 'completed':
          if (!result.storage_path) {
            throw new ClientError('server returned status=completed without storage_path', 500)
          }
          return { bucketId: taskId, bucketUrl: result.storage_path }

        case 'failed':
          throw new ClientError(result.error || 'merge failed', 500)

        case 'merging':
        case 'uploading':
          await this.sleep(interval)
          continue

        default:
          throw new ClientError(`unknown merge status: ${result.status}`, 500)
      }
    }

    throw new ClientError(`merge status polling timed out for task: ${taskId}`, 500)
  }
}
