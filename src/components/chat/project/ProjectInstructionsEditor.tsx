import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Check, Eye, FileText, Loader2 } from "lucide-react"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { formatBytes } from "@/lib/format"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { ProjectInstructionsFile } from "@/types/project"

type EditorMode = "edit" | "preview"

export default function ProjectInstructionsEditor({
  projectId,
  readOnly = false,
}: {
  projectId: string
  readOnly?: boolean
}) {
  const { t } = useTranslation()
  const requestSeq = useRef(0)
  const [mode, setMode] = useState<EditorMode>("edit")
  const [draft, setDraft] = useState("")
  const [savedContent, setSavedContent] = useState("")
  const [contentHash, setContentHash] = useState("")
  const [filePath, setFilePath] = useState("")
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState("")
  const [saveError, setSaveError] = useState("")
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved">("idle")

  const dirty = draft !== savedContent
  const lineCount = useMemo(
    () => (draft.length === 0 ? 0 : draft.split(/\r\n|\r|\n/).length),
    [draft],
  )
  const sizeLabel = useMemo(
    () => formatBytes(new TextEncoder().encode(draft).byteLength, { trimTrailingZeros: true }),
    [draft],
  )

  const load = useCallback(async () => {
    const seq = ++requestSeq.current
    setLoading(true)
    setLoadError("")
    setSaveError("")
    try {
      const file = await getTransport().call<ProjectInstructionsFile>(
        "get_project_instructions_cmd",
        { id: projectId },
      )
      if (seq !== requestSeq.current) return
      setDraft(file.content)
      setSavedContent(file.content)
      setContentHash(file.contentHash)
      setFilePath(file.path)
      setSaveStatus("idle")
    } catch (error) {
      if (seq !== requestSeq.current) return
      setLoadError(error instanceof Error ? error.message : String(error))
    } finally {
      if (seq === requestSeq.current) setLoading(false)
    }
  }, [projectId])

  useEffect(() => {
    setMode("edit")
    setDraft("")
    setSavedContent("")
    setContentHash("")
    setFilePath("")
    setSaving(false)
    void load()
    return () => {
      requestSeq.current += 1
    }
  }, [load])

  const save = useCallback(async () => {
    if (readOnly || saving || !dirty) return
    const seq = requestSeq.current
    setSaving(true)
    setSaveError("")
    setSaveStatus("idle")
    try {
      const file = await getTransport().call<ProjectInstructionsFile>(
        "save_project_instructions_cmd",
        { id: projectId, content: draft, expectedFileHash: contentHash },
      )
      if (seq !== requestSeq.current) return
      setSavedContent(file.content)
      setContentHash(file.contentHash)
      setFilePath(file.path)
      setSaveStatus("saved")
      window.setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (error) {
      if (seq !== requestSeq.current) return
      setSaveError(error instanceof Error ? error.message : String(error))
    } finally {
      if (seq === requestSeq.current) setSaving(false)
    }
  }, [contentHash, dirty, draft, projectId, readOnly, saving])

  function handleEditorKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (!readOnly && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault()
      void save()
    }
  }

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        {t("common.loading")}
      </div>
    )
  }

  if (loadError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
        <p className="text-sm font-medium text-destructive">
          {t("project.projectInstructionsLoadFailed")}
        </p>
        <p className="max-w-xl break-words text-xs text-muted-foreground">{loadError}</p>
        <Button variant="outline" size="sm" onClick={() => void load()}>
          {t("common.retry")}
        </Button>
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col gap-3">
      <div className="flex shrink-0 flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs leading-5 text-muted-foreground">
            {t("project.projectInstructionsHint")}
          </p>
          <div className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-muted-foreground/80">
            <span className="min-w-0 truncate" data-ha-title-tip={filePath}>
              {filePath}
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
        </div>
        <div className="flex shrink-0 rounded-lg bg-muted p-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setMode("edit")}
            className={cn(
              "h-7 gap-1.5 px-2.5 text-xs shadow-none",
              mode === "edit" && "bg-background text-foreground hover:bg-background",
            )}
          >
            <FileText className="h-3.5 w-3.5" />
            {t("common.edit")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => setMode("preview")}
            className={cn(
              "h-7 gap-1.5 px-2.5 text-xs shadow-none",
              mode === "preview" && "bg-background text-foreground hover:bg-background",
            )}
          >
            <Eye className="h-3.5 w-3.5" />
            {t("knowledge.mode.preview")}
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-hidden rounded-lg border border-border/70 bg-background">
        {mode === "edit" ? (
          <Textarea
            surface="embedded"
            value={draft}
            readOnly={readOnly}
            onChange={(event) => {
              setDraft(event.target.value)
              setSaveStatus("idle")
              setSaveError("")
            }}
            onKeyDown={handleEditorKeyDown}
            spellCheck={false}
            aria-label={t("project.projectInstructions")}
            placeholder={t("project.projectInstructionsPlaceholder")}
            className="h-full min-h-[360px] resize-none px-4 py-3 font-mono text-sm"
          />
        ) : (
          <div className="h-full min-h-[360px] overflow-y-auto px-5 py-4">
            {draft.trim() ? (
              <MarkdownRenderer content={draft} />
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("project.projectInstructionsEmptyPreview")}
              </p>
            )}
          </div>
        )}
      </div>

      {saveError && (
        <div className="flex shrink-0 items-start justify-between gap-3 rounded-md border border-destructive/20 bg-destructive/10 px-3 py-2">
          <div className="min-w-0">
            <p className="text-sm font-medium text-destructive">
              {t("project.projectInstructionsSaveFailed")}
            </p>
            <p className="mt-1 break-words text-xs text-destructive/80">{saveError}</p>
          </div>
          <Button variant="outline" size="sm" className="shrink-0" onClick={() => void load()}>
            {t("knowledge.externalChange.reload")}
          </Button>
        </div>
      )}

      {!readOnly && (
        <div className="flex shrink-0 justify-end gap-2">
          <Button
            variant="outline"
            onClick={() => {
              setDraft(savedContent)
              setSaveError("")
              setSaveStatus("idle")
            }}
            disabled={saving || !dirty}
          >
            {t("common.cancel")}
          </Button>
          <Button
            onClick={() => void save()}
            disabled={saving || !dirty}
            className={saveStatus === "saved" ? "bg-emerald-600 hover:bg-emerald-600" : ""}
          >
            {saving && <Loader2 className="mr-1 h-4 w-4 animate-spin" />}
            {saveStatus === "saved" && <Check className="mr-1 h-4 w-4" />}
            {saving
              ? t("common.saving")
              : saveStatus === "saved"
                ? t("common.saved")
                : t("common.save")}
          </Button>
        </div>
      )}
    </div>
  )
}
