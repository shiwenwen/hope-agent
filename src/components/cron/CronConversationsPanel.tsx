import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, CheckCheck, Loader2, MessagesSquare } from "lucide-react"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { markAllCronRead, refreshCronUnread } from "@/hooks/useCronUnreadStore"
import { runLogDotColor, runStatusDisplay } from "./cronHelpers"
import type { CronTimelineRow } from "./CronJobForm.types"
import type { AgentSummaryForSidebar } from "@/types/chat"
import CronSessionViewer from "./CronSessionViewer"

const PAGE_SIZE = 50

function useRelativeTime() {
  const { t } = useTranslation()
  return useCallback(
    (dateStr: string) => {
      const date = new Date(dateStr)
      if (isNaN(date.getTime())) return ""
      const minutes = Math.floor((Date.now() - date.getTime()) / 60000)
      if (minutes < 1) return t("chat.justNow")
      if (minutes < 60) return t("chat.minutesAgo", { count: minutes })
      const hours = Math.floor(minutes / 60)
      if (hours < 24) return t("chat.hoursAgo", { count: hours })
      const days = Math.floor(hours / 24)
      if (days < 7) return t("chat.daysAgo", { count: days })
      if (days < 30) return t("chat.weeksAgo", { count: Math.floor(days / 7) })
      return date.toLocaleDateString()
    },
    [t],
  )
}

/**
 * Cron panel "conversations" view: a master-detail of every cron run across all
 * jobs. Left = a newest-first timeline (cron_run_timeline, paginated); right =
 * the selected run's conversation rendered read-only via CronSessionViewer.
 */
export default function CronConversationsPanel() {
  const { t } = useTranslation()
  const relativeTime = useRelativeTime()

  const [rows, setRows] = useState<CronTimelineRow[]>([])
  const [offset, setOffset] = useState(0)
  const [hasMore, setHasMore] = useState(false)
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [markingRead, setMarkingRead] = useState(false)
  const [markStatus, setMarkStatus] = useState<"idle" | "saved" | "failed">("idle")
  const markResetRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const fetchPage = useCallback(async (pageOffset: number) => {
    const page = await getTransport().call<CronTimelineRow[]>("cron_run_timeline", {
      limit: PAGE_SIZE,
      offset: pageOffset,
    })
    return Array.isArray(page) ? page : []
  }, [])

  // Initial load (timeline + agents for message bubbles).
  useEffect(() => {
    let cancelled = false
    setLoading(true)
    Promise.all([
      fetchPage(0),
      getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[]),
    ])
      .then(([page, agentList]) => {
        if (cancelled) return
        setRows(page)
        setOffset(page.length)
        setHasMore(page.length === PAGE_SIZE)
        setAgents(Array.isArray(agentList) ? agentList : [])
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("cron", "CronConversationsPanel::load", "Failed to load cron timeline", e)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [fetchPage])

  // Keep the timeline live while open: when a cron run completes, refresh the
  // first page so the new run shows up at the top (mirrors CronCalendarView,
  // which also listens to cron:run_completed). Resetting to page 0 is fine —
  // new runs sort newest-first; the selected conversation on the right is keyed
  // by sessionId and is unaffected.
  useEffect(() => {
    const unlisten = getTransport().listen("cron:run_completed", () => {
      fetchPage(0)
        .then((page) => {
          setRows(page)
          setOffset(page.length)
          setHasMore(page.length === PAGE_SIZE)
        })
        .catch(() => {})
    })
    return unlisten
  }, [fetchPage])

  useEffect(() => {
    return () => {
      if (markResetRef.current) clearTimeout(markResetRef.current)
    }
  }, [])

  const loadMore = useCallback(async () => {
    if (loadingMore || !hasMore) return
    setLoadingMore(true)
    try {
      const page = await fetchPage(offset)
      setRows((prev) => [...prev, ...page])
      setOffset((prev) => prev + page.length)
      setHasMore(page.length === PAGE_SIZE)
    } catch (e) {
      logger.error("cron", "CronConversationsPanel::loadMore", "Failed to load more cron runs", e)
    } finally {
      setLoadingMore(false)
    }
  }, [fetchPage, hasMore, loadingMore, offset])

  const handleMarkAllRead = useCallback(async () => {
    setMarkingRead(true)
    try {
      await markAllCronRead()
      // Reflect the cleared unread state locally without a full refetch.
      setRows((prev) => prev.map((r) => (r.unreadCount > 0 ? { ...r, unreadCount: 0 } : r)))
      setMarkStatus("saved")
    } catch {
      setMarkStatus("failed")
    } finally {
      setMarkingRead(false)
      if (markResetRef.current) clearTimeout(markResetRef.current)
      markResetRef.current = setTimeout(() => setMarkStatus("idle"), 2000)
    }
  }, [])

  const handleSelect = useCallback((row: CronTimelineRow) => {
    setSelectedSessionId(row.sessionId)
    // Opening a run does not auto-mark it read (clearing is explicit via
    // "mark all read"), but pull a fresh badge count so it stays in sync.
    refreshCronUnread()
  }, [])

  return (
    <div className="flex flex-1 min-h-0">
      {/* Left — timeline list */}
      <div className="flex w-80 shrink-0 flex-col border-r border-border">
        <div className="flex items-center justify-between px-4 py-2.5 border-b border-border shrink-0">
          <span className="text-sm font-medium">{t("cron.conversationsTitle")}</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 px-2 text-xs"
            disabled={markingRead || rows.length === 0}
            onClick={() => void handleMarkAllRead()}
          >
            {markingRead ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : markStatus === "saved" ? (
              <Check className="h-3.5 w-3.5 text-emerald-500" />
            ) : (
              <CheckCheck className="h-3.5 w-3.5" />
            )}
            {t("cron.markAllRead")}
          </Button>
        </div>

        <div className="flex-1 overflow-y-auto">
          {loading ? (
            <div className="flex items-center justify-center py-10 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : rows.length === 0 ? (
            <p className="px-4 py-10 text-center text-xs text-muted-foreground">
              {t("cron.noConversations")}
            </p>
          ) : (
            <div className="py-1">
              {rows.map((row) => {
                const display = runStatusDisplay(row.status)
                const isActive = row.sessionId === selectedSessionId
                return (
                  <button
                    key={row.sessionId}
                    onClick={() => handleSelect(row)}
                    className={cn(
                      "w-full px-3 py-2.5 text-left transition-colors border-l-2",
                      isActive
                        ? "bg-primary/10 border-l-primary"
                        : "border-l-transparent hover:bg-secondary/50",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          "h-2 w-2 shrink-0 rounded-full",
                          runLogDotColor(row.status, "active"),
                        )}
                      />
                      <span className="flex-1 truncate text-xs font-medium">
                        {row.title || row.jobName}
                      </span>
                      {row.unreadCount > 0 && (
                        <span className="flex h-[16px] min-w-[16px] items-center justify-center rounded-full bg-destructive px-1 text-[9px] font-semibold leading-none text-white tabular-nums">
                          {row.unreadCount > 99 ? "99+" : row.unreadCount}
                        </span>
                      )}
                    </div>
                    <div className="mt-1 flex items-center justify-between gap-2 pl-4">
                      <span className={cn("text-[10px]", display.className)}>
                        {display.symbol}
                        {t(display.labelKey)}
                      </span>
                      <span className="text-[10px] text-muted-foreground">
                        {relativeTime(row.startedAt)}
                      </span>
                    </div>
                    {row.resultPreview && (
                      <p className="mt-1 line-clamp-2 pl-4 text-[11px] text-muted-foreground">
                        {row.resultPreview}
                      </p>
                    )}
                  </button>
                )
              })}
              {hasMore && (
                <div className="px-3 py-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 w-full text-xs"
                    disabled={loadingMore}
                    onClick={() => void loadMore()}
                  >
                    {loadingMore ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      t("cron.loadMore")
                    )}
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Right — read-only conversation */}
      <div className="flex flex-1 min-w-0 flex-col">
        {selectedSessionId ? (
          <CronSessionViewer key={selectedSessionId} sessionId={selectedSessionId} agents={agents} />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-muted-foreground">
            <MessagesSquare className="h-10 w-10 opacity-40" />
            <p className="text-sm">{t("cron.conversationsSelectHint")}</p>
          </div>
        )}
      </div>
    </div>
  )
}
