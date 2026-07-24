import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import {
  Archive,
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  Download,
  ExternalLink,
  FileCheck2,
  History,
  Loader2,
  Maximize2,
  Minimize2,
  PackageOpen,
  PanelLeft,
  PanelLeftDashed,
  PanelRight,
  PanelRightDashed,
  RefreshCw,
  RotateCcw,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react"
import { toast } from "sonner"

import type { ArtifactExportFormat, ArtifactRecord, ArtifactVersionSummary } from "@/lib/transport"
import { downloadBlob } from "@/lib/fileDownload"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { useDragWidth } from "@/hooks/useDragWidth"
import { useFullscreenTransition } from "@/hooks/useFullscreenTransition"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import ArtifactViewer from "./ArtifactViewer"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { FileTarget } from "@/components/chat/files/types"
import { FileContextMenu } from "@/components/chat/files/FileActionMenu"

interface ArtifactsViewProps {
  onBack: () => void
}

const PAGE_SIZE = 30
const LIST_WIDTH_STORAGE_KEY = "hope.artifacts.listWidth"
const LIST_COLLAPSED_STORAGE_KEY = "hope.artifacts.listCollapsed"
const LIST_DEFAULT_WIDTH = 340
const LIST_MIN_WIDTH = 260
const LIST_MAX_WIDTH = 520
const DETAILS_WIDTH_STORAGE_KEY = "hope.artifacts.detailsWidth"
const DETAILS_COLLAPSED_STORAGE_KEY = "hope.artifacts.detailsCollapsed"
const DETAILS_DEFAULT_WIDTH = 280
const DETAILS_MIN_WIDTH = 240
const DETAILS_MAX_WIDTH = 520
const LIST_WIDTH_TRANSITION =
  "transition-[width] duration-[250ms] ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width] motion-reduce:transition-none"
const LIST_SURFACE_TRANSITION =
  "transition-[opacity,transform,border-color] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] [contain:layout_paint] motion-reduce:transition-none"
const ARTIFACT_ENUM_LABEL_KEYS: Record<string, string> = {
  report: "artifacts.enumLabels.report",
  dashboard: "artifacts.enumLabels.dashboard",
  data_table: "artifacts.enumLabels.dataTable",
  explainer: "artifacts.enumLabels.explainer",
  pr_walkthrough: "artifacts.enumLabels.prWalkthrough",
  diagram: "artifacts.enumLabels.diagram",
  slides: "artifacts.enumLabels.slides",
  custom: "artifacts.enumLabels.custom",
  local_private: "artifacts.enumLabels.localPrivate",
  shareable_snapshot: "artifacts.enumLabels.shareableSnapshot",
  sensitive: "artifacts.enumLabels.sensitive",
  incognito: "artifacts.enumLabels.incognito",
  ready: "artifacts.enumLabels.ready",
  partial: "artifacts.enumLabels.partial",
  blocked: "artifacts.enumLabels.blocked",
  passed: "artifacts.enumLabels.passed",
  failed: "artifacts.enumLabels.failed",
  insufficient_data: "artifacts.enumLabels.insufficientData",
  freeform: "artifacts.enumLabels.freeform",
  analysis: "artifacts.enumLabels.analysis",
  file: "artifacts.enumLabels.file",
  local_file: "artifacts.enumLabels.localFile",
  project_file: "artifacts.enumLabels.projectFile",
  attachment: "artifacts.enumLabels.attachment",
  knowledge: "artifacts.enumLabels.knowledgeSpace",
  knowledge_space: "artifacts.enumLabels.knowledgeSpace",
  connector: "artifacts.enumLabels.connector",
  database: "artifacts.enumLabels.database",
  web: "artifacts.enumLabels.webPage",
  web_page: "artifacts.enumLabels.webPage",
  fixture: "artifacts.enumLabels.testFixture",
  text: "artifacts.enumLabels.text",
  narrative: "artifacts.enumLabels.narrative",
  legacy_canvas: "artifacts.enumLabels.legacyCanvas",
  test: "artifacts.enumLabels.test",
  private: "artifacts.enumLabels.private",
  public: "artifacts.enumLabels.public",
  session: "artifacts.enumLabels.session",
  restricted: "workspace.domainAccessScope.restricted",
  unspecified: "artifacts.enumLabels.unspecified",
  unknown: "artifacts.enumLabels.unknown",
  not_run: "artifacts.enumLabels.notVerified",
  active: "artifacts.active",
  archived: "artifacts.archived",
}
const TECHNICAL_SOURCE_TYPE_LABELS: Record<string, string> = {
  api: "API",
  csv: "CSV",
  excel: "Excel",
  html: "HTML",
  json: "JSON",
  markdown: "Markdown",
  md: "Markdown",
  pdf: "PDF",
  tsv: "TSV",
  url: "URL",
  xls: "XLS",
  xlsx: "XLSX",
}
const KINDS = [
  "report",
  "dashboard",
  "data_table",
  "explainer",
  "pr_walkthrough",
  "diagram",
  "slides",
  "custom",
]

function humanizeArtifactValue(value: string): string {
  return value
    .replace(/[._-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase())
}

function readStoredBoolean(key: string): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(key) === "true"
  } catch {
    return false
  }
}

function readStoredWidth(
  key: string,
  fallback: number,
  min = LIST_MIN_WIDTH,
  max = LIST_MAX_WIDTH,
): number {
  if (typeof window === "undefined") return fallback
  try {
    const value = Number(window.localStorage.getItem(key))
    return Number.isFinite(value) && value > 0 ? Math.min(max, Math.max(min, value)) : fallback
  } catch {
    return fallback
  }
}

function requiresLocalExportConfirmation(
  artifact: ArtifactRecord,
  format: ArtifactExportFormat,
): boolean {
  if (artifact.privacy === "sensitive") return true
  if (isExecutableArtifact(artifact) && (format === "html" || format === "zip")) {
    return true
  }
  return artifact.sourceSummaries.some((source) =>
    ["private", "connector", "sensitive"].includes(source.accessScope.trim().toLowerCase()),
  )
}

function isExecutableArtifact(artifact: ArtifactRecord): boolean {
  return (
    artifact.capabilities?.executableContent === true || artifact.capabilities?.scripts === true
  )
}

function Badge({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={`inline-flex items-center rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground ${className}`}
    >
      {children}
    </span>
  )
}

function ArtifactListRow({
  artifact,
  selected,
  onSelect,
  enumLabel,
}: {
  artifact: ArtifactRecord
  selected: boolean
  onSelect: () => void
  enumLabel: (value: string | null | undefined) => string
}) {
  const { t } = useTranslation()
  const target: Extract<FileTarget, { kind: "artifact" }> = {
    kind: "artifact",
    artifactId: artifact.id,
    name: `${artifact.title}.html`,
    projectPath: artifact.projectPath,
  }
  const resource = useFileResource(target, { onPreviewFile: onSelect })
  return (
    <FileContextMenu target={target} overrides={{ onPreviewFile: onSelect }}>
      <button
        className={cn(
          "mb-1.5 w-full rounded-xl p-3 text-left text-foreground transition-colors",
          selected ? "bg-secondary" : "hover:bg-secondary/40",
        )}
        onClick={() => void resource.run(resource.primary)}
      >
        <div className="flex items-start justify-between gap-2">
          <span className="line-clamp-2 text-sm font-medium">{artifact.title}</span>
          <Badge className="shrink-0">v{artifact.currentVersion}</Badge>
        </div>
        <div className="mt-2 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
          <span>{enumLabel(artifact.kind)}</span>
          <span>·</span>
          <span>{enumLabel(artifact.privacy)}</span>
          {isExecutableArtifact(artifact) && (
            <>
              <span>·</span>
              <span>{t("artifacts.executable", "executable")}</span>
            </>
          )}
          <span>·</span>
          <span>{new Date(artifact.updatedAt).toLocaleDateString()}</span>
        </div>
      </button>
    </FileContextMenu>
  )
}

export default function ArtifactsView({ onBack }: ArtifactsViewProps) {
  const { t } = useTranslation()
  const enumLabel = (value: string | null | undefined): string => {
    const normalized = value?.trim().toLowerCase() || "unknown"
    const technicalLabel = TECHNICAL_SOURCE_TYPE_LABELS[normalized]
    if (technicalLabel) return technicalLabel
    const key = ARTIFACT_ENUM_LABEL_KEYS[normalized]
    if (key) return t(key)
    return humanizeArtifactValue(value?.trim() || t(ARTIFACT_ENUM_LABEL_KEYS.unknown))
  }
  const [artifacts, setArtifacts] = useState<ArtifactRecord[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [selected, setSelected] = useState<ArtifactRecord | null>(null)
  const [versions, setVersions] = useState<ArtifactVersionSummary[]>([])
  const [kind, setKind] = useState("all")
  const [state, setState] = useState("active")
  const [query, setQuery] = useState("")
  const [offset, setOffset] = useState(0)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<string | null>(null)
  const [refreshKey, setRefreshKey] = useState(0)
  const [exportFormat, setExportFormat] = useState<ArtifactExportFormat>("html")
  const selectedTarget = useMemo<Extract<FileTarget, { kind: "artifact" }> | null>(
    () =>
      selected
        ? {
            kind: "artifact",
            artifactId: selected.id,
            name: `${selected.title}.html`,
            projectPath: selected.projectPath,
          }
        : null,
    [selected],
  )
  const artifactResource = useFileResource(selectedTarget)
  const [listCollapsed, setListCollapsed] = useState(() =>
    readStoredBoolean(LIST_COLLAPSED_STORAGE_KEY),
  )
  const [listWidth, setListWidth] = useState(() =>
    readStoredWidth(LIST_WIDTH_STORAGE_KEY, LIST_DEFAULT_WIDTH),
  )
  const [detailsCollapsed, setDetailsCollapsed] = useState(() =>
    readStoredBoolean(DETAILS_COLLAPSED_STORAGE_KEY),
  )
  const [detailsWidth, setDetailsWidth] = useState(() =>
    readStoredWidth(
      DETAILS_WIDTH_STORAGE_KEY,
      DETAILS_DEFAULT_WIDTH,
      DETAILS_MIN_WIDTH,
      DETAILS_MAX_WIDTH,
    ),
  )
  const [viewerMaximized, setViewerMaximized] = useState(false)
  const [isResizingList, setIsResizingList] = useState(false)
  const [isResizingDetails, setIsResizingDetails] = useState(false)
  const [isListResizeHandleHovered, setIsListResizeHandleHovered] = useState(false)
  const [isDetailsResizeHandleHovered, setIsDetailsResizeHandleHovered] = useState(false)
  const selectedIdRef = useRef<string | null>(selectedId)
  const {
    ref: viewerMainRef,
    animating: viewerAnimating,
    transitionTo: transitionViewerMaximized,
    reset: resetViewerMaximized,
  } = useFullscreenTransition<HTMLElement>({
    maximized: viewerMaximized,
    onMaximizedChange: setViewerMaximized,
  })

  const selectArtifact = useCallback((artifactId: string) => {
    selectedIdRef.current = artifactId
    setSelectedId(artifactId)
  }, [])

  const clearSelectedArtifact = useCallback(() => {
    selectedIdRef.current = null
    setSelectedId(null)
    setSelected(null)
    setVersions([])
    resetViewerMaximized()
  }, [resetViewerMaximized])

  useEffect(() => {
    try {
      window.localStorage.setItem(LIST_COLLAPSED_STORAGE_KEY, String(listCollapsed))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [listCollapsed])

  useEffect(() => {
    try {
      window.localStorage.setItem(LIST_WIDTH_STORAGE_KEY, String(Math.round(listWidth)))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [listWidth])

  useEffect(() => {
    try {
      window.localStorage.setItem(DETAILS_COLLAPSED_STORAGE_KEY, String(detailsCollapsed))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [detailsCollapsed])

  useEffect(() => {
    try {
      window.localStorage.setItem(DETAILS_WIDTH_STORAGE_KEY, String(Math.round(detailsWidth)))
    } catch {
      // localStorage may be unavailable in restricted browser modes.
    }
  }, [detailsWidth])

  useEffect(() => {
    if (!viewerMaximized) return
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || viewerAnimating) return
      event.preventDefault()
      transitionViewerMaximized(false)
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [transitionViewerMaximized, viewerAnimating, viewerMaximized])

  const onDragList = useDragWidth({
    width: listWidth,
    min: LIST_MIN_WIDTH,
    max: LIST_MAX_WIDTH,
    onChange: setListWidth,
    direction: "ltr",
    onResizingChange: setIsResizingList,
  })

  const onDragDetails = useDragWidth({
    width: detailsWidth,
    min: DETAILS_MIN_WIDTH,
    max: DETAILS_MAX_WIDTH,
    onChange: setDetailsWidth,
    direction: "rtl",
    onResizingChange: setIsResizingDetails,
  })

  const loadList = useCallback(async () => {
    setLoading(true)
    try {
      const rows = await getTransport().listArtifacts({
        limit: PAGE_SIZE,
        offset,
        kind: kind === "all" ? undefined : kind,
        lifecycleState: state === "all" ? undefined : state,
      })
      setArtifacts(rows)
      const activeSelectedId = selectedIdRef.current
      if (activeSelectedId && !rows.some((artifact) => artifact.id === activeSelectedId)) {
        clearSelectedArtifact()
      }
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t("artifacts.loadFailed", "Failed to load Artifacts"),
      )
    } finally {
      setLoading(false)
    }
  }, [clearSelectedArtifact, kind, offset, state, t])

  useEffect(() => {
    void loadList()
  }, [loadList])

  useEffect(() => {
    if (!selectedId) return
    let cancelled = false
    Promise.all([
      getTransport().getArtifact(selectedId),
      getTransport().listArtifactVersions(selectedId),
    ])
      .then(([artifact, history]) => {
        if (cancelled) return
        setSelected(artifact)
        setVersions(history)
      })
      .catch((error) => {
        if (!cancelled) toast.error(error instanceof Error ? error.message : String(error))
      })
    return () => {
      cancelled = true
    }
  }, [selectedId, refreshKey])

  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase()
    if (!needle) return artifacts
    return artifacts.filter((artifact) =>
      [artifact.title, artifact.kind, artifact.projectId, artifact.agentId]
        .filter(Boolean)
        .some((value) => String(value).toLocaleLowerCase().includes(needle)),
    )
  }, [artifacts, query])

  const runVerify = async () => {
    if (!selected) return
    setBusy("verify")
    try {
      const verification = await getTransport().verifyArtifact(selected.id)
      setSelected({ ...selected, verification })
      setArtifacts((rows) =>
        rows.map((row) => (row.id === selected.id ? { ...row, verification } : row)),
      )
      toast.success(
        verification.status === "passed"
          ? t("artifacts.verifyPassed", "Verification passed")
          : t("artifacts.verifyFailed", "Verification found blockers"),
      )
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(null)
    }
  }

  const runExport = async () => {
    if (!selected) return
    const artifactId = selected.id
    setBusy("export")
    try {
      const transport = getTransport()
      const current = await transport.getArtifact(artifactId)
      if (selectedIdRef.current !== artifactId) return
      setSelected((artifact) => (artifact?.id === current.id ? current : artifact))
      setArtifacts((rows) =>
        rows.map((artifact) => (artifact.id === current.id ? current : artifact)),
      )
      if (current.currentVersion !== selected.currentVersion) {
        setRefreshKey((key) => key + 1)
        toast.info(
          t(
            "artifacts.exportVersionChanged",
            "This Artifact changed while you were reviewing it. The latest version is now displayed; review it and export again.",
          ),
        )
        return
      }
      if (
        requiresLocalExportConfirmation(current, exportFormat) &&
        !window.confirm(
          t(
            "artifacts.localExportRiskConfirm",
            "This Artifact contains sensitive, private, connector-backed, or executable content. Exported files are managed by you, and executable HTML opened outside Hope is no longer protected by Hope's sandbox. Continue after reviewing the content?",
          ),
        )
      ) {
        return
      }
      const result = await transport.exportArtifact(
        current.id,
        exportFormat,
        current.currentVersion,
      )
      if (!result) return
      if (result.receipt.status !== "ready") {
        toast.error(result.receipt.error ?? t("artifacts.exportUnavailable", "Export unavailable"))
        return
      }
      if (result.blob) downloadBlob(result.blob, result.filename)
      toast.success(t("artifacts.exportReady", "Artifact exported"))
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(null)
    }
  }

  const runRestore = async (version: number) => {
    if (!selected || version === selected.currentVersion) return
    setBusy(`restore-${version}`)
    try {
      const artifact = await getTransport().restoreArtifact(selected.id, version)
      setSelected(artifact)
      setRefreshKey((value) => value + 1)
      await loadList()
      toast.success(t("artifacts.restoreReady", "Restored as a new version"))
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setBusy(null)
    }
  }

  const runArchive = async () => {
    if (!selected || !window.confirm(t("artifacts.archiveConfirm", "Archive this Artifact?")))
      return
    setBusy("archive")
    try {
      await getTransport().archiveArtifact(selected.id)
      clearSelectedArtifact()
      await loadList()
    } finally {
      setBusy(null)
    }
  }

  const runDelete = async () => {
    if (
      !selected ||
      !window.confirm(
        t("artifacts.deleteConfirm", "Permanently delete this Artifact and all versions?"),
      )
    )
      return
    setBusy("delete")
    try {
      await getTransport().deleteArtifact(selected.id)
      clearSelectedArtifact()
      await loadList()
    } finally {
      setBusy(null)
    }
  }

  const detailsHidden = detailsCollapsed || viewerMaximized

  return (
    <div className="flex min-w-0 flex-1 flex-col bg-background">
      <header
        className="flex h-10 shrink-0 items-center gap-2 border-b border-border-soft/60 px-3"
        data-tauri-drag-region
      >
        <IconTip label={t("common.back", "Back")} side="bottom">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={onBack}
            aria-label={t("common.back", "Back")}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </IconTip>
        <IconTip
          label={
            listCollapsed
              ? t("artifacts.expandList", "Expand artifact list")
              : t("artifacts.collapseList", "Collapse artifact list")
          }
          side="bottom"
        >
          <Button
            variant="ghost"
            size="icon"
            aria-label={
              listCollapsed
                ? t("artifacts.expandList", "Expand artifact list")
                : t("artifacts.collapseList", "Collapse artifact list")
            }
            aria-expanded={!listCollapsed}
            className="h-8 w-8"
            onClick={() => setListCollapsed((collapsed) => !collapsed)}
          >
            {listCollapsed ? (
              <PanelLeftDashed className="h-4 w-4" />
            ) : (
              <PanelLeft className="h-4 w-4" />
            )}
          </Button>
        </IconTip>
        <PackageOpen className="h-4 w-4 text-primary" data-tauri-drag-region />
        <div className="flex min-w-0 flex-1 items-baseline gap-2" data-tauri-drag-region>
          <h1 className="shrink-0 truncate text-sm font-semibold" data-tauri-drag-region>
            {t("artifacts.title", "Artifacts")}
          </h1>
          <span className="shrink-0 text-xs text-muted-foreground/40" aria-hidden="true">
            ·
          </span>
          <p className="min-w-0 truncate text-xs text-muted-foreground" data-tauri-drag-region>
            {t("artifacts.subtitle", "Versioned local reports, dashboards, and explainers")}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          onClick={() => void loadList()}
          disabled={loading}
        >
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <div
          style={{ width: listCollapsed ? 0 : listWidth }}
          className={cn("relative h-full shrink-0", !isResizingList && LIST_WIDTH_TRANSITION)}
        >
          <div className="h-full overflow-hidden">
            <aside
              style={{ width: listWidth }}
              aria-hidden={listCollapsed}
              inert={listCollapsed ? true : undefined}
              className={cn(
                "flex h-full flex-col border-r",
                isResizingList
                  ? "border-r-primary/50"
                  : isListResizeHandleHovered
                    ? "border-r-primary/35"
                    : "border-r-border-soft",
                LIST_SURFACE_TRANSITION,
                listCollapsed
                  ? "pointer-events-none -translate-x-4 opacity-0"
                  : "translate-x-0 opacity-100",
              )}
            >
              <div className="grid grid-cols-2 gap-2 border-b border-border-soft p-3">
                <div className="relative col-span-2">
                  <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/60" />
                  <SearchInput
                    className="h-9 pl-8 pr-8"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder={t("artifacts.search", "Search title, project, or Agent")}
                    aria-label={t("artifacts.search", "Search title, project, or Agent")}
                  />
                  {query && (
                    <button
                      type="button"
                      className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground transition-colors hover:text-foreground"
                      onClick={() => setQuery("")}
                      aria-label={t("common.clear", "Clear")}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>
                <Select
                  value={kind}
                  onValueChange={(value) => {
                    setKind(value)
                    setOffset(0)
                  }}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("artifacts.allKinds", "All kinds")}</SelectItem>
                    {KINDS.map((value) => (
                      <SelectItem key={value} value={value}>
                        {enumLabel(value)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select
                  value={state}
                  onValueChange={(value) => {
                    setState(value)
                    setOffset(0)
                  }}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active">{enumLabel("active")}</SelectItem>
                    <SelectItem value="archived">{enumLabel("archived")}</SelectItem>
                    <SelectItem value="all">{t("artifacts.allStates", "All states")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto p-2">
                {loading ? (
                  <div className="flex h-32 items-center justify-center">
                    <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                  </div>
                ) : filtered.length === 0 ? (
                  <div className="flex h-40 flex-col items-center justify-center gap-2 text-center text-muted-foreground">
                    <PackageOpen className="h-7 w-7" />
                    <p className="text-sm">{t("artifacts.empty", "No Artifacts found")}</p>
                  </div>
                ) : (
                  filtered.map((artifact) => (
                    <ArtifactListRow
                      key={artifact.id}
                      artifact={artifact}
                      selected={selectedId === artifact.id}
                      onSelect={() => selectArtifact(artifact.id)}
                      enumLabel={enumLabel}
                    />
                  ))
                )}
              </div>
              <div className="flex items-center justify-between border-t border-border-soft p-2">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
                >
                  <ChevronLeft className="mr-1 h-4 w-4" />
                  {t("common.previous", "Previous")}
                </Button>
                <span className="text-xs text-muted-foreground">
                  {offset + 1}–{offset + artifacts.length}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={artifacts.length < PAGE_SIZE}
                  onClick={() => setOffset(offset + PAGE_SIZE)}
                >
                  {t("common.next", "Next")}
                  <ChevronRight className="ml-1 h-4 w-4" />
                </Button>
              </div>
            </aside>
          </div>
          <div
            className={cn(
              "absolute inset-y-0 right-0 z-20 translate-x-full cursor-col-resize",
              listCollapsed ? "w-0 pointer-events-none opacity-0" : "w-3 opacity-100",
            )}
            onMouseDown={onDragList}
            onMouseEnter={() => setIsListResizeHandleHovered(true)}
            onMouseLeave={() => setIsListResizeHandleHovered(false)}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("artifacts.resizeList", "Resize artifact list")}
          />
        </div>

        {!selected ? (
          <main className="flex min-w-0 flex-1 items-center justify-center text-muted-foreground">
            <div className="text-center">
              <PackageOpen className="mx-auto mb-3 h-10 w-10" />
              <p>{t("artifacts.selectHint", "Select an Artifact to inspect it")}</p>
            </div>
          </main>
        ) : (
          <main
            ref={viewerMainRef}
            className={cn(
              "flex min-w-0 flex-1 flex-col bg-background",
              viewerMaximized && "fixed inset-0 z-50 min-h-0",
              viewerAnimating && "will-change-transform",
            )}
          >
            <div
              className={cn(
                "flex shrink-0 flex-wrap items-center gap-2 border-b border-border-soft px-3 py-2",
                viewerMaximized && "min-h-[72px] items-end pb-2 pt-7",
              )}
              data-tauri-drag-region={viewerMaximized ? true : undefined}
            >
              <div
                className="mr-auto min-w-0"
                data-tauri-drag-region={viewerMaximized ? true : undefined}
              >
                <h2
                  className="truncate text-sm font-semibold"
                  data-tauri-drag-region={viewerMaximized ? true : undefined}
                >
                  {selected.title}
                </h2>
                <p
                  className="text-xs text-muted-foreground"
                  data-tauri-drag-region={viewerMaximized ? true : undefined}
                >
                  {enumLabel(selected.kind)} · v{selected.currentVersion} · {selected.sourceCount}{" "}
                  {t("artifacts.sources", "sources")}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={busy !== null}
                onClick={() => void runVerify()}
              >
                {busy === "verify" ? (
                  <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
                ) : (
                  <ShieldCheck className="mr-1.5 h-4 w-4" />
                )}
                {t("artifacts.verify", "Verify")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={busy !== null}
                onClick={() => void artifactResource.run("open")}
              >
                <ExternalLink className="mr-1.5 h-4 w-4" />
                {t("fileActions.open", "Open")}
              </Button>
              <Select
                value={exportFormat}
                onValueChange={(value) => setExportFormat(value as ArtifactExportFormat)}
              >
                <SelectTrigger className="h-8 w-[112px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="html">HTML</SelectItem>
                  <SelectItem value="zip">ZIP</SelectItem>
                  <SelectItem value="markdown">Markdown</SelectItem>
                  <SelectItem value="pdf">PDF</SelectItem>
                </SelectContent>
              </Select>
              <Button size="sm" disabled={busy !== null} onClick={() => void runExport()}>
                {busy === "export" ? (
                  <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
                ) : (
                  <Download className="mr-1.5 h-4 w-4" />
                )}
                {t("artifacts.export", "Export")}
              </Button>
              {!viewerMaximized && (
                <IconTip
                  label={
                    detailsCollapsed
                      ? t("artifacts.expandDetails", "Expand properties panel")
                      : t("artifacts.collapseDetails", "Collapse properties panel")
                  }
                >
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={
                      detailsCollapsed
                        ? t("artifacts.expandDetails", "Expand properties panel")
                        : t("artifacts.collapseDetails", "Collapse properties panel")
                    }
                    aria-expanded={!detailsCollapsed}
                    onClick={() => setDetailsCollapsed((collapsed) => !collapsed)}
                  >
                    {detailsCollapsed ? (
                      <PanelRightDashed className="h-4 w-4" />
                    ) : (
                      <PanelRight className="h-4 w-4" />
                    )}
                  </Button>
                </IconTip>
              )}
              <IconTip
                label={
                  viewerMaximized
                    ? t("canvas.minimize", "Restore")
                    : t("canvas.maximize", "Maximize")
                }
              >
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={
                    viewerMaximized
                      ? t("canvas.minimize", "Restore")
                      : t("canvas.maximize", "Maximize")
                  }
                  aria-pressed={viewerMaximized}
                  disabled={viewerAnimating}
                  onClick={() => transitionViewerMaximized(!viewerMaximized)}
                >
                  {viewerMaximized ? (
                    <Minimize2 className="h-4 w-4" />
                  ) : (
                    <Maximize2 className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
              <IconTip label={t("artifacts.archive", "Archive")}>
                <Button
                  variant="ghost"
                  size="icon"
                  disabled={busy !== null}
                  onClick={() => void runArchive()}
                  aria-label={t("artifacts.archive", "Archive")}
                >
                  <Archive className="h-4 w-4" />
                </Button>
              </IconTip>
              <IconTip label={t("artifacts.delete", "Delete")}>
                <Button
                  variant="ghost"
                  size="icon"
                  disabled={busy !== null}
                  onClick={() => void runDelete()}
                  aria-label={t("artifacts.delete", "Delete")}
                >
                  <Trash2 className="h-4 w-4 text-destructive" />
                </Button>
              </IconTip>
            </div>

            <div className="flex min-h-0 flex-1">
              <div className="min-h-0 min-w-0 flex-1 bg-white dark:bg-surface-app">
                <ArtifactViewer
                  artifactId={selected.id}
                  projectPath={selected.projectPath}
                  title={selected.title}
                  refreshKey={refreshKey}
                />
              </div>
              <div
                style={{ width: detailsHidden ? 0 : detailsWidth }}
                className={cn(
                  "relative h-full min-w-0 shrink-0",
                  !isResizingDetails && LIST_WIDTH_TRANSITION,
                )}
              >
                <div className="h-full overflow-hidden">
                  <aside
                    style={{ width: detailsWidth }}
                    aria-hidden={detailsHidden}
                    inert={detailsHidden ? true : undefined}
                    className={cn(
                      "h-full min-h-0 overflow-y-auto border-l p-3",
                      isResizingDetails
                        ? "border-l-primary/50"
                        : isDetailsResizeHandleHovered
                          ? "border-l-primary/35"
                          : "border-l-border-soft",
                      LIST_SURFACE_TRANSITION,
                      detailsHidden
                        ? "pointer-events-none translate-x-4 opacity-0"
                        : "translate-x-0 opacity-100",
                    )}
                  >
                    <div className="mb-4 rounded-xl bg-muted/40 p-3 text-xs">
                      <div className="mb-2 flex items-center gap-2 font-medium">
                        <FileCheck2 className="h-4 w-4" />
                        {t("artifacts.artifactStatus", "Artifact status")}
                      </div>
                      <dl className="space-y-1.5 text-muted-foreground">
                        <div className="flex justify-between gap-3">
                          <dt>{t("artifacts.privacy", "Privacy")}</dt>
                          <dd className="text-right text-foreground">
                            {enumLabel(selected.privacy)}
                          </dd>
                        </div>
                        <div className="flex justify-between gap-3">
                          <dt>{t("artifacts.analysisStatus", "Analysis")}</dt>
                          <dd className="text-right text-foreground">
                            {selected.analysisStatus ? enumLabel(selected.analysisStatus) : "—"}
                          </dd>
                        </div>
                        <div className="flex justify-between gap-3">
                          <dt>{t("artifacts.verification", "Verification")}</dt>
                          <dd className="text-right text-foreground">
                            {enumLabel(selected.verification?.status ?? "not_run")}
                          </dd>
                        </div>
                        <div className="flex justify-between gap-3">
                          <dt>{t("artifacts.contentMode", "Content")}</dt>
                          <dd className="text-right text-foreground">
                            {isExecutableArtifact(selected)
                              ? t("artifacts.executable", "executable")
                              : t("artifacts.static", "static")}
                          </dd>
                        </div>
                        <div className="flex justify-between gap-3">
                          <dt>SHA-256</dt>
                          <dd
                            className="max-w-[130px] truncate text-right font-mono text-foreground"
                            data-ha-title-tip={selected.currentHash}
                          >
                            {selected.currentHash || t("artifacts.enumLabels.legacyPendingHash")}
                          </dd>
                        </div>
                      </dl>
                    </div>

                    <div className="mb-4 rounded-xl border border-border-soft p-3 text-xs">
                      <div className="mb-2 flex items-center gap-2 font-medium">
                        <FileCheck2 className="h-4 w-4" />
                        {t("artifacts.sourceContext", "Sources & quality")}
                      </div>
                      <p className="mb-2 text-muted-foreground">
                        {t("artifacts.qualityChecks", "Quality checks")}:{" "}
                        {selected.evidenceSummary?.data_quality_checked ?? 0}
                      </p>
                      {selected.sourceSummaries.length === 0 ? (
                        <p className="text-muted-foreground">
                          {t("artifacts.noSources", "No canonical sources recorded")}
                        </p>
                      ) : (
                        <div className="space-y-2">
                          {selected.sourceSummaries.map((source, index) => (
                            <div
                              key={source.id || `${source.label}-${index}`}
                              className="rounded-lg bg-muted/40 p-2"
                            >
                              <p className="truncate font-medium" data-ha-title-tip={source.label}>
                                {source.label}
                              </p>
                              <p className="mt-0.5 text-[10px] text-muted-foreground">
                                {enumLabel(source.sourceType)} · {enumLabel(source.accessScope)}
                              </p>
                              {source.sha256 && (
                                <p
                                  className="mt-0.5 truncate font-mono text-[9px] text-muted-foreground"
                                  data-ha-title-tip={source.sha256}
                                >
                                  {source.sha256}
                                </p>
                              )}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>

                    <div className="mb-2 flex items-center gap-2 text-xs font-semibold">
                      <History className="h-4 w-4" />
                      {t("artifacts.versionHistory", "Version history")}
                    </div>
                    <div className="space-y-2">
                      {versions.map((version) => (
                        <div
                          key={version.versionNumber}
                          className="rounded-lg border border-border-soft p-2.5 text-xs"
                        >
                          <div className="flex items-center justify-between gap-2">
                            <span className="font-medium">v{version.versionNumber}</span>
                            {version.versionNumber === selected.currentVersion ? (
                              <Badge>{t("artifacts.current", "Current")}</Badge>
                            ) : (
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-7 px-2"
                                disabled={busy !== null}
                                onClick={() => void runRestore(version.versionNumber)}
                              >
                                {busy === `restore-${version.versionNumber}` ? (
                                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                ) : (
                                  <RotateCcw className="mr-1 h-3.5 w-3.5" />
                                )}
                                {t("artifacts.restore", "Restore")}
                              </Button>
                            )}
                          </div>
                          <p className="mt-1 line-clamp-2 text-muted-foreground">
                            {version.message ?? enumLabel(version.payloadKind)}
                          </p>
                          <p className="mt-1 text-[10px] text-muted-foreground">
                            {new Date(version.createdAt).toLocaleString()}
                          </p>
                        </div>
                      ))}
                    </div>
                  </aside>
                </div>
                <div
                  className={cn(
                    "absolute inset-y-0 left-0 z-20 cursor-col-resize transition-[width,opacity] duration-200 ease-out",
                    detailsHidden ? "w-0 pointer-events-none opacity-0" : "w-3 opacity-100",
                  )}
                  onMouseDown={onDragDetails}
                  onMouseEnter={() => setIsDetailsResizeHandleHovered(true)}
                  onMouseLeave={() => setIsDetailsResizeHandleHovered(false)}
                  role="separator"
                  aria-orientation="vertical"
                  aria-label={t("artifacts.resizeDetails", "Resize properties panel")}
                />
              </div>
            </div>
          </main>
        )}
      </div>
    </div>
  )
}
