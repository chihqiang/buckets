export interface ChunkStsOptions {
  file_name: string
  file_size: number
  file_md5: string
  chunk_size?: number
}

export interface ChunkStsResult {
  task_id: string
  bucket_key: string
  session_signature: string
  session_timestamp: number
  session_salt: string
}

export interface ChunkPrecheckResult {
  exists: boolean
  bucket_id: string | null
  storage_path: string | null
  task_id: string | null
  uploaded_chunks: number[]
  chunk_size: number
}

export interface ChunkItemResult {
  chunk_index: number
  status: string
  md5: string
}

export interface ChunkStatusResult {
  task_id: string
  chunk_count: number
  uploaded_count: number
  missing_chunks: number[]
  is_complete: boolean
}

export interface ChunkMergeOptions {
  task_id: string
  file_name: string
  file_md5: string
  file_size: number
  content_type?: string
}

export interface ChunkMergeResult {
  task_id: string
  message: string
}

export interface ChunkMergeStatusResult {
  task_id: string
  status: string
  storage_path: string | null
  error?: string
}

export interface ChunkUploadProgress {
  totalChunks: number
  uploadedChunks: number
  percent: number
}

export interface ChunkUploadOptions {
  chunkSize?: number
  parallel?: number
  onProgress?: (progress: ChunkUploadProgress) => void
  signal?: AbortSignal
}

export interface ChunkUploadResult {
  bucketId: string
  bucketUrl: string
}

export interface ChunkSessionCredentials {
  session_signature: string
  session_timestamp: number
  session_salt: string
}

export interface TusUploadProgress {
  bytesUploaded: number
  totalBytes: number
  percent: number
}

export interface TusUploadOptions {
  chunkSize?: number
  deferLength?: boolean
  onProgress?: (progress: TusUploadProgress) => void
  signal?: AbortSignal
  metadata?: Record<string, string>
}

export interface TusUploadResult {
  objectId: string
  storagePath: string
}

export interface TusCapabilities {
  version: string
  extensions: string[]
  maxSize: number
}
