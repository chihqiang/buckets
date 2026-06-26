export interface LoginResult {
  token: string
  refresh_token: string
  expires_in: number
  is_super_admin: boolean
}

export interface ObjectItem {
  id: number
  uuid: string
  name: string
  size: number
  md5: string
  content_type: string | null
  extension: string | null
  bucket: string
  storage_path: string | null
  upload_method: string | null
  image_width: number
  image_height: number
  image_type: string
  created_at: string
  updated_at: string
}

export interface User {
  id: number
  email: string
  created_at: string
  updated_at: string
}

export interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}
