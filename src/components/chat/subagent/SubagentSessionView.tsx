import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react"
import { useTranslation } from "react-i18next"
import { AlertCircle, Loader2 } from "lucide-react"

import MessageList from "@/components/chat/MessageList"
import { cn } from "@/lib/utils"
import { FileActionsContext } from "../files/fileActionsContext"
import { PAGE_SIZE } from "../hooks/constants"
import { useChatStreamReattach } from "../hooks/useChatStreamReattach"
import { materializeMessages } from "../chatUtils"
import { getTransport } from "@/lib/transport-provider"
import type {
  AgentSummaryForSidebar,
  ChatTurnStatus,
  Message,
  SessionMessage,
  SessionMeta,
} from "@/types/chat"
import type { SubagentOpenTarget } from "./subagentRunModel"

interface SubagentSessionViewProps {
  sessionId: string | null
  agents: AgentSummaryForSidebar[]
  className?: string
  /** Bump to force a reload of the transcript (e.g. a refresh button). */
  reloadToken?: number
  /** A sub-agent chip inside this transcript was clicked (nested drill-down). */
  onOpenSubagentRun?: (target: SubagentOpenTarget) => void
  /** Legacy child-session opener (skill fork cards inside the transcript). */
  onOpenNestedSession?: (sessionId: string) => void
  /** Fires with the loaded session meta so a host (the dialog shell) can reuse it
   *  for a title instead of fetching `get_session_cmd` a second time. */
  onMeta?: (meta: SessionMeta | null) => void
}

/**
 * Read-only live view of a sub-agent's child session transcript. Owns the load
 * + streaming reattach + pagination state machine (extracted verbatim from the
 * former SubagentSessionDialog so both the dialog shell and the sub-agent panel
 * can host it). MessageList mounts its own `SubagentRunsProvider` keyed on this
 * session, so nested sub-agent chips inside the transcript get correct data for
 * free.
 */
export default function SubagentSessionView({
  sessionId,
  agents,
  className,
  reloadToken = 0,
  onOpenSubagentRun,
  onOpenNestedSession,
  onMeta,
}: SubagentSessionViewProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [hasMore, setHasMore] = useState(false)
  const [oldestDbId, setOldestDbId] = useState<number | null>(null)
  const [initialLoading, setInitialLoading] = useState(false)
  const [streaming, setStreaming] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [reattachSessionId, setReattachSessionId] = useState<string | null>(null)
  const [executionState, setExecutionState] = useState<ChatTurnStatus | null>(null)
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [, setShowCodexAuthExpired] = useState(false)
  const sessionsRef = useRef<SessionMeta[]>([])
  const currentSessionIdRef = useRef<string | null>(null)
  const lastSeqRef = useRef<Map<string, number>>(new Map())
  const endedStreamIdsRef = useRef<Map<string, string>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadedSessionIdRef = useRef<string | null>(null)
  const onMetaRef = useRef(onMeta)
  onMetaRef.current = onMeta
  currentSessionIdRef.current = reattachSessionId

  const updateSessionMessages = useCallback(
    (sid: string, updater: (prev: Message[]) => Message[]) => {
      if (sid !== currentSessionIdRef.current) return
      setMessages((prev) => {
        const next = updater(prev)
        sessionCacheRef.current.set(sid, next)
        return next
      })
    },
    [],
  )

  const replaceMessagesFromReload = useCallback<Dispatch<SetStateAction<Message[]>>>((value) => {
    setMessages((prev) => {
      const next = typeof value === "function" ? value(prev) : value
      const sid = currentSessionIdRef.current
      if (sid) sessionCacheRef.current.set(sid, next)
      return next
    })
  }, [])

  const handleTurnStarted = useCallback((sid: string) => {
    if (sid !== currentSessionIdRef.current) return
    setExecutionState("running")
  }, [])

  const handleTurnEnded = useCallback((sid: string, status?: ChatTurnStatus | null) => {
    if (sid !== currentSessionIdRef.current) return
    setExecutionState(status ?? null)
  }, [])

  useChatStreamReattach({
    currentSessionId: reattachSessionId,
    currentSessionIdRef,
    lastSeqRef,
    endedStreamIdsRef,
    updateSessionMessages,
    setShowCodexAuthExpired,
    setMessages: replaceMessagesFromReload,
    setLoading: setStreaming,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions: async () => {},
    onTurnStarted: handleTurnStarted,
    onTurnEnded: handleTurnEnded,
  })

  useEffect(() => {
    if (!sessionId) {
      sessionsRef.current = []
      sessionCacheRef.current.clear()
      lastSeqRef.current.clear()
      endedStreamIdsRef.current.clear()
      loadingSessionsRef.current.clear()
      loadedSessionIdRef.current = null
      currentSessionIdRef.current = null
      setReattachSessionId(null)
      setLoadingSessionIds(new Set())
      setMessages([])
      setHasMore(false)
      setOldestDbId(null)
      setInitialLoading(false)
      setStreaming(false)
      setLoadingMore(false)
      setError(null)
      setExecutionState(null)
      return
    }

    const sid = sessionId
    const isSessionChange = loadedSessionIdRef.current !== sid
    let cancelled = false
    const transport = getTransport()
    setInitialLoading(true)
    setLoadingMore(false)
    setError(null)
    if (isSessionChange) {
      currentSessionIdRef.current = null
      setReattachSessionId(null)
      lastSeqRef.current.clear()
      endedStreamIdsRef.current.clear()
      loadingSessionsRef.current.clear()
      setLoadingSessionIds(new Set())
      setMessages([])
      setHasMore(false)
      setOldestDbId(null)
      setExecutionState(null)
    }

    async function load() {
      try {
        const metaPromise = transport
          .call<SessionMeta | null>("get_session_cmd", { sessionId: sid })
          .catch(() => null)
        const messagesPromise = transport.call<[SessionMessage[], number, boolean]>(
          "load_session_messages_latest_cmd",
          { sessionId: sid, limit: PAGE_SIZE },
        )
        const [meta, [rawMessages, , hasMoreBefore]] = await Promise.all([
          metaPromise,
          messagesPromise,
        ])
        if (cancelled) return

        sessionsRef.current = meta ? [meta] : []
        onMetaRef.current?.(meta)
        const displayMessages = await materializeMessages(sid, rawMessages, sessionsRef)
        if (cancelled) return

        setMessages(displayMessages)
        sessionCacheRef.current.set(sid, displayMessages)
        loadedSessionIdRef.current = sid
        currentSessionIdRef.current = sid
        setReattachSessionId(sid)
        setHasMore(hasMoreBefore)
        setOldestDbId(rawMessages[0]?.id ?? null)
      } catch (e) {
        if (cancelled) return
        if (isSessionChange) {
          setMessages([])
          setHasMore(false)
          setOldestDbId(null)
        }
        setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (!cancelled) setInitialLoading(false)
      }
    }

    void load()
    return () => {
      cancelled = true
    }
  }, [sessionId, reloadToken])

  const handleLoadMore = useCallback(async () => {
    if (!sessionId || loadingMore || !hasMore || oldestDbId == null) return
    setLoadingMore(true)
    try {
      const [olderMessages, hasMoreBefore] = await getTransport().call<[SessionMessage[], boolean]>(
        "load_session_messages_before_cmd",
        {
          sessionId,
          beforeId: oldestDbId,
          limit: PAGE_SIZE,
        },
      )
      if (olderMessages.length === 0) {
        setHasMore(false)
        return
      }
      const olderDisplay = await materializeMessages(sessionId, olderMessages, sessionsRef)
      setOldestDbId(olderMessages[0]?.id ?? oldestDbId)
      setHasMore(hasMoreBefore)
      setMessages((prev) => {
        const seenDbIds = new Set(
          prev.map((msg) => msg.dbId).filter((id): id is number => typeof id === "number"),
        )
        const fresh = olderDisplay.filter((msg) => msg.dbId == null || !seenDbIds.has(msg.dbId))
        const next = [...fresh, ...prev]
        sessionCacheRef.current.set(sessionId, next)
        return next
      })
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoadingMore(false)
    }
  }, [hasMore, loadingMore, oldestDbId, sessionId])

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col bg-background", className)}>
      {initialLoading && messages.length === 0 ? (
        <div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("common.loading", { defaultValue: "Loading..." })}
        </div>
      ) : error ? (
        <div className="flex flex-1 items-center justify-center px-6 text-sm text-destructive">
          <div className="flex max-w-lg items-start gap-2 rounded-md border border-destructive/20 bg-destructive/5 px-3 py-2">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <span className="min-w-0 break-words">{error}</span>
          </div>
        </div>
      ) : messages.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          {t("chat.conversationEmpty", { defaultValue: "No messages in this conversation" })}
        </div>
      ) : (
        <FileActionsContext.Provider value={{ sessionId }}>
          <MessageList
            messages={messages}
            loading={streaming}
            executionState={executionState}
            agents={agents}
            hasMore={hasMore}
            loadingMore={loadingMore}
            onLoadMore={handleLoadMore}
            sessionId={sessionId}
            heroComposer
            bottomInset
            scrollFade
            onOpenSubagentRun={onOpenSubagentRun}
            onViewChildSession={onOpenNestedSession}
            displayMode="timeline"
          />
        </FileActionsContext.Provider>
      )}
    </div>
  )
}
