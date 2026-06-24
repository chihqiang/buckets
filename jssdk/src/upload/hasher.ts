import SparkMD5 from 'spark-md5'
import { readFileSlice } from './read-file-slice'

export function md5ArrayBuffer(data: ArrayBuffer): string {
  const spark = new SparkMD5.ArrayBuffer()
  spark.append(data)
  return spark.end()
}

export async function computeChunkMd5s(
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
    chunkMd5s.push(md5ArrayBuffer(data))
  }

  const spark = new SparkMD5()
  for (const md5 of chunkMd5s) {
    spark.append(md5)
  }
  return [spark.end(), chunkMd5s]
}
