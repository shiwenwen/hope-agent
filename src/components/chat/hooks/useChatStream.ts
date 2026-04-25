import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { Channel } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { loadNotificationConfig, isAgentNotifyEnabled, notify } from "@/lib/notifications"
import type {
  Message,
  ActiveModel,
  AgentSummaryForSidebar,
  ToolPermissionMode,
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
import { expandMentionsToAttachments } from "@/components/chat/file-mention/expandMentions"
import { useNotificationListeners } from "./useNotificationListeners"

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
  sessions: { id: string; title?: string | null }[]
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
  toolPermissionMode: ToolPermissionMode
  setToolPermissionMode: React.Dispatch<React.SetStateAction<ToolPermissionMode>>
  handleSend: (directText?: string, options?: { hidden?: boolean; displayText?: string }) => Promise<void>
  handleStop: () => Promise<void>
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
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
  sessions,
  agents,
  activeModel,
  reloadSessions,
  updateSessionMessages,
  lastSeqRef,
  endedStreamIdsRef,
  planMode,
  temperatureOverride,
  incognitoEnabled = false,
  draftWorkingDir = null,
}: UseChatStreamOptions): UseChatStreamReturn {
  const { t } = useTranslation()
  const [input, setInput] = useState("")
  const [attachedFiles, setAttachedFiles] = useState<File[]>([])
  const [pendingMessage, setPendingMessage] = useState<string | null>(null)
  const pendingMessageRef = useRef<string | null>(null)
  const [showCodexAuthExpired, setShowCodexAuthExpired] = useState(false)
  const [toolPermissionMode, setToolPermissionModeState] = useState<ToolPermissionMode>("auto")
  const toolPermissionModeRef = useRef<ToolPermissionMode>("auto")

  // Sync toggle changes to backend immediately — the `chat` command only
  // snapshots the mode on entry, so without this the toggle has no effect on
  // in-flight tool loops or non-chat paths (subagent / cron / IM channels).
  //
  // Also ships the current `sessionId` so the backend persists the choice to
  // the session row; switching back to the same session later restores the
  // toggle instead of snapping to the global singleton's value.
  const setToolPermissionMode = useCallback<
    React.Dispatch<React.SetStateAction<ToolPermissionMode>>
  >((value) => {
    setToolPermissionModeState((prev) => {
      const next =
        typeof value === "function"
          ? (value as (p: ToolPermissionMode) => ToolPermissionMode)(prev)
          : value
      if (next !== prev) {
        const sid = currentSessionIdRef.current
        getTransport()
          .call("set_tool_permission_mode", {
            mode: next,
            ...(sid ? { sessionId: sid } : {}),
          })
          .catch((e) => {
            logger.error(
              "chat",
              "setToolPermissionMode",
              "Failed to sync tool permission mode",
              e,
            )
          })
      }
      return next
    })
  }, [currentSessionIdRef])

  // Auto-send pending messages setting
  const autoSendPendingRef = useRef(true)
  const autoSendRef = useRef(false)

  // Delta batch buffer
  const deltaBuffersRef = useRef(createStreamDeltaBuffers())

  useEffect(() => {
    const unlisten = getTransport().listen("chat:stream_end", (raw) => {
      const sid = (raw as { sessionId?: string } | null)?.sessionId
      if (!sid) return
      const streamId = streamIdFromPayload(raw)
      if (streamId) endedStreamIdsRef.current.set(sid, streamId)
      discardPendingStreamDeltas(sid, deltaBuffersRef)
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(deltaBuffersRef)
    }
  }, [endedStreamIdsRef])

  // Compose sub-hooks
  const { approvalRequests, handleApprovalResponse } = useApprovals()

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
    pendingMessageRef.current = pendingMessage
  }, [pendingMessage])
  useEffect(() => {
    toolPermissionModeRef.current = toolPermissionMode
  }, [toolPermissionMode])

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
    try {
      await getTransport().call("stop_chat")
    } catch (e) {
      logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
    }
  }

  /**
   * Send a message. If `directText` is provided, use it directly instead of the input box.
   * This avoids flashing text in the input (used by Plan Mode approve).
   */
  async function handleSend(directText?: string, options?: { hidden?: boolean; displayText?: string }) {
    const rawText = directText ?? input
    if (!rawText.trim()) return

    // If currently loading, queue the message as pending
    if (loading) {
      setPendingMessage(rawText.trim())
      if (!directText) setInput("")
      return
    }

    const text = rawText.trim()
    // `text` goes to the LLM; `displayed` is the user bubble. Slash-skill passThrough
    // uses this split so the UI shows "/drawio ..." while the LLM receives the expansion.
    const displayed = options?.displayText?.trim() || text
    const filesToSend = directText ? [] : [...attachedFiles]
    setInput("")
    setAttachedFiles([])
    const now = new Date().toISOString()
    setMessages((prev) => [...prev, { role: "user", content: displayed, timestamp: now, ...(options?.hidden && { isMeta: true }) }])
    setLoading(true)

    // Process attached files: images → base64 data, non-images → save to disk via Rust
    const attachments: {
      name: string
      mime_type: string
      data?: string
      file_path?: string
    }[] = []

    // Expand `@path` mentions into file_path attachments. Working dir resolves
    // from the current session (committed) or the draft picker (new chat).
    const sessionWorkingDir =
      sessions.find((s) => s.id === currentSessionId)?.workingDir ?? null
    const resolvedWorkingDir = currentSessionId ? sessionWorkingDir : draftWorkingDir
    const mentionAttachments = expandMentionsToAttachments(text, resolvedWorkingDir ?? null)
    for (const m of mentionAttachments) {
      attachments.push(m)
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

    // Add empty assistant message that we'll stream into
    setMessages((prev) => [
      ...prev,
      { role: "assistant", content: "", timestamp: new Date().toISOString() },
    ])

    let targetSessionId = currentSessionId

    try {
      const onEvent = new Channel<string>()
      onEvent.onmessage = (raw) => {
        try {
          const event = JSON.parse(raw)

          // Handle session_created first
          if (event.type === "session_created" && event.session_id) {
            targetSessionId = event.session_id
            const current = sessionCacheRef.current.get("__pending__")
            if (current) {
              sessionCacheRef.current.delete("__pending__")
              sessionCacheRef.current.set(event.session_id, current)
            }
            loadingSessionsRef.current.add(event.session_id)
            setLoadingSessionIds(new Set(loadingSessionsRef.current))
            setCurrentSessionId(event.session_id)
            reloadSessions()
            return
          }

          const sid = targetSessionId || "__pending__"
          const streamId = streamIdFromEvent(event)
          if (streamId && endedStreamIdsRef.current.get(sid) === streamId) return

          // Primary path bumps the seq cursor so identical events arriving
          // later via the EventBus reattach listener are dropped.
          const seqRaw = event._oc_seq
          if (typeof seqRaw === "number" && sid !== "__pending__") {
            const cursorKey = streamCursorKey(sid, streamId)
            const prev = lastSeqRef.current.get(cursorKey) ?? 0
            if (seqRaw <= prev) return
            lastSeqRef.current.set(cursorKey, seqRaw)
          }

          const handled = handleStreamEvent(event, sid, {
            updateSessionMessages,
            deltaBuffersRef,
            setShowCodexAuthExpired,
          })
          if (handled) return
        } catch {
          const sid = targetSessionId || "__pending__"
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
      }

      // Track loading state for this session
      const freshMessages = [
        ...messages,
        { role: "user" as const, content: displayed, timestamp: now, ...(options?.hidden && { isMeta: true }) },
        {
          role: "assistant" as const,
          content: "",
          timestamp: new Date().toISOString(),
        },
      ]
      if (targetSessionId) {
        loadingSessionsRef.current.add(targetSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        sessionCacheRef.current.set(targetSessionId, freshMessages)
      } else {
        sessionCacheRef.current.set("__pending__", freshMessages)
      }

      const modelOverride = activeModel
        ? `${activeModel.providerId}::${activeModel.modelId}`
        : undefined
      await getTransport().call<string>("chat", {
        message: text,
        attachments,
        sessionId: currentSessionId,
        incognito: currentSessionId ? undefined : incognitoEnabled,
        modelOverride,
        agentId: currentAgentId,
        toolPermissionMode: toolPermissionModeRef.current,
        planMode: planMode && planMode !== "off" ? planMode : undefined,
        temperatureOverride: temperatureOverride ?? undefined,
        displayText: options?.displayText?.trim() || undefined,
        workingDir: currentSessionId ? undefined : draftWorkingDir ?? undefined,
        onEvent,
      })
    } catch (e) {
      const sid = targetSessionId || "__pending__"
      updateSessionMessages(sid, (prev) => {
        const updated = [...prev]
        const last = updated[updated.length - 1]
        if (last && last.role === "assistant" && last.content === "" && !last.toolCalls?.length) {
          updated.pop()
        }
        updated.push({ role: "event", content: `${e}` })
        return updated
      })
      // Notify on error for non-current sessions
      if (targetSessionId && currentSessionIdRef.current !== targetSessionId) {
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
      loadingSessionsRef.current.delete(sid)
      setLoadingSessionIds(new Set(loadingSessionsRef.current))
      if (currentSessionIdRef.current === sid) {
        setLoading(false)
      }
      // Notify on completion for non-current sessions
      if (targetSessionId && currentSessionIdRef.current !== targetSessionId) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title || agentName
          notify(t("notification.chatCompleted"), sessionTitle)
        }
      }
      // Mark current session as read so unread count stays 0 for active session
      if (targetSessionId) {
        getTransport().call("mark_session_read_cmd", { sessionId: targetSessionId }).catch(() => {})
      }
      reloadSessions()

      // Handle pending message after loading finishes
      if (pendingMessageRef.current) {
        const pending = pendingMessageRef.current
        setPendingMessage(null)
        setInput(pending)
        if (autoSendPendingRef.current) {
          autoSendRef.current = true
        }
      }
    }
  }

  // Auto-send: fires after React flushes the input state + loading=false
  useEffect(() => {
    if (autoSendRef.current && input.trim() && !loading) {
      autoSendRef.current = false
      handleSend()
    }
  }, [input, loading]) // eslint-disable-line react-hooks/exhaustive-deps

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
    toolPermissionMode,
    setToolPermissionMode,
    handleSend,
    handleStop,
    handleApprovalResponse,
  }
}
