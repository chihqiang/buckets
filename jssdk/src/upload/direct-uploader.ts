import { AuthClient } from '../auth-client'

export interface DirectUploadResult {
  object_id: string
  storage_path: string
  size: number
  md5: string
}

export interface DirectUploadOptions {
  signal?: AbortSignal
}

export class DirectUploader {
  constructor(private client: AuthClient) {}

  async upload(file: File, options?: DirectUploadOptions): Promise<DirectUploadResult> {
    if (options?.signal?.aborted) throw new DOMException('Aborted', 'AbortError')

    const formData = new FormData()
    formData.append('file', file)

    return this.client.uploadBinary<DirectUploadResult>('/api/v1/upload/direct', formData, {})
  }
}
