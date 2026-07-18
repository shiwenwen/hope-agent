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

// Stable per-agent tints for the icon fallback (no avatar, no emoji), so each
// agent keeps the same colour everywhere instead of a uniform grey Bot.
const FALLBACK_TINTS = [
  "bg-rose-500/15 text-rose-600 dark:text-rose-300",
  "bg-amber-500/15 text-amber-600 dark:text-amber-300",
  "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300",
  "bg-sky-500/15 text-sky-600 dark:text-sky-300",
  "bg-violet-500/15 text-violet-600 dark:text-violet-300",
  "bg-fuchsia-500/15 text-fuchsia-600 dark:text-fuchsia-300",
  "bg-teal-500/15 text-teal-600 dark:text-teal-300",
  "bg-orange-500/15 text-orange-600 dark:text-orange-300",
]

function fallbackTint(seed: string): string {
  let hash = 0
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0
  return FALLBACK_TINTS[hash % FALLBACK_TINTS.length]
}

export function AgentAvatarBadge({
  agent,
  size = "sm",
  className,
  colorSeed,
}: {
  agent?: AgentSelectAgent | null
  size?: AgentAvatarSize
  className?: string
  /** Opt in to a stable colour for the icon fallback, derived from this seed
   *  (usually the agent id). Omit to keep the neutral default. */
  colorSeed?: string | null
}) {
  const avatarUrl = agent?.avatar
    ? (getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar)
    : null
  // Only the icon fallback is tinted — a real avatar or emoji carries its own colour.
  const tint = !avatarUrl && !agent?.emoji && colorSeed ? fallbackTint(colorSeed) : null

  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center overflow-hidden rounded-full",
        tint ?? "bg-primary/15 text-primary",
        avatarSizeClasses[size],
        className,
      )}
    >
      {avatarUrl ? (
        <img src={avatarUrl} className="h-full w-full object-cover" alt="" />
      ) : agent?.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className={cn(botSizeClasses[size], !tint && "text-muted-foreground")} />
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
  return <span className={cn("truncate text-muted-foreground", className)}>{label}</span>
}
