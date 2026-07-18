import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"
import { formatRelativeTime } from "@/lib/format"
import { AgentAvatarBadge, type AgentSelectAgent } from "@/components/common/AgentSelectDisplay"
import type { SubagentRun } from "@/types/chat"
import { formatDuration } from "../chatUtils"
import { statusConfig, TERMINAL_STATUSES } from "../subagentShared"
import { markdownPreview } from "./subagentRunModel"

export function SubagentStatusBadge({ status }: { status: string }) {
  const { t } = useTranslation()
  const config = statusConfig[status] || statusConfig.error
  return (
    <span className={cn("flex shrink-0 items-center gap-1", config.color)}>
      {config.icon}
      <span>{t(`executionStatus.subagent.status.${status}`, status)}</span>
    </span>
  )
}

/** One sub-agent run as a two-line list row — shared by the sub-agent panel's
 *  run list and the workspace panel's sub-agent section.
 *
 *  The secondary line is status-aware: a finished run shows what it PRODUCED
 *  (its result), a live one shows what it was ASKED to do — a running agent has
 *  no result yet, and a finished one's task is usually the less useful half. */
export function SubagentRunRow({
  run,
  agent,
  selected = false,
  onClick,
}: {
  run: SubagentRun
  agent?: AgentSelectAgent | null
  selected?: boolean
  onClick?: () => void
}) {
  const { i18n } = useTranslation()
  const name = run.label || agent?.name || run.childAgentId
  const terminal = TERMINAL_STATUSES.has(run.status)
  // A result that is nothing but code strips to empty — fall back to the task
  // rather than showing a blank line.
  const preview =
    (terminal && run.result ? markdownPreview(run.result) : "") || markdownPreview(run.task || "")
  // Finished runs read better as "when" (they may be days old); live ones as
  // "what state" — their elapsed time is still moving.
  const stamp = terminal
    ? run.finishedAt
      ? formatRelativeTime(run.finishedAt, i18n.language)
      : run.durationMs != null
        ? formatDuration(run.durationMs)
        : null
    : null

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className={cn(
        "flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left transition-colors",
        selected ? "bg-secondary/70" : onClick ? "hover:bg-secondary/40" : "cursor-default",
      )}
    >
      <AgentAvatarBadge
        agent={agent ?? { id: run.childAgentId }}
        size="sm"
        colorSeed={run.childAgentId}
        className="mt-0.5"
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="min-w-0 truncate text-xs font-medium text-foreground">{name}</span>
          <span className="ml-auto shrink-0 text-[11px] tabular-nums text-muted-foreground">
            {stamp}
          </span>
        </span>
        <span className="mt-0.5 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
            {preview}
          </span>
          {!terminal && <SubagentStatusBadge status={run.status} />}
        </span>
      </span>
    </button>
  )
}
