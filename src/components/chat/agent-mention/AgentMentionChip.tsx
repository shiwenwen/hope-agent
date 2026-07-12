import { useEffect, useState } from "react"

import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import { cn } from "@/lib/utils"
import type { AgentSummaryForSidebar } from "@/types/chat"
import { loadAgents } from "../subagentShared"

export function AgentMentionChip({
  agentId,
  fallbackName,
}: {
  agentId: string
  fallbackName?: string
}) {
  const [agent, setAgent] = useState<AgentSummaryForSidebar | null>(null)

  useEffect(() => {
    let cancelled = false
    loadAgents()
      .then((agents) => {
        if (!cancelled) setAgent(agents.get(agentId) ?? null)
      })
      .catch(() => {
        if (!cancelled) setAgent(null)
      })
    return () => {
      cancelled = true
    }
  }, [agentId])

  const label = agent?.name || fallbackName || agentId

  return (
    <span
      data-agent-mention={agentId}
      data-ha-title-tip={label}
      className={cn(
        "mx-0.5 inline-flex items-center gap-1 rounded-md border px-1.5 align-baseline",
        "text-[0.95em] font-medium leading-snug",
        "border-teal-500/20 bg-teal-500/10 text-teal-700",
        "dark:border-teal-300/20 dark:bg-teal-300/15 dark:text-teal-200",
      )}
    >
      <AgentSelectDisplay
        agent={agent ?? { id: agentId, name: label }}
        fallbackName={label}
        size="xs"
        className="gap-1"
      />
    </span>
  )
}
