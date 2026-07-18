import { memo, useContext, useMemo } from "react"
import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { AgentAvatarBadge, type AgentSelectAgent } from "@/components/common/AgentSelectDisplay"
import type { SubagentRun } from "@/types/chat"
import { formatDuration } from "../chatUtils"
import { statusConfig, useAgentsMap } from "../subagentShared"
import { SubagentRunsContext } from "./useSubagentRuns"
import { matchPendingRun, type SubagentChipItem, type SubagentOpenTarget } from "./subagentRunModel"

const EMPTY_RUNS: SubagentRun[] = []

interface ResolvedChip {
  key: string
  runId: string | null
  agentId: string
  task: string
  label?: string
}

interface SubagentChipProps {
  runId: string | null
  childSessionId?: string
  agentId: string
  task: string
  label?: string
  /** Resolved agent metadata — drives avatar → emoji → tinted-icon fallback. */
  agent?: AgentSelectAgent | null
  name: string
  status: string
  durationMs?: number
  /** The run row is genuinely gone (cleaned-up history) — render muted + inert. */
  missing: boolean
  onOpen?: (target: SubagentOpenTarget) => void
}

/** Capsule shell shared by the live and the missing-record variants. */
const CHIP_SHELL =
  "inline-flex max-w-full items-center gap-1.5 rounded-full bg-secondary/60 py-1 pl-1 pr-2.5 text-xs"

const SubagentChip = memo(function SubagentChip({
  runId,
  childSessionId,
  agentId,
  task,
  label,
  agent,
  name,
  status,
  durationMs,
  missing,
  onOpen,
}: SubagentChipProps) {
  const { t } = useTranslation()

  if (missing) {
    return (
      <IconTip label={t("subagentPanel.runMissing", "Run record not found")}>
        <span className={cn(CHIP_SHELL, "opacity-60")}>
          <AgentAvatarBadge agent={agent} size="xs" colorSeed={agentId} />
          <span className="max-w-[160px] truncate font-medium text-muted-foreground">{name}</span>
        </span>
      </IconTip>
    )
  }

  const config = statusConfig[status] || statusConfig.error
  const durationLabel = durationMs != null ? formatDuration(durationMs) : null

  return (
    <IconTip label={task || name}>
      <button
        type="button"
        disabled={!onOpen}
        onClick={onOpen ? () => onOpen({ runId, childSessionId, agentId, task, label }) : undefined}
        className={cn(
          CHIP_SHELL,
          "transition-colors",
          onOpen ? "hover:bg-secondary" : "cursor-default",
        )}
      >
        <AgentAvatarBadge agent={agent} size="xs" colorSeed={agentId} />
        <span className="max-w-[160px] truncate font-medium text-foreground">{name}</span>
        <span className={cn("flex shrink-0 items-center gap-1", config.color)}>
          {config.icon}
          <span>{t(`executionStatus.subagent.status.${status}`, status)}</span>
        </span>
        {durationLabel && (
          <span className="shrink-0 tabular-nums text-muted-foreground">{durationLabel}</span>
        )}
      </button>
    </IconTip>
  )
})

/** Inline row of sub-agent chips for one (or a merged run of consecutive)
 *  `subagent` spawn tool block(s). Reads the session's live run snapshot from
 *  context to drive per-chip status + click-to-open-panel. */
function SubagentChipRow({ items }: { items: SubagentChipItem[] }) {
  const ctx = useContext(SubagentRunsContext)
  const agentsMap = useAgentsMap()
  const runs = ctx?.runs ?? EMPTY_RUNS

  // Resolve each item to a concrete runId: resolved items use theirs; pending
  // items align to a live run (each claiming a distinct one within this row).
  const chips = useMemo<ResolvedChip[]>(() => {
    const claimed = new Set<string>()
    for (const it of items) if (it.kind === "resolved") claimed.add(it.runId)
    return items.map((it) => {
      if (it.kind === "resolved") {
        return {
          key: it.key,
          runId: it.runId,
          agentId: it.agentId,
          task: it.task,
          label: it.label,
        }
      }
      const match = matchPendingRun(
        runs,
        { agentId: it.agentId, task: it.task },
        it.startedAtMs,
        claimed,
      )
      if (match) {
        claimed.add(match.runId)
        return {
          key: it.key,
          runId: match.runId,
          agentId: it.agentId,
          task: it.task,
          label: it.label,
        }
      }
      return { key: it.key, runId: null, agentId: it.agentId, task: it.task, label: it.label }
    })
  }, [items, runs])

  const loaded = ctx?.loaded ?? false

  return (
    <div className="my-1.5 flex min-w-0 flex-wrap items-center gap-1.5">
      {chips.map((chip) => {
        const run = chip.runId ? ctx?.byId.get(chip.runId) : undefined
        const meta = agentsMap.get(chip.agentId)
        const label = chip.label ?? run?.label
        return (
          <SubagentChip
            key={chip.key}
            runId={chip.runId}
            childSessionId={run?.childSessionId}
            agentId={chip.agentId}
            task={chip.task}
            label={label}
            agent={meta ?? { id: chip.agentId }}
            name={label ?? meta?.name ?? chip.agentId}
            status={run?.status ?? "spawning"}
            durationMs={run?.durationMs}
            missing={chip.runId != null && loaded && !run}
            onOpen={chip.runId ? ctx?.openRun : undefined}
          />
        )
      })}
    </div>
  )
}

export default memo(SubagentChipRow)
