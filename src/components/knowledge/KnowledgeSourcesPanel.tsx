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
  KnowledgeSource,
  KnowledgeSourceImportInput,
  KnowledgeSourceKind,
  KnowledgeSourceReadResult,
} from "@/types/knowledge"

import KnowledgeCompilePanel from "./KnowledgeCompilePanel"

interface KnowledgeSourcesPanelProps {
  kbId: string | null
}

type ImportMode = "url" | "text" | "file"

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
  const [fileName, setFileName] = useState("")
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
    return text.trim().length > 0
  }, [importing, kbId, mode, text, url])

  function resetImport() {
    setTitle("")
    setUrl("")
    setText("")
    setFileName("")
    setMode("url")
    if (fileInputRef.current) fileInputRef.current.value = ""
  }

  async function importSource() {
    if (!kbId || !canImport) return
    setImporting(true)
    const input: KnowledgeSourceImportInput =
      mode === "url"
        ? { url: url.trim(), title: title.trim() || null, kind: "url_snapshot" }
        : {
            content: text,
            title: title.trim() || null,
            fileName: fileName || null,
            kind: mode === "file" ? inferKind(fileName) : "text",
          }
    try {
      await getTransport().call<KnowledgeSource>("kb_source_import_cmd", { kbId, input })
      toast.success(t("knowledge.sources.imported", "Source imported"))
      setImportOpen(false)
      resetImport()
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

  async function onPickFile(file: File | null) {
    if (!file) return
    try {
      const content = await file.text()
      setMode("file")
      setFileName(file.name)
      setTitle((v) => v || stripExt(file.name))
      setText(content)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::file", "file read failed", e)
      toast.error(t("knowledge.sources.fileReadFailed", "Couldn't read file"))
    }
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
                  {source.kind === "url_snapshot" ? (
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
            <TabsList className="grid w-full grid-cols-3">
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
                  accept=".md,.markdown,.txt,text/markdown,text/plain"
                  className="hidden"
                  onChange={(e) => void onPickFile(e.target.files?.[0] ?? null)}
                />
                <Button
                  type="button"
                  variant="outline"
                  className="w-fit gap-1.5"
                  onClick={() => fileInputRef.current?.click()}
                >
                  <Upload className="h-3.5 w-3.5" />
                  {t("knowledge.sources.chooseFile", "Choose file")}
                </Button>
                {fileName ? (
                  <div className="rounded-md border border-border-soft/60 px-3 py-2 text-xs">
                    <div className="truncate font-medium">{fileName}</div>
                    <div className="mt-1 text-muted-foreground">
                      {formatBytes(new Blob([text]).size)}
                    </div>
                  </div>
                ) : null}
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
  return lower.endsWith(".md") || lower.endsWith(".markdown") ? "markdown" : "text"
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
