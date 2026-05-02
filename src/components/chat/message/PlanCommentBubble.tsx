import { useTranslation } from "react-i18next"
import { MessageSquareQuote } from "lucide-react"

interface PlanCommentBubbleProps {
  selectedText: string
  comment: string
}

/** Custom user-bubble renderer for plan inline-comment messages.
 *
 * Layered visual structure (top → bottom):
 *   1. Header chip — purple icon + "Plan comment" label, signals this is
 *      a meta-action on the plan rather than an ordinary chat turn.
 *   2. Quoted selection — left purple bar + soft purple background, italic
 *      muted text. The bar mirrors a markdown blockquote but stays inside
 *      the bubble's visual frame instead of bleeding to the page edge.
 *   3. Comment body — the user's actual words at normal size, no styling
 *      tricks. This is the focal point.
 *
 * Why a dedicated component instead of markdown: the markdown displayText
 * (used for IM channels) renders as three loose blocks; on desktop we have
 * the layout budget to show context (quote) + focus (comment) as a single
 * cohesive card. Color palette tracks the plan-mode purple used by
 * `SubmitPlanResult` and the `ClipboardList` icon throughout the panel. */
export function PlanCommentBubble({ selectedText, comment }: PlanCommentBubbleProps) {
  const { t } = useTranslation()
  const headerLabel = String(t("planMode.commentDisplay"))

  return (
    <div className="max-w-[95%] overflow-hidden rounded-xl border border-purple-500/20 bg-purple-500/5 shadow-sm">
      {/* Header chip */}
      <div className="flex items-center gap-1.5 border-b border-purple-500/15 bg-purple-500/[0.07] px-3 py-1.5 text-xs font-medium text-purple-600 dark:text-purple-400">
        <MessageSquareQuote className="h-3.5 w-3.5 shrink-0" />
        <span>{headerLabel}</span>
      </div>
      {/* Body */}
      <div className="space-y-2.5 px-3 py-2.5">
        {/* Quoted selection — soft background + left bar so it reads as
            "context the user is responding to" without dominating. */}
        <div className="rounded-md border-l-2 border-purple-400/60 bg-purple-500/[0.06] px-2.5 py-1.5 text-xs italic leading-relaxed text-muted-foreground whitespace-pre-wrap break-words">
          {selectedText}
        </div>
        {/* Comment body — the user's actual words. Normal size, no italics,
            preserved whitespace so multi-line replies render cleanly. */}
        <div className="text-sm leading-relaxed text-foreground whitespace-pre-wrap break-words">
          {comment}
        </div>
      </div>
    </div>
  )
}
