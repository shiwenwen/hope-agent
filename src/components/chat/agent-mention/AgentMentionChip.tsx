import { useEffect, useState } from "react"
import { Bot } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { AgentSummaryForSidebar } from "@/types/chat"
import { loadAgents } from "../subagentShared"

export const AGENT_MENTION_INLINE_CLASS =
  "mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 whitespace-nowrap align-baseline font-normal leading-[inherit] text-teal-700 dark:text-teal-300"
export const AGENT_MENTION_ICON_CLASS = "h-[1em] w-[1em] shrink-0 self-center"

/** Keep an Agent's configured identity visible without restoring the old
 * framed avatar badge: avatar first, then emoji, then the fixed Bot fallback. */
export function AgentMentionIcon({
  agent,
  className,
}: {
  agent?: AgentSummaryForSidebar | null
  className?: string
}) {
  const avatarUrl = agent?.avatar
    ? (getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar)
    : null
  const iconClass = cn(AGENT_MENTION_ICON_CLASS, className)

  if (avatarUrl) {
    return <img src={avatarUrl} className={cn(iconClass, "rounded-full object-cover")} alt="" />
  }
  if (agent?.emoji) {
    return (
      <span
        aria-hidden
        className={cn(iconClass, "inline-flex items-center justify-center leading-none")}
      >
        {agent.emoji}
      </span>
    )
  }
  return <Bot className={iconClass} />
}

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
      className={AGENT_MENTION_INLINE_CLASS}
    >
      <AgentMentionIcon agent={agent} />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
