import {
  Check,
  Download,
  EyeOff,
  ExternalLink,
  FileAudio,
  FileText,
  FileVideo,
  GitCompare,
  Globe,
  History,
  Image as ImageIcon,
  Layers,
  Link2,
  Loader2,
  Plus,
  RefreshCw,
  RotateCcw,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react"
import type { TFunction } from "i18next"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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
  KnowledgeEvidenceClaim,
  KnowledgeSource,
  KnowledgeSourceDiff,
  KnowledgeSourceImportBatchInput,
  KnowledgeSourceImportRun,
  KnowledgeSourceImportRunDetail,
  KnowledgeSourceImportInput,
  KnowledgeSourceKind,
  KnowledgeSourceOcrPage,
  KnowledgeSourceReadResult,
  KnowledgeSourceRefreshResult,
  KnowledgeSourceSimilarityGroup,
  KnowledgeSourceSimilarityResolveResult,
  KnowledgeSourceVersionHistory,
} from "@/types/knowledge"

import KnowledgeCompilePanel from "./KnowledgeCompilePanel"

interface KnowledgeSourcesPanelProps {
  kbId: string | null
}

type ImportMode = "url" | "text" | "file" | "browser"
type UrlSourceKind = "url_snapshot" | "audio_transcript" | "video_transcript" | "image_ocr"

interface SourceFileDraft {
  file: File
  kind: KnowledgeSourceKind
}

const SOURCE_FILE_ACCEPT =
  ".md,.markdown,.txt,.pdf,.docx,.mp3,.m4a,.wav,.ogg,.opus,.flac,.mp4,.mov,.m4v,.webm,.mkv,.png,.jpg,.jpeg,.webp,.gif,.bmp,.tif,.tiff,.heic,text/markdown,text/plain,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,audio/*,video/*,image/*"

export default function KnowledgeSourcesPanel({ kbId }: KnowledgeSourcesPanelProps) {
  const { t } = useTranslation()
  const fileInputRef = useRef<HTMLInputElement>(null)
  const sourceReadTokenRef = useRef(0)
  const [sources, setSources] = useState<KnowledgeSource[]>([])
  const [importRuns, setImportRuns] = useState<KnowledgeSourceImportRun[]>([])
  const [runDetail, setRunDetail] = useState<KnowledgeSourceImportRunDetail | null>(null)
  const [similarGroups, setSimilarGroups] = useState<KnowledgeSourceSimilarityGroup[]>([])
  const [loading, setLoading] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [similarOpen, setSimilarOpen] = useState(false)
  const [importing, setImporting] = useState(false)
  const [retryingRunId, setRetryingRunId] = useState<string | null>(null)
  const [resolvingSimilarityId, setResolvingSimilarityId] = useState<string | null>(null)
  const [mode, setMode] = useState<ImportMode>("url")
  const [urlKind, setUrlKind] = useState<UrlSourceKind>("url_snapshot")
  const [title, setTitle] = useState("")
  const [url, setUrl] = useState("")
  const [text, setText] = useState("")
  const [fileDrafts, setFileDrafts] = useState<SourceFileDraft[]>([])
  const [browserMode, setBrowserMode] = useState<KnowledgeBrowserCaptureMode>("auto")
  const [selected, setSelected] = useState<KnowledgeSourceReadResult | null>(null)
  const [sourceClaims, setSourceClaims] = useState<KnowledgeEvidenceClaim[]>([])
  const [reading, setReading] = useState(false)
  const [claimsLoading, setClaimsLoading] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<KnowledgeSource | null>(null)
  const [refreshingSourceId, setRefreshingSourceId] = useState<string | null>(null)
  const [versionHistory, setVersionHistory] = useState<KnowledgeSourceVersionHistory | null>(null)
  const [sourceDiff, setSourceDiff] = useState<KnowledgeSourceDiff | null>(null)
  const [diffLoading, setDiffLoading] = useState(false)
  const [selectedSourceIds, setSelectedSourceIds] = useState<Set<string>>(() => new Set())
  const [compileOpen, setCompileOpen] = useState(false)
  const [compileSourceIds, setCompileSourceIds] = useState<string[]>([])
  const [compileRequestToken, setCompileRequestToken] = useState(0)
  const [ocrRoundActive, setOcrRoundActive] = useState<Record<string, boolean>>({})

  const reload = useCallback(async () => {
    if (!kbId) {
      setSources([])
      setImportRuns([])
      setSimilarGroups([])
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
    try {
      const runs = await getTransport().call<KnowledgeSourceImportRun[]>(
        "kb_source_import_runs_list_cmd",
        { kbId, limit: 8 },
      )
      setImportRuns(runs)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::reload", "source import runs failed", e)
    }
    try {
      const groups = await getTransport().call<KnowledgeSourceSimilarityGroup[]>(
        "kb_source_similarity_groups_cmd",
        { kbId },
      )
      setSimilarGroups(groups)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::reload", "source groups failed", e)
    }
  }, [kbId])

  useEffect(() => {
    void reload()
  }, [reload])

  useEffect(() => {
    setSelectedSourceIds(new Set())
    setCompileSourceIds([])
    setRunDetail(null)
    setImportRuns([])
    setSimilarGroups([])
    setResolvingSimilarityId(null)
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

  // `partially_extracted` is set the instant a scanned-PDF OCR placeholder is
  // created (0 pages attempted yet), not just once a round genuinely leaves
  // some pages failed — so the status alone can't tell "still running" from
  // "finished with real failures" apart. Cross-check against the per-page
  // ledger for any source in that state to decide which badge to show.
  // Re-runs whenever `sources` refreshes (including the `knowledge:changed`
  // reload fired when an OCR round finishes), so it naturally stays current.
  useEffect(() => {
    const pdfPartial = sources.filter(
      (s) => s.kind === "pdf" && s.status === "partially_extracted",
    )
    if (!kbId || pdfPartial.length === 0) return
    let cancelled = false
    void (async () => {
      const entries = await Promise.all(
        pdfPartial.map(async (s) => {
          try {
            const pages = await getTransport().call<KnowledgeSourceOcrPage[]>(
              "kb_source_ocr_pages_cmd",
              { kbId, sourceId: s.id },
            )
            const active = pages.some((p) => p.status === "pending" || p.status === "running")
            return [s.id, active] as const
          } catch (e) {
            logger.warn(
              "knowledge",
              "KnowledgeSourcesPanel::ocrRoundActive",
              "OCR page status fetch failed",
              e,
            )
            return [s.id, false] as const
          }
        }),
      )
      if (cancelled) return
      setOcrRoundActive(Object.fromEntries(entries))
    })()
    return () => {
      cancelled = true
    }
  }, [sources, kbId])

  const activeRunId = runDetail?.status === "running" ? runDetail.id : null

  useEffect(() => {
    if (!kbId || !activeRunId) return
    let cancelled = false
    const refreshRun = async () => {
      try {
        const detail = await getTransport().call<KnowledgeSourceImportRunDetail>(
          "kb_source_import_run_detail_cmd",
          { kbId, runId: activeRunId },
        )
        if (cancelled) return
        setRunDetail(detail)
        setImportRuns((prev) =>
          prev.map((run) =>
            run.id === detail.id
              ? {
                  ...run,
                  status: detail.status,
                  backgroundJobId: detail.backgroundJobId,
                  totalCount: detail.totalCount,
                  importedCount: detail.importedCount,
                  duplicateCount: detail.duplicateCount,
                  failedCount: detail.failedCount,
                  startedAt: detail.startedAt,
                  finishedAt: detail.finishedAt,
                  updatedAt: detail.updatedAt,
                }
              : run,
          ),
        )
        if (detail.status !== "running") {
          await reload()
        }
      } catch (e) {
        logger.warn("knowledge", "KnowledgeSourcesPanel::pollRun", "source run poll failed", e)
      }
    }
    void refreshRun()
    const timer = window.setInterval(() => void refreshRun(), 1500)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [activeRunId, kbId, reload])

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
    setUrlKind("url_snapshot")
    setMode("url")
    if (fileInputRef.current) fileInputRef.current.value = ""
  }

  function showImportRunToast(detail: KnowledgeSourceImportRunDetail) {
    if (detail.status === "running") {
      toast.info(
        t("knowledge.sources.importRunStarted", {
          defaultValue: "Import started for {{count}} source(s)",
          count: detail.totalCount,
        }),
      )
      return
    }
    const imported = detail.importedCount
    const duplicate = detail.duplicateCount
    const failed = detail.failedCount
    if (failed > 0) {
      toast.error(
        t("knowledge.sources.importRunPartial", {
          defaultValue: "Imported {{imported}}, skipped {{duplicate}} duplicate, failed {{failed}}",
          imported,
          duplicate,
          failed,
        }),
      )
    } else if (duplicate > 0) {
      toast.success(
        t("knowledge.sources.importRunDeduped", {
          defaultValue: "Imported {{imported}}, skipped {{duplicate}} duplicate",
          imported,
          duplicate,
        }),
      )
    } else {
      toast.success(
        t("knowledge.sources.importedCount", {
          defaultValue: "Imported {{count}} sources",
          count: imported,
        }),
      )
    }
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
        const singleTitle = fileDrafts.length === 1 ? title.trim() || null : null
        const items = await Promise.all(
          fileDrafts.map(async (draft, idx) => ({
            clientId: `${draft.file.name}-${draft.file.lastModified}-${draft.file.size}-${idx}`,
            label: draft.file.name,
            input: await inputForFileDraft(draft, singleTitle),
          })),
        )
        const detail = await getTransport().call<KnowledgeSourceImportRunDetail>(
          "kb_source_import_batch_cmd",
          { kbId, input: { items } satisfies KnowledgeSourceImportBatchInput },
        )
        setRunDetail(detail)
        showImportRunToast(detail)
        const failedPositions = new Set(
          detail.items.filter((item) => item.status === "failed").map((item) => item.position),
        )
        const failed = fileDrafts.filter((_, idx) => failedPositions.has(idx))
        if (detail.status !== "running" && failed.length > 0) {
          setFileDrafts(failed)
        } else {
          setImportOpen(false)
          resetImport()
        }
      } else {
        const input: KnowledgeSourceImportInput =
          mode === "url"
            ? { url: url.trim(), title: title.trim() || null, kind: urlKind }
            : {
                content: text,
                title: title.trim() || null,
                kind: "text",
              }
        const detail = await getTransport().call<KnowledgeSourceImportRunDetail>(
          "kb_source_import_batch_cmd",
          {
            kbId,
            input: {
              items: [
                {
                  label: title.trim() || (mode === "url" ? url.trim() : null),
                  input,
                },
              ],
            } satisfies KnowledgeSourceImportBatchInput,
          },
        )
        setRunDetail(detail)
        showImportRunToast(detail)
        if (detail.status === "running" || detail.failedCount === 0) {
          setImportOpen(false)
          resetImport()
        }
      }
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::import", "source import failed", e)
      toast.error(t("knowledge.sources.importFailed", "Couldn't import source"))
    } finally {
      setImporting(false)
    }
  }

  async function openRunDetail(run: KnowledgeSourceImportRun) {
    if (!kbId) return
    try {
      const detail = await getTransport().call<KnowledgeSourceImportRunDetail>(
        "kb_source_import_run_detail_cmd",
        { kbId, runId: run.id },
      )
      setRunDetail(detail)
      setHistoryOpen(true)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::runDetail", "source run detail failed", e)
      toast.error(t("knowledge.sources.importHistoryFailed", "Couldn't open import history"))
    }
  }

  async function retryFailed(run: KnowledgeSourceImportRun) {
    if (!kbId || retryingRunId) return
    setRetryingRunId(run.id)
    try {
      const detail = await getTransport().call<KnowledgeSourceImportRunDetail>(
        "kb_source_import_retry_failed_cmd",
        { kbId, runId: run.id },
      )
      setRunDetail(detail)
      showImportRunToast(detail)
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::retryFailed", "source retry failed", e)
      toast.error(t("knowledge.sources.retryFailed", "Couldn't retry failed imports"))
    } finally {
      setRetryingRunId(null)
    }
  }

  async function dismissSimilarityGroup(group: KnowledgeSourceSimilarityGroup) {
    if (!kbId || resolvingSimilarityId) return
    setResolvingSimilarityId(`${group.id}:dismiss`)
    try {
      const groups = await getTransport().call<KnowledgeSourceSimilarityGroup[]>(
        "kb_source_similarity_dismiss_cmd",
        { kbId, input: { fingerprint: group.fingerprint, reason: "dismissed" } },
      )
      setSimilarGroups(groups)
      toast.message(t("knowledge.sources.similarDismissed", "Similarity suggestion hidden"))
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::dismissSimilar", "source similarity dismiss failed", e)
      toast.error(t("knowledge.sources.similarDismissFailed", "Couldn't hide similarity suggestion"))
    } finally {
      setResolvingSimilarityId(null)
    }
  }

  async function resolveSimilarityGroup(group: KnowledgeSourceSimilarityGroup, keep: KnowledgeSource) {
    if (!kbId || resolvingSimilarityId) return
    const deleteSourceIds = group.sources
      .filter((source) => source.kbId === kbId && source.id !== keep.id)
      .map((source) => source.id)
    if (deleteSourceIds.length === 0) {
      toast.message(t("knowledge.sources.noLocalDuplicates", "No duplicate source in this space to delete"))
      return
    }
    const confirmed = window.confirm(
      t("knowledge.sources.resolveSimilarConfirm", {
        defaultValue: "Keep {{name}} and delete {{count}} duplicate source(s) from this space?",
        name: keep.title,
        count: deleteSourceIds.length,
      }),
    )
    if (!confirmed) return
    const token = `${group.id}:${keep.id}`
    setResolvingSimilarityId(token)
    try {
      const result = await getTransport().call<KnowledgeSourceSimilarityResolveResult>(
        "kb_source_similarity_resolve_cmd",
        {
          kbId,
          input: {
            fingerprint: group.fingerprint,
            keepSourceId: keep.id,
            deleteSourceIds,
          },
        },
      )
      toast.success(
        t("knowledge.sources.similarResolved", {
          defaultValue: "Deleted {{count}} duplicate source(s)",
          count: result.deletedSourceIds.length,
        }),
      )
      setRunDetail(null)
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::resolveSimilar", "source similarity resolve failed", e)
      toast.error(t("knowledge.sources.similarResolveFailed", "Couldn't resolve duplicate sources"))
    } finally {
      setResolvingSimilarityId(null)
    }
  }

  async function openSource(source: KnowledgeSource) {
    if (!kbId) return
    const token = sourceReadTokenRef.current + 1
    sourceReadTokenRef.current = token
    setSelected(null)
    setSourceClaims([])
    setReading(true)
    setClaimsLoading(true)
    void getTransport()
      .call<KnowledgeEvidenceClaim[]>("kb_evidence_source_claims_cmd", {
        kbId,
        sourceId: source.id,
      })
      .then((claims) => {
        if (sourceReadTokenRef.current === token) setSourceClaims(claims)
      })
      .catch((e) => {
        logger.warn("knowledge", "KnowledgeSourcesPanel::read", "source claims failed", e)
      })
      .finally(() => {
        if (sourceReadTokenRef.current === token) setClaimsLoading(false)
      })
    try {
      const data = await getTransport().call<KnowledgeSourceReadResult>("kb_source_read_cmd", {
        kbId,
        sourceId: source.id,
      })
      if (sourceReadTokenRef.current === token) {
        setSelected(data)
      }
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::read", "source read failed", e)
      toast.error(t("knowledge.sources.readFailed", "Couldn't open source"))
    } finally {
      if (sourceReadTokenRef.current === token) setReading(false)
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

  async function retryOcrPages(source: KnowledgeSource) {
    if (!kbId) return
    try {
      const updated = await getTransport().call<KnowledgeSource>("kb_source_ocr_retry_cmd", {
        kbId,
        sourceId: source.id,
      })
      setSources((items) => items.map((item) => (item.id === updated.id ? updated : item)))
      toast.success(t("knowledge.sources.ocrRetryStarted", "Retrying failed pages…"))
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::retryOcr", "OCR page retry failed", e)
      toast.error(t("knowledge.sources.ocrRetryFailed", "Couldn't retry OCR pages"))
    }
  }

  async function refreshSource(source: KnowledgeSource) {
    if (!kbId || !isRefreshableSourceKind(source.kind) || refreshingSourceId) return
    setRefreshingSourceId(source.id)
    try {
      const result = await getTransport().call<KnowledgeSourceRefreshResult>(
        "kb_source_refresh_cmd",
        {
          kbId,
          sourceId: source.id,
          input: { browserMode: "auto", requireSameUrl: true },
        },
      )
      if (!result.changed) {
        toast.info(t("knowledge.sources.refreshUnchanged", "Source is already up to date"))
        setSources((items) =>
          items.map((item) => (item.id === result.source.id ? result.source : item)),
        )
        return
      }
      toast.success(
        t("knowledge.sources.refreshChanged", {
          defaultValue: "Source refreshed to v{{version}}",
          version: result.source.versionIndex ?? 1,
        }),
      )
      if (result.diff) setSourceDiff(result.diff)
      await reload()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::refresh", "source refresh failed", e)
      toast.error(t("knowledge.sources.refreshFailed", "Couldn't refresh source"))
    } finally {
      setRefreshingSourceId(null)
    }
  }

  async function openVersions(source: KnowledgeSource) {
    if (!kbId) return
    try {
      const history = await getTransport().call<KnowledgeSourceVersionHistory>(
        "kb_source_versions_cmd",
        { kbId, sourceId: source.id },
      )
      setVersionHistory(history)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::versions", "source versions failed", e)
      toast.error(t("knowledge.sources.versionsFailed", "Couldn't load source versions"))
    }
  }

  async function openSourceDiff(fromSourceId: string, toSourceId: string) {
    if (!kbId) return
    setSourceDiff(null)
    setDiffLoading(true)
    try {
      const diff = await getTransport().call<KnowledgeSourceDiff>("kb_source_diff_cmd", {
        kbId,
        sourceId: fromSourceId,
        toSourceId,
      })
      setSourceDiff(diff)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeSourcesPanel::diff", "source diff failed", e)
      toast.error(t("knowledge.sources.diffFailed", "Couldn't load source diff"))
    } finally {
      setDiffLoading(false)
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
    const drafts = picked.map((file) => ({ file, kind: inferKind(file.name, file.type) }))
    setMode("file")
    setFileDrafts(drafts)
    setTitle((v) => (picked.length === 1 ? v || stripExt(picked[0].name) : v))
  }

  const selectedSources = sources.filter((source) => selectedSourceIds.has(source.id))
  const selectedIdsInOrder = selectedSources.map((source) => source.id)
  const selectedCount = selectedIdsInOrder.length
  const hasUnreadySelected = selectedSources.some((source) => source.status !== "ready")
  const latestRun = importRuns[0]

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
          <IconTip label={t("knowledge.sources.importHistory", "Import history")} side="bottom">
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={() => setHistoryOpen(true)}
              disabled={importRuns.length === 0}
            >
              <History className="h-3 w-3" />
            </Button>
          </IconTip>
          <IconTip label={t("knowledge.sources.similarGroups", "Similar sources")} side="bottom">
            <Button
              variant="ghost"
              size="icon"
              className="relative h-6 w-6"
              onClick={() => setSimilarOpen(true)}
              disabled={similarGroups.length === 0}
            >
              <Layers className="h-3 w-3" />
              {similarGroups.length > 0 ? (
                <span className="absolute -right-1 -top-1 rounded-full bg-amber-500 px-1 text-[9px] leading-3 text-white">
                  {similarGroups.length}
                </span>
              ) : null}
            </Button>
          </IconTip>
          <IconTip
            label={t("knowledge.sources.compileSelected", "Organize selected into notes")}
            side="bottom"
          >
            <Button
              variant="ghost"
              size="icon"
              className="relative h-6 w-6"
              onClick={() => openCompile(selectedIdsInOrder)}
              disabled={selectedCount === 0 || hasUnreadySelected}
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

      {latestRun || similarGroups.length > 0 ? (
        <div className="border-b border-border-soft/50 px-2 py-1 text-[10px] text-muted-foreground">
          {latestRun ? (
            <button
              type="button"
              className="mr-2 rounded-sm px-1 py-0.5 hover:bg-muted/60"
              onClick={() => void openRunDetail(latestRun)}
            >
              {formatDate(latestRun.createdAt)} · +{latestRun.importedCount} · ={latestRun.duplicateCount} · !{latestRun.failedCount}
            </button>
          ) : null}
          {similarGroups.length > 0 ? (
            <button
              type="button"
              className="rounded-sm px-1 py-0.5 text-amber-600 hover:bg-muted/60 dark:text-amber-400"
              onClick={() => setSimilarOpen(true)}
            >
              {t("knowledge.sources.similarCount", {
                defaultValue: "{{count}} similar groups",
                count: similarGroups.length,
              })}
            </button>
          ) : null}
        </div>
      ) : null}

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
                  <SourceKindIcon
                    source={source}
                    className="mt-0.5 h-7 w-7 shrink-0"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-foreground/90">
                      {source.title}
                    </span>
                    <span className="mt-0.5 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
                      <span>{formatBytes(source.size)}</span>
                      <span>·</span>
                      <span>{sourceKindLabel(source.kind, t)}</span>
                      {(source.versionIndex ?? 1) > 1 ? (
                        <>
                          <span>·</span>
                          <span>{sourceVersionLabel(source)}</span>
                        </>
                      ) : null}
                      <span>·</span>
                      <span>{source.chunkCount}</span>
                      <span>·</span>
                      <span>{formatDate(source.createdAt)}</span>
                      <span>·</span>
                      <span>
                        {source.compiledAt
                          ? t("knowledge.sources.compiled", "Organized")
                          : t("knowledge.sources.uncompiled", "Not organized")}
                      </span>
                      {source.originUri ? (
                        <>
                          <span>·</span>
                          <Link2 className="h-2.5 w-2.5" />
                        </>
                      ) : null}
                      {source.assets?.original ? (
                        <>
                          <span>·</span>
                          <span>{t("knowledge.sources.mediaRetained", "Media retained")}</span>
                        </>
                      ) : null}
                      {source.status === "partially_extracted" ? (
                        <>
                          <span>·</span>
                          {ocrRoundActive[source.id] ? (
                            <span className="inline-flex items-center gap-1 text-muted-foreground">
                              <Loader2 className="h-2.5 w-2.5 animate-spin" />
                              {t("knowledge.sources.ocrRunning", "OCR running…")}
                            </span>
                          ) : (
                            <span className="text-amber-600 dark:text-amber-500">
                              {t("knowledge.sources.ocrPartial", "OCR: some pages failed")}
                            </span>
                          )}
                        </>
                      ) : null}
                      {source.status === "failed" && source.kind === "pdf" ? (
                        <>
                          <span>·</span>
                          <span className="text-destructive">
                            {t("knowledge.sources.ocrFailed", "OCR failed")}
                          </span>
                        </>
                      ) : null}
                      {source.externalRawPath ? (
                        <>
                          <span>·</span>
                          <span className="font-mono">{source.externalRawPath}</span>
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
              <ContextMenuItem
                disabled={source.status !== "ready"}
                onClick={() => openCompile([source.id])}
              >
                <Sparkles className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.compileOne", "Organize into note")}
              </ContextMenuItem>
              <ContextMenuItem
                disabled={!isRefreshableSourceKind(source.kind) || refreshingSourceId === source.id}
                onClick={() => void refreshSource(source)}
              >
                {refreshingSourceId === source.id ? (
                  <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="mr-2 h-3.5 w-3.5" />
                )}
                {t("knowledge.sources.refreshSource", "Refresh snapshot")}
              </ContextMenuItem>
              <ContextMenuItem onClick={() => void openVersions(source)}>
                <History className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.versionHistory", "Version history")}
              </ContextMenuItem>
              <ContextMenuItem onClick={() => void reextractSource(source)}>
                <RefreshCw className="mr-2 h-3.5 w-3.5" />
                {t("knowledge.sources.reextract", "Re-extract")}
              </ContextMenuItem>
              {source.kind === "pdf" &&
              (source.status === "partially_extracted" || source.status === "failed") ? (
                <ContextMenuItem onClick={() => void retryOcrPages(source)}>
                  <RotateCcw className="mr-2 h-3.5 w-3.5" />
                  {t("knowledge.sources.ocrRetry", "Retry failed OCR pages")}
                </ContextMenuItem>
              ) : null}
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
                <div className="grid gap-2 sm:grid-cols-[11rem_1fr]">
                  <Select value={urlKind} onValueChange={(v) => setUrlKind(v as UrlSourceKind)}>
                    <SelectTrigger className="h-9 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="url_snapshot">
                        {t("knowledge.sources.urlKindPage", "Web page")}
                      </SelectItem>
                      <SelectItem value="audio_transcript">
                        {t("knowledge.sources.urlKindAudio", "Audio transcript")}
                      </SelectItem>
                      <SelectItem value="video_transcript">
                        {t("knowledge.sources.urlKindVideo", "Video transcript")}
                      </SelectItem>
                      <SelectItem value="image_ocr">
                        {t("knowledge.sources.urlKindImage", "Image OCR")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  <Input
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                    placeholder={
                      urlKind === "url_snapshot"
                        ? "https://example.com/article"
                        : "https://example.com/media"
                    }
                  />
                </div>
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
                        <SourceKindIcon
                          kind={draft.kind}
                          className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                        />
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-medium">{draft.file.name}</div>
                          <div className="mt-0.5 text-muted-foreground">
                            {sourceKindLabel(draft.kind, t)} · {formatBytes(draft.file.size)}
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

      <Dialog
        open={!!selected}
        onOpenChange={(open) => {
          if (!open) {
            sourceReadTokenRef.current += 1
            setSelected(null)
            setSourceClaims([])
            setClaimsLoading(false)
          }
        }}
      >
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle className="truncate">{selected?.title}</DialogTitle>
            {selected?.originUri ? (
              <DialogDescription className="truncate">{selected.originUri}</DialogDescription>
            ) : null}
          </DialogHeader>
          {selected?.assets?.original || selected?.assets?.thumbnail ? (
            <SourceAssetSummary source={selected} />
          ) : null}
          {selected?.externalRawPath ? (
            <div className="rounded-md border border-border-soft/60 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
              <span>{t("knowledge.sources.externalRawPath", "External snapshot")}</span>
              <span className="mx-1">·</span>
              <span className="font-mono text-foreground/80">{selected.externalRawPath}</span>
            </div>
          ) : null}
          <SourceClaimsSummary claims={sourceClaims} loading={claimsLoading} />
          <pre className="max-h-[54vh] overflow-auto whitespace-pre-wrap rounded-md border border-border-soft/60 bg-muted/30 p-3 text-xs leading-relaxed">
            {reading ? t("knowledge.sources.loading", "Loading…") : selected?.content}
          </pre>
        </DialogContent>
      </Dialog>

      <Dialog
        open={!!versionHistory}
        onOpenChange={(open) => !open && setVersionHistory(null)}
      >
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.versionHistory", "Version history")}</DialogTitle>
            <DialogDescription>
              {versionHistory
                ? t("knowledge.sources.versionHistoryDesc", {
                    defaultValue: "{{count}} archived snapshots. Current: {{current}}",
                    count: versionHistory.versions.length,
                    current: versionHistory.currentSourceId.slice(0, 8),
                  })
                : null}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-[70vh] space-y-2 overflow-auto">
            {versionHistory?.versions.map((version, index) => {
              const older = versionHistory.versions[index + 1]
              const current = version.id === versionHistory.currentSourceId
              return (
                <div
                  key={version.id}
                  className={cn(
                    "rounded-md border border-border-soft/60 p-2 text-xs",
                    current && "border-primary/60 bg-primary/5",
                  )}
                >
                  <div className="flex min-w-0 items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="flex min-w-0 items-center gap-1.5">
                        <span className="shrink-0 rounded-sm bg-muted px-1.5 py-0.5 text-[10px] font-medium">
                          {sourceVersionLabel(version)}
                        </span>
                        <span className="truncate font-medium">{version.title}</span>
                        {current ? (
                          <span className="shrink-0 rounded-sm bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
                            {t("knowledge.sources.currentVersion", "Current")}
                          </span>
                        ) : null}
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                        <span>{formatDateTime(version.createdAt)}</span>
                        <span>·</span>
                        <span>{formatBytes(version.size)}</span>
                        <span>·</span>
                        <span className="font-mono">{version.id.slice(0, 8)}</span>
                        {version.originUri ? (
                          <>
                            <span>·</span>
                            <span className="truncate">{version.originUri}</span>
                          </>
                        ) : null}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <IconTip label={t("knowledge.sources.open", "Open")}>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          onClick={() => {
                            setVersionHistory(null)
                            void openSource(version)
                          }}
                        >
                          <FileText className="h-3 w-3" />
                        </Button>
                      </IconTip>
                      <IconTip label={t("knowledge.sources.comparePrevious", "Compare previous")}>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          disabled={!older || diffLoading}
                          onClick={() => older && void openSourceDiff(older.id, version.id)}
                        >
                          {diffLoading ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <GitCompare className="h-3 w-3" />
                          )}
                        </Button>
                      </IconTip>
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        </DialogContent>
      </Dialog>

      <Dialog
        open={!!sourceDiff || diffLoading}
        onOpenChange={(open) => {
          if (!open) {
            setSourceDiff(null)
            setDiffLoading(false)
          }
        }}
      >
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.sourceDiff", "Source diff")}</DialogTitle>
            {sourceDiff ? (
              <DialogDescription className="truncate">
                {sourceDiff.fromTitle} -&gt; {sourceDiff.toTitle}
              </DialogDescription>
            ) : null}
          </DialogHeader>
          {diffLoading && !sourceDiff ? (
            <div className="flex items-center gap-2 rounded-md border border-border-soft/60 p-3 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("knowledge.sources.loadingDiff", "Loading diff...")}
            </div>
          ) : sourceDiff ? (
            <div className="space-y-2">
              <div className="flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                <span className="rounded-sm bg-emerald-500/10 px-1.5 py-0.5 text-emerald-600 dark:text-emerald-400">
                  +{sourceDiff.addedLines}
                </span>
                <span className="rounded-sm bg-destructive/10 px-1.5 py-0.5 text-destructive">
                  -{sourceDiff.removedLines}
                </span>
                {sourceDiff.truncated ? (
                  <span>{t("knowledge.sources.diffTruncated", "Preview truncated")}</span>
                ) : null}
              </div>
              <div className="max-h-[70vh] overflow-auto rounded-md border border-border-soft/60 bg-muted/20 font-mono text-[11px] leading-relaxed">
                {sourceDiff.lines.map((line, index) => (
                  <div
                    key={`${line.kind}-${index}-${line.oldLine ?? ""}-${line.newLine ?? ""}`}
                    className={cn(
                      "grid grid-cols-[3rem_3rem_1rem_minmax(0,1fr)] gap-2 px-2 py-0.5",
                      diffLineClass(line.kind),
                    )}
                  >
                    <span className="select-none text-right text-muted-foreground">
                      {line.oldLine ?? ""}
                    </span>
                    <span className="select-none text-right text-muted-foreground">
                      {line.newLine ?? ""}
                    </span>
                    <span className="select-none">{diffLinePrefix(line.kind)}</span>
                    <span className="whitespace-pre-wrap break-words">{line.text || " "}</span>
                  </div>
                ))}
              </div>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>

      <Dialog open={historyOpen} onOpenChange={setHistoryOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.importHistory", "Import history")}</DialogTitle>
          </DialogHeader>
          <div className="grid max-h-[70vh] gap-3 overflow-auto md:grid-cols-[minmax(0,0.95fr)_minmax(0,1.2fr)]">
            <div className="space-y-2">
              {importRuns.length === 0 ? (
                <div className="rounded-md border border-border-soft/60 p-3 text-xs text-muted-foreground">
                  {t("knowledge.sources.noImportRuns", "No import runs yet.")}
                </div>
              ) : null}
              {importRuns.map((run) => (
                <div
                  key={run.id}
                  className={cn(
                    "rounded-md border border-border-soft/60 p-2 text-xs",
                    runDetail?.id === run.id && "border-primary/60 bg-primary/5",
                  )}
                >
                  <button
                    type="button"
                    className="w-full rounded-sm text-left hover:bg-muted/50"
                    onClick={() => void openRunDetail(run)}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-medium">{runStatusLabel(run.status, t)}</span>
                      <span className="text-[10px] text-muted-foreground">
                        {formatDateTime(run.createdAt)}
                      </span>
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                      <span>{run.totalCount}</span>
                      <span>·</span>
                      <span>+{run.importedCount}</span>
                      <span>·</span>
                      <span>={run.duplicateCount}</span>
                      <span>·</span>
                      <span>!{run.failedCount}</span>
                    </div>
                  </button>
                  {run.status !== "running" && run.failedCount > 0 ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="mt-2 h-6 gap-1 px-2 text-[10px]"
                      onClick={(e) => {
                        e.stopPropagation()
                        void retryFailed(run)
                      }}
                      disabled={!!retryingRunId}
                    >
                      {retryingRunId === run.id ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <RotateCcw className="h-3 w-3" />
                      )}
                      {t("knowledge.sources.retryFailedAction", "Retry failed")}
                    </Button>
                  ) : null}
                </div>
              ))}
            </div>
            <div className="min-w-0 rounded-md border border-border-soft/60">
              {runDetail ? (
                <div className="divide-y divide-border-soft/50">
                  {runDetail.items.map((item) => (
                    <div key={item.id} className="p-2 text-xs">
                      <div className="flex min-w-0 items-center justify-between gap-2">
                        <span className="truncate font-medium">
                          {item.label || item.sourceId || `#${item.position + 1}`}
                        </span>
                        <span className={cn("shrink-0 text-[10px]", item.status === "failed" && "text-destructive")}>
                          {itemStatusLabel(item.status, t)}
                        </span>
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                        {item.kind ? <span>{sourceKindLabel(item.kind, t)}</span> : null}
                        {item.sourceId ? (
                          <>
                            <span>·</span>
                            <span className="font-mono">{item.sourceId.slice(0, 8)}</span>
                          </>
                        ) : null}
                        {item.duplicateOfSourceId ? (
                          <>
                            <span>·</span>
                            <span className="font-mono">={item.duplicateOfSourceId.slice(0, 8)}</span>
                          </>
                        ) : null}
                      </div>
                      {item.error ? (
                        <div className="mt-1 rounded bg-destructive/10 px-2 py-1 text-[10px] text-destructive">
                          {item.error}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : (
                <div className="p-3 text-xs text-muted-foreground">
                  {t("knowledge.sources.selectImportRun", "Select an import run.")}
                </div>
              )}
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={similarOpen} onOpenChange={setSimilarOpen}>
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle>{t("knowledge.sources.similarGroups", "Similar sources")}</DialogTitle>
          </DialogHeader>
          <div className="max-h-[70vh] space-y-2 overflow-auto">
            {similarGroups.length === 0 ? (
              <div className="rounded-md border border-border-soft/60 p-3 text-xs text-muted-foreground">
                {t("knowledge.sources.noSimilarGroups", "No similar source groups.")}
              </div>
            ) : null}
            {similarGroups.map((group) => (
              <div key={group.id} className="rounded-md border border-border-soft/60 p-2 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className="font-medium">{groupKindLabel(group.kind, t)}</span>
                    <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {groupScopeLabel(group.scope, t)}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-[10px] text-muted-foreground">
                      {Math.round(group.similarity * 100)}%
                    </span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 gap-1 px-2 text-[10px]"
                      onClick={() => void dismissSimilarityGroup(group)}
                      disabled={!!resolvingSimilarityId}
                    >
                      {resolvingSimilarityId === `${group.id}:dismiss` ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <EyeOff className="h-3 w-3" />
                      )}
                      {t("knowledge.sources.dismissSimilar", "Ignore")}
                    </Button>
                  </div>
                </div>
                <div className="mt-2 space-y-1">
                  {group.sources.map((source) => (
                    <button
                      key={source.id}
                      type="button"
                      className={cn(
                        "flex w-full min-w-0 items-center justify-between gap-2 rounded px-2 py-1 text-left",
                        source.kbId === kbId ? "hover:bg-muted/60" : "cursor-default opacity-80",
                      )}
                      onClick={() => {
                        if (source.kbId !== kbId) return
                        setSimilarOpen(false)
                        void openSource(source)
                      }}
                    >
                      <span className="min-w-0 truncate">
                        {source.title}
                        {source.kbId !== kbId ? (
                          <span className="ml-1 text-[10px] text-muted-foreground">
                            {t("knowledge.sources.otherKb", {
                              defaultValue: "Other space {{id}}",
                              id: source.kbId.slice(0, 8),
                            })}
                          </span>
                        ) : null}
                      </span>
                      <span className="shrink-0 text-[10px] text-muted-foreground">
                        {sourceKindLabel(source.kind, t)} · {formatBytes(source.size)}
                      </span>
                    </button>
                  ))}
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5 border-t border-border-soft/50 pt-2">
                  {group.sources
                    .filter((source) => source.kbId === kbId)
                    .map((source) => {
                      const deleteCount = localDuplicateDeleteCount(group, source.id, kbId)
                      const token = `${group.id}:${source.id}`
                      return (
                        <Button
                          key={source.id}
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-6 gap-1 px-2 text-[10px]"
                          onClick={() => void resolveSimilarityGroup(group, source)}
                          disabled={deleteCount === 0 || !!resolvingSimilarityId}
                        >
                          {resolvingSimilarityId === token ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <Check className="h-3 w-3" />
                          )}
                          {t("knowledge.sources.keepSource", {
                            defaultValue: "Keep {{name}}",
                            name: truncateLabel(source.title, 22),
                          })}
                          {deleteCount > 0 ? (
                            <span className="text-muted-foreground">-{deleteCount}</span>
                          ) : null}
                        </Button>
                      )
                    })}
                </div>
              </div>
            ))}
          </div>
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

function inferKind(fileName: string, mimeType?: string): KnowledgeSourceKind {
  const mime = (mimeType || "").toLowerCase()
  if (mime.startsWith("audio/")) return "audio_transcript"
  if (mime.startsWith("video/")) return "video_transcript"
  if (mime.startsWith("image/")) return "image_ocr"
  const lower = fileName.toLowerCase()
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) return "markdown"
  if (lower.endsWith(".pdf")) return "pdf"
  if (lower.endsWith(".docx")) return "docx"
  if (hasExt(lower, [".mp3", ".m4a", ".wav", ".ogg", ".opus", ".flac"])) {
    return "audio_transcript"
  }
  if (hasExt(lower, [".mp4", ".mov", ".m4v", ".webm", ".mkv"])) {
    return "video_transcript"
  }
  if (
    hasExt(lower, [
      ".png",
      ".jpg",
      ".jpeg",
      ".webp",
      ".gif",
      ".bmp",
      ".tif",
      ".tiff",
      ".heic",
    ])
  ) {
    return "image_ocr"
  }
  return "text"
}

async function inputForFileDraft(
  draft: SourceFileDraft,
  title: string | null,
): Promise<KnowledgeSourceImportInput> {
  const mimeType = draft.file.type || defaultMimeType(draft.kind)
  if (isBinarySourceKind(draft.kind)) {
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
    case "audio_transcript":
      return "audio/mpeg"
    case "video_transcript":
      return "video/mp4"
    case "image_ocr":
      return "image/png"
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

function sourceKindLabel(kind: KnowledgeSourceKind, t: TFunction): string {
  switch (kind) {
    case "markdown":
      return t("knowledge.sources.kind.markdown", "Markdown")
    case "pdf":
      return t("knowledge.sources.kind.pdf", "PDF")
    case "docx":
      return t("knowledge.sources.kind.docx", "DOCX")
    case "audio_transcript":
      return t("knowledge.sources.kind.audioTranscript", "Audio transcript")
    case "video_transcript":
      return t("knowledge.sources.kind.videoTranscript", "Video transcript")
    case "image_ocr":
      return t("knowledge.sources.kind.imageOcr", "Image OCR")
    case "browser_snapshot":
      return t("knowledge.sources.kind.browserSnapshot", "Browser")
    case "url_snapshot":
      return t("knowledge.sources.kind.urlSnapshot", "URL")
    case "text":
    default:
      return t("knowledge.sources.kind.text", "Text")
  }
}

function SourceAssetSummary({ source }: { source: KnowledgeSource }) {
  const { t } = useTranslation()
  const transport = getTransport()
  const original = source.assets?.original ?? null
  const thumbnail = source.assets?.thumbnail ?? null
  const thumbnailUrl = thumbnail?.localPath ? transport.resolveAssetUrl(thumbnail.localPath) : null
  const originalUrl = original?.localPath ? transport.resolveAssetUrl(original.localPath) : null

  function openOriginal() {
    if (!original?.localPath) return
    if (transport.supportsLocalFileOps()) {
      void transport.openFilePath(original.localPath)
      return
    }
    if (originalUrl) window.open(originalUrl, "_blank", "noopener,noreferrer")
  }

  function downloadOriginal() {
    if (!original?.localPath) return
    if (transport.supportsLocalFileOps()) {
      void transport.downloadFilePath(original.localPath, { filename: original.fileName })
      return
    }
    if (originalUrl) {
      const url = new URL(originalUrl)
      url.searchParams.set("download", "1")
      const a = document.createElement("a")
      a.href = url.toString()
      a.download = original.fileName
      a.rel = "noopener noreferrer"
      document.body.appendChild(a)
      a.click()
      a.remove()
    }
  }

  return (
    <div className="flex items-center gap-3 rounded-md border border-border-soft/60 bg-muted/20 p-2 text-xs">
      {thumbnailUrl ? (
        <img
          src={thumbnailUrl}
          alt=""
          className="h-14 w-14 shrink-0 rounded border border-border-soft/60 object-cover"
        />
      ) : (
        <SourceKindIcon
          kind={source.kind}
          className="h-10 w-10 shrink-0 rounded border border-border-soft/60 p-2 text-muted-foreground"
        />
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium">
          {original?.fileName ?? t("knowledge.sources.retainedMedia", "Retained media")}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
          {original ? (
            <>
              <span>{original.mimeType}</span>
              <span>·</span>
              <span>{formatBytes(original.size)}</span>
              {original.width && original.height ? (
                <>
                  <span>·</span>
                  <span>{original.width}x{original.height}</span>
                </>
              ) : null}
            </>
          ) : (
            <span>{t("knowledge.sources.thumbnailOnly", "Thumbnail only")}</span>
          )}
        </div>
      </div>
      {original?.localPath ? (
        <div className="flex shrink-0 items-center gap-1">
          <IconTip label={t("knowledge.sources.openOriginal", "Open original")}>
            <Button type="button" variant="ghost" size="icon" className="h-7 w-7" onClick={openOriginal}>
              <ExternalLink className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("knowledge.sources.downloadOriginal", "Download original")}>
            <Button type="button" variant="ghost" size="icon" className="h-7 w-7" onClick={downloadOriginal}>
              <Download className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      ) : null}
    </div>
  )
}

function SourceClaimsSummary({
  claims,
  loading,
}: {
  claims: KnowledgeEvidenceClaim[]
  loading: boolean
}) {
  const { t } = useTranslation()
  const visibleClaims = claims.slice(0, 24)

  return (
    <div className="rounded-md border border-border-soft/60 bg-muted/20 p-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5 font-medium">
          <FileText className="h-3.5 w-3.5 text-primary" />
          <span>{t("knowledge.sources.compiledClaims", "Note claims using this source")}</span>
          {claims.length > 0 ? (
            <span className="rounded-full bg-muted px-1.5 text-[10px] text-muted-foreground">
              {claims.length}
            </span>
          ) : null}
        </div>
        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
      </div>
      {loading ? (
        <div className="mt-2 text-[11px] text-muted-foreground">
          {t("knowledge.sources.loadingClaims", "Loading evidence index...")}
        </div>
      ) : claims.length === 0 ? (
        <div className="mt-2 text-[11px] text-muted-foreground">
          {t(
            "knowledge.sources.noCompiledClaims",
            "No note claim cites this source yet.",
          )}
        </div>
      ) : (
        <div className="mt-2 max-h-40 space-y-1 overflow-auto">
          {visibleClaims.map((claim) => (
            <div
              key={`${claim.relPath}:${claim.sourceId}:${claim.claimIndex}`}
              className="rounded bg-background/70 px-2 py-1.5"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <span className="truncate font-medium">{claim.noteTitle}</span>
                <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                  {claim.relPath}
                </span>
                {claim.stale ? (
                  <span className="shrink-0 rounded bg-amber-500/10 px-1 text-[10px] text-amber-700 dark:text-amber-300">
                    {t("knowledge.sources.staleSource", "Source changed after it was organized")}
                  </span>
                ) : null}
                {claim.missing ? (
                  <span className="shrink-0 rounded bg-destructive/10 px-1 text-[10px] text-destructive">
                    {t("knowledge.sources.missingSource", "Missing source")}
                  </span>
                ) : null}
              </div>
              <div className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-muted-foreground">
                {claim.claimText}
              </div>
            </div>
          ))}
          {claims.length > visibleClaims.length ? (
            <div className="px-2 py-1 text-[10px] text-muted-foreground">
              {t("knowledge.sources.moreClaims", {
                defaultValue: "+{{count}} more claims",
                count: claims.length - visibleClaims.length,
              })}
            </div>
          ) : null}
        </div>
      )}
    </div>
  )
}

function SourceKindIcon({
  kind,
  source,
  className,
}: {
  kind?: KnowledgeSourceKind
  source?: KnowledgeSource
  className?: string
}) {
  if (source) {
    const thumbnail = source.assets?.thumbnail
    const thumbnailUrl = thumbnail?.localPath ? getTransport().resolveAssetUrl(thumbnail.localPath) : null
    if (thumbnailUrl) {
      return (
        <img
          src={thumbnailUrl}
          alt=""
          className={cn("rounded border border-border-soft/60 object-cover", className)}
        />
      )
    }
    kind = source.kind
  }
  kind = kind ?? "text"
  switch (kind) {
    case "audio_transcript":
      return <FileAudio className={className} />
    case "video_transcript":
      return <FileVideo className={className} />
    case "image_ocr":
      return <ImageIcon className={className} />
    case "browser_snapshot":
    case "url_snapshot":
      return <Globe className={className} />
    case "markdown":
    case "pdf":
    case "docx":
    case "text":
    default:
      return <FileText className={className} />
  }
}

function isBinarySourceKind(kind: KnowledgeSourceKind): boolean {
  return (
    kind === "pdf" ||
    kind === "docx" ||
    kind === "audio_transcript" ||
    kind === "video_transcript" ||
    kind === "image_ocr"
  )
}

function isRefreshableSourceKind(kind: KnowledgeSourceKind): boolean {
  return kind === "url_snapshot" || kind === "browser_snapshot"
}

function sourceVersionLabel(source: Pick<KnowledgeSource, "versionIndex">): string {
  return `v${source.versionIndex ?? 1}`
}

function diffLineClass(kind: "context" | "added" | "removed"): string {
  switch (kind) {
    case "added":
      return "bg-emerald-500/10 text-emerald-800 dark:text-emerald-200"
    case "removed":
      return "bg-destructive/10 text-destructive"
    case "context":
    default:
      return ""
  }
}

function diffLinePrefix(kind: "context" | "added" | "removed"): string {
  switch (kind) {
    case "added":
      return "+"
    case "removed":
      return "-"
    case "context":
    default:
      return " "
  }
}

function hasExt(fileName: string, exts: string[]): boolean {
  return exts.some((ext) => fileName.endsWith(ext))
}

function runStatusLabel(status: KnowledgeSourceImportRun["status"], t: TFunction): string {
  switch (status) {
    case "completed":
      return t("knowledge.sources.runStatus.completed", "Completed")
    case "completed_with_errors":
      return t("knowledge.sources.runStatus.completedWithErrors", "Completed with errors")
    case "failed":
      return t("knowledge.sources.runStatus.failed", "Failed")
    case "running":
    default:
      return t("knowledge.sources.runStatus.running", "Running")
  }
}

function itemStatusLabel(status: KnowledgeSourceImportRunDetail["items"][number]["status"], t: TFunction): string {
  switch (status) {
    case "imported":
      return t("knowledge.sources.itemStatus.imported", "Imported")
    case "duplicate":
      return t("knowledge.sources.itemStatus.duplicate", "Duplicate")
    case "failed":
      return t("knowledge.sources.itemStatus.failed", "Failed")
    case "running":
      return t("knowledge.sources.itemStatus.running", "Running")
    case "pending":
    default:
      return t("knowledge.sources.itemStatus.pending", "Pending")
  }
}

function groupKindLabel(kind: KnowledgeSourceSimilarityGroup["kind"], t: TFunction): string {
  switch (kind) {
    case "exact_duplicate":
      return t("knowledge.sources.similarKind.exactDuplicate", "Exact duplicate")
    case "similar":
    default:
      return t("knowledge.sources.similarKind.similar", "Similar")
  }
}

function groupScopeLabel(scope: KnowledgeSourceSimilarityGroup["scope"], t: TFunction): string {
  return scope === "cross_kb"
    ? t("knowledge.sources.similarScope.crossKb", "Cross-space")
    : t("knowledge.sources.similarScope.sameKb", "This space")
}

function localDuplicateDeleteCount(
  group: KnowledgeSourceSimilarityGroup,
  keepSourceId: string,
  kbId: string | null,
): number {
  if (!kbId) return 0
  return group.sources.filter((source) => source.kbId === kbId && source.id !== keepSourceId).length
}

function truncateLabel(value: string, max: number): string {
  if (value.length <= max) return value
  return `${value.slice(0, Math.max(1, max - 1))}...`
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

function formatDateTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return ""
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(ms))
  } catch {
    return ""
  }
}
