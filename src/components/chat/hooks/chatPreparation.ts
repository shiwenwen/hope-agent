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

/**
 * Linearize local Stop against publishing the backend request. JavaScript
 * executes the membership check and marker write in one synchronous turn, so
 * Stop either wins here or observes backend ownership and calls `stop_chat`.
 */
export function beginChatBackendHandoff(
  requestId: string,
  stoppedRequestIds: ReadonlySet<string>,
  backendStartedRequestIds: Set<string>,
): void {
  if (stoppedRequestIds.has(requestId)) {
    throw new ChatPreparationCancelledError()
  }
  backendStartedRequestIds.add(requestId)
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

/**
 * Start best-effort cleanup for staged attachment leases. A stopped send must
 * release its composer state immediately even if the cleanup transport is
 * stalled; ordinary failures still wait so their leases are settled before
 * the error lifecycle completes.
 */
export async function discardChatAttachmentUploads(
  attachments: ChatAttachment[],
  transport: Transport,
  waitForCompletion: boolean,
): Promise<void> {
  const cleanup = Promise.allSettled(
    attachments
      .map((attachment) => attachment.upload_id)
      .filter((id): id is string => !!id)
      .map((id) => transport.discardChatAttachmentUpload(id)),
  )
  if (waitForCompletion) await cleanup
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

/**
 * Resolve the visible loading state when one session's local preparation ends.
 * `undefined` means the completed request belongs to a background session and
 * must not mutate the currently displayed session's loading indicator.
 */
export function loadingStateAfterPreparationRelease(
  requestSessionKey: string,
  currentSessionId: string | null,
  loadingSessionIds: ReadonlySet<string>,
): boolean | undefined {
  const currentSessionKey = currentSessionId ?? "__pending__"
  if (requestSessionKey !== currentSessionKey) return undefined
  return currentSessionId ? loadingSessionIds.has(currentSessionId) : false
}
