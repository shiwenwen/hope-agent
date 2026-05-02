import { Bot } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

export interface AgentSelectAgent {
  id?: string
  name?: string | null
  emoji?: string | null
  avatar?: string | null
}

export const INHERIT_AGENT_SENTINEL = "__inherit_global_default_agent__"

type AgentAvatarSize = "xs" | "sm" | "md" | "lg"

const avatarSizeClasses: Record<AgentAvatarSize, string> = {
  xs: "h-4 w-4 text-[9px]",
  sm: "h-5 w-5 text-[10px]",
  md: "h-6 w-6 text-xs",
  lg: "h-9 w-9 text-base",
}

const botSizeClasses: Record<AgentAvatarSize, string> = {
  xs: "h-2.5 w-2.5",
  sm: "h-3 w-3",
  md: "h-3.5 w-3.5",
  lg: "h-5 w-5",
}

export function AgentAvatarBadge({
  agent,
  size = "sm",
  className,
}: {
  agent?: AgentSelectAgent | null
  size?: AgentAvatarSize
  className?: string
}) {
  const avatarUrl = agent?.avatar
    ? (getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar)
    : null

  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-primary/15 text-primary",
        avatarSizeClasses[size],
        className,
      )}
    >
      {avatarUrl ? (
        <img src={avatarUrl} className="h-full w-full object-cover" alt="" />
      ) : agent?.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className={cn(botSizeClasses[size], "text-muted-foreground")} />
      )}
    </span>
  )
}

export function AgentSelectDisplay({
  agent,
  fallbackName,
  size = "sm",
  className,
}: {
  agent?: AgentSelectAgent | null
  fallbackName?: string
  size?: AgentAvatarSize
  className?: string
}) {
  const name = agent?.name || fallbackName || agent?.id || ""

  return (
    <span className={cn("!inline-flex min-w-0 items-center gap-2", className)}>
      <AgentAvatarBadge agent={agent} size={size} />
      <span className="truncate">{name}</span>
    </span>
  )
}

export function InheritAgentSelectDisplay({
  label,
  className,
}: {
  label: string
  className?: string
}) {
  return (
    <span className={cn("truncate text-muted-foreground", className)}>
      {label}
    </span>
  )
}
