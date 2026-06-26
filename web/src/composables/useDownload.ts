import { reactive } from 'vue'
import { getApi } from '../stores/api'
import type { ObjectItem } from '@chihqiang/buckets'

interface DownloadState {
  status: 'idle' | 'downloading' | 'paused' | 'completed' | 'error'
  progress: number
  bytesReceived: number
  total: number
  error: string
}

interface InternalState extends DownloadState {
  controller: AbortController | null
  chunks: Uint8Array[]
  currentOffset: number
}

export function useDownload() {
  const map = reactive(new Map<number, InternalState>())

  function initState(id: number): InternalState {
    const s: InternalState = {
      status: 'idle',
      progress: 0,
      bytesReceived: 0,
      total: 0,
      error: '',
      controller: null,
      chunks: [],
      currentOffset: 0,
    }
    map.set(id, s)
    return s
  }

  function getState(id: number): DownloadState {
    const s = map.get(id)
    if (!s) return { status: 'idle', progress: 0, bytesReceived: 0, total: 0, error: '' }
    return { status: s.status, progress: s.progress, bytesReceived: s.bytesReceived, total: s.total, error: s.error }
  }

  async function start(f: ObjectItem) {
    let s = map.get(f.id)
    if (!s) s = initState(f.id)

    if (s.status === 'downloading') return

    const api = getApi()
    const controller = new AbortController()
    s.controller = controller
    s.status = 'downloading'
    s.error = ''

    try {
      const headers: Record<string, string> = {}
      if (s.currentOffset > 0) {
        headers['Range'] = `bytes=${s.currentOffset}-`
      }
      const res = await api.authClient.requestRaw('GET', `/api/v1/object/${f.id}/download`, {
        signal: controller.signal,
        headers,
      })

      const contentLength = res.headers.get('content-length')
      if (s.total === 0 && contentLength) {
        s.total = s.currentOffset + parseInt(contentLength, 10)
      } else if (s.total === 0) {
        s.total = 0
      }

      const reader = res.body!.getReader()
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        s.chunks.push(value)
        s.bytesReceived += value.length
        if (s.total > 0) {
          s.progress = Math.round((s.bytesReceived / s.total) * 100)
        }
      }

      s.currentOffset = s.bytesReceived
      s.status = 'completed'
      s.progress = 100

      const blob = new Blob(s.chunks as BlobPart[])
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = f.name
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)

      map.delete(f.id)
    } catch (e: any) {
      if (e instanceof DOMException && e.name === 'AbortError') {
        s.currentOffset = s.bytesReceived
        s.status = 'paused'
      } else {
        s.status = 'error'
        s.error = e.message || String(e)
      }
    }
  }

  function pause(id: number) {
    const s = map.get(id)
    if (s && s.controller) {
      s.controller.abort()
    }
  }

  return { getState, start, pause }
}
