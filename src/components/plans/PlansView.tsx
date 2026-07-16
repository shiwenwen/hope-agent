import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ArrowLeft,
  ClipboardList,
  ExternalLink,
  FilePlus,
  History,
  Loader2,
  RefreshCw,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IconTip } from "@/components/ui/tooltip"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { AgentSummary } from "@/components/settings/types"
import type {
  PlanIndexEntry,
  PlanIndexFilter,
  PlanModeStateString,
  PlanVersionInfoTs,
} from "./types"

interface PlansViewProps {
  onBack: () => void
  onJumpToSession: (sessionId: string) => void
  onInsertMention: (token: string) => void
}

// Radix Select forbids empty-string item values, so the "all" option uses this
// sentinel (matching the dashboard filter's "__all__" convention) and maps back
// to "" at the filter boundary.
const ALL_FILTER = "__all__"

const STATE_FILTERS: { value: "" | PlanModeStateString; labelKey: string }[] = [
  { value: "", labelKey: "plans.filter.state.all" },
  { value: "planning", labelKey: "plans.filter.state.planning" },
  { value: "review", labelKey: "plans.filter.state.review" },
  { value: "executing", labelKey: "plans.filter.state.executing" },
  { value: "completed", labelKey: "plans.filter.state.completed" },
  { value: "off", labelKey: "plans.filter.state.archived" },
]

export default function PlansView({
  onBack,
  onJumpToSession,
  onInsertMention,
}: PlansViewProps) {
  const { t } = useTranslation()
  const [entries, setEntries] = useState<PlanIndexEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [stateFilter, setStateFilter] = useState<"" | PlanModeStateString>("")
  const [agentFilter, setAgentFilter] = useState<string>("")
  const [agents, setAgents] = useState<AgentSummary[]>([])
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)

  // State filter is sent to the backend so the result set stays small. Agent
  // filter stays client-side: filtering it on the server would shrink the
  // agent dropdown to "only the currently selected agent" and lock the user
  // out of switching, so we keep the full list and filter visually here.
  const loadPlans = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const filter: PlanIndexFilter = { state: stateFilter || null }
      const data = await getTransport().call<PlanIndexEntry[]>("list_plans", { filter })
      setEntries(data)
    } catch (e) {
      logger.error("plans", "PlansView::load", "Failed to load plans", e)
      setError(String((e as Error)?.message ?? e))
    } finally {
      setLoading(false)
    }
  }, [stateFilter])

  useEffect(() => {
    void loadPlans()
  }, [loadPlans])

  // Agent metadata (name / emoji / avatar) is loaded once so the agent filter
  // can render the shared <AgentSelectDisplay> instead of a bare agent id,
  // mirroring the dashboard filter. It is decorative: a missing entry (e.g. a
  // deleted agent that still has plans) falls back to the raw id.
  useEffect(() => {
    let alive = true
    getTransport()
      .call<AgentSummary[]>("list_agents")
      .then((list) => {
        if (alive) setAgents(list)
      })
      .catch(() => {
        // ignore — fall back to raw agentId
      })
    return () => {
      alive = false
    }
  }, [])

  const agentMetaById = useMemo(() => {
    const map = new Map<string, AgentSummary>()
    for (const a of agents) map.set(a.id, a)
    return map
  }, [agents])

  const agentOptions = useMemo(() => {
    const seen = new Set<string>()
    for (const e of entries) seen.add(e.agentId)
    return Array.from(seen).sort()
  }, [entries])

  const visibleEntries = useMemo(
    () => (agentFilter ? entries.filter((e) => e.agentId === agentFilter) : entries),
    [entries, agentFilter],
  )

  // Derived selection: prefer the user's explicit pick when still present,
  // otherwise fall back to the first visible entry. Avoids a separate effect
  // racing against `loadPlans` to reset state on every reload.
  const selectedEntry = useMemo(
    () =>
      visibleEntries.find((e) => e.sessionId === selectedSessionId) ??
      visibleEntries[0] ??
      null,
    [visibleEntries, selectedSessionId],
  )

  const selectedAgentMeta = agentFilter ? agentMetaById.get(agentFilter) : undefined

  return (
    <div className="flex-1 flex min-h-0 flex-col overflow-hidden bg-background">
      <header
        className="flex h-10 shrink-0 items-center gap-2 border-b border-border-soft/60 bg-secondary/30 px-3"
        data-tauri-drag-region
      >
        <Button variant="ghost" size="icon" onClick={onBack} className="h-8 w-8">
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <ClipboardList className="h-4 w-4 text-primary" />
        <h2 className="text-sm font-medium">{t("plans.title")}</h2>
        <span className="text-xs text-muted-foreground">
          {t("plans.count", { count: visibleEntries.length })}
        </span>
        <div className="flex-1" />
        <IconTip label={t("plans.refresh")}>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={() => void loadPlans()}
            disabled={loading}
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </Button>
        </IconTip>
      </header>

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <div className="w-[360px] shrink-0 border-r border-border bg-background flex flex-col">
          <div className="px-3 py-2 border-b border-border/70 flex items-center gap-2">
            <Select
              value={stateFilter || ALL_FILTER}
              onValueChange={(v) =>
                setStateFilter(v === ALL_FILTER ? "" : (v as PlanModeStateString))
              }
            >
              <SelectTrigger className="h-7 w-auto gap-1 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATE_FILTERS.map((opt) => (
                  <SelectItem
                    key={opt.value || ALL_FILTER}
                    value={opt.value || ALL_FILTER}
                    className="text-xs"
                  >
                    {t(opt.labelKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {agentOptions.length > 0 && (
              <Select
                value={agentFilter || ALL_FILTER}
                onValueChange={(v) => setAgentFilter(v === ALL_FILTER ? "" : v)}
              >
                <SelectTrigger className="h-7 min-w-0 flex-1 gap-1 px-2 text-xs">
                  {selectedAgentMeta ? (
                    <AgentSelectDisplay agent={selectedAgentMeta} size="xs" />
                  ) : (
                    <SelectValue placeholder={t("plans.filter.allAgents")} />
                  )}
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL_FILTER} className="text-xs">
                    {t("plans.filter.allAgents")}
                  </SelectItem>
                  {agentOptions.map((agentId) => {
                    const meta = agentMetaById.get(agentId)
                    return (
                      <SelectItem
                        key={agentId}
                        value={agentId}
                        textValue={meta?.name ?? agentId}
                        className="text-xs"
                      >
                        {meta ? (
                          <AgentSelectDisplay agent={meta} size="xs" />
                        ) : (
                          agentId
                        )}
                      </SelectItem>
                    )
                  })}
                </SelectContent>
              </Select>
            )}
          </div>

          <div className="flex-1 overflow-y-auto">
            {error && (
              <div className="px-3 py-2 text-xs text-destructive bg-destructive/10">
                {error}
              </div>
            )}
            {!loading && visibleEntries.length === 0 && !error && (
              <div className="flex flex-col items-center justify-center py-12 px-4 text-muted-foreground text-xs">
                <ClipboardList className="h-8 w-8 mb-2 opacity-30" />
                <span>{t("plans.empty")}</span>
              </div>
            )}
            {visibleEntries.map((entry) => (
              <PlanListRow
                key={entry.sessionId}
                entry={entry}
                active={entry.sessionId === selectedEntry?.sessionId}
                onSelect={() => setSelectedSessionId(entry.sessionId)}
              />
            ))}
          </div>
        </div>

        <div className="flex-1 min-w-0 flex flex-col">
          {selectedEntry ? (
            <PlanReadOnlyDetail
              entry={selectedEntry}
              onJumpToSession={onJumpToSession}
              onInsertMention={onInsertMention}
            />
          ) : (
            <div className="flex-1 flex items-center justify-center text-muted-foreground text-sm">
              {t("plans.selectHint")}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

interface PlanListRowProps {
  entry: PlanIndexEntry
  active: boolean
  onSelect: () => void
}

function PlanListRow({ entry, active, onSelect }: PlanListRowProps) {
  const { t } = useTranslation()
  const title = entry.title || entry.sessionTitle || t("plans.untitled")
  return (
    <button
      onClick={onSelect}
      className={cn(
        "w-full text-left px-3 py-2 border-b border-border/40 transition-colors",
        active ? "bg-secondary/70" : "hover:bg-secondary/40",
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        <StateBadge state={entry.state} orphan={entry.orphan} />
        <span className="text-xs font-medium truncate flex-1" data-ha-title-tip={title}>
          {title}
        </span>
      </div>
      <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <span className="truncate" data-ha-title-tip={entry.agentId}>
          {entry.agentId}
        </span>
        <span>·</span>
        <span>v{entry.versionCount}</span>
        <span>·</span>
        <span className="ml-auto" data-ha-title-tip={entry.updatedAt}>
          {formatShortDate(entry.updatedAt)}
        </span>
      </div>
    </button>
  )
}

interface PlanReadOnlyDetailProps {
  entry: PlanIndexEntry
  onJumpToSession: (sessionId: string) => void
  onInsertMention: (token: string) => void
}

function PlanReadOnlyDetail({
  entry,
  onJumpToSession,
  onInsertMention,
}: PlanReadOnlyDetailProps) {
  const { t } = useTranslation()
  const [versions, setVersions] = useState<PlanVersionInfoTs[]>([])
  const [selectedVersion, setSelectedVersion] = useState<number>(0)
  const [content, setContent] = useState<string>("")
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setContent("")
    setVersions([])
    setSelectedVersion(0)
    void (async () => {
      try {
        const v = await getTransport().call<PlanVersionInfoTs[]>("get_plan_versions", {
          sessionId: entry.sessionId,
        })
        if (cancelled) return
        setVersions(v)
        const current = v.find((x) => x.isCurrent) ?? v[0]
        if (current) {
          setSelectedVersion(current.version)
          const text = await getTransport().call<string>("load_plan_version_content", {
            filePath: current.filePath,
          })
          if (!cancelled) setContent(text)
        }
      } catch (e) {
        if (!cancelled) {
          logger.error("plans", "PlanReadOnlyDetail::load", "Failed to load detail", e)
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [entry.sessionId])

  const handleVersionChange = useCallback(
    async (version: number) => {
      const target = versions.find((v) => v.version === version)
      if (!target) return
      setSelectedVersion(version)
      setLoading(true)
      try {
        const text = await getTransport().call<string>("load_plan_version_content", {
          filePath: target.filePath,
        })
        setContent(text)
      } catch (e) {
        logger.error(
          "plans",
          "PlanReadOnlyDetail::version",
          "Failed to load plan version content",
          e,
        )
      } finally {
        setLoading(false)
      }
    },
    [versions],
  )

  const handleAddToChat = useCallback(() => {
    const versionSuffix = selectedVersion > 0 ? `:v${selectedVersion}` : ":v0"
    onInsertMention(`@plan:${entry.sessionShortId}${versionSuffix}`)
  }, [entry.sessionShortId, selectedVersion, onInsertMention])

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-card/40">
        <StateBadge state={entry.state} orphan={entry.orphan} />
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium truncate">
            {entry.title || entry.sessionTitle || t("plans.untitled")}
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            {entry.agentId} · {entry.sessionShortId}
            {entry.projectId ? ` · ${entry.projectId}` : ""}
          </div>
        </div>

        {versions.length > 1 && (
          <div className="flex items-center gap-1">
            <History className="h-3.5 w-3.5 text-muted-foreground" />
            <Select
              value={String(selectedVersion)}
              onValueChange={(v) => void handleVersionChange(Number(v))}
            >
              <SelectTrigger className="h-7 w-auto gap-1 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {versions.map((v) => (
                  <SelectItem key={v.version} value={String(v.version)} className="text-xs">
                    v{v.version} {v.isCurrent ? `(${t("planMode.currentVersion")})` : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}

        <IconTip label={t("plans.addToChat")}>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={handleAddToChat}
            disabled={entry.orphan}
          >
            <FilePlus className="h-4 w-4" />
          </Button>
        </IconTip>
        <IconTip
          label={
            entry.orphan ? t("plans.sessionDeleted") : t("plans.openInSession")
          }
        >
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={() => onJumpToSession(entry.sessionId)}
            disabled={entry.orphan}
          >
            <ExternalLink className="h-4 w-4" />
          </Button>
        </IconTip>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-3">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : content ? (
          <div className="text-sm leading-relaxed">
            <MarkdownRenderer content={content} />
          </div>
        ) : (
          <div className="text-xs text-muted-foreground text-center py-8">
            {t("plans.emptyPlan")}
          </div>
        )}
      </div>
    </div>
  )
}

function StateBadge({
  state,
  orphan,
}: {
  state: PlanModeStateString
  orphan: boolean
}) {
  const { t } = useTranslation()
  if (orphan) {
    return (
      <span className="text-[10px] px-1.5 py-0.5 rounded bg-muted/50 text-muted-foreground border border-border">
        {t("plans.badge.orphan")}
      </span>
    )
  }
  const tone =
    state === "completed"
      ? "bg-green-500/15 text-green-600 border-green-500/30"
      : state === "executing"
        ? "bg-blue-500/15 text-blue-600 border-blue-500/30"
        : state === "review"
          ? "bg-purple-500/15 text-purple-600 border-purple-500/30"
          : state === "planning"
            ? "bg-amber-500/15 text-amber-600 border-amber-500/30"
            : "bg-muted/40 text-muted-foreground border-border"
  const labelKey =
    state === "off" ? "plans.badge.archived" : `plans.badge.${state}`
  return (
    <span
      className={cn(
        "text-[10px] px-1.5 py-0.5 rounded border whitespace-nowrap",
        tone,
      )}
    >
      {t(labelKey)}
    </span>
  )
}

function formatShortDate(rfc3339: string): string {
  if (!rfc3339) return ""
  const date = new Date(rfc3339)
  if (Number.isNaN(date.getTime())) return rfc3339
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" })
}
