import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from "react"
import { useTranslation } from "react-i18next"
import { ArrowUpRight, Loader2 } from "lucide-react"
import MessageList from "@/components/chat/MessageList"
import { parseSessionMessages } from "@/components/chat/chatUtils"
import { useChatStreamReattach } from "@/components/chat/hooks/useChatStreamReattach"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { IconTip } from "@/components/ui/tooltip"
import { Button } from "@/components/ui/button"
import { requestChatFocus } from "@/components/chat/chatFocus"
import type { Message, SessionMessage, AgentSummaryForSidebar } from "@/types/chat"

const PAGE_SIZE = 50

interface CronSessionViewerProps {
  runLogId: number
  sessionId: string
  requiresExactTarget?: boolean
  targetMessageId?: number | null
  expectedTurnId?: string | null
  agents: AgentSummaryForSidebar[]
  /** Fires only after the transcript request succeeds (an empty transcript still counts). */
  onLoaded?: (runLogId: number) => void
}

interface CronSessionLiveReattachProps {
  sessionId: string
  setMessages: Dispatch<SetStateAction<Message[]>>
  setStreamLoading: Dispatch<SetStateAction<boolean>>
  sessionCacheRef: MutableRefObject<Map<string, Message[]>>
  messagesPreloaded: boolean
  expectedTurnId?: string | null
}

/** Mounted only after the viewer's one initial transcript read has settled. */
function CronSessionLiveReattach({
  sessionId,
  setMessages,
  setStreamLoading,
  sessionCacheRef,
  messagesPreloaded,
  expectedTurnId,
}: CronSessionLiveReattachProps) {
  const currentSessionIdRef = useRef<string | null>(sessionId)
  const lastSeqRef = useRef(new Map<string, number>())
  const endedStreamIdsRef = useRef(new Map<string, Set<string>>())
  const loadingSessionsRef = useRef(new Set<string>())
  const [, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [, setShowCodexAuthExpired] = useState(false)

  const updateSessionMessages = useCallback(
    (sid: string, updater: (prev: Message[]) => Message[]) => {
      if (sid !== sessionId) return
      setMessages((prev) => {
        const next = updater(prev)
        sessionCacheRef.current.set(sid, next)
        return next
      })
    },
    [sessionCacheRef, sessionId, setMessages],
  )
  const reloadSessions = useCallback(async () => {}, [])

  useChatStreamReattach({
    currentSessionId: sessionId,
    currentSessionIdRef,
    lastSeqRef,
    endedStreamIdsRef,
    updateSessionMessages,
    setShowCodexAuthExpired,
    setMessages,
    setLoading: setStreamLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
    messagesPreloaded,
    expectedTurnId,
  })
  return null
}

/**
 * Read-only preview for a single scheduled run's conversation. Reuses the main
 * chat `MessageList` renderer with every interaction callback omitted and no
 * `ChatInput`; interactive follow-up happens in the linked ordinary session.
 * Mounted with `key={runLogId}` by the parent so a row switch fully remounts.
 *
 * Supports loading older messages (scroll-up) so a tool-heavy run with more than
 * PAGE_SIZE stored messages keeps its earlier prompt/tool context accessible.
 */
export default function CronSessionViewer({
  runLogId,
  sessionId,
  requiresExactTarget,
  targetMessageId,
  expectedTurnId,
  agents,
  onLoaded,
}: CronSessionViewerProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  // Mounted with key={runLogId} by both call sites, so each run starts
  // fresh — loading begins true and no synchronous reset is needed in the effect.
  const [historyLoading, setHistoryLoading] = useState(true)
  const [initialLoadSucceeded, setInitialLoadSucceeded] = useState(false)
  const [streamLoading, setStreamLoading] = useState(false)
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  // DB id of the oldest message currently loaded; the cursor for "load earlier".
  const oldestDbId = useRef<number | null>(null)
  const sessionCacheRef = useRef(new Map<string, Message[]>())
  const onLoadedRef = useRef(onLoaded)
  onLoadedRef.current = onLoaded

  useEffect(() => {
    let cancelled = false
    if (targetMessageId == null && requiresExactTarget) {
      setHistoryLoading(false)
      return
    }
    const request =
      targetMessageId != null
        ? getTransport().call<[SessionMessage[], number, boolean, boolean]>(
            "load_session_messages_around_cmd",
            { sessionId, targetMessageId, before: PAGE_SIZE, after: PAGE_SIZE },
          )
        : getTransport().call<[SessionMessage[], number, boolean]>(
            "load_session_messages_latest_cmd",
            { sessionId, limit: PAGE_SIZE },
          )
    request
      .then(([rawMsgs, , hasMoreBefore]) => {
        if (cancelled) return
        const persisted = parseSessionMessages(rawMsgs)
        sessionCacheRef.current.set(sessionId, persisted)
        setMessages(persisted)
        setInitialLoadSucceeded(true)
        oldestDbId.current = rawMsgs.length > 0 ? rawMsgs[0].id : null
        setHasMore(!!hasMoreBefore)
        onLoadedRef.current?.(runLogId)
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("cron", "CronSessionViewer::load", "Failed to load cron session messages", e)
      })
      .finally(() => {
        if (!cancelled) setHistoryLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [requiresExactTarget, runLogId, sessionId, targetMessageId])

  const handleLoadMore = useCallback(async () => {
    if (loadingMore || !hasMore || oldestDbId.current === null) return
    setLoadingMore(true)
    try {
      const [olderMsgs, hasMoreBefore] = await getTransport().call<[SessionMessage[], boolean]>(
        "load_session_messages_before_cmd",
        { sessionId, beforeId: oldestDbId.current, limit: PAGE_SIZE },
      )
      if (olderMsgs.length === 0) {
        setHasMore(false)
        return
      }
      oldestDbId.current = olderMsgs[0].id
      setHasMore(hasMoreBefore)
      const older = parseSessionMessages(olderMsgs)
      setMessages((prev) => {
        const existingIds = new Set(
          prev.flatMap((message) => (typeof message.dbId === "number" ? [message.dbId] : [])),
        )
        const next = [
          ...older.filter(
            (message) => typeof message.dbId !== "number" || !existingIds.has(message.dbId),
          ),
          ...prev,
        ]
        sessionCacheRef.current.set(sessionId, next)
        return next
      })
    } catch (e) {
      logger.error("cron", "CronSessionViewer::loadMore", "Failed to load older cron messages", e)
    } finally {
      setLoadingMore(false)
    }
  }, [sessionId, hasMore, loadingMore])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!historyLoading && initialLoadSucceeded && (
        <CronSessionLiveReattach
          sessionId={sessionId}
          setMessages={setMessages}
          setStreamLoading={setStreamLoading}
          sessionCacheRef={sessionCacheRef}
          messagesPreloaded={initialLoadSucceeded}
          expectedTurnId={expectedTurnId}
        />
      )}
      <div className="flex h-9 shrink-0 items-center justify-end border-b border-border-soft px-2">
        <IconTip label={t("subagent.openSession")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs text-muted-foreground"
            disabled={requiresExactTarget && targetMessageId == null}
            onClick={() =>
              requestChatFocus({ sessionId, targetMessageId: targetMessageId ?? undefined })
            }
          >
            <ArrowUpRight className="h-3.5 w-3.5" />
            {t("subagent.openSession")}
          </Button>
        </IconTip>
      </div>
      {historyLoading ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      ) : messages.length === 0 ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
          {t("cron.conversationEmpty")}
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col pt-3">
          <MessageList
            messages={messages}
            loading={streamLoading}
            agents={agents}
            hasMore={hasMore}
            loadingMore={loadingMore}
            onLoadMore={handleLoadMore}
            sessionId={sessionId}
            pendingScrollIntent={
              targetMessageId ? { messageId: targetMessageId, highlightTerms: null } : null
            }
            heroComposer
            bottomInset
          />
        </div>
      )}
    </div>
  )
}
