import { useEffect, useState } from "react"
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
 */
export default function CronSessionViewer({ sessionId, agents }: CronSessionViewerProps) {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  // Mounted with key={sessionId} by both call sites, so each session starts
  // fresh — loading begins true and no synchronous reset is needed in the effect.
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<[SessionMessage[], number, boolean]>("load_session_messages_latest_cmd", {
        sessionId,
        limit: PAGE_SIZE,
      })
      .then(([rawMsgs]) => {
        if (cancelled) return
        setMessages(parseSessionMessages(rawMsgs))
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
        hasMore={false}
        loadingMore={false}
        onLoadMore={() => {}}
        sessionId={sessionId}
        heroComposer
      />
    </div>
  )
}
