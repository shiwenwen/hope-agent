import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { ChatAttachment } from "@/lib/transport"
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
  ActiveModel,
  AgentSummaryForSidebar,
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
import { useNotificationListeners } from "./useNotificationListeners"
import type { SessionStreamState } from "./useChatStreamReattach"

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

function latestAssistantNotificationPreview(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (message.role !== "assistant") continue
    const text = assistantNotificationText(message)
    if (text) return text
  }
  return null
}

function chatCompletionNotificationBody(
  sessionTitle: string,
  messages: Message[],
  showChatContent: boolean,
): string {
  if (!showChatContent) return sessionTitle
  const preview = latestAssistantNotificationPreview(messages)
  return preview ? `${sessionTitle}\n${preview}` : sessionTitle
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
  text: string
  options?: SendOptions
  attachedFiles?: File[]
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
  sessions: { id: string; title?: string | null; workingDir?: string | null }[]
  agents: AgentSummaryForSidebar[]
  activeModel: ActiveModel | null
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
}

export interface UseChatStreamReturn {
  input: string
  setInput: React.Dispatch<React.SetStateAction<string>>
  attachedFiles: File[]
  setAttachedFiles: React.Dispatch<React.SetStateAction<File[]>>
  pendingMessage: string | null
  setPendingMessage: React.Dispatch<React.SetStateAction<string | null>>
  approvalRequests: ApprovalRequest[]
  showCodexAuthExpired: boolean
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  permissionMode: SessionMode
  setPermissionMode: React.Dispatch<React.SetStateAction<SessionMode>>
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
  activeModel,
  reloadSessions,
  updateSessionMessages,
  lastSeqRef,
  endedStreamIdsRef,
  planMode,
  temperatureOverride,
  reasoningEffort,
  incognitoEnabled = false,
  draftWorkingDir = null,
}: UseChatStreamOptions): UseChatStreamReturn {
  const { t } = useTranslation()
  const [input, setInputState] = useState("")
  const [attachedFiles, setAttachedFilesState] = useState<File[]>([])
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
        const next =
          typeof value === "function" ? (value as (p: File[]) => File[])(prev) : value
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
    const nextDraft =
      inputDraftsRef.current.get(nextKey) ??
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

  // Pending send queued while a response is streaming. Stores the LLM-bound
  // `text` plus the original `options` (displayText / planMode / isPlanTrigger)
  // so the replay path can resend with the exact same metadata — otherwise a
  // queued plan-mode approve would lose its `isPlanTrigger` flag and render
  // as a plain user bubble.
  const [pendingSend, setPendingSendState] = useState<PendingSend | null>(null)
  const pendingSendRef = useRef<PendingSend | null>(null)
  // External views: keep the original `pendingMessage: string | null` API for
  // ChatScreen / ChatInput, derived from the user-facing displayed text.
  const pendingMessage = pendingSend
    ? pendingSend.options?.displayText?.trim() ||
      pendingSend.text ||
      (pendingSend.attachedFiles?.length ? t("chat.attachPhotosAndFiles") : "")
    : null
  const setPendingMessage = useCallback<
    React.Dispatch<React.SetStateAction<string | null>>
  >((value) => {
    setPendingSendState((prev) => {
      const next =
        typeof value === "function"
          ? (value as (p: string | null) => string | null)(
              prev ? prev.options?.displayText?.trim() || prev.text : null,
            )
          : value
      return next === null ? null : { text: next }
    })
  }, [])
  const [showCodexAuthExpired, setShowCodexAuthExpired] = useState(false)
  const [permissionMode, setPermissionModeState] = useState<SessionMode>("default")
  const permissionModeRef = useRef<SessionMode>("default")
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
  const setPermissionMode = useCallback<
    React.Dispatch<React.SetStateAction<SessionMode>>
  >((value) => {
    setPermissionModeState((prev) => {
      const next =
        typeof value === "function"
          ? (value as (p: SessionMode) => SessionMode)(prev)
          : value
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
  }, [currentSessionIdRef])

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
  })

  // Keep refs in sync
  useEffect(() => {
    pendingSendRef.current = pendingSend
  }, [pendingSend])
  useEffect(() => {
    permissionModeRef.current = permissionMode
  }, [permissionMode])

  // Seed `permissionMode` from the agent's `capabilities.defaultSessionPermissionMode`
  // whenever the user is sitting on a fresh chat (no session row yet, no
  // messages). Once the first message lands the session row owns the mode
  // and the title-bar switcher updates the row directly.
  //
  // Skipping when there is already a session id keeps the user's manual
  // choice intact across navigation — only "new chat" or agent swap re-seeds.
  useEffect(() => {
    if (currentSessionId || messages.length > 0 || !currentAgentId) return
    let cancelled = false
    void (async () => {
      try {
        const config = await getTransport().call<{
          capabilities?: { defaultSessionPermissionMode?: SessionMode | null }
        }>("get_agent_config", { id: currentAgentId })
        if (cancelled) return
        const fallback =
          (config?.capabilities?.defaultSessionPermissionMode as SessionMode | undefined) ??
          "default"
        setPermissionModeState(fallback)
      } catch (e) {
        logger.error(
          "chat",
          "useChatStream",
          "Failed to seed permission mode from agent capabilities",
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
    getTransport().call<{ autoSendPending?: boolean }>("get_user_config")
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

  /**
   * Send a message. If `directText` is provided, use it directly instead of the input box.
   * This avoids flashing text in the input (used by Plan Mode approve).
   */
  async function handleSend(directText?: string, options?: SendOptions) {
    const rawText = directText ?? input
    const hasAttachedFiles = !directText && attachedFiles.length > 0
    if (!rawText.trim() && !hasAttachedFiles) return

    // If currently loading, queue the message as pending. Capture both the
    // LLM-bound text and the original options so the replay below resends
    // with identical metadata (Plan Mode triggers carry `isPlanTrigger`,
    // slash-skill expansions carry `displayText`, etc.).
    if (loading) {
      const queuedFiles = directText ? [] : [...attachedFiles]
      setPendingSendState({
        text: rawText.trim(),
        options,
        ...(queuedFiles.length > 0 && { attachedFiles: queuedFiles }),
      })
      if (!directText) {
        setInput("")
        setAttachedFiles([])
      }
      return
    }

    const text = rawText.trim()
    // `text` goes to the LLM; `displayed` is the user bubble. Slash-skill passThrough
    // uses this split so the UI shows "/drawio ..." while the LLM receives the expansion.
    const filesToSend = directText ? [] : [...attachedFiles]
    const displayed = options?.displayText?.trim() || text
    const optimisticAttachments = filesToSend.map(optimisticAttachmentForFile)
    setInput("")
    setAttachedFiles([])
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
      return sidForCap && capMessagesForSession
        ? capMessagesForSession(sidForCap, next)
        : next
    })
    setLoading(true)

    // Process attached files: images → base64 data, non-images → save to disk via Rust
    const attachments: ChatAttachment[] = []

    // Expand `@path` mentions into file_path attachments. Working dir resolves
    // from the current session (committed) or the draft picker (new chat).
    const sessionWorkingDir =
      sessions.find((s) => s.id === currentSessionId)?.workingDir ?? null
    const resolvedWorkingDir = currentSessionId ? sessionWorkingDir : draftWorkingDir
    const mentionAttachments = expandMentionsToAttachments(text, resolvedWorkingDir ?? null)
    for (const m of mentionAttachments) {
      attachments.push(m)
    }
    // `@plan:<short>:v<n>` tokens resolve through the backend so we can
    // address plan files outside the working dir without weakening the
    // file-mention rules.
    const planAttachments = await expandPlanMentionsToAttachments(text)
    for (const p of planAttachments) {
      attachments.push(p)
    }

    for (const file of filesToSend) {
      try {
        const mimeType = file.type || "application/octet-stream"
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
            source: "upload",
            data: btoa(binary),
          })
        } else {
          const data = getTransport().prepareFileData(arrayBuffer, mimeType)
          const filePath = await getTransport().call<string>("save_attachment", {
            sessionId: currentSessionId,
            fileName: file.name,
            mimeType,
            data,
          })
          attachments.push({
            name: file.name,
            mime_type: mimeType,
            source: "upload",
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
      return sidForCap && capMessagesForSession
        ? capMessagesForSession(sidForCap, next)
        : next
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

      const shouldDropStreamEvent = (
        event: Record<string, unknown>,
        sid: string,
      ): boolean => {
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

      const modelOverride = activeModel
        ? `${activeModel.providerId}::${activeModel.modelId}`
        : undefined
      const effectivePlanMode = options?.planMode ?? planMode
      await getTransport().startChat(
        {
          message: text,
          attachments,
          sessionId: currentSessionId,
          incognito: currentSessionId ? undefined : incognitoEnabled,
          modelOverride,
          agentId: currentAgentId,
          permissionMode: permissionModeRef.current,
          planMode: effectivePlanMode && effectivePlanMode !== "off" ? effectivePlanMode : undefined,
          temperatureOverride: temperatureOverride ?? undefined,
          reasoningEffort: reasoningEffort ?? undefined,
          displayText: options?.displayText?.trim() || undefined,
          isPlanTrigger: options?.isPlanTrigger,
          planComment: options?.planComment,
          workingDir: currentSessionId ? undefined : draftWorkingDir ?? undefined,
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
          const state = await getTransport().call<SessionStreamState>(
            "get_session_stream_state",
            { sessionId: sid },
          )
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
          if (
            last &&
            last.role === "assistant" &&
            last.content === "" &&
            !last.toolCalls?.length
          ) {
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
          const sessionTitle =
            sessions.find((s) => s.id === targetSessionId)?.title || t("notification.chatError")
          notify(t("notification.chatError"), sessionTitle)
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
            status !== "failed" &&
            status !== "interrupted" &&
            status !== "cancelling"
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title || agentName
          const notificationBody = chatCompletionNotificationBody(
            sessionTitle,
            sessionCacheRef.current.get(targetSessionId) ??
              (currentSessionIdRef.current === targetSessionId ? messages : []),
            getCachedConfig()?.showChatContent === true,
          )
          if (completed && currentSessionIdRef.current !== targetSessionId) {
            void notify(t("notification.chatCompleted"), notificationBody)
          } else if (completed) {
            void notifyIfBackground(t("notification.chatCompleted"), notificationBody)
          }
        }
      }
      // Mark current session as read so unread count stays 0 for active session
      if (targetSessionId) {
        await getTransport()
          .call("mark_session_read_cmd", { sessionId: targetSessionId })
          .catch(() => {})
      }
      await reloadSessions()

      // Handle pending message after loading finishes
      const queued = pendingSendRef.current
      if (queued) {
        setPendingSendState(null)
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
          // restored for editing / confirmation using the user-facing text.
          setInput(queued.options?.displayText?.trim() || queued.text)
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

  return {
    input,
    setInput,
    attachedFiles,
    setAttachedFiles,
    pendingMessage,
    setPendingMessage,
    approvalRequests,
    showCodexAuthExpired,
    setShowCodexAuthExpired,
    permissionMode,
    setPermissionMode,
    handleSend,
    handleStop,
    handleApprovalResponse,
    handleTurnStarted,
    handleTurnEnded,
    executionStateBySession,
  }
}
