import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import { Copy, Check, Loader2, Sparkles } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface ImportResult {
  created: number
  skippedDuplicate: number
  failed: number
  errors: string[]
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImported: () => void
}

/** Strip leading/trailing Markdown code fences that external AIs often wrap output in. */
function stripCodeFence(raw: string): string {
  const trimmed = raw.trim()
  if (!trimmed.startsWith("```")) return trimmed
  return trimmed
    .replace(/^`{3,}[ \t]*\w*[ \t]*\r?\n?/, "")
    .replace(/\r?\n?[ \t]*`{3,}[ \t]*$/, "")
    .trim()
}

export default function ImportFromAIDialog({ open, onOpenChange, onImported }: Props) {
  const { t, i18n } = useTranslation()
  const [prompt, setPrompt] = useState<string>("")
  const [loadingPrompt, setLoadingPrompt] = useState(false)
  const [copied, setCopied] = useState(false)
  const [pasted, setPasted] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Cache prompts by locale to skip the IPC round-trip on reopen.
  const promptCache = useRef<Map<string, string>>(new Map())

  useEffect(() => {
    if (!open) return
    setPasted("")
    setError(null)
    setCopied(false)

    const locale = i18n.language?.toLowerCase().split("-")[0] || "en"
    const cached = promptCache.current.get(locale)
    if (cached !== undefined) {
      setPrompt(cached)
      setLoadingPrompt(false)
      return
    }

    setLoadingPrompt(true)
    getTransport()
      .call<string>("memory_get_import_from_ai_prompt", { locale })
      .then((p) => {
        promptCache.current.set(locale, p)
        setPrompt(p)
      })
      .catch((e) => {
        logger.error("settings", "ImportFromAIDialog::fetchPrompt", "Failed", e)
        setError(String(e))
      })
      .finally(() => setLoadingPrompt(false))
  }, [open, i18n.language])

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (e) {
      logger.error("settings", "ImportFromAIDialog::copy", "Clipboard write failed", e)
    }
  }

  const handleParseAndImport = async () => {
    const cleaned = stripCodeFence(pasted)
    if (!cleaned) {
      setError(t("settings.memoryImportFromAIEmpty"))
      return
    }
    setBusy(true)
    setError(null)
    try {
      const result = await getTransport().call<ImportResult>("memory_import", {
        content: cleaned,
        format: "json",
        dedup: true,
      })
      logger.info(
        "settings",
        "ImportFromAIDialog::import",
        `created=${result.created} skipped=${result.skippedDuplicate} failed=${result.failed}`,
      )
      onImported()
      onOpenChange(false)
    } catch (e) {
      logger.error("settings", "ImportFromAIDialog::import", "Parse/import failed", e)
      setError(t("settings.memoryImportFromAIParseError", { error: String(e) }))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-primary" />
            {t("settings.memoryImportFromAI")}
          </DialogTitle>
          <DialogDescription>{t("settings.memoryImportFromAIDesc")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium">{t("settings.memoryImportFromAIStep1")}</h3>
            <Button
              variant="outline"
              size="sm"
              onClick={handleCopy}
              disabled={loadingPrompt || !prompt}
              className="gap-1.5"
            >
              {copied ? (
                <>
                  <Check className="h-3.5 w-3.5 text-green-500" />
                  {t("settings.memoryImportFromAICopied")}
                </>
              ) : (
                <>
                  <Copy className="h-3.5 w-3.5" />
                  {t("settings.memoryImportFromAICopyBtn")}
                </>
              )}
            </Button>
          </div>
          <pre className="relative max-h-[280px] overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap">
            {loadingPrompt ? (
              <span className="flex items-center gap-2 text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("settings.memoryImportFromAILoadingPrompt")}
              </span>
            ) : (
              prompt
            )}
          </pre>
        </div>

        <div className="space-y-2">
          <h3 className="text-sm font-medium">{t("settings.memoryImportFromAIStep2")}</h3>
          <Textarea
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
            placeholder={t("settings.memoryImportFromAIPastePlaceholder")}
            className="min-h-[200px] max-h-[40vh] font-mono text-xs"
            disabled={busy}
          />
        </div>

        {error && <p className="text-xs text-destructive break-all">{error}</p>}

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleParseAndImport}
            disabled={busy || !pasted.trim()}
            className="gap-1.5"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {t("settings.memoryImportFromAIParseBtn")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
