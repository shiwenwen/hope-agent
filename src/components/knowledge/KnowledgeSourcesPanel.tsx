import {
  Check,
  FileText,
  Globe,
  Link2,
  Loader2,
  Plus,
  RefreshCw,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { formatBytes } from "@/lib/format"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type {
  KnowledgeBrowserCaptureMode,
  KnowledgeBrowserSourceImportInput,
  KnowledgeSource,
  KnowledgeSourceImportInput,
  KnowledgeSourceKind,
  KnowledgeSourceReadResult,
} from "@/types/knowledge"

import KnowledgeCompilePanel from "./KnowledgeCompilePanel"

interface KnowledgeSourcesPanelProps {
  kbId: string | null
}

type ImportMode = "url" | "text" | "file" | "browser"

interface SourceFileDraft {
  file: File
  kind: KnowledgeSourceKind
}

const SOURCE_FILE_ACCEPT =
  ".md,.markdown,.txt,.pdf,.docx,text/markdown,text/plain,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document"

export default function KnowledgeSourcesPanel({ kbId }: KnowledgeSourcesPanelProps) {
  const { t } = useTranslation()
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [sources, setSources] = useState<KnowledgeSource[]>([])
  const [loading, setLoading] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [importing, setImporting] = useState(false)
  const [mode, setMode] = useState<ImportMode>("url")
  const [title, setTitle] = useState("")
  const [url, setUrl] = useState("")
  const [text, setText] = useState("")
  const [fileDrafts, setFileDrafts] = useState<SourceFileDraft[]>([])
  const [browserMode, setBrowserMode] = useState<KnowledgeBrowserCaptureMode>("auto")
  const [selected, setSelected] = useState<KnowledgeSourceReadResult | null>(null)
  const [reading, setReading] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<KnowledgeSource | null>(null)
  const [selectedSourceIds, setSelectedSourceIds] = useState<Set<string>>(() => new Set())
  const [compileOpen, setCompileOpen] = useState(false)
  const [compileSourceIds, setCompileSourceIds] = useState<string[]>([])
  const [compileRequestToken, setCompileRequestToken] = useState(0)

  const reload = useCallback(async () => {
    if (!kbId) {
      setSources([])
      return
    }
    setLoading(true)
    try {
      const list = await getTransport().call<KnowledgeSource[]>("kb_source_list_cmd", { kbId })
      setSources(list)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::reload", "source list failed", e)
    } finally {
      setLoading(false)
    }
  }, [kbId])

  useEffect(() => {
    void reload()
  }, [reload])

  useEffect(() => {
    setSelectedSourceIds(new Set())
    setCompileSourceIds([])
  }, [kbId])

  useEffect(() => {
    setSelectedSourceIds((prev) => {
      const live = new Set(sources.map((source) => source.id))
      const next = new Set([...prev].filter((id) => live.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [sources])

  useEffect(() => {
    return getTransport().listen("knowledge:changed", () => void reload())
  }, [reload])

  const canImport = useMemo(() => {
    if (!kbId || importing) return false
    if (mode === "url") return url.trim().length > 0
    if (mode === "file") return fileDrafts.length > 0
    if (mode === "browser") return true
    return text.trim().length > 0
  }, [fileDrafts.length, importing, kbId, mode, text, url])

  function resetImport() {
    setTitle("")
    setUrl("")
    setText("")
    setFileDrafts([])
    setBrowserMode("auto")
    setMode("url")
    if (fileInputRef.current) fileInputRef.current.value = ""
  }

  async function importSource() {
    if (!kbId || !canImport) return
    setImporting(true)
    try {
      if (mode === "browser") {
        const input: KnowledgeBrowserSourceImportInput = {
          mode: browserMode,
          title: title.trim() || null,
        }
        await getTransport().call<KnowledgeSource>("kb_source_import_browser_cmd", { kbId, input })
        toast.success(t("knowledge.sources.imported", "Source imported"))
        setImportOpen(false)
        resetImport()
      } else if (mode === "file") {
        const failed: SourceFileDraft[] = []
        let imported = 0
        const singleTitle = fileDrafts.length === 1 ? title.trim() || null : null
        for (const draft of fileDrafts) {
          try {
            const input = await inputForFileDraft(draft, singleTitle)
            await getTransport().call<KnowledgeSource>("kb_source_import_cmd", { kbId, input })
            imported += 1
          } catch (e) {
            logger.warn("knowledge", "KnowledgeSourcesPanel::import", "source file import failed", e)
            failed.push(draft)
          }
        }
        if (imported > 0) {
          toast.success(
            t("knowledge.sources.importedCount", {
              defaultValue: "Imported {{count}} sources",
              count: imported,
            }),
          )
        }
        if (failed.length > 0) {
          setFileDrafts(failed)
          toast.error(
            t("knowledge.sources.importFailedCount", {
              defaultValue: "Couldn't import {{count}} sources",
              count: failed.length,
            }),
          )
        } else {
          setImportOpen(false)
          resetImport()
        }
      } else {
        const input: KnowledgeSourceImportInput =
          mode === "url"
            ? { url: url.trim(), title: title.trim() || null, kind: "url_snapshot" }
            : {
                content: text,
                title: title.trim() || null,
                kind: "text",
              }
        await getTransport().call<KnowledgeSource>("kb_source_import_cmd", { kbId, input })
        toast.success(t("knowledge.sources.imported", "Source imported"))
        setImportOpen(false)
        resetImport()
      }
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::import", "source import failed", e)
      toast.error(t("knowledge.sources.importFailed", "Couldn't import source"))
    } finally {
      setImporting(false)
    }
  }

  async function openSource(source: KnowledgeSource) {
    if (!kbId) return
    setReading(true)
    try {
      const data = await getTransport().call<KnowledgeSourceReadResult>("kb_source_read_cmd", {
        kbId,
        sourceId: source.id,
      })
      setSelected(data)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::read", "source read failed", e)
      toast.error(t("knowledge.sources.readFailed", "Couldn't open source"))
    } finally {
      setReading(false)
    }
  }

  async function deleteSource() {
    if (!kbId || !deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    try {
      await getTransport().call<boolean>("kb_source_delete_cmd", { kbId, sourceId: target.id })
      if (selected?.id === target.id) setSelected(null)
      toast.success(t("knowledge.sources.deleted", "Source deleted"))
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::delete", "source delete failed", e)
      toast.error(t("knowledge.sources.deleteFailed", "Couldn't delete source"))
    }
  }

  async function reextractSource(source: KnowledgeSource) {
    if (!kbId) return
    try {
      const updated = await getTransport().call<KnowledgeSource>("kb_source_reextract_cmd", {
        kbId,
        sourceId: source.id,
      })
      setSources((items) => items.map((item) => (item.id === updated.id ? updated : item)))
      toast.success(t("knowledge.sources.reextracted", "Source re-extracted"))
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::reextract", "source reextract failed", e)
      toast.error(t("knowledge.sources.reextractFailed", "Couldn't re-extract source"))
    }
  }

  function toggleSourceSelection(sourceId: string) {
    setSelectedSourceIds((prev) => {
      const next = new Set(prev)
      if (next.has(sourceId)) {
        next.delete(sourceId)
      } else {
        next.add(sourceId)
      }
      return next
    })
  }

  function openCompile(ids: string[]) {
    if (!kbId || ids.length === 0) return
    setCompileSourceIds(ids)
    setCompileRequestToken((n) => n + 1)
    setCompileOpen(true)
  }

  function onPickFiles(files: FileList | null) {
    const picked = Array.from(files ?? [])
    if (picked.length === 0) return
    if (fileInputRef.current) fileInputRef.current.value = ""
    const drafts = picked.map((file) => ({ file, kind: inferKind(file.name) }))
    setMode("file")
    setFileDrafts(drafts)
    setTitle((v) => (picked.length === 1 ? v || stripExt(picked[0].name) : v))
  }

  const selectedIdsInOrder = sources
    .filter((source) => selectedSourceIds.has(source.id))
    .map((source) => source.id)
  const selectedCount = selectedIdsInOrder.length

  if (!kbId) {
    return (
      <div className="px-3 py-3 text-xs text-muted-foreground">
        {t("knowledge.sources.noKb", "Select a space to see sources.")}
      </div>
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between border-b border-border-soft/60 px-2 py-1.5">
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
          {t("knowledge.sources.title", "Sources")}
        </span>
        <div className="flex items-center gap-1">
          <IconTip
            label={t("knowledge.sources.compileSelected", "Compile selected sources")}
            side="bottom"
          >
            <Button
              variant="ghost"
              size="icon"
              className="relative h-6 w-6"
              onClick={() => openCompile(selectedIdsInOrder)}
              disabled={selectedCount === 0}
            >
              <Sparkles className="h-3 w-3" />
              {selectedCount > 0 ? (
                <span className="absolute -right-1 -top-1 rounded-full bg-primary px-1 text-[9px] leading-3 text-primary-foreground">
                  {selectedCount}
                </span>
              ) : null}
            </Button>
          </IconTip>
          <IconTip label={t("knowledge.sources.refresh", "Refresh")} side="bottom">
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={() => void reload()}
              disabled={loading}
            >
              <Loader2 className={cn("h-3 w-3", loading && "animate-spin")} />
            </Button>
          </IconTip>
          <IconTip label={t("knowledge.sources.import", "Import source")} side="bottom">
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={() => setImportOpen(true)}
            >
              <Plus className="h-3 w-3" />
            </Button>
          </IconTip>
        </div>
      </div>

      <div className="flex-1 overflow-auto py-0.5">
        {sources.length === 0 && !loading ? (
          <div className="px-3 py-3 text-xs text-muted-foreground">
            {t("knowledge.sources.empty", "No sources yet.")}
          </div>
        ) : null}
        {sources.map((source) => (
          <ContextMenu key={source.id}>
            <ContextMenuTrigger asChild>
              <div className="flex w-full min-w-0 items-start gap-2 px-2 py-2 text-left text-xs hover:bg-muted/50">
                <button
                  type="button"
                  aria-pressed={selectedSourceIds.has(source.id)}
                  className={cn(
                    "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-border-soft/70 text-primary",
                    selectedSourceIds.has(source.id) && "border-primary bg-primary/10",
                  )}
                  onClick={(e) => {
                    e.stopPropagation()
                    toggleSourceSelection(source.id)
                  }}
                >
                  {selectedSourceIds.has(source.id) ? <Check className="h-3 w-3" /> : null}
                </button>
                <button
                  type="button"
                  className="flex min-w-0 flex-1 items-start gap-2 text-left"
                  onClick={() => void openSource(source)}
                >
                  {source.kind === "url_snapshot" || source.kind === "browser_snapshot" ? (
                    <Globe className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <FileText className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-foreground/90">
                      {source.title}
                    </span>
                    <span className="mt-0.5 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
                      <span>{formatBytes(source.size)}</span>
                      <span>·</span>
                      <span>{sourceKindLabel(source.kind)}</span>
                      <span>·</span>
                      <span>{source.chunkCount}</span>
                      <span>·</span>
                      <span>{formatDate(source.createdAt)}</span>
                      <span>·</span>
                      <span>
                        {source.compiledAt
                          ? t("knowledge.sources.compiled", "Compiled")
                          : t("knowledge.sources.uncompiled", "Uncompiled")}
                      </span>
                      {source.originUri ? (
                        <>
                          <span>·</span>
                          <Link2 className="h-2.5 w-2.5" />
                        </>
                      ) : null}
                    </span>
                  </span>
                </button>
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem onClick={() => void openSource(source)}>
                <FileText className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.open", "Open")}
              </ContextMenuItem>
              <ContextMenuItem onClick={() => openCompile([source.id])}>
                <Sparkles className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.compileOne", "Compile")}
              </ContextMenuItem>
              <ContextMenuItem onClick={() => void reextractSource(source)}>
                <RefreshCw className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.reextract", "Re-extract")}
              </ContextMenuItem>
              <ContextMenuItem
                className="text-destructive focus:text-destructive"
                onClick={() => setDeleteTarget(source)}
              >
                <Trash2 className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.delete", "Delete")}
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        ))}
      </div>

      <KnowledgeCompilePanel
        kbId={kbId}
        open={compileOpen}
        onOpenChange={setCompileOpen}
        sourceIds={compileSourceIds}
        requestToken={compileRequestToken}
        onAfterRun={() => void reload()}
        onAfterApply={() => void reload()}
      />

      <Dialog open={importOpen} onOpenChange={(open) => {
        setImportOpen(open)
        if (!open && !importing) resetImport()
      }}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.import", "Import source")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.sources.importDesc", "Add raw material to this knowledge space.")}
            </DialogDescription>
          </DialogHeader>
          <Tabs value={mode} onValueChange={(v) => setMode(v as ImportMode)}>
            <TabsList className="grid w-full grid-cols-4">
              <TabsTrigger value="url" className="gap-1.5 text-xs">
                <Globe className="h-3.5 w-3.5" />
                {t("knowledge.sources.url", "URL")}
              </TabsTrigger>
              <TabsTrigger value="text" className="gap-1.5 text-xs">
                <FileText className="h-3.5 w-3.5" />
                {t("knowledge.sources.text", "Text")}
              </TabsTrigger>
              <TabsTrigger value="file" className="gap-1.5 text-xs">
                <Upload className="h-3.5 w-3.5" />
                {t("knowledge.sources.file", "File")}
              </TabsTrigger>
              <TabsTrigger value="browser" className="gap-1.5 text-xs">
                <Globe className="h-3.5 w-3.5" />
                {t("knowledge.sources.browser", "Browser")}
              </TabsTrigger>
            </TabsList>
            <div className="mt-3 space-y-3">
              <Input
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder={t("knowledge.sources.titlePlaceholder", "Title")}
              />
              <TabsContent value="url" className="mt-0">
                <Input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://example.com/article"
                />
              </TabsContent>
              <TabsContent value="text" className="mt-0">
                <Textarea
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                  placeholder={t("knowledge.sources.textPlaceholder", "Paste source text…")}
                  className="min-h-64 font-mono text-xs"
                />
              </TabsContent>
              <TabsContent value="file" className="mt-0 gap-3">
                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  accept={SOURCE_FILE_ACCEPT}
                  className="hidden"
                  onChange={(e) => onPickFiles(e.target.files)}
                />
                <Button
                  type="button"
                  variant="outline"
                  className="w-fit gap-1.5"
                  onClick={() => fileInputRef.current?.click()}
                >
                  <Upload className="h-3.5 w-3.5" />
                  {t("knowledge.sources.chooseFile", "Choose files")}
                </Button>
                {fileDrafts.length > 0 ? (
                  <div className="max-h-48 overflow-auto rounded-md border border-border-soft/60 text-xs">
                    {fileDrafts.map((draft) => (
                      <div
                        key={`${draft.file.name}-${draft.file.lastModified}-${draft.file.size}`}
                        className="flex min-w-0 items-center gap-2 border-b border-border-soft/40 px-3 py-2 last:border-b-0"
                      >
                        <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-medium">{draft.file.name}</div>
                          <div className="mt-0.5 text-muted-foreground">
                            {sourceKindLabel(draft.kind)} · {formatBytes(draft.file.size)}
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                ) : null}
              </TabsContent>
              <TabsContent value="browser" className="mt-0">
                <Tabs
                  value={browserMode}
                  onValueChange={(v) => setBrowserMode(v as KnowledgeBrowserCaptureMode)}
                >
                  <TabsList className="grid w-full grid-cols-3">
                    <TabsTrigger value="auto" className="text-xs">
                      {t("knowledge.sources.browserAuto", "Auto")}
                    </TabsTrigger>
                    <TabsTrigger value="selection" className="text-xs">
                      {t("knowledge.sources.browserSelection", "Selection")}
                    </TabsTrigger>
                    <TabsTrigger value="page" className="text-xs">
                      {t("knowledge.sources.browserPage", "Page")}
                    </TabsTrigger>
                  </TabsList>
                </Tabs>
              </TabsContent>
            </div>
          </Tabs>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setImportOpen(false)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="button" onClick={() => void importSource()} disabled={!canImport}>
              {importing && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
              {t("knowledge.sources.importAction", "Import")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!selected} onOpenChange={(open) => !open && setSelected(null)}>
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle className="truncate">{selected?.title}</DialogTitle>
            {selected?.originUri ? (
              <DialogDescription className="truncate">{selected.originUri}</DialogDescription>
            ) : null}
          </DialogHeader>
          <pre className="max-h-[70vh] overflow-auto whitespace-pre-wrap rounded-md border border-border-soft/60 bg-muted/30 p-3 text-xs leading-relaxed">
            {reading ? t("knowledge.sources.loading", "Loading…") : selected?.content}
          </pre>
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.deleteTitle", "Delete source")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.sources.deleteBody", {
                defaultValue: "Delete {{name}} from the raw source inbox?",
                name: deleteTarget?.title ?? "",
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setDeleteTarget(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button type="button" variant="destructive" onClick={() => void deleteSource()}>
              {t("knowledge.sources.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function inferKind(fileName: string): KnowledgeSourceKind {
  const lower = fileName.toLowerCase()
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) return "markdown"
  if (lower.endsWith(".pdf")) return "pdf"
  if (lower.endsWith(".docx")) return "docx"
  return "text"
}

async function inputForFileDraft(
  draft: SourceFileDraft,
  title: string | null,
): Promise<KnowledgeSourceImportInput> {
  const mimeType = draft.file.type || defaultMimeType(draft.kind)
  if (draft.kind === "pdf" || draft.kind === "docx") {
    return {
      kind: draft.kind,
      title,
      fileName: draft.file.name,
      mimeType,
      dataBase64: await fileToBase64(draft.file),
    }
  }
  return {
    kind: draft.kind,
    title,
    fileName: draft.file.name,
    mimeType,
    content: await draft.file.text(),
  }
}

function defaultMimeType(kind: KnowledgeSourceKind): string {
  switch (kind) {
    case "markdown":
      return "text/markdown"
    case "pdf":
      return "application/pdf"
    case "docx":
      return "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    case "browser_snapshot":
      return "text/markdown"
    case "url_snapshot":
      return "text/markdown"
    case "text":
    default:
      return "text/plain"
  }
}

async function fileToBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const chunks: string[] = []
  const chunkSize = 0x8000
  for (let i = 0; i < bytes.length; i += chunkSize) {
    chunks.push(String.fromCharCode(...bytes.subarray(i, i + chunkSize)))
  }
  return btoa(chunks.join(""))
}

function sourceKindLabel(kind: KnowledgeSourceKind): string {
  switch (kind) {
    case "markdown":
      return "Markdown"
    case "pdf":
      return "PDF"
    case "docx":
      return "DOCX"
    case "browser_snapshot":
      return "Browser"
    case "url_snapshot":
      return "URL"
    case "text":
    default:
      return "Text"
  }
}

function stripExt(fileName: string): string {
  return fileName.replace(/\.[^.]+$/, "")
}

function formatDate(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return ""
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
    }).format(new Date(ms))
  } catch {
    return ""
  }
}
