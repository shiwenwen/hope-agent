import { describe, expect, test } from "vitest"

import { readResponseArrayBufferWithLimit } from "./readResponseWithLimit"

describe("readResponseArrayBufferWithLimit", () => {
  test("rejects a declared oversized response before reading its body", async () => {
    const response = new Response(new Uint8Array([1, 2, 3]), {
      headers: { "content-length": "99" },
    })

    await expect(readResponseArrayBufferWithLimit(response, 3)).rejects.toThrow("preview limit")
  })

  test("cancels an undeclared streaming response once it crosses the limit", async () => {
    let cancelled = false
    const response = new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(new Uint8Array([1, 2]))
          controller.enqueue(new Uint8Array([3, 4]))
        },
        cancel() {
          cancelled = true
        },
      }),
    )

    await expect(readResponseArrayBufferWithLimit(response, 3)).rejects.toThrow("preview limit")
    expect(cancelled).toBe(true)
  })

  test("combines chunks within the limit", async () => {
    const response = new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(new Uint8Array([1, 2]))
          controller.enqueue(new Uint8Array([3]))
          controller.close()
        },
      }),
    )

    const result = await readResponseArrayBufferWithLimit(response, 3)
    expect([...new Uint8Array(result)]).toEqual([1, 2, 3])
  })
})
