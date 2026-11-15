import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import type { AssistantUpdate } from "./usePersonalAssistant"

/** A durable timeline entry; appearing in the viewport is not a read receipt. */
export function AssistantUpdateEntry({
  update,
  name,
  onView,
}: {
  update: AssistantUpdate
  name: string
  onView: (id: string) => Promise<void>
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [pending, setPending] = useState(false)
  const open = async () => {
    setExpanded(true)
    if (update.viewedAt || pending) return
    setPending(true)
    try {
      await onView(update.id)
    } catch (error) {
      toast.error(String(error))
    } finally {
      setPending(false)
    }
  }
  return (
    <article
      className="my-3 rounded-lg bg-muted/30 px-4 py-3 text-sm"
      data-assistant-update={update.id}
    >
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>{name}</span>
        <span>
          {t(`personalAssistant.updates.${update.kind}`, {
            defaultValue: t("personalAssistant.updates.update"),
          })}
        </span>
        <time dateTime={update.createdAt}>{new Date(update.createdAt).toLocaleString()}</time>
      </div>
      <p
        className={
          expanded ? "mt-1 whitespace-pre-wrap break-words" : "mt-1 line-clamp-2 break-words"
        }
      >
        {update.summaryCode
          ? t(`personalAssistant.itemReason.${update.summaryCode}`)
          : update.summary}
      </p>
      {(!expanded || !update.viewedAt) && (
        <Button size="sm" variant="ghost" disabled={pending} onClick={() => void open()}>
          {t("personalAssistant.readUpdate")}
        </Button>
      )}
    </article>
  )
}
