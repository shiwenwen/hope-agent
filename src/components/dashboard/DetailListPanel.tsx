import React, { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { X, MessageSquare, Wrench, AlertTriangle, Bot, Clock, MessagesSquare } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type {
  DetailListType,
  DashboardFilter,
  DashboardSessionItem,
  DashboardMessageItem,
  DashboardToolCallItem,
  DashboardErrorItem,
  DashboardAgentItem,
  CronJob,
  CronSchedule,
} from "./types"
import { formatNumber, formatDuration } from "./types"

interface DetailListPanelProps {
  listType: DetailListType
  filter: DashboardFilter
  agentNameMap: Record<string, string>
  onClose: () => void
}

function formatRelativeTime(ts: string): string {
  try {
    const d = new Date(ts)
    const now = new Date()
    const diff = now.getTime() - d.getTime()
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return "< 1m"
    if (mins < 60) return `${mins}m ago`
    const hours = Math.floor(mins / 60)
    if (hours < 24) return `${hours}h ago`
    const days = Math.floor(hours / 24)
    return `${days}d ago`
  } catch {
    return ts
  }
}

/** Shared data-fetching hook for detail lists */
function useListData<T>(command: string, params: Record<string, unknown>) {
  const [data, setData] = useState<T[] | null>(null)
  const [loading, setLoading] = useState(false)
  const paramsKey = JSON.stringify(params)

  useEffect(() => {
    let cancelled = false

    async function load() {
      setLoading(true)
      try {
        const result = await getTransport().call<T[]>(command, params)
        if (!cancelled) setData(result)
      } catch (e) {
        logger.error("dashboard", command, `${e}`)
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    void load()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [command, paramsKey])

  return { data, loading }
}

// ── Session List ────────────────────────────────────────────────

function SessionList({
  filter,
  agentNameMap,
}: {
  filter: DashboardFilter
  agentNameMap: Record<string, string>
}) {
  const { t } = useTranslation()
  const { data, loading } = useListData<DashboardSessionItem>("dashboard_session_list", { filter })

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[1fr_120px_100px_100px_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>{t("dashboard.detail.sessionTitle")}</span>
        <span>Agent</span>
        <span>{t("dashboard.detail.messageCount")}</span>
        <span>Token</span>
        <span>{t("dashboard.detail.time")}</span>
      </div>
      {data.map((s) => (
        <div
          key={s.id}
          className="grid grid-cols-[1fr_120px_100px_100px_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <span className="truncate">{s.title || s.id.slice(0, 8)}</span>
          <span className="truncate text-muted-foreground">
            {agentNameMap[s.agentId] || s.agentId}
          </span>
          <span>{formatNumber(s.messageCount)}</span>
          <span>{formatNumber(s.totalTokens)}</span>
          <span className="text-muted-foreground text-xs">{formatRelativeTime(s.updatedAt)}</span>
        </div>
      ))}
    </div>
  )
}

// ── Message List ────────────────────────────────────────────────

function MessageList({ filter }: { filter: DashboardFilter }) {
  const { t } = useTranslation()
  const { data, loading } = useListData<DashboardMessageItem>("dashboard_message_list", { filter })

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[80px_1fr_140px_100px_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>{t("dashboard.detail.role")}</span>
        <span>{t("dashboard.detail.content")}</span>
        <span>{t("dashboard.detail.session")}</span>
        <span>Token</span>
        <span>{t("dashboard.detail.time")}</span>
      </div>
      {data.map((m) => (
        <div
          key={m.id}
          className="grid grid-cols-[80px_1fr_140px_100px_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <span
            className={cn(
              "text-xs font-medium px-1.5 py-0.5 rounded w-fit",
              m.role === "user"
                ? "bg-blue-500/10 text-blue-500"
                : m.role === "assistant"
                  ? "bg-green-500/10 text-green-500"
                  : "bg-muted text-muted-foreground",
            )}
          >
            {m.role}
          </span>
          <span className="truncate text-muted-foreground">{m.contentPreview || "—"}</span>
          <span className="truncate text-xs text-muted-foreground">
            {m.sessionTitle || m.sessionId.slice(0, 8)}
          </span>
          <span className="text-xs">{formatNumber(m.tokensIn + m.tokensOut)}</span>
          <span className="text-muted-foreground text-xs">{formatRelativeTime(m.timestamp)}</span>
        </div>
      ))}
    </div>
  )
}

// ── Tool Call List ──────────────────────────────────────────────

function ToolCallList({ filter }: { filter: DashboardFilter }) {
  const { t } = useTranslation()
  const { data, loading } = useListData<DashboardToolCallItem>("dashboard_tool_call_list", {
    filter,
  })

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[1fr_140px_100px_80px_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>{t("dashboard.detail.toolName")}</span>
        <span>{t("dashboard.detail.session")}</span>
        <span>{t("dashboard.detail.duration")}</span>
        <span>{t("dashboard.detail.status")}</span>
        <span>{t("dashboard.detail.time")}</span>
      </div>
      {data.map((tc) => (
        <div
          key={tc.id}
          className="grid grid-cols-[1fr_140px_100px_80px_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <span className="font-mono text-xs truncate">{tc.toolName}</span>
          <span className="truncate text-xs text-muted-foreground">
            {tc.sessionTitle || tc.sessionId.slice(0, 8)}
          </span>
          <span className="text-xs">
            {tc.durationMs != null ? formatDuration(tc.durationMs) : "—"}
          </span>
          <span
            className={cn("text-xs font-medium", tc.isError ? "text-red-500" : "text-green-500")}
          >
            {tc.isError ? "Error" : "OK"}
          </span>
          <span className="text-muted-foreground text-xs">{formatRelativeTime(tc.timestamp)}</span>
        </div>
      ))}
    </div>
  )
}

// ── Error List ──────────────────────────────────────────────────

function ErrorList({ filter }: { filter: DashboardFilter }) {
  const { t } = useTranslation()
  const { data, loading } = useListData<DashboardErrorItem>("dashboard_error_list", { filter })

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[70px_120px_120px_1fr_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>{t("dashboard.detail.level")}</span>
        <span>{t("dashboard.detail.category")}</span>
        <span>{t("dashboard.detail.source")}</span>
        <span>{t("dashboard.detail.errorMessage")}</span>
        <span>{t("dashboard.detail.time")}</span>
      </div>
      {data.map((e) => (
        <div
          key={e.id}
          className="grid grid-cols-[70px_120px_120px_1fr_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <span
            className={cn(
              "text-xs font-medium px-1.5 py-0.5 rounded w-fit",
              e.level === "error" ? "bg-red-500/10 text-red-500" : "bg-amber-500/10 text-amber-500",
            )}
          >
            {e.level}
          </span>
          <span className="truncate text-xs">{e.category}</span>
          <span className="truncate text-xs text-muted-foreground">{e.source}</span>
          <span className="truncate text-muted-foreground">{e.message}</span>
          <span className="text-muted-foreground text-xs">{formatRelativeTime(e.timestamp)}</span>
        </div>
      ))}
    </div>
  )
}

// ── Agent List ──────────────────────────────────────────────────

function AgentList({
  filter,
  agentNameMap,
}: {
  filter: DashboardFilter
  agentNameMap: Record<string, string>
}) {
  const { t } = useTranslation()
  const { data, loading } = useListData<DashboardAgentItem>("dashboard_agent_list", { filter })

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[1fr_120px_120px_120px_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>Agent</span>
        <span>{t("dashboard.detail.sessionCount")}</span>
        <span>{t("dashboard.detail.messageCount")}</span>
        <span>Token</span>
        <span>{t("dashboard.detail.lastActive")}</span>
      </div>
      {data.map((a) => (
        <div
          key={a.agentId}
          className="grid grid-cols-[1fr_120px_120px_120px_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <span className="font-medium truncate">{agentNameMap[a.agentId] || a.agentId}</span>
          <span>{formatNumber(a.sessionCount)}</span>
          <span>{formatNumber(a.messageCount)}</span>
          <span>{formatNumber(a.totalTokens)}</span>
          <span className="text-muted-foreground text-xs">
            {formatRelativeTime(a.lastActiveAt)}
          </span>
        </div>
      ))}
    </div>
  )
}

function formatSchedule(s: CronSchedule): string {
  switch (s.type) {
    case "at":
      return s.timestamp
    case "every":
      return `${formatDuration(s.intervalMs ?? s.interval_ms ?? 0)}`
    case "cron":
      return s.expression
  }
}

// ── Cron Job List ───────────────────────────────────────────────

function CronJobList() {
  const { t } = useTranslation()
  const { data, loading } = useListData<CronJob>("cron_list_jobs", {})

  if (loading || !data) return <ListSkeleton />
  if (!data.length) return <EmptyState message={t("dashboard.detail.empty")} />

  return (
    <div className="divide-y">
      <div className="grid grid-cols-[1fr_1fr_100px_100px_140px] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground">
        <span>{t("dashboard.detail.cronName")}</span>
        <span>{t("dashboard.detail.schedule")}</span>
        <span>{t("dashboard.detail.status")}</span>
        <span>{t("dashboard.detail.failures")}</span>
        <span>{t("dashboard.detail.lastRun")}</span>
      </div>
      {data.map((j) => (
        <div
          key={j.id}
          className="grid grid-cols-[1fr_1fr_100px_100px_140px] gap-3 px-4 py-2.5 text-sm hover:bg-muted/50 transition-colors"
        >
          <div className="min-w-0">
            <div className="truncate font-medium">{j.name}</div>
            {j.description && (
              <div className="truncate text-xs text-muted-foreground">{j.description}</div>
            )}
          </div>
          <span className="font-mono text-xs truncate">{formatSchedule(j.schedule)}</span>
          <span
            className={cn(
              "text-xs font-medium px-1.5 py-0.5 rounded w-fit h-fit leading-none",
              j.status === "active"
                ? "bg-blue-500/10 text-blue-500"
                : j.status === "paused"
                  ? "bg-amber-500/10 text-amber-500"
                  : j.status === "completed"
                    ? "bg-green-500/10 text-green-500"
                    : j.status === "disabled"
                      ? "bg-red-500/10 text-red-500"
                      : "bg-muted text-muted-foreground",
            )}
          >
            {j.status}
          </span>
          <span
            className={cn(
              "text-xs",
              j.consecutiveFailures > 0 ? "text-red-500" : "text-muted-foreground",
            )}
          >
            {j.consecutiveFailures}/{j.maxFailures}
          </span>
          <span className="text-muted-foreground text-xs">
            {j.lastRunAt ? formatRelativeTime(j.lastRunAt) : "—"}
          </span>
        </div>
      ))}
    </div>
  )
}

// ── Shared Components ───────────────────────────────────────────

function ListSkeleton() {
  return (
    <div className="space-y-2 p-4">
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} className="h-10 bg-muted animate-pulse rounded" />
      ))}
    </div>
  )
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="flex items-center justify-center py-12 text-sm text-muted-foreground">
      {message}
    </div>
  )
}

const listConfig: Record<DetailListType, { icon: React.ElementType; titleKey: string }> = {
  sessions: { icon: MessageSquare, titleKey: "dashboard.detail.sessions" },
  messages: { icon: MessagesSquare, titleKey: "dashboard.detail.messages" },
  toolCalls: { icon: Wrench, titleKey: "dashboard.detail.toolCalls" },
  errors: { icon: AlertTriangle, titleKey: "dashboard.detail.errors" },
  agents: { icon: Bot, titleKey: "dashboard.detail.agents" },
  cronJobs: { icon: Clock, titleKey: "dashboard.detail.cronJobs" },
}

export default function DetailListPanel({
  listType,
  filter,
  agentNameMap,
  onClose,
}: DetailListPanelProps) {
  const { t } = useTranslation()
  const config = listConfig[listType]
  const Icon = config.icon

  return (
    <div className="bg-card border rounded-xl overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3 border-b">
        <Icon className="h-4 w-4 text-muted-foreground" />
        <h3 className="text-sm font-semibold flex-1">{t(config.titleKey)}</h3>
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose}>
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>

      {/* Content */}
      <div className="max-h-[400px] overflow-y-auto">
        {listType === "sessions" && <SessionList filter={filter} agentNameMap={agentNameMap} />}
        {listType === "messages" && <MessageList filter={filter} />}
        {listType === "toolCalls" && <ToolCallList filter={filter} />}
        {listType === "errors" && <ErrorList filter={filter} />}
        {listType === "agents" && <AgentList filter={filter} agentNameMap={agentNameMap} />}
        {listType === "cronJobs" && <CronJobList />}
      </div>
    </div>
  )
}
