import { useState, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2, AlertCircle, Download } from "lucide-react"
import { toast } from "sonner"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { RadioPills } from "@/components/ui/radio-pills"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

type ExportFormat = "md" | "json" | "html"

interface ExportSessionDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  sessionId: string
  /** Used as the default filename suggestion. Falls back to "Untitled" when null. */
  sessionTitle: string | null
}

/** Replicates the Rust `sanitize_filename_stem` so the dialog's suggested
 *  filename matches what the backend will actually write. */
function sanitizeStem(name: string): string {
  const cleaned = Array.from(name)
    .map((c) => (/[\p{L}\p{N}\-_ ]/u.test(c) ? c : "_"))
    .join("")
    .trim()
  return cleaned.length > 0 ? cleaned : "session"
}

export function ExportSessionDialog({
  open,
  onOpenChange,
  sessionId,
  sessionTitle,
}: ExportSessionDialogProps) {
  const { t } = useTranslation()
  const [format, setFormat] = useState<ExportFormat>("md")
  // Default to lean export — thinking and tool args/results can contain file
  // contents, command output, or secrets. Users opt in explicitly.
  const [includeThinking, setIncludeThinking] = useState(false)
  const [includeTools, setIncludeTools] = useState(false)
  const [status, setStatus] = useState<"idle" | "busy" | "saved" | "failed">("idle")
  const isBusy = status === "busy"

  const stem = sanitizeStem(
    sessionTitle && sessionTitle.trim() ? sessionTitle : t("chat.exportSession.untitled"),
  )
  const defaultFilename = `${stem}.${format}`

  const handleExport = useCallback(async () => {
    if (status === "busy") return
    setStatus("busy")
    try {
      const result = await getTransport().exportSession({
        sessionId,
        format,
        includeThinking,
        includeTools,
        defaultFilename,
      })
      if (!result) {
        // User cancelled the Tauri save dialog.
        setStatus("idle")
        return
      }
      // HTTP path: trigger a browser download from the blob.
      if (result.blob) {
        const url = URL.createObjectURL(result.blob)
        const a = document.createElement("a")
        a.href = url
        a.download = result.filename
        a.rel = "noopener"
        document.body.appendChild(a)
        a.click()
        document.body.removeChild(a)
        URL.revokeObjectURL(url)
        toast.success(t("chat.exportSession.downloaded", { filename: result.filename }))
      } else if (result.savedPath) {
        toast.success(t("chat.exportSession.savedTo", { path: result.savedPath }))
      }
      setStatus("saved")
      window.setTimeout(() => {
        onOpenChange(false)
        setStatus("idle")
      }, 600)
    } catch (e) {
      logger.error("ui", "ExportSessionDialog::export", "Export failed", e)
      toast.error(t("chat.exportSession.failed"))
      setStatus("failed")
      window.setTimeout(() => setStatus("idle"), 2000)
    }
  }, [
    status,
    sessionId,
    format,
    includeThinking,
    includeTools,
    defaultFilename,
    onOpenChange,
    t,
  ])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("chat.exportSession.dialogTitle")}</DialogTitle>
          <DialogDescription>{t("chat.exportSession.dialogHint")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label className="text-xs font-medium text-muted-foreground">
              {t("chat.exportSession.formatLabel")}
            </Label>
            <RadioPills<ExportFormat>
              value={format}
              onChange={setFormat}
              variant="strong"
              ariaLabel={t("chat.exportSession.formatLabel")}
              options={[
                { value: "md", label: t("chat.exportSession.formatMarkdown") },
                { value: "json", label: t("chat.exportSession.formatJson") },
                { value: "html", label: t("chat.exportSession.formatHtml") },
              ]}
            />
          </div>

          <div className="space-y-2">
            <Label className="text-xs font-medium text-muted-foreground">
              {t("chat.exportSession.contentLabel")}
            </Label>
            <div className="grid gap-1.5">
              <ToggleRow
                checked={includeTools}
                onChange={setIncludeTools}
                label={t("chat.exportSession.includeTools")}
                hint={t("chat.exportSession.includeToolsHint")}
              />
              <ToggleRow
                checked={includeThinking}
                onChange={setIncludeThinking}
                label={t("chat.exportSession.includeThinking")}
                hint={t("chat.exportSession.includeThinkingHint")}
              />
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isBusy}
          >
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            onClick={handleExport}
            disabled={isBusy}
            className={cn(
              "min-w-[120px]",
              status === "saved" && "bg-emerald-600 hover:bg-emerald-600 text-white",
              status === "failed" && "bg-destructive hover:bg-destructive text-white",
            )}
          >
            {isBusy ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin mr-1.5" />
                {t("chat.exportSession.exporting")}
              </>
            ) : status === "saved" ? (
              <>
                <Check className="h-4 w-4 mr-1.5" />
                {t("chat.exportSession.exported")}
              </>
            ) : status === "failed" ? (
              <>
                <AlertCircle className="h-4 w-4 mr-1.5" />
                {t("chat.exportSession.failedShort")}
              </>
            ) : (
              <>
                <Download className="h-4 w-4 mr-1.5" />
                {t("chat.exportSession.exportButton")}
              </>
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

interface ToggleRowProps {
  checked: boolean
  onChange: (next: boolean) => void
  label: string
  hint: string
}

function ToggleRow({ checked, onChange, label, hint }: ToggleRowProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        "w-full flex items-start gap-2 text-left rounded-md px-2 py-1.5 border transition-colors",
        checked
          ? "border-border/40 bg-secondary/70"
          : "border-border/40 bg-secondary/30 hover:bg-secondary/50",
      )}
    >
      <span
        className={cn(
          "mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded border",
          checked
            ? "border-transparent bg-primary text-primary-foreground"
            : "border-border bg-background",
        )}
      >
        {checked ? <Check className="h-3 w-3" /> : null}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-medium">{label}</div>
        <div className="text-[11px] text-muted-foreground leading-tight">{hint}</div>
      </div>
    </button>
  )
}
