import { ClipboardList } from "lucide-react"

export const PLAN_MENTION_INLINE_CLASS =
  "mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 whitespace-nowrap align-baseline font-normal leading-[inherit] text-amber-700 dark:text-amber-300"
export const PLAN_MENTION_ICON_CLASS = "h-[1em] w-[1em] shrink-0 self-center"

export function PlanMentionChip({
  targetId,
  displayLabel,
}: {
  targetId: string
  displayLabel: string
}) {
  return (
    <span
      data-plan-mention={targetId}
      data-ha-title-tip={displayLabel}
      className={PLAN_MENTION_INLINE_CLASS}
    >
      <ClipboardList className={PLAN_MENTION_ICON_CLASS} />
      <span className="min-w-0 truncate">{displayLabel}</span>
    </span>
  )
}
