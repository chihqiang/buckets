import SparkMD5 from 'spark-md5'
import { Client, ClientError } from './client'

/** STS 令牌请求参数 */
export interface ChunkStsOptions {
  file_name: string
  file_size: number
  file_md5: string
  chunk_size?: number
}

/** STS 令牌响应 */
export interface ChunkStsResult {
  task_id: string
  bucket_key: string
  session_signature: string
  session_timestamp: number
  session_salt: string
}

/** 预检响应：去重查询与断点续传信息 */
export interface ChunkPrecheckResult {
  exists: boolean
  bucket_id: string | null
  storage_path: string | null
  task_id: string | null
  uploaded_chunks: number[]
  chunk_size: number
}

/** 分块上传响应 */
export interface ChunkItemResult {
  chunk_index: number
  status: string
  md5: string
}

/** 分块上传状态查询响应 */
export interface ChunkStatusResult {
  task_id: string
  chunk_count: number
  uploaded_count: number
  missing_chunks: number[]
  is_complete: boolean
}

/** 合并请求参数 */
export interface ChunkMergeOptions {
  task_id: string
  file_name: string
  file_md5: string
  file_size: number
  content_type?: string
}

/** 合并启动响应 */
export interface ChunkMergeResult {
  task_id: string
  message: string
}

/** 合并状态轮询响应 */
export interface ChunkMergeStatusResult {
  task_id: string
  status: string
  storage_path: string | null
  error?: string
}

/** 上传进度回调参数 */
export interface ChunkUploadProgress {
  totalChunks: number
  uploadedChunks: number
  percent: number
}

/** 上传选项 */
export interface ChunkUploadOptions {
  chunkSize?: number
  parallel?: number
  onProgress?: (progress: ChunkUploadProgress) => void
  signal?: AbortSignal
}

/** 上传最终结果 */
export interface ChunkUploadResult {
  bucketId: string
  bucketUrl: string
}

/** 会话级别签名凭据，用于分块上传认证 */
export interface ChunkSessionCredentials {
  session_signature: string
  session_timestamp: number
  session_salt: string
}

/** 文件上传器，封装 STS → 预检 → 分块上传 → 合并 完整流程 */
export class ChunkUploader {
  private readonly client: Client

  /**
   * @param client - HTTP 客户端实例
   */
  constructor(client: Client) {
    this.client = client
  }

  /**
   * 请求 STS 令牌，获取会话级别签名
   * @param options - 文件信息
   */
  async sts(options: ChunkStsOptions): Promise<ChunkStsResult> {
    return this.client.request<ChunkStsResult>('POST', '/api/v1/upload/sts', { ...options })
  }

  /**
   * 预检文件：去重查询 + 断点续传
   * @param options - 文件信息
   */
  async precheck(options: ChunkStsOptions): Promise<ChunkPrecheckResult> {
    return this.client.request<ChunkPrecheckResult>('POST', '/api/v1/upload/precheck', { ...options })
  }

  /**
   * 查询指定上传任务的分块上传进度
   * @param taskId - 上传任务 ID
   */
  async chunkStatus(taskId: string): Promise<ChunkStatusResult> {
    return this.client.request<ChunkStatusResult>('POST', '/api/v1/upload/chunk/status', {
      task_id: taskId,
    })
  }

  /**
   * 请求合并所有已上传的分块（异步）
   * @param options - 合并参数
   */
  async merge(options: ChunkMergeOptions): Promise<ChunkMergeResult> {
    return this.client.request<ChunkMergeResult>('POST', '/api/v1/upload/merge', { ...options })
  }

  /**
   * 查询合并状态
   * @param taskId - 上传任务 ID
   */
  async mergeStatus(taskId: string): Promise<ChunkMergeStatusResult> {
    return this.client.request<ChunkMergeStatusResult>('GET', `/api/v1/upload/merge/status?task_id=${taskId}`)
  }

  /**
   * 完整上传流程：计算 MD5 → STS → 预检 → 分块上传 → 合并 → 轮询
   * @param file - 要上传的文件
   * @param options - 上传选项
   */
  async upload(file: File, options?: ChunkUploadOptions): Promise<ChunkUploadResult> {
    const signal = options?.signal
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const chunkSize = options?.chunkSize || 8 * 1024 * 1024
    const parallel = Math.min(options?.parallel || 3, 6)
    const totalChunks = Math.ceil(file.size / chunkSize)

    const [fileMd5, chunkMd5s] = await this.computeChunkMd5s(file, chunkSize, totalChunks, signal)
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const stsResult = await this.sts({
      file_name: file.name,
      file_size: file.size,
      file_md5: fileMd5,
      chunk_size: chunkSize,
    })
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const precheckResult = await this.precheck({
      file_name: file.name,
      file_size: file.size,
      file_md5: fileMd5,
      chunk_size: chunkSize,
    })
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    if (precheckResult.exists) {
      return {
        bucketId: precheckResult.bucket_id!,
        bucketUrl: precheckResult.storage_path!,
      }
    }

    const taskId = precheckResult.task_id!
    const session: ChunkSessionCredentials = {
      session_signature: stsResult.session_signature,
      session_timestamp: stsResult.session_timestamp,
      session_salt: stsResult.session_salt,
    }

    await this.uploadChunks(file, taskId, totalChunks, chunkSize, chunkMd5s, precheckResult.uploaded_chunks, session, parallel, signal, options?.onProgress)

    await this.merge({
      task_id: taskId,
      file_name: file.name,
      file_md5: fileMd5,
      file_size: file.size,
      content_type: file.type || undefined,
    })

    return this.pollMergeStatus(taskId)
  }

  /**
   * 并行上传未完成的分块，支持断点续传
   * @param file - 源文件
   * @param taskId - 上传任务 ID
   * @param totalChunks - 总分块数
   * @param chunkSize - 分块大小
   * @param chunkMd5s - 各分块预计算的 MD5
   * @param alreadyUploaded - 已上传的分块索引
   * @param session - 会话签名凭据
   * @param parallel - 并行数
   * @param signal - 取消信号
   * @param onProgress - 进度回调
   */
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

    await Promise.all(Array.from({ length: Math.min(parallel, queue.length) }, () => slot()))
  }

  /**
   * 读取文件并计算各分块的 MD5，再组合为文件级 MD5（Merkle 根）
   * @param file - 源文件
   * @param chunkSize - 分块大小
   * @param totalChunks - 总分块数
   * @param signal - 取消信号
   * @returns [fileMd5, chunkMd5s]
   */
  private async computeChunkMd5s(
    file: File,
    chunkSize: number,
    totalChunks: number,
    signal?: AbortSignal,
  ): Promise<[string, string[]]> {
    const chunkMd5s: string[] = []
    for (let i = 0; i < totalChunks; i++) {
      if (signal?.aborted) throw new DOMException('Aborted', 'AbortError')
      const start = i * chunkSize
      const end = Math.min(start + chunkSize, file.size)
      const data = await readFileSlice(file, start, end)
      chunkMd5s.push(this.md5ArrayBuffer(data))
    }
    const spark = new SparkMD5()
    for (const md5 of chunkMd5s) {
      spark.append(md5)
    }
    return [spark.end(), chunkMd5s]
  }

  /** 计算 ArrayBuffer 的 MD5 哈希 */
  private md5ArrayBuffer(data: ArrayBuffer): string {
    const spark = new SparkMD5.ArrayBuffer()
    spark.append(data)
    return spark.end()
  }

  /** 延迟指定毫秒数 */
  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * 轮询合并状态直到完成
   * @param taskId - 上传任务 ID
   * @param interval - 轮询间隔（毫秒）
   * @param maxAttempts - 最大轮询次数
   */
  private async pollMergeStatus(
    taskId: string,
    interval = 2000,
    maxAttempts = 3600,
  ): Promise<ChunkUploadResult> {
    for (let i = 0; i < maxAttempts; i++) {
      const result = await this.mergeStatus(taskId)

      switch (result.status) {
        case 'completed':
          return {
            bucketId: taskId,
            bucketUrl: result.storage_path!,
          }
        case 'failed': {
          throw new ClientError(result.error || 'merge failed', 500)
        }
        case 'merging':
        case 'uploading':
          await this.sleep(interval)
          continue
        default:
          throw new ClientError(`unknown merge status: ${result.status}`, 500)
      }
    }

    throw new ClientError('merge status polling timed out', 500)
  }
}

/**
 * 通过 FileReader 读取文件切片
 * @param file - 源文件
 * @param start - 起始字节偏移
 * @param end - 结束字节偏移（不含）
 */
function readFileSlice(file: File, start: number, end: number): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as ArrayBuffer)
    reader.onerror = () => reject(reader.error)
    reader.readAsArrayBuffer(file.slice(start, end))
  })
}
