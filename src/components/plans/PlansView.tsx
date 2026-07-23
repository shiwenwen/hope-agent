import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ArrowLeft,
  ClipboardList,
  ExternalLink,
  FilePlus,
  History,
  Loader2,
  PanelLeft,
  PanelLeftDashed,
  RefreshCw,
  Search,
  X,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
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
import { useDragWidth } from "@/hooks/useDragWidth"
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
const LIST_WIDTH_STORAGE_KEY = "hope.plans.listWidth"
const LIST_COLLAPSED_STORAGE_KEY = "hope.plans.listCollapsed"
const LIST_DEFAULT_WIDTH = 340
const LIST_MIN_WIDTH = 280
const LIST_MAX_WIDTH = 520
const LIST_WIDTH_TRANSITION =
  "transition-[width] duration-[250ms] ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width] motion-reduce:transition-none"
const LIST_SURFACE_TRANSITION =
  "transition-[opacity,transform,border-color] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] [contain:layout_paint] motion-reduce:transition-none"

const STATE_FILTERS: { value: "" | PlanModeStateString; labelKey: string }[] = [
  { value: "", labelKey: "plans.filter.state.all" },
  { value: "planning", labelKey: "plans.filter.state.planning" },
  { value: "review", labelKey: "plans.filter.state.review" },
  { value: "executing", labelKey: "plans.filter.state.executing" },
  { value: "completed", labelKey: "plans.filter.state.completed" },
  { value: "off", labelKey: "plans.filter.state.archived" },
]

function readStoredBoolean(key: string): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(key) === "true"
  } catch {
    return false
  }
}

function readStoredWidth(key: string, fallback: number): number {
  if (typeof window === "undefined") return fallback
  try {
    const value = Number(window.localStorage.getItem(key))
    return Number.isFinite(value) && value > 0
      ? Math.min(LIST_MAX_WIDTH, Math.max(LIST_MIN_WIDTH, value))
      : fallback
  } catch {
    return fallback
  }
}

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
  const [query, setQuery] = useState("")
  const [agents, setAgents] = useState<AgentSummary[]>([])
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [listCollapsed, setListCollapsed] = useState(() =>
    readStoredBoolean(LIST_COLLAPSED_STORAGE_KEY),
  )
  const [listWidth, setListWidth] = useState(() =>
    readStoredWidth(LIST_WIDTH_STORAGE_KEY, LIST_DEFAULT_WIDTH),
  )
  const [isResizingList, setIsResizingList] = useState(false)
  const [isListResizeHandleHovered, setIsListResizeHandleHovered] = useState(false)

  const onDragList = useDragWidth({
    width: listWidth,
    min: LIST_MIN_WIDTH,
    max: LIST_MAX_WIDTH,
    onChange: setListWidth,
    direction: "ltr",
    onResizingChange: setIsResizingList,
  })

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

  useEffect(() => {
    try {
      window.localStorage.setItem(LIST_COLLAPSED_STORAGE_KEY, String(listCollapsed))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [listCollapsed])

  useEffect(() => {
    try {
      window.localStorage.setItem(LIST_WIDTH_STORAGE_KEY, String(Math.round(listWidth)))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [listWidth])

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

  const visibleEntries = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase()
    return entries.filter((entry) => {
      if (agentFilter && entry.agentId !== agentFilter) return false
      if (!normalizedQuery) return true
      const searchable = [
        entry.title,
        entry.sessionTitle,
        entry.agentId,
        entry.projectId,
        entry.sessionShortId,
      ]
        .filter(Boolean)
        .join(" ")
        .toLocaleLowerCase()
      return searchable.includes(normalizedQuery)
    })
  }, [agentFilter, entries, query])

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
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background">
      <header
        className="flex h-10 shrink-0 items-center gap-2 border-b border-border-soft/60 px-3"
        data-tauri-drag-region
      >
        <IconTip label={t("common.back")} side="bottom">
          <Button
            variant="ghost"
            size="icon"
            onClick={onBack}
            className="h-8 w-8"
            aria-label={t("common.back")}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </IconTip>
        <IconTip
          label={listCollapsed ? t("common.expand") : t("common.collapse")}
          side="bottom"
        >
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            aria-label={listCollapsed ? t("common.expand") : t("common.collapse")}
            aria-expanded={!listCollapsed}
            onClick={() => setListCollapsed((collapsed) => !collapsed)}
          >
            {listCollapsed ? (
              <PanelLeftDashed className="h-4 w-4" />
            ) : (
              <PanelLeft className="h-4 w-4" />
            )}
          </Button>
        </IconTip>
        <ClipboardList className="h-4 w-4 text-primary" data-tauri-drag-region />
        <div className="flex min-w-0 flex-1 items-baseline gap-2" data-tauri-drag-region>
          <h1 className="shrink-0 truncate text-sm font-semibold" data-tauri-drag-region>
            {t("plans.title")}
          </h1>
          <span className="shrink-0 text-xs text-muted-foreground/40" aria-hidden="true">
            ·
          </span>
          <p className="min-w-0 truncate text-xs text-muted-foreground" data-tauri-drag-region>
            {t("plans.count", { count: visibleEntries.length })}
          </p>
        </div>
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

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <div
          style={{ width: listCollapsed ? 0 : listWidth }}
          className={cn("relative h-full shrink-0", !isResizingList && LIST_WIDTH_TRANSITION)}
        >
          <div className="h-full overflow-hidden">
            <aside
              style={{ width: listWidth }}
              aria-hidden={listCollapsed}
              inert={listCollapsed ? true : undefined}
              className={cn(
                "flex h-full flex-col border-r",
                isResizingList
                  ? "border-r-primary/50"
                  : isListResizeHandleHovered
                    ? "border-r-primary/35"
                    : "border-r-border-soft",
                LIST_SURFACE_TRANSITION,
                listCollapsed
                  ? "pointer-events-none -translate-x-4 opacity-0"
                  : "translate-x-0 opacity-100",
              )}
            >
              <div className="grid grid-cols-2 gap-2 border-b border-border-soft p-3">
                <div className="relative col-span-2">
                  <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/60" />
                  <SearchInput
                    className="h-9 pl-8 pr-8"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder={`${t("common.search")} Plan`}
                    aria-label={`${t("common.search")} Plan`}
                  />
                  {query && (
                    <button
                      type="button"
                      className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground transition-colors hover:text-foreground"
                      onClick={() => setQuery("")}
                      aria-label={t("common.clear")}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>
                <Select
                  value={stateFilter || ALL_FILTER}
                  onValueChange={(v) =>
                    setStateFilter(v === ALL_FILTER ? "" : (v as PlanModeStateString))
                  }
                >
                  <SelectTrigger className="h-9 min-w-0 text-xs">
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
                {agentOptions.length > 0 ? (
                  <Select
                    value={agentFilter || ALL_FILTER}
                    onValueChange={(v) => setAgentFilter(v === ALL_FILTER ? "" : v)}
                  >
                    <SelectTrigger className="h-9 min-w-0 text-xs">
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
                            {meta ? <AgentSelectDisplay agent={meta} size="xs" /> : agentId}
                          </SelectItem>
                        )
                      })}
                    </SelectContent>
                  </Select>
                ) : (
                  <div />
                )}
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto p-2">
                {error ? (
                  <div className="mb-2 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    {error}
                  </div>
                ) : null}
                {loading && visibleEntries.length === 0 ? (
                  <div className="flex h-32 items-center justify-center">
                    <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                  </div>
                ) : visibleEntries.length === 0 ? (
                  error ? null : (
                    <div className="flex h-40 flex-col items-center justify-center gap-2 px-4 text-center text-muted-foreground">
                      <ClipboardList className="h-7 w-7" />
                      <p className="text-sm">{t("plans.empty")}</p>
                    </div>
                  )
                ) : (
                  visibleEntries.map((entry) => (
                    <PlanListRow
                      key={entry.sessionId}
                      entry={entry}
                      agent={agentMetaById.get(entry.agentId)}
                      active={entry.sessionId === selectedEntry?.sessionId}
                      onSelect={() => setSelectedSessionId(entry.sessionId)}
                    />
                  ))
                )}
              </div>
            </aside>
          </div>
          <div
            className={cn(
              "absolute inset-y-0 right-0 z-20 translate-x-full cursor-col-resize",
              listCollapsed ? "pointer-events-none w-0 opacity-0" : "w-3 opacity-100",
            )}
            onMouseDown={onDragList}
            onMouseEnter={() => setIsListResizeHandleHovered(true)}
            onMouseLeave={() => setIsListResizeHandleHovered(false)}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("plans.resizeList")}
          />
        </div>

        <main className="flex min-w-0 flex-1 flex-col">
          {selectedEntry ? (
            <PlanReadOnlyDetail
              entry={selectedEntry}
              onJumpToSession={onJumpToSession}
              onInsertMention={onInsertMention}
            />
          ) : (
            <div className="flex flex-1 items-center justify-center text-muted-foreground">
              <div className="text-center">
                <ClipboardList className="mx-auto mb-3 h-10 w-10" />
                <p>{t("plans.selectHint")}</p>
              </div>
            </div>
          )}
        </main>
      </div>
    </div>
  )
}

interface PlanListRowProps {
  entry: PlanIndexEntry
  agent?: AgentSummary
  active: boolean
  onSelect: () => void
}

function PlanListRow({ entry, agent, active, onSelect }: PlanListRowProps) {
  const { t } = useTranslation()
  const title = entry.title || entry.sessionTitle || t("plans.untitled")
  const agentLabel = agent?.name || entry.agentId
  return (
    <button
      onClick={onSelect}
      className={cn(
        "mb-1.5 w-full rounded-xl p-3 text-left text-foreground transition-colors",
        active ? "bg-secondary" : "hover:bg-secondary/40",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <span className="line-clamp-2 min-w-0 flex-1 text-sm font-medium" data-ha-title-tip={title}>
          {title}
        </span>
        <StateBadge state={entry.state} orphan={entry.orphan} />
      </div>
      <div className="mt-2 flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="truncate" data-ha-title-tip={agentLabel}>
          {agentLabel}
        </span>
        <span>·</span>
        <span>v{entry.versionCount}</span>
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
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-border-soft px-3 py-2">
        <StateBadge state={entry.state} orphan={entry.orphan} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold">
            {entry.title || entry.sessionTitle || t("plans.untitled")}
          </div>
          <div className="truncate text-[11px] text-muted-foreground">
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

      <div className="min-h-0 flex-1 overflow-y-auto bg-muted/[0.14] p-4 sm:p-6">
        {loading ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : content ? (
          <article className="mx-auto min-h-full w-full max-w-4xl rounded-2xl bg-background px-6 py-5 text-sm leading-relaxed sm:px-8 sm:py-7">
            <MarkdownRenderer content={content} />
          </article>
        ) : (
          <div className="flex h-full items-center justify-center text-center text-xs text-muted-foreground">
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
      <span className="whitespace-nowrap rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
        {t("plans.badge.orphan")}
      </span>
    )
  }
  const tone =
    state === "completed"
      ? "bg-green-500/15 text-green-600"
      : state === "executing"
        ? "bg-blue-500/15 text-blue-600"
        : state === "review"
          ? "bg-purple-500/15 text-purple-600"
          : state === "planning"
            ? "bg-amber-500/15 text-amber-600"
            : "bg-muted text-muted-foreground"
  const labelKey =
    state === "off" ? "plans.badge.archived" : `plans.badge.${state}`
  return (
    <span className={cn("whitespace-nowrap rounded-md px-1.5 py-0.5 text-[10px]", tone)}>
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
