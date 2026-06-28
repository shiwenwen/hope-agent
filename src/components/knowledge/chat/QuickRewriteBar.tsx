import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Loader2, Sparkles, X } from "lucide-react"

import { UnifiedDiffView } from "@/components/chat/diff-panel/UnifiedDiffView"
import {
  buildUnifiedRows,
  buildVisibleRowItems,
  isUnifiedRowChanged,
} from "@/components/chat/diff-panel/diffLayout"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ActiveModel, AvailableModel, FileChangeMetadata } from "@/types/chat"

interface Props {
  kbId: string
  notePath: string | null
  /** The selected text to rewrite. */
  before: string
  /** Splice the accepted rewrite back into the editor (caller saves to disk). */
  onApply: (after: string) => void
  onClose: () => void
}

/**
 * One-shot floating rewrite bar for the selected note text (replaces the old AI
 * rewrite modal). Generate → diff preview → apply; the rewrite is NOT part of
 * the conversation history but IS logged for stats (`kb_rewrite_log_cmd`). The
 * model defaults to the conversation's model and can be overridden per use.
 */
export function QuickRewriteBar({ kbId, notePath, before, onApply, onClose }: Props) {
  const { t } = useTranslation()
  const [instruction, setInstruction] = useState("")
  const [busy, setBusy] = useState(false)
  const [after, setAfter] = useState<string | null>(null)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [modelKey, setModelKey] = useState<string>("")

  // Default the picker to the current conversation model (the global active
  // model) and load the list for the override dropdown. Quick rewrites are
  // short-lived, so a per-mount fetch is fine.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const [models, active] = await Promise.all([
          getTransport().call<AvailableModel[]>("get_available_models"),
          getTransport().call<ActiveModel | null>("get_active_model"),
        ])
        if (cancelled) return
        setAvailableModels(models)
        if (active) setModelKey(`${active.providerId}::${active.modelId}`)
      } catch {
        /* picker just stays empty → backend default model */
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])
  // Whether the current `after` has been logged as discarded, so closing after
  // an apply doesn't double-log.
  const loggedRef = useRef(false)

  const log = (accepted: boolean) => {
    if (loggedRef.current) return
    loggedRef.current = true
    getTransport()
      .call("kb_rewrite_log_cmd", {
        kbId,
        notePath: notePath || undefined,
        instruction: instruction.trim(),
        model: modelKey || undefined,
        charsBefore: before.length,
        charsAfter: after?.length ?? 0,
        accepted,
      })
      .catch(() => {})
  }

  const generate = async () => {
    const instr = instruction.trim()
    if (!instr || busy) return
    setBusy(true)
    loggedRef.current = false
    try {
      const result = await getTransport().call<string>("kb_ai_rewrite_cmd", {
        text: before,
        instruction: instr,
        modelOverride: modelKey || undefined,
      })
      setAfter(result)
    } catch (e) {
      logger.error("ui", "QuickRewriteBar::generate", "rewrite failed", e)
      toast.error(t("knowledge.quickRewrite.failed"))
    } finally {
      setBusy(false)
    }
  }

  const handleClose = () => {
    if (after !== null) log(false)
    onClose()
  }

  const change: FileChangeMetadata = {
    kind: "file_change",
    path: notePath ?? "",
    action: "edit",
    linesAdded: 0,
    linesRemoved: 0,
    before,
    after: after ?? "",
    language: "markdown",
    truncated: false,
  }
  const diffRows = useMemo(
    () => buildUnifiedRows(change.before ?? "", change.after ?? ""),
    [change.before, change.after],
  )
  const diffItems = useMemo(
    () =>
      buildVisibleRowItems(diffRows, {
        collapseContext: false,
        expandedFoldIds: new Set(),
        isChanged: isUnifiedRowChanged,
      }),
    [diffRows],
  )

  return (
    <div className="w-[420px] max-w-[90vw] rounded-xl border border-border/60 bg-popover/95 p-3 shadow-[0_8px_30px_rgb(0,0,0,0.18)] backdrop-blur-xl">
      <div className="mb-2 flex items-center gap-1.5">
        <Sparkles className="h-3.5 w-3.5 text-primary" />
        <span className="text-xs font-medium">{t("knowledge.quickRewrite.title")}</span>
        <div className="flex-1" />
        <Button variant="ghost" size="icon" className="h-6 w-6" onClick={handleClose}>
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>

      {after === null ? (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            void generate()
          }}
          className="space-y-2"
        >
          <Textarea
            autoFocus
            value={instruction}
            onChange={(e) => setInstruction(e.target.value)}
            placeholder={t("knowledge.quickRewrite.placeholder")}
            className="min-h-16 text-sm"
          />
          <div className="flex items-center gap-2">
            <Select value={modelKey} onValueChange={setModelKey}>
              <SelectTrigger className="h-7 flex-1 text-xs">
                <SelectValue placeholder={t("knowledge.quickRewrite.model")} />
              </SelectTrigger>
              <SelectContent>
                {availableModels.map((m) => (
                  <SelectItem
                    key={`${m.providerId}::${m.modelId}`}
                    value={`${m.providerId}::${m.modelId}`}
                    className="text-xs"
                  >
                    {m.modelName || m.modelId}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button type="submit" size="sm" className="h-7" disabled={busy || !instruction.trim()}>
              {busy ? (
                <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="mr-1 h-3.5 w-3.5" />
              )}
              {t("knowledge.quickRewrite.generate")}
            </Button>
          </div>
        </form>
      ) : (
        <div className="space-y-2">
          <div className="max-h-[40vh] overflow-auto rounded-md border border-border/50 bg-muted/20">
            <UnifiedDiffView
              items={diffItems}
              omittedItemCount={0}
              onToggleFold={() => {}}
              onRenderAll={() => {}}
              onCopyLocation={() => {}}
              onOpenLocation={() => {}}
            />
          </div>
          <div className="flex items-center justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7"
              disabled={busy}
              onClick={() => {
                loggedRef.current = false
                void generate()
              }}
            >
              {busy ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : null}
              {t("knowledge.quickRewrite.regenerate")}
            </Button>
            <Button
              type="button"
              size="sm"
              className="h-7"
              disabled={busy}
              onClick={() => {
                onApply(after)
                log(true)
                onClose()
              }}
            >
              {t("knowledge.quickRewrite.apply")}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

export default QuickRewriteBar
