import { Cable, Puzzle } from "lucide-react"

import { cn } from "@/lib/utils"

export function CapabilityMentionChip({
  kind,
  targetId,
  fallbackName,
}: {
  kind: "plugin" | "connector"
  targetId: string
  fallbackName?: string
}) {
  const label = fallbackName || targetId
  const Icon = kind === "plugin" ? Puzzle : Cable

  return (
    <span
      data-capability-mention={targetId}
      data-capability-kind={kind}
      data-ha-title-tip={label}
      className={cn(
        "mx-0.5 inline-flex max-w-[16rem] items-center gap-1 rounded-md border px-1.5 align-baseline",
        "text-[0.95em] font-medium leading-snug",
        "border-cyan-500/20 bg-cyan-500/10 text-cyan-700",
        "dark:border-cyan-300/20 dark:bg-cyan-300/15 dark:text-cyan-200",
      )}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
