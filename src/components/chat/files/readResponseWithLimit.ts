/** Read a response without ever retaining more than `maxBytes` in memory. */
export async function readResponseArrayBufferWithLimit(
  response: Response,
  maxBytes: number,
): Promise<ArrayBuffer> {
  const declaredLength = Number(response.headers.get("content-length"))
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new Error(`response exceeds the ${maxBytes} byte preview limit`)
  }

  if (!response.body) {
    const buffer = await response.arrayBuffer()
    if (buffer.byteLength > maxBytes) {
      throw new Error(`response exceeds the ${maxBytes} byte preview limit`)
    }
    return buffer
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let received = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      received += value.byteLength
      if (received > maxBytes) {
        await reader.cancel()
        throw new Error(`response exceeds the ${maxBytes} byte preview limit`)
      }
      chunks.push(value)
    }
  } finally {
    reader.releaseLock()
  }

  const bytes = new Uint8Array(received)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return bytes.buffer
}
