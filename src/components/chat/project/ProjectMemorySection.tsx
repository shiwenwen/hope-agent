import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Eye, FileText, Plus, RefreshCw, Trash2 } from "lucide-react"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type {
  ProjectMemoryEntry,
  ProjectMemoryFile,
  ProjectMemoryType,
  ProjectMemoryWriteInput,
} from "@/types/project"

interface ProjectMemorySectionProps {
  projectId: string
  readOnly?: boolean
}

interface MemoryDraft {
  fileName?: string
  fileHash?: string
  name: string
  description: string
  memoryType: ProjectMemoryType
  content: string
}

interface MemoryOperationError {
  title: string
  detail: string
}

type EditorMode = "edit" | "preview"

const EMPTY_DRAFT: MemoryDraft = {
  name: "",
  description: "",
  memoryType: "project",
  content: "",
}

function diagnosticDetail(cause: unknown): string {
  const value = cause instanceof Error ? cause.message : String(cause)
  return sanitizeDiagnosticText(value, 420)
}

export function ProjectMemorySection({ projectId, readOnly = false }: ProjectMemorySectionProps) {
  const { t } = useTranslation()
  const [entries, setEntries] = useState<ProjectMemoryEntry[]>([])
  const [draft, setDraft] = useState<MemoryDraft>(EMPTY_DRAFT)
  const [loading, setLoading] = useState(true)
  const [loadingFile, setLoadingFile] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<MemoryOperationError | null>(null)
  const [editorMode, setEditorMode] = useState<EditorMode>("edit")

  const loadEntries = useCallback(async () => {
    setLoading(true)
    try {
      const result = await getTransport().call<ProjectMemoryEntry[]>(
        "list_project_memory_files_cmd",
        { id: projectId },
      )
      setEntries(result)
      setError(null)
    } catch (cause) {
      const detail = diagnosticDetail(cause)
      logger.error("project", "ProjectMemorySection::loadEntries", detail)
      setError({ title: t("settings.memoryV2.core.topicLoadFailed"), detail })
    } finally {
      setLoading(false)
    }
  }, [projectId, t])

  useEffect(() => {
    setDraft(EMPTY_DRAFT)
    setEditorMode("edit")
    void loadEntries()
  }, [loadEntries])

  useEffect(() => {
    return getTransport().listen("project_memory:changed", (payload: unknown) => {
      const event = payload as { projectId?: string } | null
      if (event?.projectId === projectId) void loadEntries()
    })
  }, [loadEntries, projectId])

  async function openEntry(entry: ProjectMemoryEntry) {
    setEditorMode("edit")
    setLoadingFile(true)
    try {
      const file = await getTransport().call<ProjectMemoryFile>("read_project_memory_file_cmd", {
        id: projectId,
        fileName: entry.fileName,
      })
      setDraft(file)
      setError(null)
    } catch (cause) {
      const detail = diagnosticDetail(cause)
      logger.error("project", "ProjectMemorySection::openEntry", detail)
      setError({ title: t("settings.memoryV2.core.topicLoadFailed"), detail })
    } finally {
      setLoadingFile(false)
    }
  }

  async function saveDraft() {
    if (readOnly || !draft.name.trim() || !draft.description.trim()) return
    setSaving(true)
    try {
      const input: ProjectMemoryWriteInput = {
        fileName: draft.fileName,
        expectedFileHash: draft.fileHash,
        name: draft.name.trim(),
        description: draft.description.trim(),
        memoryType: draft.memoryType,
        content: draft.content,
      }
      const saved = await getTransport().call<ProjectMemoryFile>("write_project_memory_file_cmd", {
        id: projectId,
        input,
      })
      setDraft(saved)
      await loadEntries()
      setError(null)
    } catch (cause) {
      const detail = diagnosticDetail(cause)
      logger.error("project", "ProjectMemorySection::saveDraft", detail)
      setError({ title: t("settings.memoryV2.core.topicSaveFailed"), detail })
    } finally {
      setSaving(false)
    }
  }

  async function deleteDraft() {
    if (readOnly || !draft.fileName) return
    if (!window.confirm(t("project.autoMemory.deleteConfirm"))) return
    try {
      await getTransport().call<boolean>("delete_project_memory_file_cmd", {
        id: projectId,
        fileName: draft.fileName,
        expectedFileHash: draft.fileHash,
      })
      setDraft(EMPTY_DRAFT)
      setEditorMode("edit")
      await loadEntries()
      setError(null)
    } catch (cause) {
      const detail = diagnosticDetail(cause)
      logger.error("project", "ProjectMemorySection::deleteDraft", detail)
      setError({ title: t("settings.memoryV2.core.topicDeleteFailed"), detail })
    }
  }

  function startNewTopic() {
    setDraft(EMPTY_DRAFT)
    setEditorMode("edit")
  }

  async function rebuildIndex() {
    if (readOnly) return
    try {
      await getTransport().call<string>("rebuild_project_memory_index_cmd", { id: projectId })
      await loadEntries()
      setError(null)
    } catch (cause) {
      const detail = diagnosticDetail(cause)
      logger.error("project", "ProjectMemorySection::rebuildIndex", detail)
      setError({ title: t("project.autoMemory.rebuildFailed"), detail })
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-64 shrink-0 flex-col border-r border-border/70">
        <div className="space-y-2 border-b border-border/70 p-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("project.autoMemory.description")}
          </p>
          {!readOnly && (
            <div className="flex gap-2">
              <Button size="sm" className="flex-1" onClick={startNewTopic}>
                <Plus className="mr-1.5 h-3.5 w-3.5" />
                {t("project.autoMemory.newTopic")}
              </Button>
              <IconTip label={t("project.autoMemory.rebuildIndex")}>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={rebuildIndex}
                  aria-label={t("project.autoMemory.rebuildIndex")}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            </div>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-2">
          {loading ? (
            <p className="px-2 py-4 text-xs text-muted-foreground">{t("common.loading")}</p>
          ) : entries.length === 0 ? (
            <p className="px-2 py-4 text-xs text-muted-foreground">
              {t("project.autoMemory.empty")}
            </p>
          ) : (
            entries.map((entry) => (
              <button
                key={entry.fileName}
                type="button"
                onClick={() => void openEntry(entry)}
                className={`mb-1 w-full rounded-md px-2.5 py-2 text-left transition-colors hover:bg-accent ${
                  draft.fileName === entry.fileName ? "bg-accent" : ""
                }`}
              >
                <div className="flex items-center gap-1.5 text-sm font-medium">
                  <FileText className="h-3.5 w-3.5 shrink-0" />
                  <span className="truncate">{entry.name}</span>
                </div>
                <p className="mt-1 line-clamp-2 text-[11px] leading-relaxed text-muted-foreground">
                  {entry.description}
                </p>
              </button>
            ))
          )}
        </div>
      </aside>

      <section className="min-w-0 flex-1 overflow-y-auto p-5">
        <div className="mx-auto max-w-2xl space-y-4">
          <div>
            <h3 className="text-sm font-semibold">{t("project.autoMemory.editorTitle")}</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("project.autoMemory.editorHint")}
            </p>
          </div>

          {error && (
            <div className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
              <p className="font-medium">{error.title}</p>
              <p className="mt-1 break-words text-[11px] opacity-90">{error.detail}</p>
            </div>
          )}

          <div className="space-y-1.5">
            <Label htmlFor="project-memory-name">{t("project.autoMemory.name")}</Label>
            <Input
              id="project-memory-name"
              value={draft.name}
              readOnly={readOnly}
              disabled={loadingFile}
              onChange={(event) => setDraft((value) => ({ ...value, name: event.target.value }))}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="project-memory-description">{t("project.autoMemory.summary")}</Label>
            <Input
              id="project-memory-description"
              value={draft.description}
              readOnly={readOnly}
              disabled={loadingFile}
              onChange={(event) =>
                setDraft((value) => ({ ...value, description: event.target.value }))
              }
            />
            <p className="text-[11px] text-muted-foreground">
              {t("project.autoMemory.summaryHint")}
            </p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="project-memory-type">{t("project.autoMemory.type")}</Label>
            <Select
              value={draft.memoryType}
              disabled={readOnly || loadingFile}
              onValueChange={(memoryType: ProjectMemoryType) =>
                setDraft((value) => ({ ...value, memoryType }))
              }
            >
              <SelectTrigger id="project-memory-type" className="w-52">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(["project", "feedback", "reference", "user"] as const).map((type) => (
                  <SelectItem key={type} value={type}>
                    {t(`project.autoMemory.types.${type}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between gap-3">
              <Label htmlFor="project-memory-content">{t("project.autoMemory.content")}</Label>
              <div className="flex shrink-0 rounded-lg bg-muted p-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={loadingFile}
                  onClick={() => setEditorMode("edit")}
                  className={cn(
                    "h-7 gap-1.5 px-2.5 text-xs shadow-none",
                    editorMode === "edit" &&
                      "bg-background text-foreground hover:bg-background",
                  )}
                >
                  <FileText className="h-3.5 w-3.5" />
                  {t("common.edit")}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={loadingFile}
                  onClick={() => setEditorMode("preview")}
                  className={cn(
                    "h-7 gap-1.5 px-2.5 text-xs shadow-none",
                    editorMode === "preview" &&
                      "bg-background text-foreground hover:bg-background",
                  )}
                >
                  <Eye className="h-3.5 w-3.5" />
                  {t("knowledge.mode.preview")}
                </Button>
              </div>
            </div>
            <div className="min-h-[22rem] overflow-hidden rounded-lg border border-border/70 bg-background">
              {editorMode === "edit" ? (
                <Textarea
                  id="project-memory-content"
                  surface="embedded"
                  value={draft.content}
                  disabled={loadingFile}
                  readOnly={readOnly}
                  spellCheck={false}
                  className="min-h-[22rem] resize-y px-4 py-3 font-mono text-sm"
                  onChange={(event) =>
                    setDraft((value) => ({ ...value, content: event.target.value }))
                  }
                />
              ) : (
                <div className="min-h-[22rem] max-h-[32rem] overflow-y-auto px-5 py-4">
                  {draft.content.trim() ? (
                    <MarkdownRenderer content={draft.content} />
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      {t("project.autoMemory.emptyPreview")}
                    </p>
                  )}
                </div>
              )}
            </div>
          </div>

          <div className="flex items-center justify-between">
            <div className="text-[11px] text-muted-foreground">
              {draft.fileName ?? t("project.autoMemory.fileNameGenerated")}
            </div>
            {!readOnly && (
              <div className="flex gap-2">
                {draft.fileName && (
                  <Button variant="outline" onClick={() => void deleteDraft()}>
                    <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                    {t("common.delete")}
                  </Button>
                )}
                <Button
                  onClick={() => void saveDraft()}
                  disabled={
                    saving || loadingFile || !draft.name.trim() || !draft.description.trim()
                  }
                >
                  {saving ? t("common.saving") : t("common.save")}
                </Button>
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  )
}
