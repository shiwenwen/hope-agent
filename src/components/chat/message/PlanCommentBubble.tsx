import { useTranslation } from "react-i18next"
import { MessageSquareQuote } from "lucide-react"

interface PlanCommentBubbleProps {
  selectedText: string
  comment: string
}

/** Custom user bubble for plan inline comments. The IM-fallback markdown
 *  in `messages.content` reads as three loose blocks; on desktop we group
 *  context (quote) + focus (comment) into a single layered card. */
export function PlanCommentBubble({ selectedText, comment }: PlanCommentBubbleProps) {
  const { t } = useTranslation()
  const headerLabel = String(t("planMode.commentDisplay"))

  return (
    <div className="max-w-[95%] overflow-hidden rounded-xl border border-purple-500/20 bg-purple-500/5 shadow-sm">
      <div className="flex items-center gap-1.5 border-b border-purple-500/15 bg-purple-500/[0.07] px-3 py-1.5 text-xs font-medium text-purple-600 dark:text-purple-400">
        <MessageSquareQuote className="h-3.5 w-3.5 shrink-0" />
        <span>{headerLabel}</span>
      </div>
      <div className="space-y-2.5 px-3 py-2.5">
        <div className="rounded-md border-l-2 border-purple-400/60 bg-purple-500/[0.06] px-2.5 py-1.5 text-xs italic leading-relaxed text-muted-foreground whitespace-pre-wrap break-words">
          {selectedText}
        </div>
        <div className="text-sm leading-relaxed text-foreground whitespace-pre-wrap break-words">
          {comment}
        </div>
      </div>
    </div>
  )
}
