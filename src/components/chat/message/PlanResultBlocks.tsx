import { useState, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { Check, ChevronRight, ClipboardList, FolderOpen, PanelRight } from "lucide-react"

/** Collapsible Q&A summary for ask_user_question tool results */
export function AskUserQuestionResult({ result }: { result: string }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const items = useMemo(() => {
    try {
      const data = JSON.parse(result) as {
        answers: Array<{ question: string; selected: string[]; customInput?: string }>
      }
      return data.answers || []
    } catch {
      return []
    }
  }, [result])

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

/** Compact inline card for submit_plan tool results */
export function SubmitPlanResult({
  title,
  sessionId,
  onOpenPanel,
}: {
  title: string
  sessionId?: string | null
  onOpenPanel?: () => void
}) {
  const { t } = useTranslation()

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
        <PanelRight className="h-3.5 w-3.5 text-muted-foreground" />
        <button
          onClick={(e) => { e.stopPropagation(); handleRevealFile() }}
          className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
        >
          <FolderOpen className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  )
}
