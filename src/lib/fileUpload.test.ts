import { describe, expect, it } from "vitest"

import type { FileUploadLease } from "./transport"
import {
  FILE_UPLOAD_CHUNK_BYTES,
  uploadFileInChunks,
  type FileUploadOperations,
} from "./fileUpload"

function lease(overrides: Partial<FileUploadLease> = {}): FileUploadLease {
  return {
    uploadId: "1e196d90-7317-4bf5-b62d-b13fcf1ad9dd",
    purpose: "chat_attachment",
    fileName: "sample.bin",
    mimeType: "application/octet-stream",
    sizeBytes: 0,
    receivedBytes: 0,
    state: "uploading",
    expiresAt: "2099-01-01T00:00:00Z",
    ...overrides,
  }
}

describe("uploadFileInChunks", () => {
  it("sends fixed chunks in order and completes with progress", async () => {
    const file = new File([new Uint8Array(FILE_UPLOAD_CHUNK_BYTES + 3)], "sample.bin")
    const offsets: number[] = []
    const progress: number[] = []
    let received = 0
    const operations: FileUploadOperations = {
      start: async () => lease({ sizeBytes: file.size }),
      status: async () => lease({ sizeBytes: file.size, receivedBytes: received }),
      chunk: async (_uploadId, offset, data) => {
        offsets.push(offset)
        received += data.size
        return lease({ sizeBytes: file.size, receivedBytes: received })
      },
      complete: async () =>
        lease({ sizeBytes: file.size, receivedBytes: received, state: "complete" }),
      discard: async () => undefined,
    }

    const result = await uploadFileInChunks(file, "chat_attachment", operations, (current) =>
      progress.push(current),
    )

    expect(offsets).toEqual([0, FILE_UPLOAD_CHUNK_BYTES])
    expect(progress).toEqual([0, FILE_UPLOAD_CHUNK_BYTES, file.size, file.size])
    expect(result.state).toBe("complete")
  })

  it("uses status to recover when a successful chunk response is lost", async () => {
    const file = new File(["hello"], "sample.txt", { type: "text/plain" })
    let received = 0
    let chunkCalls = 0
    const operations: FileUploadOperations = {
      start: async () => lease({ sizeBytes: file.size }),
      status: async () => lease({ sizeBytes: file.size, receivedBytes: received }),
      chunk: async (_uploadId, _offset, data) => {
        chunkCalls += 1
        received += data.size
        throw new Error("response lost")
      },
      complete: async () =>
        lease({ sizeBytes: file.size, receivedBytes: received, state: "complete" }),
      discard: async () => undefined,
    }

    await expect(uploadFileInChunks(file, "chat_attachment", operations)).resolves.toMatchObject({
      state: "complete",
      receivedBytes: file.size,
    })
    expect(chunkCalls).toBe(1)
  })

  it("uses status to recover when the successful completion response is lost", async () => {
    const file = new File(["hello"], "sample.txt", { type: "text/plain" })
    let received = 0
    let complete = false
    let discarded = false
    const operations: FileUploadOperations = {
      start: async () => lease({ sizeBytes: file.size }),
      status: async () =>
        lease({
          sizeBytes: file.size,
          receivedBytes: received,
          state: complete ? "complete" : "uploading",
        }),
      chunk: async (_uploadId, _offset, data) => {
        received += data.size
        return lease({ sizeBytes: file.size, receivedBytes: received })
      },
      complete: async () => {
        complete = true
        throw new Error("response lost")
      },
      discard: async () => {
        discarded = true
      },
    }

    await expect(uploadFileInChunks(file, "chat_attachment", operations)).resolves.toMatchObject({
      state: "complete",
      receivedBytes: file.size,
    })
    expect(discarded).toBe(false)
  })

  it("discards the lease when finalization fails", async () => {
    const file = new File(["hello"], "sample.txt", { type: "text/plain" })
    let discarded = false
    const operations: FileUploadOperations = {
      start: async () => lease({ sizeBytes: file.size }),
      status: async () => lease({ sizeBytes: file.size, receivedBytes: file.size }),
      chunk: async () => lease({ sizeBytes: file.size, receivedBytes: file.size }),
      complete: async () => {
        throw new Error("limit lowered")
      },
      discard: async () => {
        discarded = true
      },
    }

    await expect(uploadFileInChunks(file, "chat_attachment", operations)).rejects.toThrow(
      "limit lowered",
    )
    expect(discarded).toBe(true)
  })

  it("discards an acquired lease when the upload is cancelled", async () => {
    const controller = new AbortController()
    const file = new File(["hello"], "sample.txt", { type: "text/plain" })
    let discarded = false
    const operations: FileUploadOperations = {
      start: async () => {
        controller.abort()
        return lease({ sizeBytes: file.size })
      },
      status: async () => lease({ sizeBytes: file.size }),
      chunk: async () => lease({ sizeBytes: file.size }),
      complete: async () => lease({ sizeBytes: file.size, state: "complete" }),
      discard: async () => {
        discarded = true
      },
    }

    await expect(
      uploadFileInChunks(file, "chat_attachment", operations, undefined, controller.signal),
    ).rejects.toMatchObject({ name: "AbortError" })
    expect(discarded).toBe(true)
  })
})
