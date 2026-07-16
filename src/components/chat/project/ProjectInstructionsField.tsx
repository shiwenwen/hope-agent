import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Eye, FileText, Loader2 } from "lucide-react"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { formatBytes } from "@/lib/format"
import { cn } from "@/lib/utils"

interface ProjectInstructionsFieldProps {
  value: string
  path: string
  loading: boolean
  error: string
  disabled?: boolean
  onChange: (value: string) => void
}

type EditorMode = "edit" | "preview"

export default function ProjectInstructionsField({
  value,
  path,
  loading,
  error,
  disabled = false,
  onChange,
}: ProjectInstructionsFieldProps) {
  const { t } = useTranslation()
  const [mode, setMode] = useState<EditorMode>("edit")
  const lineCount = useMemo(
    () => (value.length === 0 ? 0 : value.split(/\r\n|\r|\n/).length),
    [value],
  )
  const sizeLabel = useMemo(
    () => formatBytes(new TextEncoder().encode(value).byteLength, { trimTrailingZeros: true }),
    [value],
  )

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <Label htmlFor="project-instructions" className="flex items-center gap-2">
            <FileText className="h-4 w-4 text-muted-foreground" />
            {t("project.projectInstructions")}
          </Label>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {t("project.projectInstructionsHint")}
          </p>
        </div>
        <div className="flex shrink-0 rounded-lg bg-muted p-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={loading}
            onClick={() => setMode("edit")}
            className={cn(
              "h-7 gap-1.5 px-2.5 text-xs shadow-none",
              mode === "edit" && "bg-background text-foreground shadow-sm hover:bg-background",
            )}
          >
            <FileText className="h-3.5 w-3.5" />
            {t("common.edit")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={loading}
            onClick={() => setMode("preview")}
            className={cn(
              "h-7 gap-1.5 px-2.5 text-xs shadow-none",
              mode === "preview" && "bg-background text-foreground shadow-sm hover:bg-background",
            )}
          >
            <Eye className="h-3.5 w-3.5" />
            {t("knowledge.mode.preview")}
          </Button>
        </div>
      </div>

      <div className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-muted-foreground/80">
        <span className="min-w-0 truncate" data-ha-title-tip={path || "AGENTS.md"}>
          {path || "AGENTS.md"}
        </span>
        <span aria-hidden="true" className="shrink-0">
          ·
        </span>
        <span className="shrink-0">
          {t("project.projectInstructionsLineCount", { count: lineCount })}
        </span>
        <span aria-hidden="true" className="shrink-0">
          ·
        </span>
        <span className="shrink-0">{sizeLabel}</span>
      </div>

      <div className="min-h-44 overflow-hidden rounded-lg border border-border/70 bg-background">
        {loading ? (
          <div className="flex min-h-44 items-center justify-center text-sm text-muted-foreground">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("common.loading")}
          </div>
        ) : mode === "edit" ? (
          <Textarea
            id="project-instructions"
            surface="embedded"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            disabled={disabled}
            spellCheck={false}
            placeholder={t("project.projectInstructionsPlaceholder")}
            className="min-h-44 max-h-64 resize-y px-4 py-3 font-mono text-sm"
          />
        ) : (
          <div className="min-h-44 max-h-64 overflow-y-auto px-4 py-3">
            {value.trim() ? (
              <MarkdownRenderer content={value} />
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("project.projectInstructionsEmptyPreview")}
              </p>
            )}
          </div>
        )}
      </div>

      {error && (
        <div className="rounded-md border border-destructive/20 bg-destructive/10 px-3 py-2">
          <p className="text-sm font-medium text-destructive">
            {t("project.projectInstructionsLoadFailed")}
          </p>
          <p className="mt-1 break-words text-xs text-destructive/80">{error}</p>
        </div>
      )}
    </div>
  )
}
