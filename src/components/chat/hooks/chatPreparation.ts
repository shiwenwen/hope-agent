import type { ChatAttachment, Transport } from "@/lib/transport"
import type { ChatTurnStatus, StopChatResult } from "@/types/chat"

/** Composer drafts are isolated by materialized session or lazy project.
 * Project A's typed filesystem bindings must never become Project B's active
 * draft merely because neither session has been created yet. */
export function composerInputDraftKey(
  sessionId: string | null,
  draftProjectId: string | null,
): string {
  if (sessionId) return `session:${sessionId}`
  return draftProjectId ? `draft:project:${draftProjectId}` : "draft:plain"
}

export function isUnmaterializedComposerDraftKey(key: string): boolean {
  return key.startsWith("draft:")
}

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

/**
 * A preflight Stop is authoritative regardless of which client initiated it:
 * the backend guarantees that no user message was persisted. Preparation and
 * active-stream errors are ambiguous, so only the local request's Stop marker
 * may turn those into a draft rollback.
 */
export function shouldRollbackNonPersistedStoppedSend(
  requestWasUserStopped: boolean,
  preflightStopError: boolean,
  preparationCancelled: boolean,
  activeStreamError: boolean,
): boolean {
  return (
    preflightStopError || (requestWasUserStopped && (preparationCancelled || activeStreamError))
  )
}

/**
 * Decide whether a completed Stop leaves the UI with nothing to wait for.
 *
 * Issue #657: an exact Stop for a turn that is already durable-terminal (crash
 * recovery, a lost terminal event) settles nothing and arms no broadcast, and a
 * session-scoped Stop with no live turn only writes a durable pause receipt.
 * Neither can produce a `chat:stream_end` / `chat:turn_status`, so a busy-
 * looking session must reconcile itself against the authoritative snapshot
 * instead of waiting forever. The three "keep waiting" signals are exhaustive:
 * a pre-registration latch registers its turn shortly, a sealed completion is
 * already on its way to a terminal state, and an armed watchdog guarantees one.
 */
export function shouldReconcileAfterStop(
  result: Partial<StopChatResult> | null | undefined,
): boolean {
  // A backend too old to report these fields never promises a terminal event
  // either; the reconcile is idempotent and re-reads authoritative state, so
  // running it is the safe default.
  if (!result) return false
  return !result.latched && !result.completionSealed && !result.terminalEventPending
}

/** Mirrors `ChatTurnStatus::is_terminal` in `crates/ha-core/src/session/turns.rs`. */
export function isTerminalTurnStatus(
  status: ChatTurnStatus | null | undefined,
): status is "completed" | "interrupted" | "failed" {
  return status === "completed" || status === "interrupted" || status === "failed"
}

/**
 * Pick the status to publish when tearing down a session the backend reports as
 * idle.
 *
 * `SessionStreamState.status` is NOT terminal-filtered — only
 * `lastTerminalStatus` is. With no admitted turn it degrades to the latest
 * turn's raw status, which is still `running` for a row orphaned by a crash
 * (a Secondary such as `hope-agent server` never runs startup recovery) or
 * `cancelling` while a watchdog is mid-convergence. Publishing that verbatim
 * while clearing `loading` is worse than the stall it replaces:
 * `resolveWorkspaceTaskExecutionState` returns `running` regardless of
 * `loading`, so the status bar pins "running" forever with the Stop button
 * already gone. A session with nothing admitted is interrupted, whatever its
 * stale row says.
 */
export function settledTurnStatus(
  status: ChatTurnStatus | null | undefined,
  lastTerminalStatus: ChatTurnStatus | null | undefined,
): ChatTurnStatus {
  if (isTerminalTurnStatus(status)) return status
  if (isTerminalTurnStatus(lastTerminalStatus)) return lastTerminalStatus
  return "interrupted"
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

/**
 * Keep the exact turn identity visible to every listener handling the current
 * terminal event. Some consumers use a later listener to clear loading; an
 * immediate delete in an earlier listener would make that same event look
 * stale. The exact-id check prevents the deferred cleanup from deleting a
 * replacement turn that starts in the meantime.
 */
export function deferActiveTurnRelease(
  activeTurns: Map<string, string>,
  sessionId: string,
  turnId: string,
): void {
  queueMicrotask(() => {
    if (activeTurns.get(sessionId) === turnId) {
      activeTurns.delete(sessionId)
    }
  })
}
