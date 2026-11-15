import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { researchActivityKey } from "./useAssistantResearch"
import type {
  AssistantFinding,
  AssistantResearchState,
  AssistantSource,
  ResearchActivity,
  ResearchQueryReceipt,
} from "./useAssistantResearch"

/** This body is an owner projection. It is never appended to chat messages. */
export function AssistantFindingEntry({
  finding,
  sources,
  name,
  onView,
  onAsk,
  activity,
}: {
  finding: AssistantFinding
  sources: AssistantSource[]
  name: string
  onView: AssistantResearchState["viewFinding"]
  onAsk: AssistantResearchState["askFinding"]
  activity: ResearchActivity[]
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [question, setQuestion] = useState("")
  const [busy, setBusy] = useState(false)
  const [receipt, setReceipt] = useState<ResearchQueryReceipt | null>(null)
  const query = activity.find((entry) => entry.id === receipt?.id) ?? receipt
  const [error, setError] = useState<string | null>(null)
  const open = async () => {
    setExpanded(true)
    if (finding.viewedAt || busy) return
    setBusy(true)
    try {
      await onView(finding.id, finding.revision)
    } catch (cause) {
      setError(String(cause))
    } finally {
      setBusy(false)
    }
  }
  const ask = async () => {
    if (!question.trim() || busy) return
    setBusy(true)
    setError(null)
    try {
      const accepted = await onAsk(finding.id, finding.revision, question.trim())
      setQuestion("")
      setReceipt(accepted)
    } catch (cause) {
      setError(String(cause))
    } finally {
      setBusy(false)
    }
  }
  return (
    <article
      className="my-3 rounded-lg bg-muted/30 px-4 py-3 text-sm"
      data-assistant-finding={finding.id}
    >
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>{name}</span>
        <span>{t("personalAssistant.research.finding")}</span>
        <time dateTime={finding.createdAt}>{new Date(finding.createdAt).toLocaleString()}</time>
      </div>
      <p
        className={
          expanded ? "mt-1 whitespace-pre-wrap break-words" : "mt-1 line-clamp-2 break-words"
        }
      >
        {finding.fact}
      </p>
      {expanded && (
        <div className="mt-3 space-y-3">
          {(
            [
              ["inference", finding.inference],
              ["uncertainty", finding.uncertainty],
              ["prepared", finding.prepared],
            ] as const
          ).map(
            ([kind, body]) =>
              body && (
                <div key={kind}>
                  <p className="text-xs font-medium text-muted-foreground">
                    {t(`personalAssistant.research.${kind}`)}
                  </p>
                  <p className="mt-1 whitespace-pre-wrap break-words">{body}</p>
                </div>
              ),
          )}
          <ul className="space-y-1 text-xs text-muted-foreground">
            {finding.references.map((reference, index) => (
              <li key={`${reference.sourceId}:${index}`}>
                {sources.find((source) => source.id === reference.sourceId)?.relativePath ??
                  reference.sourceId.slice(0, 8)}
                {" · "}
                {t("personalAssistant.research.lines", {
                  start: reference.startLine,
                  end: reference.endLine,
                })}
                {" · "}
                {reference.version.slice(0, 10)}
              </li>
            ))}
          </ul>
          <form
            className="space-y-2"
            onSubmit={(event) => {
              event.preventDefault()
              void ask()
            }}
          >
            <Textarea
              value={question}
              maxLength={8_000}
              disabled={busy}
              aria-label={t("personalAssistant.research.ask")}
              placeholder={t("personalAssistant.research.askPlaceholder")}
              onChange={(event) => {
                setQuestion(event.target.value)
              }}
            />
            <Button type="submit" variant="ghost" size="sm" disabled={busy || !question.trim()}>
              {t("personalAssistant.research.ask")}
            </Button>
            {query && (
              <p role="status" className="text-xs text-muted-foreground">
                {t(researchActivityKey(query))}
              </p>
            )}
          </form>
        </div>
      )}
      {(!expanded || !finding.viewedAt) && (
        <Button variant="ghost" size="sm" disabled={busy} onClick={() => void open()}>
          {t("personalAssistant.readUpdate")}
        </Button>
      )}
      {error && (
        <p role="alert" className="mt-2 text-xs text-destructive">
          {error}
        </p>
      )}
    </article>
  )
}
