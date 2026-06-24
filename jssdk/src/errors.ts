export class ClientError extends Error {
  readonly status: number
  readonly traceId: string

  constructor(message: string, status: number, traceId = '-') {
    super(message)
    this.name = 'ClientError'
    this.status = status
    this.traceId = traceId
  }
}
