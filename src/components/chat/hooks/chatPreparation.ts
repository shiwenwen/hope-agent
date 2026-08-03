import type { ChatAttachment, Transport } from "@/lib/transport"

export class ChatPreparationCancelledError extends Error {
  constructor() {
    super("Chat preparation cancelled by user")
    this.name = "ChatPreparationCancelledError"
  }
}

export function isChatPreparationCancelled(error: unknown): boolean {
  return error instanceof ChatPreparationCancelledError
}

export function awaitUnlessAborted<T>(
  promise: Promise<T>,
  signal?: AbortSignal,
  onLateResolve?: (value: T) => void | Promise<void>,
): Promise<T> {
  if (!signal) return promise
  return new Promise<T>((resolve, reject) => {
    let settled = false
    const onAbort = () => {
      if (settled) return
      settled = true
      signal.removeEventListener("abort", onAbort)
      reject(new ChatPreparationCancelledError())
    }
    promise.then(
      (value) => {
        if (settled) {
          if (signal.aborted && onLateResolve) {
            void Promise.resolve(onLateResolve(value)).catch(() => {})
          }
          return
        }
        settled = true
        signal.removeEventListener("abort", onAbort)
        resolve(value)
      },
      (error) => {
        if (settled) return
        settled = true
        signal.removeEventListener("abort", onAbort)
        reject(error)
      },
    )
    if (signal.aborted) onAbort()
    else signal.addEventListener("abort", onAbort, { once: true })
  })
}

export async function validateChatAttachmentCount(
  attachments: ChatAttachment[],
  transport: Transport,
  tooManyMessage: string,
  signal?: AbortSignal,
): Promise<void> {
  if (attachments.length <= 64) return
  const cleanup = Promise.allSettled(
    attachments
      .map((attachment) => attachment.upload_id)
      .filter((id): id is string => !!id)
      .map((id) => transport.discardChatAttachmentUpload(id)),
  )
  await awaitUnlessAborted(cleanup, signal)
  throw new Error(tooManyMessage)
}
