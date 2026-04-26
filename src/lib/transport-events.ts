import { getTransport } from "@/lib/transport-provider"

export async function withEventListener<T>(
  eventName: string,
  handler: (payload: unknown) => void,
  fn: () => Promise<T>,
): Promise<T> {
  const off = getTransport().listen(eventName, handler)
  try {
    return await fn()
  } finally {
    off()
  }
}
