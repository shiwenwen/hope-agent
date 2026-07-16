import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, CheckCheck, Loader2, MessagesSquare } from "lucide-react"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { markAllCronRead, markCronSessionRead } from "@/hooks/useCronUnreadStore"
import { cronDisplayTitle, runLogDotColor, runStatusDisplay } from "./cronHelpers"
import type { CronTimelineRow } from "./CronJobForm.types"
import type { AgentSummaryForSidebar } from "@/types/chat"
import CronSessionViewer from "./CronSessionViewer"
import CronLoopBadge from "./CronLoopBadge"

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
  const [selectedRunLogId, setSelectedRunLogId] = useState<number | null>(null)
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [markingRead, setMarkingRead] = useState(false)
  const [markStatus, setMarkStatus] = useState<"idle" | "saved" | "failed">("idle")
  const markResetRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const markingSessionIdsRef = useRef(new Set<string>())
  const pendingReadSessionIdRef = useRef<string | null>(null)

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
        // The history view is a reader, so open the newest run immediately
        // instead of presenting an avoidable empty pane on every visit.
        setSelectedSessionId((current) => current ?? page[0]?.sessionId ?? null)
        setSelectedRunLogId((current) => current ?? page[0]?.runLogId ?? null)
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

  const markRunRead = useCallback((sessionId: string) => {
    if (markingSessionIdsRef.current.has(sessionId)) return
    markingSessionIdsRef.current.add(sessionId)
    void markCronSessionRead(sessionId)
      .then(() => {
        setRows((prev) =>
          prev.map((row) => (row.sessionId === sessionId ? { ...row, unreadCount: 0 } : row)),
        )
      })
      .catch((error) => {
        logger.warn(
          "cron",
          "CronConversationsPanel::markRunRead",
          "Failed to mark viewed cron run as read",
          error,
        )
      })
      .finally(() => markingSessionIdsRef.current.delete(sessionId))
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

  const handleSelect = useCallback(
    (row: CronTimelineRow) => {
      setSelectedRunLogId(row.runLogId)
      if (row.sessionId === selectedSessionId) {
        // The default-selected transcript may already be loaded; an explicit row
        // click is sufficient user intent even when no remount will occur.
        markRunRead(row.sessionId)
        return
      }
      pendingReadSessionIdRef.current = row.sessionId
      setSelectedSessionId(row.sessionId)
    },
    [markRunRead, selectedSessionId],
  )

  const handleViewerLoaded = useCallback(
    (sessionId: string) => {
      if (pendingReadSessionIdRef.current !== sessionId) return
      pendingReadSessionIdRef.current = null
      markRunRead(sessionId)
    },
    [markRunRead],
  )

  return (
    <div className="flex min-h-0 flex-1 px-3 pb-3">
      {/* Left — timeline list */}
      <div className="flex w-[19.5rem] shrink-0 flex-col pr-3">
        <div className="flex shrink-0 items-center justify-between px-2 pb-2 pt-1">
          <span className="text-xs font-medium text-muted-foreground">
            {t("cron.conversationsTitle")}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 rounded-lg px-2 text-xs text-muted-foreground"
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
            <div className="grid auto-rows-max gap-1">
              {rows.map((row) => {
                const display = runStatusDisplay(row.status)
                const isActive = row.runLogId === selectedRunLogId
                const isLoop = row.payloadType === "sessionLoop"
                const title = cronDisplayTitle(row.title || row.jobName, row.payloadType)
                return (
                  <button
                    type="button"
                    key={row.runLogId}
                    onClick={() => handleSelect(row)}
                    className={cn(
                      "h-auto min-h-0 w-full rounded-xl px-3 py-3 text-left transition-colors",
                      isActive ? "bg-secondary/70" : "hover:bg-secondary/40",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          "h-2 w-2 shrink-0 rounded-full",
                          runLogDotColor(row.status, "active"),
                        )}
                      />
                      <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs font-medium">
                        {isLoop && <CronLoopBadge />}
                        <span className="truncate">{title}</span>
                      </span>
                      {row.unreadCount > 0 && (
                        <>
                          <span
                            aria-hidden="true"
                            className="h-2.5 w-2.5 shrink-0 rounded-full bg-destructive"
                          />
                          <span className="sr-only">{t("chat.unreadStatus")}</span>
                        </>
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
                      <p className="mt-1 line-clamp-1 pl-4 text-[11px] text-muted-foreground">
                        {row.resultPreview}
                      </p>
                    )}
                  </button>
                )
              })}
              {hasMore && (
                <div className="px-3 py-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 w-full rounded-lg text-xs text-muted-foreground"
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
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-muted/[0.14]">
        {selectedSessionId ? (
          <CronSessionViewer
            key={selectedSessionId}
            sessionId={selectedSessionId}
            agents={agents}
            onLoaded={handleViewerLoaded}
          />
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
