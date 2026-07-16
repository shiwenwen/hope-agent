import type { FileUploadLease, FileUploadPurpose } from "@/lib/transport"

export const FILE_UPLOAD_CHUNK_BYTES = 4 * 1024 * 1024
const MAX_CHUNK_RETRIES = 3

export interface FileUploadOperations {
  start(input: {
    purpose: FileUploadPurpose
    fileName: string
    mimeType: string
    sizeBytes: number
  }): Promise<FileUploadLease>
  status(uploadId: string): Promise<FileUploadLease>
  chunk(uploadId: string, offset: number, data: Blob): Promise<FileUploadLease>
  complete(uploadId: string): Promise<FileUploadLease>
  discard(uploadId: string): Promise<void>
}

function abortError(): DOMException {
  return new DOMException("Upload cancelled", "AbortError")
}

async function waitForRetry(delayMs: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) throw abortError()
  await new Promise<void>((resolve, reject) => {
    const onAbort = () => {
      globalThis.clearTimeout(timer)
      reject(abortError())
    }
    const timer = globalThis.setTimeout(() => {
      signal?.removeEventListener("abort", onAbort)
      resolve()
    }, delayMs)
    signal?.addEventListener("abort", onAbort, { once: true })
  })
}

export async function uploadFileInChunks(
  file: File,
  purpose: FileUploadPurpose,
  operations: FileUploadOperations,
  progress?: (receivedBytes: number, sizeBytes: number) => void,
  signal?: AbortSignal,
): Promise<FileUploadLease> {
  if (signal?.aborted) throw abortError()
  const lease = await operations.start({
    purpose,
    fileName: file.name,
    mimeType: file.type || "application/octet-stream",
    sizeBytes: file.size,
  })
  let completed = false
  try {
    let offset = lease.receivedBytes
    progress?.(offset, file.size)
    while (offset < file.size) {
      if (signal?.aborted) throw abortError()
      const end = Math.min(offset + FILE_UPLOAD_CHUNK_BYTES, file.size)
      const data = file.slice(offset, end)
      let uploaded = false
      let lastError: unknown
      for (let attempt = 0; attempt <= MAX_CHUNK_RETRIES && !uploaded; attempt += 1) {
        if (signal?.aborted) throw abortError()
        try {
          const next = await operations.chunk(lease.uploadId, offset, data)
          offset = next.receivedBytes
          uploaded = true
        } catch (error) {
          lastError = error
          if (signal?.aborted) throw abortError()
          const status = await operations.status(lease.uploadId).catch(() => null)
          if (status && status.receivedBytes >= end) {
            offset = status.receivedBytes
            uploaded = true
            break
          }
          if (status && status.receivedBytes !== offset) {
            offset = status.receivedBytes
            uploaded = true
            break
          }
          if (attempt < MAX_CHUNK_RETRIES) {
            await waitForRetry(250 * 2 ** attempt, signal)
          }
        }
      }
      if (!uploaded) throw lastError instanceof Error ? lastError : new Error("Upload failed")
      progress?.(offset, file.size)
    }
    let result: FileUploadLease
    try {
      result = await operations.complete(lease.uploadId)
    } catch (error) {
      if (signal?.aborted) throw abortError()
      const status = await operations.status(lease.uploadId).catch(() => null)
      if (!status || status.state !== "complete") throw error
      result = status
    }
    completed = true
    progress?.(file.size, file.size)
    return result
  } catch (error) {
    if (!completed) await operations.discard(lease.uploadId).catch(() => undefined)
    throw error
  }
}
