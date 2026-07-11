import { useState, useEffect, useCallback, useRef } from "react"
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
import { AlertTriangle, Copy, Check, FileSearch, Loader2, Sparkles } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { MemoryImportPreview } from "./types"
import {
  copyMemoryImportPreviewDiagnostics,
  formatMemoryImportOperationError,
  formatMemoryImportPromptLoadError,
  formatMemoryImportScopeLabel,
  formatMemoryImportScopeSummaryKey,
  memoryImportPreviewCanApply,
  memoryImportPreviewIsCurrent,
  memoryImportPreviewIssueMessages,
  memoryImportPreviewSampleWindowLabel,
  memoryImportPreviewStatusLabel,
  memoryImportSortedCountEntries,
  memoryImportTotal,
  showMemoryImportResultToast,
  type MemoryImportResult,
} from "./memoryImportFeedback"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImported: () => void
  memoryEnabled?: boolean
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

function previewTypeOrder(type: string): number {
  const idx = ["user", "feedback", "project", "reference"].indexOf(type)
  return idx >= 0 ? idx : 99
}

function sampleDedupClass(status?: string | null): string {
  if (status === "new") {
    return "rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300"
  }
  if (status === "duplicate") {
    return "rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300"
  }
  if (status === "merge") {
    return "rounded bg-muted px-1.5 py-0.5 text-muted-foreground"
  }
  return "rounded bg-muted px-1.5 py-0.5 text-muted-foreground"
}

export default function ImportFromAIDialog({
  open,
  onOpenChange,
  onImported,
  memoryEnabled = true,
}: Props) {
  const { t, i18n } = useTranslation()
  const [prompt, setPrompt] = useState<string>("")
  const [loadingPrompt, setLoadingPrompt] = useState(false)
  const [copied, setCopied] = useState(false)
  const [pasted, setPasted] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [preview, setPreview] = useState<MemoryImportPreview | null>(null)
  const [previewContent, setPreviewContent] = useState("")

  // Cache prompts by locale to skip the IPC round-trip on reopen.
  const promptCache = useRef<Map<string, string>>(new Map())

  useEffect(() => {
    if (!open) return
    setPasted("")
    setError(null)
    setCopied(false)
    setPreview(null)
    setPreviewContent("")

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
        setError(formatMemoryImportPromptLoadError(t, e))
      })
      .finally(() => setLoadingPrompt(false))
  }, [open, i18n.language, t])

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setError(null)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (e) {
      logger.error("settings", "ImportFromAIDialog::copy", "Clipboard write failed", e)
      setError(formatMemoryImportOperationError(t, "copyPrompt", e))
    }
  }, [prompt, t])

  const previewMatchesInput = preview !== null && previewContent === stripCodeFence(pasted)
  const showCurrentPreview = memoryImportPreviewIsCurrent(preview, previewMatchesInput)
  const canApplyPreview = memoryImportPreviewCanApply(preview, previewMatchesInput)
  const previewTypeEntries = preview
    ? Object.entries(preview.byType).sort(
        ([left], [right]) => previewTypeOrder(left) - previewTypeOrder(right),
      )
    : []
  const previewScopeEntries = preview ? memoryImportSortedCountEntries(preview.byScope) : []
  const previewLikelyImportCount =
    (preview?.likelyNewCount ?? preview?.candidateCount ?? 0) + (preview?.likelyMergeCount ?? 0)
  const previewIssueMessages = preview ? memoryImportPreviewIssueMessages(t, preview) : []
  const previewVisibleSamples = preview ? preview.samples.slice(0, 4) : []
  const previewSampleWindowLabel = preview
    ? memoryImportPreviewSampleWindowLabel(t, preview.samples.length, previewVisibleSamples.length)
    : null
  const handlePreview = useCallback(async () => {
    const cleaned = stripCodeFence(pasted)
    if (!cleaned) {
      setError(t("settings.memoryImportFromAIEmpty"))
      return
    }
    setBusy(true)
    setError(null)
    setPreview(null)
    setPreviewContent("")
    try {
      const nextPreview = await getTransport().call<MemoryImportPreview>("memory_import_preview", {
        content: cleaned,
        format: "auto",
        dedup: true,
      })
      setPreview(nextPreview)
      setPreviewContent(cleaned)
      if (!nextPreview.valid) {
        if (nextPreview.issues.length === 0) {
          setError(t("settings.memoryImportNoEntries", "No importable memories found."))
        }
        return
      }
    } catch (e) {
      logger.error("settings", "ImportFromAIDialog::preview", "Parse preview failed", e)
      setError(formatMemoryImportOperationError(t, "preview", e))
    } finally {
      setBusy(false)
    }
  }, [pasted, t])

  const handleImport = useCallback(async () => {
    const cleaned = stripCodeFence(pasted)
    if (!cleaned) {
      setError(t("settings.memoryImportFromAIEmpty"))
      return
    }
    if (!previewMatchesInput || !preview?.valid) {
      await handlePreview()
      return
    }
    setBusy(true)
    setError(null)
    try {
      const result = await getTransport().call<MemoryImportResult>("memory_import", {
        content: cleaned,
        format: "auto",
        dedup: true,
      })
      logger.info(
        "settings",
        "ImportFromAIDialog::import",
        `created=${result.created} skipped=${result.skippedDuplicate} failed=${result.failed}`,
      )
      if (memoryImportTotal(result) === 0) {
        setError(t("settings.memoryImportNoEntries", "No importable memories found."))
        return
      }
      showMemoryImportResultToast(t, result, preview)
      onImported()
      onOpenChange(false)
    } catch (e) {
      logger.error("settings", "ImportFromAIDialog::import", "Parse/import failed", e)
      setError(formatMemoryImportOperationError(t, "apply", e))
    } finally {
      setBusy(false)
    }
  }, [handlePreview, onImported, onOpenChange, pasted, preview, previewMatchesInput, t])

  const handleCopyPreviewDiagnostics = useCallback(async () => {
    if (!preview) return
    await copyMemoryImportPreviewDiagnostics(t, preview, t("settings.memoryImportFromAI"))
  }, [preview, t])

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

        {!memoryEnabled && (
          <div className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-muted-foreground">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
            <span>{t("settings.memoryOffImportNotice")}</span>
          </div>
        )}

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
            onChange={(e) => {
              setPasted(e.target.value)
              setPreview(null)
              setPreviewContent("")
            }}
            placeholder={t("settings.memoryImportFromAIPastePlaceholder")}
            className="min-h-[200px] max-h-[40vh] font-mono text-xs"
            disabled={busy}
          />
        </div>

        {showCurrentPreview && preview && (
          <div className="space-y-3 rounded-md border bg-muted/20 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2 text-sm font-medium">
                <FileSearch className="h-4 w-4 text-primary" />
                {t("settings.memoryImportPreviewTitle", "Preview")}
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground">
                  {t("settings.memoryImportPreviewCount", "{{count}} memories", {
                    count: preview.candidateCount,
                  })}
                </span>
                <span
                  className={
                    preview.valid
                      ? "rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-700 dark:text-emerald-300"
                      : "rounded border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-300"
                  }
                >
                  {memoryImportPreviewStatusLabel(t, preview)}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1.5 px-2"
                  onClick={() => void handleCopyPreviewDiagnostics()}
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("chat.copy")}
                </Button>
              </div>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {preview.dedupChecked && (
                <>
                  <span className="rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-700 dark:text-emerald-300">
                    {t("settings.memoryImportPreviewLikelyImport", "{{count}} will import", {
                      count: previewLikelyImportCount,
                    })}
                  </span>
                  {(preview.likelyDuplicateCount ?? 0) > 0 && (
                    <span className="rounded border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-300">
                      {t("settings.memoryImportPreviewLikelyDuplicate", "{{count}} duplicates", {
                        count: preview.likelyDuplicateCount,
                      })}
                    </span>
                  )}
                  {(preview.likelyMergeCount ?? 0) > 0 && (
                    <span className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground">
                      {t("settings.memoryImportPreviewLikelyMerge", "{{count}} may merge", {
                        count: preview.likelyMergeCount,
                      })}
                    </span>
                  )}
                </>
              )}
              {previewTypeEntries.map(([type, count]) => (
                <span key={type} className="rounded border bg-background px-2 py-0.5 text-xs">
                  {t(`settings.memoryType_${type}`)} · {count}
                </span>
              ))}
              {previewScopeEntries.map(([scope, count]) => (
                <span
                  key={scope}
                  className="rounded border bg-background px-2 py-0.5 text-xs text-muted-foreground"
                >
                  {formatMemoryImportScopeSummaryKey(t, scope)} · {count}
                </span>
              ))}
            </div>
            {previewIssueMessages.length > 0 && (
              <div className="space-y-1 rounded border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-xs text-amber-700 dark:text-amber-300">
                <div className="font-medium">
                  {t("settings.memoryImportPreviewReport.issues", "Issues")}
                </div>
                {previewIssueMessages.map((message, index) => (
                  <div key={`${index}:${message}`} className="leading-relaxed">
                    {message}
                  </div>
                ))}
              </div>
            )}
            <div className="space-y-2">
              {previewSampleWindowLabel && (
                <div className="text-xs text-muted-foreground">{previewSampleWindowLabel}</div>
              )}
              {previewVisibleSamples.map((sample, index) => (
                <div
                  key={`${sample.contentPreview}-${index}`}
                  className="rounded border bg-background p-2"
                >
                  <div className="mb-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                    <span>{t(`settings.memoryType_${sample.memoryType}`)}</span>
                    <span>·</span>
                    <span>{formatMemoryImportScopeLabel(t, sample.scope)}</span>
                    {sample.dedupStatus && (
                      <span className={sampleDedupClass(sample.dedupStatus)}>
                        {t(`settings.memoryImportPreviewSample_${sample.dedupStatus}`)}
                      </span>
                    )}
                    {sample.tags.slice(0, 3).map((tag) => (
                      <span key={tag} className="rounded bg-muted px-1.5 py-0.5">
                        {tag}
                      </span>
                    ))}
                  </div>
                  <p className="text-xs leading-relaxed text-foreground">{sample.contentPreview}</p>
                  {sample.dedupExistingPreview && (
                    <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                      {t("settings.memoryImportPreviewExisting", {
                        id: sample.dedupExistingId ?? "",
                      })}
                      : {sample.dedupExistingPreview}
                    </p>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {error && <p className="text-xs text-destructive break-all">{error}</p>}

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={canApplyPreview ? handleImport : handlePreview}
            disabled={busy || !pasted.trim()}
            className="gap-1.5"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {canApplyPreview
              ? t("settings.memoryImportConfirmBtn", "Import")
              : t("settings.memoryImportPreviewBtn", "Preview")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
