import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"
import MessageList from "@/components/chat/MessageList"
import { parseSessionMessages } from "@/components/chat/chatUtils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { Message, SessionMessage, AgentSummaryForSidebar } from "@/types/chat"

const PAGE_SIZE = 50

interface CronSessionViewerProps {
  sessionId: string
  agents: AgentSummaryForSidebar[]
}

/**
 * Read-only viewer for a single cron run's conversation. Reuses the main
 * chat `MessageList` renderer with every interaction callback omitted and no
 * `ChatInput` — mirroring how ChatScreen renders an `isCronSession` read-only.
 * Mounted with `key={sessionId}` by the parent so a row switch fully remounts.
 *
 * Supports loading older messages (scroll-up) — cron sessions are no longer
 * reachable from the main chat list, so a tool-heavy run with > PAGE_SIZE stored
 * messages would otherwise have its earlier prompt/tool context inaccessible.
 */
export default function CronSessionViewer({ sessionId, agents }: CronSessionViewerProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  // Mounted with key={sessionId} by both call sites, so each session starts
  // fresh — loading begins true and no synchronous reset is needed in the effect.
  const [loading, setLoading] = useState(true)
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  // DB id of the oldest message currently loaded; the cursor for "load earlier".
  const oldestDbId = useRef<number | null>(null)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<[SessionMessage[], number, boolean]>("load_session_messages_latest_cmd", {
        sessionId,
        limit: PAGE_SIZE,
      })
      .then(([rawMsgs, , hasMoreBefore]) => {
        if (cancelled) return
        setMessages(parseSessionMessages(rawMsgs))
        oldestDbId.current = rawMsgs.length > 0 ? rawMsgs[0].id : null
        setHasMore(!!hasMoreBefore)
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("cron", "CronSessionViewer::load", "Failed to load cron session messages", e)
        setMessages([])
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [sessionId])

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
      setMessages((prev) => [...parseSessionMessages(olderMsgs), ...prev])
    } catch (e) {
      logger.error("cron", "CronSessionViewer::loadMore", "Failed to load older cron messages", e)
    } finally {
      setLoadingMore(false)
    }
  }, [sessionId, hasMore, loadingMore])

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    )
  }

  if (messages.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
        {t("cron.conversationEmpty")}
      </div>
    )
  }

  return (
    <div className="flex flex-1 min-h-0 flex-col">
      <MessageList
        messages={messages}
        loading={false}
        agents={agents}
        hasMore={hasMore}
        loadingMore={loadingMore}
        onLoadMore={handleLoadMore}
        sessionId={sessionId}
        heroComposer
      />
    </div>
  )
}
