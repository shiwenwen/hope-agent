import { createElement, useContext, useEffect, useMemo, useState, type ComponentType } from "react"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleSlash2,
  Clock,
  GitBranch,
  MessageSquare,
  Network,
  Pause,
  Play,
  RefreshCw,
  Search,
  XCircle,
} from "lucide-react"

import { SubagentRunsContext } from "@/components/chat/subagent/useSubagentRuns"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { FileChangeMetadata, FileChangesMetadata, ToolCall } from "@/types/chat"

import { formatDuration } from "../chatUtils"
import { getToolsWallClockMs, toolHasMedia } from "./executionStatus"
import { MediaHoistContext } from "./mediaHoistContext"
import {
  getRuntimeControlActivityGroupKey,
  parseRuntimeControlActivity,
  runtimeControlPluralKey,
  type RuntimeControlAction,
  type RuntimeControlActivityItem,
  type RuntimeControlActivityState,
  type RuntimeControlFamily,
} from "./runtimeControlActivity"
import ToolCallBlock from "./ToolCallBlock"
import ToolMediaPreview from "./ToolMediaPreview"

interface RuntimeControlActivityGroupProps {
  tools: ToolCall[]
  shimmer?: boolean
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
}

const STATE_PRIORITY: RuntimeControlActivityState[] = [
  "running",
  "failed",
  "partial",
  "refused",
  "accepted",
  "completed",
]

const ACTION_ICONS: Record<
  RuntimeControlFamily,
  Partial<Record<RuntimeControlAction, ComponentType<{ className?: string }>>>
> = {
  agent: {
    observe: Search,
    message: MessageSquare,
    close: XCircle,
    resume: RefreshCw,
  },
  job: { cancel: XCircle },
  process: { close: XCircle },
  team: { pause: Pause, resume: Play },
  workflow: { pause: Pause, resume: Play, cancel: GitBranch },
  cron: { pause: Pause, resume: Play },
}

const DEFAULT_ACTION_LABELS: Record<string, { one: string; other: string }> = {
  "agent.observe": { one: "check {{count}} agent", other: "check {{count}} agents" },
  "agent.message": { one: "message {{count}} agent", other: "message {{count}} agents" },
  "agent.close": { one: "close {{count}} agent", other: "close {{count}} agents" },
  "agent.resume": { one: "resume {{count}} agent", other: "resume {{count}} agents" },
  "job.cancel": { one: "cancel {{count}} task", other: "cancel {{count}} tasks" },
  "process.close": { one: "close {{count}} process", other: "close {{count}} processes" },
  "team.pause": { one: "pause {{count}} team", other: "pause {{count}} teams" },
  "team.resume": { one: "resume {{count}} team", other: "resume {{count}} teams" },
  "workflow.pause": { one: "pause {{count}} workflow", other: "pause {{count}} workflows" },
  "workflow.resume": { one: "resume {{count}} workflow", other: "resume {{count}} workflows" },
  "workflow.cancel": { one: "cancel {{count}} workflow", other: "cancel {{count}} workflows" },
  "cron.pause": { one: "pause {{count}} schedule", other: "pause {{count}} schedules" },
  "cron.resume": { one: "resume {{count}} schedule", other: "resume {{count}} schedules" },
}

const DEFAULT_STATE_LABELS: Record<RuntimeControlActivityState, string> = {
  running: "In progress · {{action}}",
  accepted: "Requested · {{action}}",
  completed: "Completed · {{action}}",
  partial: "Partially completed · {{action}}",
  failed: "Failed · {{action}}",
  refused: "Refused · {{action}}",
}

// Keep dynamic base names as constants because the repository's literal-key
// scanner does not model runtime key construction.
const RUNTIME_CONTROL_KEY = "executionStatus.runtimeControl"
const STATE_ALREADY_TERMINAL_KEY = `${RUNTIME_CONTROL_KEY}.state.alreadyTerminal`
const BADGE_ALREADY_TERMINAL_KEY = `${RUNTIME_CONTROL_KEY}.badge.alreadyTerminal`
const BADGE_REFUSED_KEY = `${RUNTIME_CONTROL_KEY}.badge.refused`

function actionLabel(t: TFunction, item: RuntimeControlActivityItem, count: number): string {
  if (item.allTargets) {
    return String(
      t(`executionStatus.runtimeControl.action.${item.family}.closeAll`, {
        defaultValue: "close all agents",
      }),
    )
  }
  const actionName = item.action
  const path = `${item.family}.${actionName}`
  const fallback = DEFAULT_ACTION_LABELS[path]
  return String(
    t(runtimeControlPluralKey(`executionStatus.runtimeControl.action.${path}`, count), {
      count,
      defaultValue: fallback ? (count === 1 ? fallback.one : fallback.other) : path,
    }),
  )
}

function stateLabel(t: TFunction, item: RuntimeControlActivityItem, count: number): string {
  if (item.outcome === "no_action_needed") {
    return String(
      t("executionStatus.runtimeControl.state.noResumeNeeded", {
        defaultValue: "Team already complete · no resume needed",
      }),
    )
  }
  if (item.outcome === "no_targets") {
    return String(
      t("executionStatus.runtimeControl.state.noAgentsToClose", {
        defaultValue: "No agents need closing",
      }),
    )
  }
  if (item.outcome === "already_terminal") {
    if (item.allTargets) {
      return String(
        t("executionStatus.runtimeControl.state.noAgentsToClose", {
          defaultValue: "No agents need closing",
        }),
      )
    }
    return String(
      t(runtimeControlPluralKey(STATE_ALREADY_TERMINAL_KEY, count), {
        count,
        defaultValue:
          count === 1
            ? "{{count}} target was already finished"
            : "{{count}} targets were already finished",
      }),
    )
  }
  return String(
    t(`executionStatus.runtimeControl.state.${item.state}`, {
      action: actionLabel(t, item, count),
      defaultValue: DEFAULT_STATE_LABELS[item.state],
    }),
  )
}

function countItems(items: readonly RuntimeControlActivityItem[]): number {
  return items.length
}

function primaryItems(items: readonly RuntimeControlActivityItem[]): RuntimeControlActivityItem[] {
  const normal = items.filter((item) => item.outcome !== "already_terminal")
  for (const state of STATE_PRIORITY) {
    const matching = normal.filter((item) => item.state === state)
    if (matching.length > 0) return matching
  }
  return items.filter((item) => item.outcome === "already_terminal")
}

function activityIcon(item: RuntimeControlActivityItem): ComponentType<{ className?: string }> {
  return ACTION_ICONS[item.family][item.action] ?? Network
}

function detailOrder(item: RuntimeControlActivityItem): number {
  if (item.state === "failed") return 0
  if (
    item.state === "partial" ||
    item.state === "refused" ||
    (item.aggregate?.refusedCount ?? 0) > 0 ||
    (item.resumeSummary?.failedCount ?? 0) > 0 ||
    (item.resumeSummary?.failures.length ?? 0) > 0
  ) {
    return 1
  }
  return 2
}

function itemHasProblem(item: RuntimeControlActivityItem): boolean {
  return (
    item.state === "failed" ||
    item.state === "partial" ||
    item.state === "refused" ||
    (item.aggregate?.refusedCount ?? 0) > 0 ||
    (item.resumeSummary?.failedCount ?? 0) > 0 ||
    (item.resumeSummary?.failures.length ?? 0) > 0
  )
}

function itemRefusedCount(item: RuntimeControlActivityItem): number {
  const aggregateCount = item.aggregate?.refusedCount ?? 0
  if (aggregateCount > 0) return aggregateCount
  return item.state === "refused" ? 1 : 0
}

function itemFailedCount(item: RuntimeControlActivityItem): number {
  const memberCount = item.resumeSummary?.failedCount ?? 0
  if (memberCount > 0) return memberCount
  return item.state === "failed" ? 1 : 0
}

function expansionStateKey(items: readonly RuntimeControlActivityItem[]): string {
  return items
    .map((item) =>
      [
        item.tool.callId,
        item.state,
        item.outcome ?? "",
        item.aggregate?.requestedCount ?? 0,
        item.aggregate?.terminalCount ?? 0,
        item.aggregate?.pendingCount ?? 0,
        item.aggregate?.refusedCount ?? 0,
        item.resumeSummary?.failedCount ?? 0,
      ].join(":"),
    )
    .join("|")
}

export default function RuntimeControlActivityGroup({
  tools,
  shimmer,
  onOpenDiff,
}: RuntimeControlActivityGroupProps) {
  const { t } = useTranslation()
  const subagentRuns = useContext(SubagentRunsContext)
  const items = useMemo(
    () =>
      tools
        .map((tool) => parseRuntimeControlActivity(tool, { subagentRuns: subagentRuns?.runs }))
        .filter((item): item is RuntimeControlActivityItem => item !== null),
    [subagentRuns?.runs, tools],
  )
  const hasProblem = items.some(itemHasProblem)
  const expansionKey = expansionStateKey(items)
  const [expansionOverride, setExpansionOverride] = useState<{
    key: string
    expanded: boolean
  } | null>(null)
  const expanded = expansionOverride?.key === expansionKey ? expansionOverride.expanded : hasProblem
  const [now, setNow] = useState(() => Date.now())
  const anyRunning = items.some((item) => item.state === "running")
  const hasTimedRunning = items.some(
    (item) => item.state === "running" && item.tool.startedAtMs != null,
  )

  useEffect(() => {
    if (!hasTimedRunning) return
    const timer = window.setInterval(() => setNow(Date.now()), 100)
    return () => window.clearInterval(timer)
  }, [hasTimedRunning])

  const leadingItems = primaryItems(items)
  const leading = leadingItems[0]
  const failedCount = items.reduce((count, item) => count + itemFailedCount(item), 0)
  const refusedCount = items.reduce((count, item) => count + itemRefusedCount(item), 0)
  const alreadyTerminalCount = countItems(
    items.filter((item) => item.outcome === "already_terminal"),
  )
  const totalElapsedMs = useMemo(() => getToolsWallClockMs(tools, now), [now, tools])
  const elapsedText = totalElapsedMs == null ? null : formatDuration(totalElapsedMs)
  const detailItems = useMemo(
    () =>
      items
        .map((item, index) => ({ item, index }))
        .sort((a, b) => detailOrder(a.item) - detailOrder(b.item) || a.index - b.index),
    [items],
  )

  if (!leading) return null
  const headerLabel = stateLabel(t, leading, countItems(leadingItems))
  const groupKey = getRuntimeControlActivityGroupKey(leading)

  return (
    <div className="my-1 text-xs" data-runtime-control-group={groupKey}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-auto w-full justify-start gap-1.5 px-0 py-1 pr-1 text-left font-normal hover:bg-secondary/60"
        onClick={() => setExpansionOverride({ key: expansionKey, expanded: !expanded })}
        aria-expanded={expanded}
      >
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        )}
        <span className="relative h-3.5 w-3.5 shrink-0">
          {createElement(activityIcon(leading), {
            className: "h-3.5 w-3.5 text-muted-foreground",
          })}
          {(anyRunning || shimmer) && (
            <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60 ring-1 ring-card animate-pulse" />
          )}
        </span>
        <span
          role="status"
          aria-live="polite"
          aria-atomic="true"
          className={cn(
            "min-w-0 truncate font-medium text-muted-foreground",
            anyRunning && "animate-text-shimmer",
            leading.state === "failed" && "text-red-500",
            (leading.state === "partial" || leading.state === "refused") &&
              "text-amber-600 dark:text-amber-400",
          )}
        >
          {headerLabel}
        </span>
        {alreadyTerminalCount > 0 && leading.outcome !== "already_terminal" && (
          <span className="shrink-0 rounded-full bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {leading.allTargets
              ? t("executionStatus.runtimeControl.state.noAgentsToClose", {
                  defaultValue: "No agents need closing",
                })
              : t(runtimeControlPluralKey(BADGE_ALREADY_TERMINAL_KEY, alreadyTerminalCount), {
                  count: alreadyTerminalCount,
                  defaultValue: "{{count}} already finished",
                })}
          </span>
        )}
        {refusedCount > 0 && leading.state !== "refused" && (
          <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-600 dark:text-amber-400">
            <CircleSlash2 className="h-3 w-3" />
            {t(runtimeControlPluralKey(BADGE_REFUSED_KEY, refusedCount), {
              count: refusedCount,
              defaultValue: "{{count}} refused",
            })}
          </span>
        )}
        {failedCount > 0 && leading.state !== "failed" && (
          <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-500">
            <AlertCircle className="h-3 w-3" />
            {t("executionStatus.tool.group.failedCount", { count: failedCount })}
          </span>
        )}
        {leading.state === "accepted" && (
          <Clock className="ml-auto h-3 w-3 shrink-0 text-muted-foreground/60" />
        )}
        {leading.state === "completed" &&
          leading.outcome !== "already_terminal" &&
          leading.outcome !== "no_targets" &&
          leading.outcome !== "no_action_needed" && (
            <CheckCircle2 className="ml-auto h-3 w-3 shrink-0 text-green-500/80" />
          )}
        {elapsedText && (
          <span
            className={cn(
              "shrink-0 text-[10px] tabular-nums text-muted-foreground/60",
              leading.state !== "accepted" && leading.state !== "completed" && "ml-auto",
            )}
          >
            {t("tools.elapsed", { time: elapsedText })}
          </span>
        )}
      </Button>

      <MediaHoistContext.Provider value={!expanded}>
        <AnimatedCollapse open={expanded} unmountOnExit={false}>
          <div className="ml-3 border-l border-border/40 pl-1">
            {detailItems.map(({ item }) => {
              const ItemIcon = activityIcon(item)
              const target = item.allTargets
                ? t("executionStatus.runtimeControl.allTargets", "所有 Agent")
                : (item.label ?? item.targetId ?? item.detail ?? "")
              const aggregateStats: Array<[string, number]> = item.aggregate
                ? (
                    [
                      ["requested", item.aggregate.requestedCount],
                      ["terminal", item.aggregate.terminalCount],
                      ["pending", item.aggregate.pendingCount],
                      ["refused", item.aggregate.refusedCount],
                    ] as Array<[string, number]>
                  ).filter(([, count]) => count > 0)
                : []
              const resumeStats: Array<[string, number]> = item.resumeSummary
                ? (
                    [
                      ["resumed", item.resumeSummary.resumedCount],
                      ["failed", item.resumeSummary.failedCount],
                    ] as Array<[string, number]>
                  ).filter(([, count]) => count > 0)
                : []
              const activityStats = [...aggregateStats, ...resumeStats]
              return (
                <div key={item.key}>
                  <ToolCallBlock
                    tool={item.tool}
                    labelOverride={stateLabel(t, item, 1)}
                    displayArgsOverride={String(target)}
                    iconOverride={ItemIcon}
                    onOpenDiff={onOpenDiff}
                  />
                  {activityStats.length > 0 && (
                    <div
                      className="ml-6 flex flex-wrap gap-x-2 gap-y-0.5 pb-1 text-[10px] text-muted-foreground/70"
                      data-runtime-control-aggregate
                    >
                      {activityStats.map(([kind, count]) => (
                        <span
                          key={kind}
                          className={cn(
                            kind === "refused" && count > 0 && "text-amber-600 dark:text-amber-400",
                          )}
                        >
                          {t(
                            runtimeControlPluralKey(
                              `executionStatus.runtimeControl.aggregate.${kind}`,
                              count,
                            ),
                            {
                              count,
                              defaultValue: `{{count}} ${kind}`,
                            },
                          )}
                        </span>
                      ))}
                    </div>
                  )}
                  {(item.resumeSummary?.failures.length ?? 0) > 0 && (
                    <div
                      className="ml-6 space-y-0.5 pb-1 text-[10px] text-amber-700 dark:text-amber-300"
                      data-runtime-control-failures
                    >
                      <div className="font-medium">
                        {t("executionStatus.runtimeControl.failureDetails", "Resume failures")}
                      </div>
                      {item.resumeSummary?.failures.map((failure, index) => (
                        <div key={`${failure.label ?? "failure"}-${index}`}>
                          {[failure.label, failure.reason, failure.status]
                            .filter(Boolean)
                            .join(" · ")}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </AnimatedCollapse>
      </MediaHoistContext.Provider>
      {!expanded &&
        tools
          .filter(toolHasMedia)
          .map((tool) => <ToolMediaPreview key={tool.callId} tool={tool} className="ml-1" />)}
    </div>
  )
}
