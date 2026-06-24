import { HttpClient, HttpClientOptions } from './http-client'
import { AuthClient } from './auth-client'
import { AuthApi } from './api/auth-api'
import { ObjectsApi } from './api/objects-api'
import { UsersApi } from './api/users-api'
import { TusUploader } from './upload/tus-uploader'
import { ChunkUploader } from './upload/chunk-uploader'

export interface BucketsClientOptions extends HttpClientOptions {
  initialToken?: string
}

export class BucketsClient {
  readonly http: HttpClient
  readonly authClient: AuthClient

  readonly auth: AuthApi
  readonly objects: ObjectsApi
  readonly users: UsersApi
  readonly tus: TusUploader
  readonly chunk: ChunkUploader

  constructor(options: BucketsClientOptions) {
    this.http = new HttpClient(options)
    this.authClient = new AuthClient(this.http, options.initialToken)

    this.auth = new AuthApi(this.authClient)
    this.objects = new ObjectsApi(this.authClient)
    this.users = new UsersApi(this.authClient)
    this.tus = new TusUploader(this.authClient)
    this.chunk = new ChunkUploader(this.authClient)
  }

  setToken(token: string): void {
    this.authClient.setToken(token)
  }

  getToken(): string {
    return this.authClient.getToken()
  }
}
