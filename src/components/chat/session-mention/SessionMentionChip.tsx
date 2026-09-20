import { MessageCircle } from "lucide-react"

export const SESSION_MENTION_INLINE_CLASS =
  "mx-0.5 inline-flex max-w-[20rem] items-baseline gap-1 whitespace-nowrap align-baseline font-normal leading-[inherit] text-blue-700 dark:text-blue-300"
export const SESSION_MENTION_ICON_CLASS = "h-[1em] w-[1em] shrink-0 self-center"

export function SessionMentionChip({
  sessionId,
  fallbackName,
}: {
  sessionId: string
  fallbackName?: string
}) {
  const label = fallbackName || sessionId

  return (
    <span
      data-session-mention={sessionId}
      data-ha-title-tip={label}
      className={SESSION_MENTION_INLINE_CLASS}
    >
      <MessageCircle className={SESSION_MENTION_ICON_CLASS} />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
