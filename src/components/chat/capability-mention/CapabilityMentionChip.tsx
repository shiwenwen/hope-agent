import { Cable, Puzzle } from "lucide-react"

export const CAPABILITY_MENTION_INLINE_CLASS =
  "mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 whitespace-nowrap align-baseline font-normal leading-[inherit] text-cyan-700 dark:text-cyan-300"
export const CAPABILITY_MENTION_ICON_CLASS = "h-[1em] w-[1em] shrink-0 self-center"

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
      className={CAPABILITY_MENTION_INLINE_CLASS}
    >
      <Icon className={CAPABILITY_MENTION_ICON_CLASS} />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
