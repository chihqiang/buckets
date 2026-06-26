import { AuthClient } from '../auth-client'
import { ObjectItem, PaginatedResponse } from './types'

export class ObjectsApi {
  constructor(private client: AuthClient) {}

  async list(page = 1, pageSize = 20): Promise<PaginatedResponse<ObjectItem>> {
    const params = new URLSearchParams({ page: String(page), page_size: String(pageSize) })
    return this.client.request<PaginatedResponse<ObjectItem>>('GET', `/api/v1/objects?${params}`)
  }

  async get(id: number): Promise<ObjectItem> {
    return this.client.request<ObjectItem>('GET', `/api/v1/object/${id}`)
  }

  async delete(id: number): Promise<void> {
    await this.client.request<void>('DELETE', `/api/v1/objects/${id}`)
  }

  async deleteOwn(id: number): Promise<void> {
    await this.client.request<void>('DELETE', `/api/v1/object/${id}`)
  }

  async downloadBlob(id: number): Promise<Blob> {
    return this.client.requestBlob('GET', `/api/v1/object/${id}/download`)
  }
}
