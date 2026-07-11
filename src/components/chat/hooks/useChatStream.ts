import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { ChatAttachment } from "@/lib/transport"
import type { KbDraftAttachment } from "@/types/knowledge"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import {
  getCachedConfig,
  loadNotificationConfig,
  isAgentNotifyEnabled,
  notify,
  notifyIfBackground,
} from "@/lib/notifications"
import type {
  Message,
  MessageAttachment,
  PendingFileQuote,
  PendingSendPreview,
  ActiveModel,
  AgentSummaryForSidebar,
  SandboxMode,
  SessionMeta,
  SessionMode,
  ChatTurnStatus,
  ChatTurnInterruptReason,
} from "@/types/chat"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import {
  createStreamDeltaBuffers,
  discardAllPendingStreamDeltas,
  discardPendingStreamDeltas,
  handleStreamEvent,
  streamCursorKey,
  streamIdFromEvent,
  streamIdFromPayload,
} from "./useStreamEventHandler"
import { useApprovals } from "./useApprovals"
import { generateClientId } from "@/components/chat/chatScrollKeys"
import { expandMentionsToAttachments } from "@/components/chat/file-mention/expandMentions"
import { expandPlanMentionsToAttachments } from "@/components/chat/plan-mention/expandPlanMentions"
import {
  getPastedTextFileMeta,
  PASTED_TEXT_ATTACHMENT_SOURCE,
} from "@/components/chat/input/pastedTextAttachment"
import { useNotificationListeners } from "./useNotificationListeners"
import type { SessionStreamState } from "./useChatStreamReattach"
import { modelOverrideFromManualSelection } from "../modelSelection"

const ACTIVE_STREAM_ERROR_CODE = "active_stream"
const CHAT_NOTIFICATION_PREVIEW_MAX_CHARS = 220

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === "string") return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

function isActiveStreamError(error: unknown): boolean {
  return errorText(error).includes(ACTIVE_STREAM_ERROR_CODE)
}

function normalizeNotificationText(text: string): string {
  return text.replace(/\s+/g, " ").trim()
}

function truncateNotificationText(text: string): string {
  if (text.length <= CHAT_NOTIFICATION_PREVIEW_MAX_CHARS) return text
  return `${text.slice(0, CHAT_NOTIFICATION_PREVIEW_MAX_CHARS - 3).trimEnd()}...`
}

function assistantNotificationText(message: Message): string {
  const blockText = message.contentBlocks
    ?.filter((block) => block.type === "text")
    .map((block) => block.content)
    .join("\n\n")
  return truncateNotificationText(normalizeNotificationText(blockText || message.content || ""))
}

function userNotificationText(message: Message): string {
  const blockText = message.contentBlocks
    ?.filter((block) => block.type === "text")
    .map((block) => block.content)
    .join("\n\n")
  return truncateNotificationText(normalizeNotificationText(blockText || message.content || ""))
}

function latestAssistantNotificationPreview(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (message.role !== "assistant") continue
    const text = assistantNotificationText(message)
    if (text) return text
  }
  return null
}

function latestUserNotificationPreview(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (message.role !== "user") continue
    const text = userNotificationText(message)
    if (text) return text
  }
  return null
}

function chatCompletionNotificationPayload(
  sessionTitle: string,
  messages: Message[],
  showChatContent: boolean,
  genericTitle: string,
  genericBody: string,
): { title: string; body: string } {
  if (!showChatContent) return { title: genericTitle, body: genericBody }
  const preview = latestAssistantNotificationPreview(messages)
  return { title: sessionTitle, body: preview || genericBody }
}

function optimisticAttachmentForFile(file: File): MessageAttachment {
  const mimeType = file.type || "application/octet-stream"
  return {
    name: file.name,
    mimeType,
    sizeBytes: file.size,
    kind: mimeType.toLowerCase().startsWith("image/") ? "image" : "file",
    ...(mimeType.toLowerCase().startsWith("image/")
      ? { previewUrl: URL.createObjectURL(file) }
      : {}),
  }
}

interface SendOptions {
  displayText?: string
  planMode?: string
  isPlanTrigger?: boolean
  /** Routed through the chat command into `attachments_meta.plan_comment`
   *  so the desktop GUI can render PlanCommentBubble with structured
   *  selection + comment fields. IM channels ignore this and use displayText. */
  planComment?: { selectedText: string; comment: string }
}

interface PendingSend {
  id: string
  createdAt: number
  text: string
  mode: "queue" | "force_insert"
  status: "queued" | "waiting_tool_boundary" | "inserted" | "fallback_after_reply"
  options?: SendOptions
  attachedFiles?: File[]
  quotes?: PendingFileQuote[]
}

interface QueueTurnUserMessageResult {
  queued: boolean
  requestId: string
  reason?: string
}

interface CancelQueuedTurnUserMessageResult {
  cancelled: boolean
}

interface InputDraft {
  input: string
  attachedFiles: File[]
}

function inputDraftKey(sessionId: string | null): string {
  return sessionId ? `session:${sessionId}` : "draft"
}

export interface UseChatStreamOptions {
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  currentAgentId: string
  agentName: string
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  /** Bound the messages array post-append. Returns `msgs` unchanged when
   *  under cap. Optional — QuickChat / Window paths don't participate in
   *  the main-app LRU/cap and can omit. */
  capMessagesForSession?: (sessionId: string, msgs: Message[]) => Message[]
  /** Bumps the LRU position of `sessionId`. Called on cache writes that
   *  don't otherwise route through `handleSwitchSession` (new-session
   *  rename in particular). */
  touchSessionCacheLru?: (sessionId: string) => void
  sessions: Pick<SessionMeta, "id" | "title" | "workingDir" | "permissionMode" | "sandboxMode">[]
  agents: AgentSummaryForSidebar[]
  /** Display-only compatibility input; never converted into a strict override. */
  activeModel?: ActiveModel | null
  /** Explicit picker intent. Display-only model restoration must never populate this ref. */
  manualModelOverrideRef?: React.MutableRefObject<ActiveModel | null>
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  /**
   * Per-session seq cursor shared with `useChatStreamReattach`. Primary-path
   * `onmessage` bumps it so redundant EventBus events are dropped.
   */
  lastSeqRef: React.MutableRefObject<Map<string, number>>
  /** Latest stream id that has ended for each session. Used to drop delayed
   *  primary frames that arrive after DB reconciliation. */
  endedStreamIdsRef: React.MutableRefObject<Map<string, string>>
  /** Current plan mode state, passed to backend chat() for reliable sync */
  planMode?: string
  /** Session-level temperature override (0.0–2.0). Overrides agent and global settings. */
  temperatureOverride?: number | null
  /** Session-level Think / reasoning effort. */
  reasoningEffort?: string | null
  /** New-chat preset; only applied when the backend auto-creates a session. */
  incognitoEnabled?: boolean
  /**
   * Draft working dir picked before the session was materialized. Sent to the
   * `chat` command only when no `sessionId` is set yet — the backend applies it
   * on the auto-create branch.
   */
  draftWorkingDir?: string | null
  /**
   * Project bound to a not-yet-materialized chat (lazy project session). Like
   * draftWorkingDir, it rides on the `chat` command payload (`projectId`) as a
   * send-time snapshot and the backend binds the auto-created session to it —
   * sent only when no `sessionId` is set yet.
   */
  draftProjectId?: string | null
  /**
   * KB attaches staged before the session existed (composer draft mode). Like
   * draftWorkingDir, they ride on the `chat` command payload (`kbAttachments`)
   * as a send-time snapshot and the backend applies them on the auto-create
   * branch via `apply_draft_attachments` — sent only when no `sessionId` is set
   * yet and not incognito.
   */
  draftKbAttachments?: KbDraftAttachment[]
  /**
   * Knowledge-space sidebar chat: the note open when the conversation started.
   * Sent only on the auto-create send so the backend promotes the new session
   * into a KB chat thread anchored to it (history / default-load key).
   */
  draftKbAnchorNote?: string | null
  /**
   * Tool-visibility scope forwarded to the `chat` command. The knowledge-space
   * sidebar passes `"knowledge"` to trim the injected tool set; omitted for the
   * main / quick chat (full tools).
   */
  toolScope?: "knowledge"
  /**
   * Per-turn extra attachments merged at send time, AFTER the visible composer
   * quotes/files. The knowledge panel uses this to inject the currently-open
   * note as an invisible `quote` so the assistant always sees "the document I'm
   * looking at" without the user manually quoting it. Returns `[]` for callers
   * that don't need it.
   */
  getExtraAttachments?: () => ChatAttachment[]
  /** Called after a persisted session sandbox-mode update succeeds. */
  onSandboxModeSynced?: (sessionId: string, mode: SandboxMode) => void
  /**
   * When true, this surface has `useChatStreamReattach` mounted and should let
   * ParentInjection deltas arrive through `chat:stream_delta` instead of the
   * legacy `parent_agent_stream` delta side channel.
   */
  parentInjectionDeltasViaChatStream?: boolean
}

export interface UseChatStreamReturn {
  input: string
  setInput: React.Dispatch<React.SetStateAction<string>>
  attachedFiles: File[]
  setAttachedFiles: React.Dispatch<React.SetStateAction<File[]>>
  pendingQuotes: PendingFileQuote[]
  setPendingQuotes: React.Dispatch<React.SetStateAction<PendingFileQuote[]>>
  pendingMessage: string | null
  setPendingMessage: React.Dispatch<React.SetStateAction<string | null>>
  pendingSends: PendingSendPreview[]
  editPendingSend: (id: string) => void
  discardPendingSend: (id: string) => void
  forceInsertPendingSend: (id: string) => Promise<void>
  cancelForceInsertPendingSend: (id: string) => Promise<void>
  approvalRequests: ApprovalRequest[]
  showCodexAuthExpired: boolean
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  permissionMode: SessionMode
  setPermissionMode: React.Dispatch<React.SetStateAction<SessionMode>>
  /** User-initiated permission-mode change (switcher / `/permission`). Marks the
   *  draft dirty so a new session's first send carries the chosen mode. */
  setPermissionModeByUser: React.Dispatch<React.SetStateAction<SessionMode>>
  sandboxMode: SandboxMode
  setSandboxMode: React.Dispatch<React.SetStateAction<SandboxMode>>
  /** User-initiated sandbox-mode change. Mirrors permission mode draft behavior. */
  setSandboxModeByUser: React.Dispatch<React.SetStateAction<SandboxMode>>
  handleSend: (directText?: string, options?: SendOptions) => Promise<void>
  handleStop: () => Promise<void>
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
  handleTurnStarted: (sessionId: string, turnId: string) => void
  handleTurnEnded: (
    sessionId: string,
    status?: ChatTurnStatus | null,
    interruptReason?: ChatTurnInterruptReason | null,
  ) => void
  executionStateBySession: Map<string, ChatTurnStatus>
}

export function useChatStream({
  messages,
  setMessages,
  currentSessionId,
  setCurrentSessionId,
  currentSessionIdRef,
  currentAgentId,
  agentName,
  loading,
  setLoading,
  loadingSessionsRef,
  setLoadingSessionIds,
  sessionCacheRef,
  capMessagesForSession,
  touchSessionCacheLru,
  sessions,
  agents,
  manualModelOverrideRef,
  reloadSessions,
  updateSessionMessages,
  lastSeqRef,
  endedStreamIdsRef,
  planMode,
  temperatureOverride,
  reasoningEffort,
  incognitoEnabled = false,
  draftWorkingDir = null,
  draftProjectId = null,
  draftKbAttachments = [],
  draftKbAnchorNote = null,
  toolScope,
  getExtraAttachments,
  onSandboxModeSynced,
  parentInjectionDeltasViaChatStream = false,
}: UseChatStreamOptions): UseChatStreamReturn {
  // Latest draft attaches, snapshotted into the startChat payload at send time
  // (mirrors how draftWorkingDir is baked into the create call) so a later
  // New Chat / session switch can't redirect them onto the wrong session.
  const draftKbAttachmentsRef = useRef(draftKbAttachments)
  // Latest draft project binding, snapshotted at send time so a mid-send project
  // switch can't materialize the session under the wrong project.
  const draftProjectIdRef = useRef(draftProjectId)
  draftProjectIdRef.current = draftProjectId
  draftKbAttachmentsRef.current = draftKbAttachments
  const { t } = useTranslation()
  const [input, setInputState] = useState("")
  const [attachedFiles, setAttachedFilesState] = useState<File[]>([])
  const [pendingQuotes, setPendingQuotes] = useState<PendingFileQuote[]>([])
  const inputRef = useRef(input)
  const attachedFilesRef = useRef(attachedFiles)
  const inputDraftsRef = useRef<Map<string, InputDraft>>(new Map())
  const activeInputDraftKeyRef = useRef(inputDraftKey(currentSessionId))

  const saveInputDraft = useCallback((key: string, draft: InputDraft) => {
    if (!draft.input && draft.attachedFiles.length === 0) {
      inputDraftsRef.current.delete(key)
      return
    }
    inputDraftsRef.current.set(key, draft)
  }, [])

  const setInput = useCallback<React.Dispatch<React.SetStateAction<string>>>(
    (value) => {
      setInputState((prev) => {
        const next = typeof value === "function" ? (value as (p: string) => string)(prev) : value
        inputRef.current = next
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: next,
          attachedFiles: attachedFilesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  const setAttachedFiles = useCallback<React.Dispatch<React.SetStateAction<File[]>>>(
    (value) => {
      setAttachedFilesState((prev) => {
        const next = typeof value === "function" ? (value as (p: File[]) => File[])(prev) : value
        attachedFilesRef.current = next
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: inputRef.current,
          attachedFiles: next,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  useLayoutEffect(() => {
    const nextKey = inputDraftKey(currentSessionId)
    const previousKey = activeInputDraftKeyRef.current
    if (previousKey === nextKey) return

    saveInputDraft(previousKey, {
      input: inputRef.current,
      attachedFiles: attachedFilesRef.current,
    })

    activeInputDraftKeyRef.current = nextKey
    const nextDraft = inputDraftsRef.current.get(nextKey) ??
      (previousKey === "draft" ? inputDraftsRef.current.get(previousKey) : undefined) ?? {
        input: "",
        attachedFiles: [],
      }
    if (previousKey === "draft" && !inputDraftsRef.current.has(nextKey)) {
      saveInputDraft(nextKey, nextDraft)
      inputDraftsRef.current.delete(previousKey)
    }
    inputRef.current = nextDraft.input
    attachedFilesRef.current = nextDraft.attachedFiles
    setInputState(nextDraft.input)
    setAttachedFilesState(nextDraft.attachedFiles)
  }, [currentSessionId, saveInputDraft])

  // Pending sends queued while a response is streaming. Stores the LLM-bound
  // `text` plus the original `options` (displayText / planMode / isPlanTrigger)
  // so replay preserves metadata. User-typed sends can additionally request
  // insertion at the next safe tool boundary.
  const [pendingSendsState, setPendingSendsState] = useState<PendingSend[]>([])
  const pendingSendsRef = useRef<PendingSend[]>([])
  const updatePendingSends = useCallback((updater: React.SetStateAction<PendingSend[]>) => {
    setPendingSendsState((prev) => {
      const next =
        typeof updater === "function"
          ? (updater as (p: PendingSend[]) => PendingSend[])(prev)
          : updater
      pendingSendsRef.current = next
      return next
    })
  }, [])
  const pendingDisplayText = useCallback(
    (pending: PendingSend): string =>
      pending.options?.displayText?.trim() ||
      pending.text ||
      (pending.attachedFiles?.length ? t("chat.attachPhotosAndFiles") : ""),
    [t],
  )
  const pendingInputText = useCallback(
    (pending: PendingSend): string => pending.options?.displayText?.trim() || pending.text,
    [],
  )
  const canForceInsertPending = useCallback(
    (pending: PendingSend): boolean =>
      !pending.options && pending.status === "queued" && pending.mode !== "force_insert",
    [],
  )
  // External views: keep the original `pendingMessage: string | null` API for
  // ChatScreen / QuickChat, derived from the first queued item.
  const pendingMessage = pendingSendsState[0] ? pendingDisplayText(pendingSendsState[0]) : null
  const pendingSends: PendingSendPreview[] = pendingSendsState.map((pending) => ({
    id: pending.id,
    text: pendingDisplayText(pending),
    mode: pending.mode,
    status: pending.status,
    canForceInsert: canForceInsertPending(pending),
    attachmentCount: pending.attachedFiles?.length ?? 0,
    quoteCount: pending.quotes?.length ?? 0,
  }))
  const setPendingMessage = useCallback<React.Dispatch<React.SetStateAction<string | null>>>(
    (value) => {
      updatePendingSends((prev) => {
        const first = prev[0]
        const next =
          typeof value === "function"
            ? (value as (p: string | null) => string | null)(
                first ? pendingDisplayText(first) : null,
              )
            : value
        if (next === null) return prev.slice(1)
        return [
          {
            id: generateClientId(),
            createdAt: Date.now(),
            text: next,
            mode: "queue",
            status: "queued",
          },
        ]
      })
    },
    [pendingDisplayText, updatePendingSends],
  )
  const [showCodexAuthExpired, setShowCodexAuthExpired] = useState(false)
  const [permissionMode, setPermissionModeState] = useState<SessionMode>("default")
  const permissionModeRef = useRef<SessionMode>("default")
  const [sandboxMode, setSandboxModeState] = useState<SandboxMode>("off")
  const sandboxModeRef = useRef<SandboxMode>("off")
  // Whether the user explicitly changed the permission mode in the current draft
  // (via the switcher / `/permission`), vs. it being seeded from the agent
  // default or set programmatically (restore / events). For a NEW session we send
  // `permissionMode` only when this is true; otherwise we omit it so the backend's
  // create-time agent default (create_session_full) stays authoritative. Set only
  // through `setPermissionModeByUser`; reset on each fresh draft. Independent of
  // seeding timing, so a user override made (or a config fetch that fails) before
  // seeding settles is still honored.
  const permissionModeDirtyRef = useRef(false)
  const sandboxModeDirtyRef = useRef(false)
  const restoredModesRef = useRef<{
    sessionId: string
    permissionMode: SessionMode
    sandboxMode: SandboxMode
  } | null>(null)
  const lastModeSessionIdRef = useRef<string | null | undefined>(undefined)
  const [executionStateBySession, setExecutionStateBySession] = useState<
    Map<string, ChatTurnStatus>
  >(() => new Map())
  const activeTurnBySessionRef = useRef<Map<string, string>>(new Map())
  const lastTurnStatusBySessionRef = useRef<
    Map<string, { status: ChatTurnStatus; interruptReason?: ChatTurnInterruptReason | null }>
  >(new Map())

  // Persist the new mode to the session row whenever the title-bar switcher
  // changes it. Backend re-reads the column at the start of each tool round,
  // so in-flight loops pick up the change without a separate global snapshot.
  // Without a session id the choice is local-only until the first send.
  const setPermissionMode = useCallback<React.Dispatch<React.SetStateAction<SessionMode>>>(
    (value) => {
      setPermissionModeState((prev) => {
        const next =
          typeof value === "function" ? (value as (p: SessionMode) => SessionMode)(prev) : value
        if (next !== prev) {
          const sid = currentSessionIdRef.current
          if (sid) {
            getTransport()
              .call("set_permission_mode", { sessionId: sid, mode: next })
              .catch((e) => {
                logger.error(
                  "chat",
                  "setPermissionMode",
                  "Failed to sync session permission mode",
                  e,
                )
              })
          }
        }
        return next
      })
    },
    [currentSessionIdRef],
  )

  // User-initiated permission-mode change (switcher / `/permission`). Marks the
  // draft "dirty" so a new session's first send carries the chosen mode, then
  // delegates to the persist/state logic. Programmatic callers (seeding, restore,
  // backend events) must use `setPermissionMode` so they don't mark the draft.
  const setPermissionModeByUser = useCallback<React.Dispatch<React.SetStateAction<SessionMode>>>(
    (value) => {
      permissionModeDirtyRef.current = true
      setPermissionMode(value)
    },
    [setPermissionMode],
  )

  const setSandboxMode = useCallback<React.Dispatch<React.SetStateAction<SandboxMode>>>(
    (value) => {
      setSandboxModeState((prev) => {
        const next =
          typeof value === "function" ? (value as (p: SandboxMode) => SandboxMode)(prev) : value
        if (next !== prev) {
          const sid = currentSessionIdRef.current
          if (sid) {
            getTransport()
              .call("set_sandbox_mode", { sessionId: sid, mode: next })
              .then(() => onSandboxModeSynced?.(sid, next))
              .catch((e) => {
                logger.error("chat", "setSandboxMode", "Failed to sync session sandbox mode", e)
              })
          }
        }
        return next
      })
    },
    [currentSessionIdRef, onSandboxModeSynced],
  )

  const setSandboxModeByUser = useCallback<React.Dispatch<React.SetStateAction<SandboxMode>>>(
    (value) => {
      sandboxModeDirtyRef.current = true
      setSandboxMode(value)
    },
    [setSandboxMode],
  )

  // Auto-send pending messages setting
  const autoSendPendingRef = useRef(true)
  const autoSendRef = useRef(false)
  // Holds a programmatic queued send (Plan Mode approve, slash-skill expansion)
  // so the auto-send effect can replay it with the original options instead of
  // rerouting through the input box. User-typed drafts go via `setInput` and
  // leave this ref null.
  const queuedReplayRef = useRef<PendingSend | null>(null)

  // Delta batch buffer
  const deltaBuffersRef = useRef(createStreamDeltaBuffers())

  useEffect(() => {
    const unlisten = getTransport().listen("chat:stream_end", (raw) => {
      const payload = raw as {
        sessionId?: string
        turnId?: string | null
        status?: ChatTurnStatus | null
        interruptReason?: ChatTurnInterruptReason | null
      } | null
      const sid = payload?.sessionId
      if (!sid) return
      if (payload?.turnId) activeTurnBySessionRef.current.delete(sid)
      if (payload?.status) {
        lastTurnStatusBySessionRef.current.set(sid, {
          status: payload.status,
          interruptReason: payload.interruptReason ?? null,
        })
        setExecutionStateBySession((prev) => new Map(prev).set(sid, payload.status!))
      }
      const streamId = streamIdFromPayload(raw)
      if (streamId) endedStreamIdsRef.current.set(sid, streamId)
      discardPendingStreamDeltas(sid, deltaBuffersRef)
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(deltaBuffersRef)
    }
  }, [endedStreamIdsRef])

  useEffect(() => {
    const unlisten = getTransport().listen("chat:turn_status", (raw) => {
      const payload = raw as {
        sessionId?: string
        turnId?: string | null
        status?: ChatTurnStatus | null
        interruptReason?: ChatTurnInterruptReason | null
      } | null
      const sid = payload?.sessionId
      if (!sid || !payload?.status) return
      if (payload.turnId) activeTurnBySessionRef.current.set(sid, payload.turnId)
      lastTurnStatusBySessionRef.current.set(sid, {
        status: payload.status,
        interruptReason: payload.interruptReason ?? null,
      })
      setExecutionStateBySession((prev) => new Map(prev).set(sid, payload.status!))
    })
    return unlisten
  }, [])

  // Compose sub-hooks
  const { approvalRequests, handleApprovalResponse } = useApprovals(currentSessionId)

  useNotificationListeners({
    currentSessionIdRef,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
    consumeParentStreamDeltas: !parentInjectionDeltasViaChatStream,
  })

  // Keep refs in sync
  useEffect(() => {
    pendingSendsRef.current = pendingSendsState
  }, [pendingSendsState])
  useEffect(() => {
    permissionModeRef.current = permissionMode
  }, [permissionMode])
  useEffect(() => {
    sandboxModeRef.current = sandboxMode
  }, [sandboxMode])

  // Restore per-session modes from SessionMeta for every surface that uses
  // this hook (main chat, quick chat, knowledge chat). Programmatic restore is
  // local-only: persisting happens only from the explicit user setters.
  useEffect(() => {
    const sid = currentSessionId
    if (!sid) {
      restoredModesRef.current = null
      if (messages.length === 0 && lastModeSessionIdRef.current !== null) {
        setPermissionModeState("default")
        permissionModeRef.current = "default"
        setSandboxModeState("off")
        sandboxModeRef.current = "off"
      }
      lastModeSessionIdRef.current = null
      return
    }

    const meta = sessions.find((session) => session.id === sid)
    if (!meta) return

    const nextPermissionMode: SessionMode = meta.permissionMode ?? "default"
    const nextSandboxMode: SandboxMode = meta.sandboxMode ?? "off"
    const restored = restoredModesRef.current
    if (
      restored?.sessionId === sid &&
      restored.permissionMode === nextPermissionMode &&
      restored.sandboxMode === nextSandboxMode
    ) {
      return
    }

    restoredModesRef.current = {
      sessionId: sid,
      permissionMode: nextPermissionMode,
      sandboxMode: nextSandboxMode,
    }
    lastModeSessionIdRef.current = sid
    setPermissionModeState(nextPermissionMode)
    permissionModeRef.current = nextPermissionMode
    setSandboxModeState(nextSandboxMode)
    sandboxModeRef.current = nextSandboxMode
  }, [currentSessionId, messages.length, sessions])

  // Seed `permissionMode` / `sandboxMode` from the agent defaults
  // whenever the user is sitting on a fresh chat (no session row yet, no
  // messages). Once the first message lands the session row owns the mode
  // and the title-bar switcher updates the row directly.
  //
  // Skipping when there is already a session id keeps the user's manual
  // choice intact across navigation — only "new chat" or agent swap re-seeds.
  useEffect(() => {
    if (currentSessionId || messages.length > 0 || !currentAgentId) return
    // Fresh draft (or agent swap): the user hasn't chosen modes yet.
    permissionModeDirtyRef.current = false
    sandboxModeDirtyRef.current = false
    let cancelled = false
    void (async () => {
      try {
        const config = await getTransport().call<{
          capabilities?: {
            sandbox?: boolean
            defaultSessionPermissionMode?: SessionMode | null
            defaultSandboxMode?: SandboxMode | null
          }
        }>("get_agent_config", { id: currentAgentId })
        // Don't clobber a mode the user changed while the fetch was in flight.
        if (!cancelled && !permissionModeDirtyRef.current) {
          const fallback =
            (config?.capabilities?.defaultSessionPermissionMode as SessionMode | undefined) ??
            "default"
          setPermissionModeState(fallback)
        }
        if (!cancelled && !sandboxModeDirtyRef.current) {
          const fallback =
            (config?.capabilities?.defaultSandboxMode as SandboxMode | undefined) ??
            (config?.capabilities?.sandbox ? "standard" : "off")
          setSandboxModeState(fallback)
        }
      } catch (e) {
        logger.error(
          "chat",
          "useChatStream",
          "Failed to seed session modes from agent capabilities",
          e,
        )
      }
    })()
    return () => {
      cancelled = true
    }
  }, [currentAgentId, currentSessionId, messages.length])

  // Load config on mount
  useEffect(() => {
    getTransport()
      .call<{ autoSendPending?: boolean }>("get_user_config")
      .then((cfg) => {
        autoSendPendingRef.current = cfg.autoSendPending !== false
      })
      .catch(() => {})
    loadNotificationConfig().catch(() => {})
  }, [])

  async function handleStop() {
    const sid = currentSessionIdRef.current ?? currentSessionId ?? null
    if (!sid) {
      const active = Array.from(activeTurnBySessionRef.current.entries()).at(-1)
      if (!active) return
      const [activeSid, activeTurnId] = active
      setExecutionStateBySession((prev) => new Map(prev).set(activeSid, "cancelling"))
      try {
        await getTransport().call("stop_chat", {
          sessionId: activeSid,
          turnId: activeTurnId,
        })
      } catch (e) {
        logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
      }
      return
    }
    const activeTurnId = activeTurnBySessionRef.current.get(sid) ?? null
    setExecutionStateBySession((prev) => new Map(prev).set(sid, "cancelling"))
    try {
      await getTransport().call("stop_chat", {
        sessionId: sid,
        turnId: activeTurnId,
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
    }
  }

  const handleTurnStarted = useCallback((sessionId: string, turnId: string) => {
    activeTurnBySessionRef.current.set(sessionId, turnId)
    lastTurnStatusBySessionRef.current.set(sessionId, { status: "running" })
    setExecutionStateBySession((prev) => new Map(prev).set(sessionId, "running"))
  }, [])

  const handleTurnEnded = useCallback(
    (
      sessionId: string,
      status?: ChatTurnStatus | null,
      interruptReason?: ChatTurnInterruptReason | null,
    ) => {
      activeTurnBySessionRef.current.delete(sessionId)
      if (status) {
        lastTurnStatusBySessionRef.current.set(sessionId, {
          status,
          interruptReason: interruptReason ?? null,
        })
        setExecutionStateBySession((prev) => new Map(prev).set(sessionId, status))
      }
    },
    [],
  )

  const quoteLineLabel = useCallback(
    (q: PendingFileQuote) =>
      q.startLine === q.endLine ? `${q.startLine}` : `${q.startLine}-${q.endLine}`,
    [],
  )

  const buildChatAttachments = useCallback(
    async (
      text: string,
      filesToSend: File[],
      quotesToSend: PendingFileQuote[],
      targetSessionId: string | null,
    ): Promise<ChatAttachment[]> => {
      const attachments: ChatAttachment[] = []

      const sessionWorkingDir = sessions.find((s) => s.id === targetSessionId)?.workingDir ?? null
      const resolvedWorkingDir = targetSessionId ? sessionWorkingDir : draftWorkingDir
      const mentionAttachments = expandMentionsToAttachments(text, resolvedWorkingDir ?? null)
      for (const m of mentionAttachments) {
        attachments.push(m)
      }

      const planAttachments = await expandPlanMentionsToAttachments(text)
      for (const p of planAttachments) {
        attachments.push(p)
      }

      for (const file of filesToSend) {
        try {
          const mimeType = file.type || "application/octet-stream"
          const source = getPastedTextFileMeta(file) ? PASTED_TEXT_ATTACHMENT_SOURCE : "upload"
          const arrayBuffer = await file.arrayBuffer()

          if (mimeType.startsWith("image/")) {
            const bytes = new Uint8Array(arrayBuffer)
            let binary = ""
            const chunkSize = 8192
            for (let i = 0; i < bytes.length; i += chunkSize) {
              binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize))
            }
            attachments.push({
              name: file.name,
              mime_type: mimeType,
              source,
              data: btoa(binary),
            })
          } else {
            const data = getTransport().prepareFileData(arrayBuffer, mimeType)
            const filePath = await getTransport().call<string>("save_attachment", {
              sessionId: targetSessionId,
              fileName: file.name,
              mimeType,
              data,
            })
            attachments.push({
              name: file.name,
              mime_type: mimeType,
              source,
              file_path: filePath,
            })
          }
        } catch (err) {
          logger.error("ui", "ChatScreen::attachment", "Failed to process attachment", {
            fileName: file.name,
            error: err,
          })
        }
      }

      for (const q of quotesToSend) {
        attachments.push({
          name: q.name,
          mime_type: "text/plain",
          source: "quote",
          data: q.content,
          file_path: q.path,
          quote_lines: quoteLineLabel(q),
        })
      }

      return attachments
    },
    [draftWorkingDir, quoteLineLabel, sessions],
  )

  /**
   * Send a message. If `directText` is provided, use it directly instead of the input box.
   * This avoids flashing text in the input (used by Plan Mode approve).
   */
  async function handleSend(directText?: string, options?: SendOptions) {
    const rawText = directText ?? input
    const hasAttachedFiles = !directText && attachedFiles.length > 0
    const hasQuotes = !directText && pendingQuotes.length > 0
    if (!rawText.trim() && !hasAttachedFiles && !hasQuotes) return

    // If currently loading, queue the message as pending. Capture the
    // LLM-bound text, the original options, and any staged files/quotes so the
    // replay below resends with identical content + metadata (Plan Mode
    // triggers carry `isPlanTrigger`, slash-skill expansions carry
    // `displayText`, etc.).
    if (loading) {
      const queuedFiles = directText ? [] : [...attachedFiles]
      const queuedQuotes = directText ? [] : [...pendingQuotes]
      updatePendingSends((prev) => [
        ...prev,
        {
          id: generateClientId(),
          createdAt: Date.now(),
          text: rawText.trim(),
          mode: "queue",
          status: "queued",
          options,
          ...(queuedFiles.length > 0 && { attachedFiles: queuedFiles }),
          ...(queuedQuotes.length > 0 && { quotes: queuedQuotes }),
        },
      ])
      if (!directText) {
        setInput("")
        setAttachedFiles([])
        setPendingQuotes([])
      }
      return
    }

    const text = rawText.trim()
    // `text` goes to the LLM; `displayed` is the user bubble. Slash-skill passThrough
    // uses this split so the UI shows "/drawio ..." while the LLM receives the expansion.
    const filesToSend = directText ? [] : [...attachedFiles]
    const quotesToSend = directText ? [] : [...pendingQuotes]
    const displayed = options?.displayText?.trim() || text
    const optimisticQuoteAttachments: MessageAttachment[] = quotesToSend.map((q) => ({
      name: q.name,
      mimeType: "text/plain",
      sizeBytes: 0,
      kind: "quote",
      quotePath: q.path,
      quoteLines: quoteLineLabel(q),
      quoteContent: q.content,
    }))
    const optimisticAttachments = [
      ...filesToSend.map(optimisticAttachmentForFile),
      ...optimisticQuoteAttachments,
    ]
    setInput("")
    setAttachedFiles([])
    setPendingQuotes([])
    const now = new Date().toISOString()
    // Both placeholders get a `_clientId` up front so `mergeMessagesByDbId`
    // can transfer them to the DB-finalized rows after stream_end. Without
    // the user-side id, `getLatestUserTurnKey` would flip from
    // `ts:user:<iso>` to `db:<N>` at finalize, fooling the forceFollow
    // effect into a phantom "new user message" scroll-to-top.
    const optimisticUserClientId = generateClientId()
    const assistantPlaceholderClientId = generateClientId()
    const optimisticUserMessage = {
      role: "user" as const,
      content: displayed,
      timestamp: now,
      _clientId: optimisticUserClientId,
      ...(optimisticAttachments.length > 0 && { attachments: optimisticAttachments }),
      ...(options?.isPlanTrigger && { isPlanTrigger: true }),
      ...(options?.planComment && { planComment: options.planComment }),
    }
    const sidForCap = currentSessionIdRef.current
    setMessages((prev) => {
      const next = [...prev, optimisticUserMessage]
      return sidForCap && capMessagesForSession ? capMessagesForSession(sidForCap, next) : next
    })
    setLoading(true)

    const attachments = await buildChatAttachments(
      text,
      filesToSend,
      quotesToSend,
      currentSessionId,
    )
    // Per-turn invisible context (knowledge panel: the currently-open note).
    if (getExtraAttachments) {
      for (const extra of getExtraAttachments()) attachments.push(extra)
    }

    // Empty assistant placeholder we'll stream into. `_clientId` was generated
    // alongside the user-side one above and survives the placeholder→DB
    // transition via `mergeMessagesByDbId`; see `messageStableId` for how the
    // row key consumes it.
    const assistantPlaceholderTimestamp = new Date().toISOString()
    setMessages((prev) => {
      const next: Message[] = [
        ...prev,
        {
          role: "assistant",
          content: "",
          timestamp: assistantPlaceholderTimestamp,
          _clientId: assistantPlaceholderClientId,
        },
      ]
      return sidForCap && capMessagesForSession ? capMessagesForSession(sidForCap, next) : next
    })

    let targetSessionId = currentSessionId
    let chatResolved = false
    let keepExistingStreamLoading = false

    try {
      const targetSid = () => targetSessionId || "__pending__"

      const handleSessionCreated = (event: Record<string, unknown>): boolean => {
        if (
          event.type !== "session_created" ||
          typeof event.session_id !== "string" ||
          !event.session_id
        ) {
          return false
        }

        targetSessionId = event.session_id
        // Bridge the ref lag: `setCurrentSessionId` below only updates
        // `currentSessionIdRef` after React commits, but the user is already on
        // this freshly-materialized session. Set it eagerly so the mark-as-read
        // guard at turn end compares against the right session even on a very
        // fast first turn — and still flips away correctly if the user navigates
        // elsewhere (handleSwitchSession / the sync effect update the ref then).
        currentSessionIdRef.current = event.session_id
        const current = sessionCacheRef.current.get("__pending__")
        if (current) {
          sessionCacheRef.current.delete("__pending__")
          sessionCacheRef.current.set(event.session_id, current)
        }
        // Promote the freshly-created session into the LRU. Without this,
        // a new chat written via this rename path would skip the LRU
        // bookkeeping done by `handleSwitchSession` and could be evicted
        // before the user even sees the first response.
        touchSessionCacheLru?.(event.session_id)
        loadingSessionsRef.current.add(event.session_id)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        setCurrentSessionId(event.session_id)
        reloadSessions()
        return true
      }

      const handleTurnStartedEvent = (event: Record<string, unknown>): boolean => {
        if (
          event.type !== "turn_started" ||
          typeof event.session_id !== "string" ||
          typeof event.turn_id !== "string"
        ) {
          return false
        }
        handleTurnStarted(event.session_id, event.turn_id)
        return true
      }

      const shouldDropStreamEvent = (event: Record<string, unknown>, sid: string): boolean => {
        const streamId = streamIdFromEvent(event)
        if (streamId && endedStreamIdsRef.current.get(sid) === streamId) return true

        // Primary path bumps the seq cursor so identical events arriving
        // later via the EventBus reattach listener are dropped.
        const seqRaw = event._oc_seq
        if (typeof seqRaw === "number" && sid !== "__pending__") {
          const cursorKey = streamCursorKey(sid, streamId)
          const prev = lastSeqRef.current.get(cursorKey) ?? 0
          if (seqRaw <= prev) return true
          lastSeqRef.current.set(cursorKey, seqRaw)
        }
        return false
      }

      const dispatchStreamEvent = (event: Record<string, unknown>) => {
        if (handleSessionCreated(event)) return
        if (handleTurnStartedEvent(event)) return

        const sid = targetSid()
        if (shouldDropStreamEvent(event, sid)) return

        if (event.type === "queued_user_message_inserted") {
          const requestId = typeof event.request_id === "string" ? event.request_id : null
          if (requestId) {
            updatePendingSends((prev) =>
              prev.map((item) =>
                item.id === requestId
                  ? { ...item, mode: "force_insert", status: "inserted" }
                  : item,
              ),
            )
          }
        } else if (event.type === "queued_user_message_blocked") {
          const requestId = typeof event.request_id === "string" ? event.request_id : null
          if (requestId) {
            updatePendingSends((prev) => prev.filter((item) => item.id !== requestId))
          }
        }

        handleStreamEvent(event, sid, {
          updateSessionMessages,
          deltaBuffersRef,
          setShowCodexAuthExpired,
        })
      }

      const appendRawStreamText = (raw: string) => {
        const sid = targetSid()
        updateSessionMessages(sid, (prev) => {
          const updated = [...prev]
          const last = updated[updated.length - 1]
          if (last && last.role === "assistant") {
            updated[updated.length - 1] = {
              ...last,
              content: last.content + raw,
            }
          }
          return updated
        })
      }

      const onEvent = (raw: string) => {
        try {
          dispatchStreamEvent(JSON.parse(raw) as Record<string, unknown>)
        } catch {
          appendRawStreamText(raw)
        }
      }

      // Track loading state for this session. The cache write must mirror
      // what `setMessages` produced — without re-capping here, the first
      // streaming frame's `updateSessionMessages` would read the
      // uncapped array back and `setMessages` it, undoing the cap on
      // every send.
      const freshMessages: Message[] = [
        ...messages,
        optimisticUserMessage,
        {
          role: "assistant" as const,
          content: "",
          timestamp: assistantPlaceholderTimestamp,
          _clientId: assistantPlaceholderClientId,
        },
      ]
      const cappedFreshMessages =
        targetSessionId && capMessagesForSession
          ? capMessagesForSession(targetSessionId, freshMessages)
          : freshMessages
      if (targetSessionId) {
        loadingSessionsRef.current.add(targetSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        sessionCacheRef.current.set(targetSessionId, cappedFreshMessages)
        touchSessionCacheLru?.(targetSessionId)
      } else {
        sessionCacheRef.current.set("__pending__", cappedFreshMessages)
      }

      const modelOverride = modelOverrideFromManualSelection(manualModelOverrideRef?.current)
      const effectivePlanMode = options?.planMode ?? planMode
      await getTransport().startChat(
        {
          message: text,
          attachments,
          sessionId: currentSessionId,
          incognito: currentSessionId ? undefined : incognitoEnabled,
          sessionDefaults: currentSessionId
            ? undefined
            : {
                model: modelOverride,
                temperature: temperatureOverride ?? undefined,
                reasoningEffort: reasoningEffort ?? undefined,
              },
          agentId: currentAgentId,
          // Existing session: always send (the title-bar switcher persisted it).
          // New session: only send when the user explicitly changed it — otherwise
          // omit so the backend's create-time agent default (create_session_full)
          // wins instead of a seeded/stale value sent before seeding settled.
          permissionMode:
            currentSessionId || permissionModeDirtyRef.current
              ? permissionModeRef.current
              : undefined,
          sandboxMode:
            currentSessionId || sandboxModeDirtyRef.current ? sandboxModeRef.current : undefined,
          planMode:
            effectivePlanMode && effectivePlanMode !== "off" ? effectivePlanMode : undefined,
          // Legacy top-level model/temperature/Think overrides remain available
          // to API clients as one-turn controls. The GUI uses sessionDefaults
          // above so draft choices are consumed only during materialization.
          displayText: options?.displayText?.trim() || undefined,
          isPlanTrigger: options?.isPlanTrigger,
          planComment: options?.planComment,
          workingDir: currentSessionId ? undefined : (draftWorkingDir ?? undefined),
          // Lazy project binding — send-time snapshot, only on the auto-create send.
          projectId: currentSessionId ? undefined : (draftProjectIdRef.current ?? undefined),
          // Send-time snapshot: only on the auto-create send, never incognito.
          kbAttachments:
            currentSessionId || incognitoEnabled
              ? undefined
              : draftKbAttachmentsRef.current.map((a) => ({
                  kbId: a.kbId,
                  access: a.access,
                })),
          ...(toolScope ? { toolScope } : {}),
          // Anchor only matters on the auto-create send; mirrors kbAttachments.
          ...(toolScope && !currentSessionId && draftKbAnchorNote
            ? { kbAnchorNote: draftKbAnchorNote }
            : {}),
        },
        onEvent,
      )
      chatResolved = true
    } catch (e) {
      const sid = targetSessionId || "__pending__"
      if (isActiveStreamError(e) && sid !== "__pending__") {
        // active_stream rejects before the backend persists anything, so the
        // optimistic user + assistant messages we just appended must be rolled
        // back. Other errors may have already saved server-side, so we keep
        // the user message visible there.
        keepExistingStreamLoading = true
        updateSessionMessages(sid, (prev) => {
          const updated = [...prev]
          const last = updated[updated.length - 1]
          if (
            last &&
            last.role === "assistant" &&
            !last.content &&
            !last.toolCalls?.length &&
            !last.contentBlocks?.length
          ) {
            updated.pop()
          }
          const maybeUser = updated[updated.length - 1]
          if (
            maybeUser &&
            maybeUser.role === "user" &&
            maybeUser.content === displayed &&
            maybeUser.timestamp === now
          ) {
            updated.pop()
          }
          return updated
        })
        try {
          const state = await getTransport().call<SessionStreamState>("get_session_stream_state", {
            sessionId: sid,
          })
          if (state.turnId && state.active) {
            activeTurnBySessionRef.current.set(sid, state.turnId)
          } else if (!state.active) {
            activeTurnBySessionRef.current.delete(sid)
          }
          if (state.status) {
            lastTurnStatusBySessionRef.current.set(sid, {
              status: state.status,
              interruptReason: state.interruptReason ?? null,
            })
            setExecutionStateBySession((prev) => new Map(prev).set(sid, state.status!))
          }
          const streamId = state.streamId || undefined
          if (streamId) endedStreamIdsRef.current.delete(sid)
          const cursorKey = streamCursorKey(sid, streamId)
          if (!lastSeqRef.current.has(cursorKey)) {
            lastSeqRef.current.set(cursorKey, Number(state.lastSeq) || 0)
          }
        } catch (stateError) {
          logger.warn(
            "chat",
            "useChatStream::active_stream",
            "Failed to refresh active stream state",
            stateError,
          )
        }
      } else {
        updateSessionMessages(sid, (prev) => {
          const updated = [...prev]
          const last = updated[updated.length - 1]
          if (last && last.role === "assistant" && last.content === "" && !last.toolCalls?.length) {
            updated.pop()
          }
          updated.push({ role: "event", content: `${e}` })
          return updated
        })
      }
      // Notify on error for non-current sessions
      if (
        !keepExistingStreamLoading &&
        targetSessionId &&
        currentSessionIdRef.current !== targetSessionId
      ) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title
          // Only fall back to the latest user message when reply previews are
          // enabled; otherwise honor the privacy setting and never leak
          // conversation content into the system notification center.
          const userPreview =
            getCachedConfig()?.showChatContent !== false
              ? latestUserNotificationPreview(sessionCacheRef.current.get(targetSessionId) ?? [])
              : null
          const title = t("notification.chatError")
          notify(title, sessionTitle || userPreview || title)
        }
      }
    } finally {
      const sid = targetSessionId || "__pending__"
      // Clean up empty assistant message if chat was stopped before any response arrived
      updateSessionMessages(sid, (prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (
          last &&
          last.role === "assistant" &&
          !last.content &&
          !last.toolCalls?.length &&
          !last.contentBlocks?.length
        ) {
          updated.pop()
        }
        return updated
      })
      if (keepExistingStreamLoading && sid !== "__pending__") {
        loadingSessionsRef.current.add(sid)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        if (currentSessionIdRef.current === sid) {
          setLoading(true)
        }
      } else {
        loadingSessionsRef.current.delete(sid)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        if (currentSessionIdRef.current === sid) {
          setLoading(false)
        }
      }
      // Notify on completion. Existing behavior: always notify for a
      // different session. For the active session, only surface an OS
      // notification when the app window is no longer foregrounded.
      if (!keepExistingStreamLoading && chatResolved && targetSessionId) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const status = lastTurnStatusBySessionRef.current.get(targetSessionId)?.status
          const completed =
            status !== "failed" && status !== "interrupted" && status !== "cancelling"
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title || agentName
          const notification = chatCompletionNotificationPayload(
            sessionTitle,
            sessionCacheRef.current.get(targetSessionId) ??
              (currentSessionIdRef.current === targetSessionId ? messages : []),
            getCachedConfig()?.showChatContent !== false,
            t("notification.chatCompletedGenericTitle"),
            t("notification.chatCompletedGenericBody"),
          )
          if (completed && currentSessionIdRef.current !== targetSessionId) {
            void notify(notification.title, notification.body)
          } else if (completed) {
            void notifyIfBackground(notification.title, notification.body)
          }
        }
      }
      // Mark as read ONLY when the completed turn belongs to the session the
      // user is actually viewing. A backgrounded turn — the user navigated away
      // before it finished (including a just-created session they then left), or
      // it was a background / injected turn — must keep its new assistant reply
      // unread so the sidebar surfaces it; otherwise the completion is silently
      // swallowed and the badge never appears. The lazy-create ref lag is
      // bridged eagerly in handleSessionCreated, so this comparison is accurate.
      if (targetSessionId && targetSessionId === currentSessionIdRef.current) {
        await getTransport()
          .call("mark_session_read_cmd", { sessionId: targetSessionId })
          .catch(() => {})
      }
      await reloadSessions()

      // Handle pending messages after loading finishes. Replays one FIFO item
      // per completed turn; the rest stay queued for subsequent turns.
      const queued = pendingSendsRef.current.find((item) => item.status !== "inserted")
      updatePendingSends((prev) =>
        queued
          ? prev.filter((item) => item.status !== "inserted" && item.id !== queued.id)
          : prev.filter((item) => item.status !== "inserted"),
      )
      if (queued) {
        // Restore staged quotes so they ride along with the replayed message
        // (user-draft path) instead of being silently dropped.
        if (queued.quotes?.length) setPendingQuotes(queued.quotes)
        if (queued.options && (queued.options.isPlanTrigger || autoSendPendingRef.current)) {
          // Programmatic queued send (Plan Mode approve, slash-skill
          // expansion). Replay through the auto-send effect with the
          // original options so `isPlanTrigger` / `displayText` / `planMode`
          // survive. Plan triggers are button-driven and should always
          // continue; slash-skill expansions still respect autoSendPending.
          queuedReplayRef.current = queued
          autoSendRef.current = true
        } else {
          // User-typed drafts and non-auto-sent programmatic sends are
          // restored for editing / confirmation without turning attachment-only
          // placeholders into real prompts.
          setInput(pendingInputText(queued))
          setAttachedFiles(queued.attachedFiles ?? [])
          if (autoSendPendingRef.current) {
            autoSendRef.current = true
          }
        }
      }
    }
  }

  // Auto-send: fires after React flushes the input state + loading=false.
  // Two replay paths:
  //   1. `queuedReplayRef` set → programmatic send (Plan Mode approve etc.)
  //      with the original options preserved.
  //   2. Otherwise → user-typed draft restored to `input`, dispatched as a
  //      regular send.
  useEffect(() => {
    if (!autoSendRef.current || loading) return
    const replay = queuedReplayRef.current
    if (replay) {
      autoSendRef.current = false
      queuedReplayRef.current = null
      void handleSend(replay.text, replay.options)
    } else if (input.trim() || attachedFiles.length > 0) {
      autoSendRef.current = false
      void handleSend()
    }
  }, [attachedFiles, input, loading]) // eslint-disable-line react-hooks/exhaustive-deps

  const editPendingSend = useCallback(
    (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (!item) return
      if (item.mode === "force_insert" && item.status === "waiting_tool_boundary") {
        const sid = currentSessionIdRef.current
        const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
        if (sid && turnId) {
          void getTransport()
            .call<CancelQueuedTurnUserMessageResult>("cancel_queued_turn_user_message", {
              sessionId: sid,
              turnId,
              requestId: id,
            })
            .catch(() => {})
        }
      }
      updatePendingSends((prev) => prev.filter((pending) => pending.id !== id))
      setInput(pendingInputText(item))
      setAttachedFiles(item.attachedFiles ?? [])
      setPendingQuotes(item.quotes ?? [])
    },
    [currentSessionIdRef, pendingInputText, setInput, updatePendingSends],
  )

  const discardPendingSend = useCallback(
    (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (item?.mode === "force_insert" && item.status === "waiting_tool_boundary") {
        const sid = currentSessionIdRef.current
        const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
        if (sid && turnId) {
          void getTransport()
            .call<CancelQueuedTurnUserMessageResult>("cancel_queued_turn_user_message", {
              sessionId: sid,
              turnId,
              requestId: id,
            })
            .catch(() => {})
        }
      }
      updatePendingSends((prev) => prev.filter((pending) => pending.id !== id))
    },
    [currentSessionIdRef, updatePendingSends],
  )

  const forceInsertPendingSend = useCallback(
    async (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (!item || !canForceInsertPending(item)) return
      const sid = currentSessionIdRef.current ?? currentSessionId
      const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
      if (!sid || !turnId) {
        updatePendingSends((prev) =>
          prev.map((pending) =>
            pending.id === id ? { ...pending, status: "fallback_after_reply" } : pending,
          ),
        )
        return
      }

      updatePendingSends((prev) =>
        prev.map((pending) =>
          pending.id === id
            ? { ...pending, mode: "force_insert", status: "waiting_tool_boundary" }
            : pending,
        ),
      )

      try {
        const attachments = await buildChatAttachments(
          item.text,
          item.attachedFiles ?? [],
          item.quotes ?? [],
          sid,
        )
        const latest = pendingSendsRef.current.find((pending) => pending.id === id)
        if (
          !latest ||
          latest.mode !== "force_insert" ||
          latest.status !== "waiting_tool_boundary"
        ) {
          return
        }
        const result = await getTransport().call<QueueTurnUserMessageResult>(
          "queue_turn_user_message",
          {
            requestId: id,
            sessionId: sid,
            turnId,
            message: item.text,
            attachments,
            displayText: item.options?.displayText,
            isPlanTrigger: item.options?.isPlanTrigger,
            planComment: item.options?.planComment,
          },
        )
        if (!result.queued) {
          updatePendingSends((prev) =>
            prev.map((pending) =>
              pending.id === id
                ? { ...pending, mode: "queue", status: "fallback_after_reply" }
                : pending,
            ),
          )
        }
      } catch (e) {
        logger.warn("chat", "useChatStream::forceInsert", "Failed to queue turn insertion", e)
        updatePendingSends((prev) =>
          prev.map((pending) =>
            pending.id === id
              ? { ...pending, mode: "queue", status: "fallback_after_reply" }
              : pending,
          ),
        )
      }
    },
    [
      buildChatAttachments,
      canForceInsertPending,
      currentSessionId,
      currentSessionIdRef,
      updatePendingSends,
    ],
  )

  const cancelForceInsertPendingSend = useCallback(
    async (id: string) => {
      const sid = currentSessionIdRef.current ?? currentSessionId
      const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
      if (sid && turnId) {
        await getTransport()
          .call<CancelQueuedTurnUserMessageResult>("cancel_queued_turn_user_message", {
            sessionId: sid,
            turnId,
            requestId: id,
          })
          .catch(() => undefined)
      }
      updatePendingSends((prev) =>
        prev.map((pending) =>
          pending.id === id ? { ...pending, mode: "queue", status: "queued" } : pending,
        ),
      )
    },
    [currentSessionId, currentSessionIdRef, updatePendingSends],
  )

  return {
    input,
    setInput,
    attachedFiles,
    setAttachedFiles,
    pendingQuotes,
    setPendingQuotes,
    pendingMessage,
    setPendingMessage,
    pendingSends,
    editPendingSend,
    discardPendingSend,
    forceInsertPendingSend,
    cancelForceInsertPendingSend,
    approvalRequests,
    showCodexAuthExpired,
    setShowCodexAuthExpired,
    permissionMode,
    setPermissionMode,
    setPermissionModeByUser,
    sandboxMode,
    setSandboxMode,
    setSandboxModeByUser,
    handleSend,
    handleStop,
    handleApprovalResponse,
    handleTurnStarted,
    handleTurnEnded,
    executionStateBySession,
  }
}
