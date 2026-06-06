import { Loader2, Sparkles } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { UnifiedDiffView } from "@/components/chat/diff-panel/UnifiedDiffView"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"
import { getTransport } from "@/lib/transport-provider"
import type { FileChangeMetadata } from "@/types/chat"

interface AiRewriteDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Text being rewritten (current selection, or the whole note). */
  before: string
  /** Human label of the scope (selection vs whole note), shown in the header. */
  scopeLabel: string
  /** Apply the accepted rewrite back into the editor. */
  onApply: (after: string) => void
}

/** AI-assisted rewrite (WS9): instruction → owner-plane side_query → diff review →
 *  apply. Nothing is written to disk here — applying splices the editor and the
 *  user still saves through the normal flow. */
export default function AiRewriteDialog({
  open,
  onOpenChange,
  before,
  scopeLabel,
  onApply,
}: AiRewriteDialogProps) {
  const { t } = useTranslation()
  const [instruction, setInstruction] = useState("")
  const [busy, setBusy] = useState(false)
  const [after, setAfter] = useState<string | null>(null)

  const reset = () => {
    setInstruction("")
    setAfter(null)
    setBusy(false)
  }

  const generate = async () => {
    const instr = instruction.trim()
    if (!instr || busy) return
    setBusy(true)
    try {
      const result = await getTransport().call<string>("kb_ai_rewrite_cmd", {
        text: before,
        instruction: instr,
      })
      setAfter(result)
    } catch (e) {
      console.error("kb_ai_rewrite failed", e)
      toast.error(t("knowledge.aiRewriteFailed", "AI rewrite failed"))
    } finally {
      setBusy(false)
    }
  }

  const change: FileChangeMetadata = {
    kind: "file_change",
    path: "",
    action: "edit",
    linesAdded: 0,
    linesRemoved: 0,
    before,
    after: after ?? "",
    language: "markdown",
    truncated: false,
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) reset()
        onOpenChange(o)
      }}
    >
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-4 w-4" />
            {t("knowledge.aiRewrite", "AI rewrite")}
          </DialogTitle>
          <DialogDescription>
            {t("knowledge.aiRewriteScope", "Rewriting: {{scope}}", { scope: scopeLabel })}
          </DialogDescription>
        </DialogHeader>

        {after === null ? (
          <form
            onSubmit={(e) => {
              e.preventDefault()
              void generate()
            }}
            className="space-y-3"
          >
            <Textarea
              autoFocus
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder={t(
                "knowledge.aiRewritePlaceholder",
                "Describe how to rewrite — e.g. make it more concise, fix grammar, translate to English…",
              )}
              className="min-h-24"
            />
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={busy || !instruction.trim()}>
                {busy ? (
                  <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                ) : (
                  <Sparkles className="mr-1 h-4 w-4" />
                )}
                {t("knowledge.aiRewriteGenerate", "Generate")}
              </Button>
            </DialogFooter>
          </form>
        ) : (
          <div className="space-y-3">
            <div className="max-h-[50vh] overflow-auto rounded-md border border-border-soft/60 bg-muted/20">
              <UnifiedDiffView change={change} />
            </div>
            <DialogFooter className="gap-2">
              <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => void generate()}
              >
                {busy ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
                {t("knowledge.aiRewriteRegenerate", "Regenerate")}
              </Button>
              <Button
                type="button"
                disabled={busy}
                onClick={() => {
                  onApply(after)
                  onOpenChange(false)
                  reset()
                }}
              >
                {t("knowledge.aiRewriteApply", "Apply")}
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
