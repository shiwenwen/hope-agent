import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react"
import { useTranslation } from "react-i18next"
import { AlertCircle, Bot, Loader2, RefreshCw } from "lucide-react"

import MessageList from "@/components/chat/MessageList"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { FileActionsContext } from "./files/fileActionsContext"
import { PAGE_SIZE } from "./hooks/constants"
import { useChatStreamReattach } from "./hooks/useChatStreamReattach"
import { materializeMessages } from "./chatUtils"
import { getTransport } from "@/lib/transport-provider"
import type {
  AgentSummaryForSidebar,
  ChatTurnStatus,
  Message,
  SessionMessage,
  SessionMeta,
} from "@/types/chat"

interface SubagentSessionDialogProps {
  sessionId: string | null
  agents: AgentSummaryForSidebar[]
  onOpenChange: (open: boolean) => void
  onOpenNestedSession?: (sessionId: string) => void
}

export default function SubagentSessionDialog({
  sessionId,
  agents,
  onOpenChange,
  onOpenNestedSession,
}: SubagentSessionDialogProps) {
  const { t } = useTranslation()
  const [sessionMeta, setSessionMeta] = useState<SessionMeta | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [hasMore, setHasMore] = useState(false)
  const [oldestDbId, setOldestDbId] = useState<number | null>(null)
  const [initialLoading, setInitialLoading] = useState(false)
  const [streaming, setStreaming] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [reloadTick, setReloadTick] = useState(0)
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
  currentSessionIdRef.current = reattachSessionId

  const updateSessionMessages = useCallback((sid: string, updater: (prev: Message[]) => Message[]) => {
    if (sid !== currentSessionIdRef.current) return
    setMessages((prev) => {
      const next = updater(prev)
      sessionCacheRef.current.set(sid, next)
      return next
    })
  }, [])

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

  const handleTurnEnded = useCallback(
    (sid: string, status?: ChatTurnStatus | null) => {
      if (sid !== currentSessionIdRef.current) return
      setExecutionState(status ?? null)
    },
    [],
  )

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
      setSessionMeta(null)
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
      setSessionMeta(null)
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
        const displayMessages = await materializeMessages(sid, rawMessages, sessionsRef)
        if (cancelled) return

        setSessionMeta(meta)
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
          setSessionMeta(null)
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
  }, [sessionId, reloadTick])

  const handleLoadMore = useCallback(async () => {
    if (!sessionId || loadingMore || !hasMore || oldestDbId == null) return
    setLoadingMore(true)
    try {
      const [olderMessages, hasMoreBefore] = await getTransport().call<
        [SessionMessage[], boolean]
      >("load_session_messages_before_cmd", {
        sessionId,
        beforeId: oldestDbId,
        limit: PAGE_SIZE,
      })
      if (olderMessages.length === 0) {
        setHasMore(false)
        return
      }
      const olderDisplay = await materializeMessages(sessionId, olderMessages, sessionsRef)
      setOldestDbId(olderMessages[0]?.id ?? oldestDbId)
      setHasMore(hasMoreBefore)
      setMessages((prev) => {
        const seenDbIds = new Set(
          prev
            .map((msg) => msg.dbId)
            .filter((id): id is number => typeof id === "number"),
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

  const title =
    sessionMeta?.title?.trim() ||
    t("subagent.dialog.untitled", { defaultValue: "Sub-agent session" })
  const subtitle = sessionId
    ? t("subagent.dialog.subtitle", {
        defaultValue: "Live view · {{sessionId}}",
        sessionId: sessionId.slice(0, 8),
      })
    : ""

  return (
    <Dialog open={!!sessionId} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(86vh,900px)] max-h-[86vh] w-[min(1200px,calc(100vw-2rem))] max-w-none flex-col gap-0 overflow-hidden p-0 sm:rounded-xl">
        <DialogHeader className="border-b border-border px-4 py-3 pr-12 sm:px-5">
          <div className="flex min-w-0 items-center gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Bot className="h-4 w-4" />
            </div>
            <div className="min-w-0 flex-1 text-left">
              <DialogTitle className="truncate text-base">{title}</DialogTitle>
              <DialogDescription className="truncate text-xs">{subtitle}</DialogDescription>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 shrink-0"
              onClick={() => setReloadTick((n) => n + 1)}
              disabled={!sessionId || initialLoading}
              aria-label={t("common.refresh", { defaultValue: "Refresh" })}
            >
              <RefreshCw className={initialLoading ? "h-4 w-4 animate-spin" : "h-4 w-4"} />
            </Button>
          </div>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col bg-background">
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
                onViewChildSession={onOpenNestedSession}
                displayMode="timeline"
              />
            </FileActionsContext.Provider>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
