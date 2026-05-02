import { useState, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"
import {
  Check,
  ChevronRight,
  ClipboardList,
  FolderOpen,
  MessageCircleQuestion,
  PanelRight,
} from "lucide-react"

/** Collapsible Q&A summary for ask_user_question tool results.
 *
 * `pending=true` is rendered while the tool is still in flight (no result yet),
 * so the user sees that the model has dispatched a question instead of staring
 * at an empty bubble. The actual interaction happens in the AskUserDialog,
 * which is wired to a separate event channel — this card is purely a passive
 * indicator on the message timeline. */
export function AskUserQuestionResult({
  result,
  pending = false,
}: {
  result?: string
  pending?: boolean
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const items = useMemo(() => {
    if (!result) return []
    try {
      const data = JSON.parse(result) as {
        answers: Array<{ question: string; selected: string[]; customInput?: string }>
      }
      return data.answers || []
    } catch {
      return []
    }
  }, [result])

  if (pending) {
    // Use the same shimmer style as ToolCallBlock running state for visual
    // consistency — the rest of the app uses animate-pulse / text-shimmer
    // for in-flight indicators, never spinner. The icon pulses subtly while
    // the label text gets the sweeping shimmer treatment.
    return (
      <div className="my-2 flex items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-2.5 text-sm text-amber-700 dark:text-amber-400">
        <MessageCircleQuestion className="h-4 w-4 shrink-0 animate-pulse" />
        <span className="font-medium animate-text-shimmer">{t("planMode.question.pending")}</span>
      </div>
    )
  }

  if (items.length === 0) return null

  return (
    <div className="my-2 rounded-lg border border-green-500/20 bg-green-500/5">
      <button
        className="flex items-center gap-2 w-full px-4 py-2.5 text-sm text-green-600 hover:bg-green-500/5 transition-colors cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Check className="h-4 w-4" />
        <span className="font-medium">{t("planMode.question.answered")}</span>
      </button>
      {expanded && (
        <div className="px-4 pb-3 space-y-2 border-t border-green-500/10 pt-2">
          {items.map((item, i) => (
            <div key={i} className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground">{item.question}</span>
              <div className="mt-0.5 pl-2">
                {item.selected.map((s, j) => (
                  <div key={j}>- {s}</div>
                ))}
                {item.customInput && <div>- {item.customInput}</div>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/** Compact inline card for submit_plan tool calls.
 *
 * `pending=true` renders a shimmer chip while the tool is in flight. The
 * normal card (with reveal + open-panel buttons) only appears once the model
 * has actually written and saved the plan file. This is the user's primary
 * feedback that the plan write is happening — without it, the bubble used to
 * be completely empty between dispatch and result. */
export function SubmitPlanResult({
  title,
  sessionId,
  onOpenPanel,
  pending = false,
}: {
  title: string
  sessionId?: string | null
  onOpenPanel?: () => void
  pending?: boolean
}) {
  const { t } = useTranslation()

  if (pending) {
    // Shimmer-style indicator (same as ToolCallBlock / ThinkingBlock running
    // state) — keeps the in-flight visual language consistent across the
    // whole bubble.
    return (
      <div className="my-2 flex items-center gap-2 rounded-lg border border-purple-500/20 bg-purple-500/5 px-4 py-2.5 text-sm text-purple-700 dark:text-purple-400">
        <ClipboardList className="h-4 w-4 shrink-0 animate-pulse" />
        <span className="font-medium truncate flex-1 animate-text-shimmer">
          {title || t("planMode.submittingPlan")}
        </span>
      </div>
    )
  }

  const handleRevealFile = async () => {
    if (!sessionId) return
    try {
      const filePath = await getTransport().call<string | null>("get_plan_file_path", { sessionId })
      if (filePath) {
        await getTransport().call("reveal_in_folder", { path: filePath })
      }
    } catch { /* ignore */ }
  }

  return (
    <div
      className="my-2 rounded-lg border border-purple-500/20 bg-purple-500/5 px-4 py-3 flex items-center gap-3 cursor-pointer hover:bg-purple-500/10 transition-colors"
      onClick={onOpenPanel}
    >
      <ClipboardList className="h-4 w-4 text-purple-600 shrink-0" />
      <span className="text-sm font-medium truncate flex-1">
        {title || t("planMode.panelTitle")}
      </span>
      <div className="flex items-center gap-1.5 shrink-0">
        <IconTip label={t("planMode.openPanel")}>
          <button
            onClick={(e) => { e.stopPropagation(); onOpenPanel?.() }}
            className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
          >
            <PanelRight className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        <IconTip label={t("chat.revealInFolder")}>
          <button
            onClick={(e) => { e.stopPropagation(); handleRevealFile() }}
            className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
          >
            <FolderOpen className="h-3.5 w-3.5" />
          </button>
        </IconTip>
      </div>
    </div>
  )
}
