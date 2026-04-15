import { useState, useEffect, useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  Users,
  CheckCircle,
  XCircle,
  Loader2,
  ArrowUpRight,
  Paperclip,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import type { AgentSummaryForSidebar, SubagentEvent, SubagentRun } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { IconTip } from "@/components/ui/tooltip"
import {
  loadAgents,
  statusConfig,
  TERMINAL_STATUSES,
  FAILED_STATUSES,
} from "./subagentShared"

export interface SubagentGroupRun {
  runId: string
  agentId: string
  task: string
}

interface SubagentGroupProps {
  runs: SubagentGroupRun[]
  onSwitchSession?: (sessionId: string) => void
}

interface RunState {
  status: string
  resultFull?: string
  error?: string
  durationMs?: number
  label?: string
  modelUsed?: string
  inputTokens?: number
  outputTokens?: number
  attachmentCount?: number
  childSessionId?: string
}

export default function SubagentGroup({ runs, onSwitchSession }: SubagentGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [states, setStates] = useState<Map<string, RunState>>(() => {
    const init = new Map<string, RunState>()
    for (const r of runs) init.set(r.runId, { status: "spawning" })
    return init
  })
  const [agentMetas, setAgentMetas] = useState<Map<string, AgentSummaryForSidebar>>(new Map())
  const [metadataLoaded, setMetadataLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    loadAgents()
      .then((m) => {
        if (cancelled) return
        setAgentMetas(m)
        setMetadataLoaded(true)
      })
      .catch(() => {
        // On failure, still mark loaded so rows fall back to the agentId
        // instead of spinning forever.
        if (!cancelled) setMetadataLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Parent keys this component on the concatenated runIds, so a change in the
  // run set remounts the group and re-runs these effects fresh — safe to use
  // empty deps + closure `runs`.
  useEffect(() => {
    if (runs.length === 0) return
    let cancelled = false
    const runIds = runs.map((r) => r.runId)
    getTransport()
      .call<Record<string, SubagentRun>>("get_subagent_runs_batch", { runIds })
      .then((byId) => {
        if (cancelled || !byId) return
        setStates((prev) => {
          const next = new Map(prev)
          for (const runId of runIds) {
            const run = byId[runId]
            if (!run) continue
            next.set(runId, {
              status: run.status,
              resultFull: run.result,
              error: run.error,
              durationMs: run.durationMs,
              label: run.label,
              modelUsed: run.modelUsed,
              inputTokens: run.inputTokens,
              outputTokens: run.outputTokens,
              attachmentCount: run.attachmentCount,
              childSessionId: run.childSessionId,
            })
          }
          return next
        })
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const runIds = new Set(runs.map((r) => r.runId))
    return getTransport().listen("subagent_event", (raw) => {
      const payload = raw as SubagentEvent
      if (!runIds.has(payload.runId)) return
      setStates((prev) => {
        const next = new Map(prev)
        const cur = next.get(payload.runId) || { status: "spawning" }
        next.set(payload.runId, {
          ...cur,
          status: payload.status,
          resultFull: payload.resultFull ?? cur.resultFull,
          error: payload.error ?? cur.error,
          durationMs: payload.durationMs ?? cur.durationMs,
          label: payload.label ?? cur.label,
          inputTokens: payload.inputTokens ?? cur.inputTokens,
          outputTokens: payload.outputTokens ?? cur.outputTokens,
        })
        return next
      })
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const agg = useMemo(() => {
    let running = 0
    let completed = 0
    let failed = 0
    let totalDurationMs = 0
    let totalInputTokens = 0
    let totalOutputTokens = 0
    for (const run of runs) {
      const s = states.get(run.runId)
      const status = s?.status ?? "spawning"
      if (status === "completed") completed++
      else if (FAILED_STATUSES.has(status)) failed++
      else running++
      if (s?.durationMs) totalDurationMs += s.durationMs
      if (s?.inputTokens) totalInputTokens += s.inputTokens
      if (s?.outputTokens) totalOutputTokens += s.outputTokens
    }
    return {
      running,
      completed,
      failed,
      totalDurationMs,
      totalInputTokens,
      totalOutputTokens,
      total: runs.length,
    }
  }, [runs, states])

  const anyRunning = agg.running > 0
  const headerLabel = anyRunning
    ? t("subagent.group.running", { count: agg.total })
    : t("subagent.group.finished", { count: agg.total })

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/50 text-xs">
      <button
        type="button"
        className="flex items-center gap-1.5 w-full px-2.5 py-1.5 text-left hover:bg-secondary/80 rounded-lg transition-colors"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
      >
        {anyRunning ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90",
            )}
          />
        )}
        <Users className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-foreground shrink-0">{headerLabel}</span>
        {/* Status pills */}
        <div className="flex items-center gap-1.5 shrink-0">
          {agg.completed > 0 && (
            <span className="flex items-center gap-0.5 text-green-500">
              <CheckCircle className="h-3 w-3" />
              {agg.completed}
            </span>
          )}
          {agg.running > 0 && (
            <span className="flex items-center gap-0.5 text-blue-500">
              <Loader2 className="h-3 w-3 animate-spin" />
              {agg.running}
            </span>
          )}
          {agg.failed > 0 && (
            <span className="flex items-center gap-0.5 text-red-500">
              <XCircle className="h-3 w-3" />
              {agg.failed}
            </span>
          )}
        </div>
        <div className="ml-auto flex items-center gap-1.5 shrink-0">
          {(agg.totalInputTokens > 0 || agg.totalOutputTokens > 0) && (
            <span className="text-[10px] text-muted-foreground tabular-nums">
              {agg.totalInputTokens.toLocaleString()}↑ {agg.totalOutputTokens.toLocaleString()}↓
            </span>
          )}
          {/* Aggregate duration only once all runs are terminal — a partial sum
              is neither wall-clock nor per-run elapsed, so it's misleading. */}
          {!anyRunning && agg.totalDurationMs > 0 && (
            <span className="text-muted-foreground tabular-nums">
              {(agg.totalDurationMs / 1000).toFixed(1)}s
            </span>
          )}
        </div>
      </button>

      {/* Expanded rows */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded ? "max-h-[2000px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="border-t border-border/60">
          {runs.map((run) => (
            <SubagentRow
              key={run.runId}
              run={run}
              state={states.get(run.runId)}
              agentMeta={agentMetas.get(run.agentId)}
              agentMetasLoaded={metadataLoaded}
              onSwitchSession={onSwitchSession}
            />
          ))}
        </div>
      </div>
    </div>
  )
}

interface SubagentRowProps {
  run: SubagentGroupRun
  state: RunState | undefined
  agentMeta: AgentSummaryForSidebar | undefined
  agentMetasLoaded: boolean
  onSwitchSession?: (sessionId: string) => void
}

function SubagentRow({
  run,
  state,
  agentMeta,
  agentMetasLoaded,
  onSwitchSession,
}: SubagentRowProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const status = state?.status ?? "spawning"
  const isTerminal = TERMINAL_STATUSES.has(status)
  const config = statusConfig[status] || statusConfig.error
  const friendlyName = state?.label || agentMeta?.name || run.agentId
  const emoji = agentMeta?.emoji?.trim() || null
  // Only mark as missing after the metadata load has resolved
  const agentMissing = agentMetasLoaded && !agentMeta
  const nameTooltip = agentMissing ? t("subagent.deletedAgentTooltip") : undefined
  const hasContent = !!(state?.resultFull || state?.error)
  const canExpand = isTerminal && hasContent
  const childSessionId = state?.childSessionId
  const canViewSession = !!(onSwitchSession && childSessionId)
  const rowInteractive = canExpand || canViewSession

  return (
    <div className="text-[11px]">
      <div
        className={cn(
          "flex items-center transition-colors",
          rowInteractive && "hover:bg-secondary/60",
        )}
      >
        <button
          type="button"
          className="flex items-center gap-1.5 flex-1 min-w-0 px-2.5 py-1 text-left disabled:cursor-default"
          onClick={() => canExpand && setExpanded(!expanded)}
          disabled={!canExpand}
          aria-expanded={canExpand ? expanded : undefined}
        >
          {!isTerminal ? (
            <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0 text-muted-foreground/60" />
          ) : hasContent ? (
            <ChevronRight
              className={cn(
                "h-3 w-3 shrink-0 text-muted-foreground/40 transition-transform duration-150",
                expanded && "rotate-90",
              )}
            />
          ) : (
            <span className="h-3 w-3 shrink-0" />
          )}
          {emoji ? (
            <span className="shrink-0 leading-none" aria-hidden>
              {emoji}
            </span>
          ) : (
            <Users className="h-3 w-3 shrink-0 text-muted-foreground/50" />
          )}
          <IconTip label={nameTooltip || friendlyName}>
            <span className="font-medium text-foreground truncate max-w-[40%]">
              {friendlyName}
            </span>
          </IconTip>
          {state?.attachmentCount !== undefined && state.attachmentCount > 0 && (
            <span className="flex items-center gap-0.5 text-muted-foreground/70 shrink-0">
              <Paperclip className="h-2.5 w-2.5" />
              {state.attachmentCount}
            </span>
          )}
          <span className="text-muted-foreground/70 truncate flex-1 min-w-0">{run.task}</span>
        </button>
        {/* Right action area — sibling of main button to avoid nested interactive elements. */}
        <div className="flex items-center gap-1.5 pr-2 shrink-0">
          <span className={cn("flex items-center gap-0.5", config.color)}>{config.icon}</span>
          {state?.durationMs !== undefined && (
            <span className="text-muted-foreground/60 tabular-nums text-[10px]">
              {(state.durationMs / 1000).toFixed(1)}s
            </span>
          )}
          {state?.inputTokens !== undefined && state?.outputTokens !== undefined && (
            <span className="text-muted-foreground/60 tabular-nums text-[10px]">
              {state.inputTokens.toLocaleString()}↑ {state.outputTokens.toLocaleString()}↓
            </span>
          )}
          {canViewSession && (
            <IconTip label={t("subagent.viewChildSession")}>
              <button
                type="button"
                className="p-0.5 rounded hover:bg-secondary text-muted-foreground/50 hover:text-foreground transition-colors"
                onClick={() => {
                  if (onSwitchSession && childSessionId) onSwitchSession(childSessionId)
                }}
                aria-label={t("subagent.viewChildSession")}
              >
                <ArrowUpRight className="h-3 w-3" />
              </button>
            </IconTip>
          )}
        </div>
      </div>

      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded && hasContent ? "max-h-[600px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="px-2.5 pb-2 pt-0.5 max-h-[600px] overflow-y-auto">
          {state?.error && (
            <pre className="whitespace-pre-wrap text-red-400 bg-background rounded p-2 text-[11px] leading-relaxed">
              {state.error}
            </pre>
          )}
          {state?.resultFull && (
            <div className="bg-background rounded p-2 text-[11px] leading-relaxed">
              <MarkdownRenderer content={state.resultFull} />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
