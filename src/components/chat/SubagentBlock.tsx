import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { ChevronRight, Users, Paperclip } from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import type { AgentSummaryForSidebar, SubagentEvent, SubagentRun } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { loadAgents, statusConfig, TERMINAL_STATUSES } from "./subagentShared"

interface SubagentBlockProps {
  runId: string
  agentId: string
  task: string
  initialStatus?: string
}

export default function SubagentBlock({ runId, agentId, task, initialStatus }: SubagentBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [status, setStatus] = useState(initialStatus || "spawning")
  const [resultFull, setResultFull] = useState<string | undefined>()
  const [error, setError] = useState<string | undefined>()
  const [durationMs, setDurationMs] = useState<number | undefined>()
  const [label, setLabel] = useState<string | undefined>()
  const [modelUsed, setModelUsed] = useState<string | undefined>()
  const [inputTokens, setInputTokens] = useState<number | undefined>()
  const [outputTokens, setOutputTokens] = useState<number | undefined>()
  const [attachmentCount, setAttachmentCount] = useState(0)
  const [agentMeta, setAgentMeta] = useState<AgentSummaryForSidebar | undefined>()
  const [agentMissing, setAgentMissing] = useState(false)

  // Resolve agentId → friendly name + emoji via shared cache
  useEffect(() => {
    let cancelled = false
    loadAgents()
      .then((m) => {
        if (cancelled) return
        const meta = m.get(agentId)
        setAgentMeta(meta)
        setAgentMissing(!meta)
      })
      .catch(() => {
        /* keep fallback to agentId */
      })
    return () => {
      cancelled = true
    }
  }, [agentId])

  // Hydrate from DB on mount (handles re-mount after switching sessions)
  useEffect(() => {
    getTransport().call<SubagentRun | null>("get_subagent_run", { runId })
      .then((run) => {
        if (!run) return
        setStatus(run.status)
        if (run.result) setResultFull(run.result)
        if (run.error) setError(run.error)
        if (run.durationMs) setDurationMs(run.durationMs)
        if (run.label) setLabel(run.label)
        if (run.modelUsed) setModelUsed(run.modelUsed)
        if (run.inputTokens) setInputTokens(run.inputTokens)
        if (run.outputTokens) setOutputTokens(run.outputTokens)
        if (run.attachmentCount) setAttachmentCount(run.attachmentCount)
      })
      .catch(() => {})
  }, [runId])

  // Live updates via transport events
  useEffect(() => {
    return getTransport().listen("subagent_event", (raw) => {
      const payload = raw as SubagentEvent
      if (payload.runId !== runId) return
      setStatus(payload.status)
      if (payload.resultFull) setResultFull(payload.resultFull)
      if (payload.error) setError(payload.error)
      if (payload.durationMs) setDurationMs(payload.durationMs)
      if (payload.label) setLabel(payload.label)
      if (payload.inputTokens) setInputTokens(payload.inputTokens)
      if (payload.outputTokens) setOutputTokens(payload.outputTokens)
    })
  }, [runId])

  const isTerminal = TERMINAL_STATUSES.has(status)
  const config = statusConfig[status] || statusConfig.error
  const toolLabel = t("tools.subagent")
  const friendlyName = label || agentMeta?.name || agentId
  const emoji = agentMeta?.emoji?.trim() || null
  const nameTooltip = agentMissing ? t("subagent.deletedAgentTooltip") : undefined

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/50 text-xs">
      <button
        type="button"
        className="flex items-center gap-1.5 w-full px-2.5 py-1.5 text-left hover:bg-secondary/80 rounded-lg transition-colors disabled:hover:bg-transparent disabled:cursor-default"
        onClick={() => isTerminal && setExpanded(!expanded)}
        disabled={!isTerminal}
        aria-expanded={isTerminal ? expanded : undefined}
      >
        {!isTerminal ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90",
            )}
          />
        )}
        {emoji ? (
          <span className="shrink-0 leading-none" aria-hidden>
            {emoji}
          </span>
        ) : (
          <Users className="h-3 w-3 shrink-0 text-muted-foreground" />
        )}
        <span
          className="font-medium text-foreground truncate max-w-[40%]"
          title={nameTooltip || friendlyName}
        >
          {friendlyName}
        </span>
        <span className="text-[10px] text-muted-foreground shrink-0 hidden sm:inline">
          {toolLabel}
        </span>
        {attachmentCount > 0 && (
          <span className="flex items-center gap-0.5 text-muted-foreground">
            <Paperclip className="h-2.5 w-2.5" />
            {attachmentCount}
          </span>
        )}
        <span className="text-muted-foreground truncate flex-1 min-w-0">{task}</span>
        <span
          className={cn("flex items-center gap-1 transition-colors duration-200", config.color)}
        >
          {config.icon}
          <span>{config.label}</span>
        </span>
        {durationMs !== undefined && (
          <span className="text-muted-foreground">{(durationMs / 1000).toFixed(1)}s</span>
        )}
      </button>
      {/* Stats bar for terminal states */}
      {isTerminal && (modelUsed || inputTokens !== undefined) && (
        <div className="flex items-center gap-2 px-2.5 pb-1 text-[10px] text-muted-foreground">
          {modelUsed && <span>{modelUsed}</span>}
          {inputTokens !== undefined && outputTokens !== undefined && (
            <span>{inputTokens.toLocaleString()}↑ {outputTokens.toLocaleString()}↓</span>
          )}
        </div>
      )}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded && (resultFull || error) ? "max-h-96 opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="px-2.5 pb-2 pt-0.5 max-h-96 overflow-y-auto">
          {error && (
            <pre className="whitespace-pre-wrap text-red-400 bg-background rounded p-2 text-[11px] leading-relaxed">
              {error}
            </pre>
          )}
          {resultFull && (
            <div className="bg-background rounded p-2 text-[11px] leading-relaxed">
              <MarkdownRenderer content={resultFull} />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
