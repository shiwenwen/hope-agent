/**
 * Bulk-import MCP servers from a `claude_desktop_config.json`-style
 * JSON blob. The blob's `mcpServers` object is parsed server-side; the
 * dialog just surfaces the per-entry imported/skipped breakdown.
 */

import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, CheckCircle2, AlertCircle } from "lucide-react"
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
import { Textarea } from "@/components/ui/textarea"
import { importClaudeDesktopConfig, type McpImportSummary } from "@/lib/mcp"

const SAMPLE = `{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-memory"]
    }
  }
}`

export default function McpImportDialog({
  open,
  onClose,
  onImported,
}: {
  open: boolean
  onClose: () => void
  onImported: () => void
}) {
  const { t } = useTranslation()
  const [json, setJson] = useState("")
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<McpImportSummary | null>(null)

  const handleImport = async () => {
    if (!json.trim()) {
      toast.error(t("settings.mcp.import.emptyError"))
      return
    }
    setBusy(true)
    try {
      const summary = await importClaudeDesktopConfig(json)
      setResult(summary)
      if (summary.imported.length > 0) {
        toast.success(
          t("settings.mcp.import.success", { count: summary.imported.length }),
        )
      }
      if (summary.imported.length > 0 && summary.skipped.length === 0) {
        // Pure success — close + refresh the parent.
        onImported()
      }
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("settings.mcp.import.title")}</DialogTitle>
          <DialogDescription>
            {t("settings.mcp.import.description")}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <>
            <Textarea
              value={json}
              onChange={(e) => setJson(e.target.value)}
              placeholder={SAMPLE}
              rows={14}
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">
              {t("settings.mcp.import.hint")}
            </p>
          </>
        ) : (
          <div className="space-y-3">
            {result.imported.length > 0 && (
              <div className="space-y-1">
                <p className="text-sm font-medium text-green-600 dark:text-green-400 flex items-center gap-1.5">
                  <CheckCircle2 className="h-4 w-4" />
                  {t("settings.mcp.import.importedHeader", {
                    count: result.imported.length,
                  })}
                </p>
                <ul className="text-xs text-muted-foreground ml-6 list-disc">
                  {result.imported.map((name) => (
                    <li key={name}>{name}</li>
                  ))}
                </ul>
              </div>
            )}
            {result.skipped.length > 0 && (
              <div className="space-y-1">
                <p className="text-sm font-medium text-amber-600 dark:text-amber-400 flex items-center gap-1.5">
                  <AlertCircle className="h-4 w-4" />
                  {t("settings.mcp.import.skippedHeader", {
                    count: result.skipped.length,
                  })}
                </p>
                <ul className="text-xs text-muted-foreground ml-6 list-disc">
                  {result.skipped.map((entry, i) => (
                    <li key={i}>
                      <span className="font-mono">{entry.name}</span>
                      {" — "}
                      {entry.reason}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}

        <DialogFooter>
          {!result ? (
            <>
              <Button variant="outline" onClick={onClose} disabled={busy}>
                {t("common.cancel")}
              </Button>
              <Button onClick={handleImport} disabled={busy}>
                {busy ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin mr-2" />
                    {t("settings.mcp.import.importing")}
                  </>
                ) : (
                  t("settings.mcp.import.doImport")
                )}
              </Button>
            </>
          ) : (
            <Button onClick={onImported}>{t("common.done")}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
