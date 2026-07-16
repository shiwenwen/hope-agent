import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react"
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
  PackageOpen,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Trash2,
  UserCheck,
} from "lucide-react"
import { toast } from "sonner"

import type {
  ArtifactExportFormat,
  ArtifactRecord,
  ArtifactVersionSummary,
  DomainArtifactExportGuardReport,
} from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import { Input } from "@/components/ui/input"
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

function Badge({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <span className={`inline-flex items-center rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground ${className}`}>
      {children}
    </span>
  )
}

function ArtifactListRow({
  artifact,
  selected,
  onSelect,
}: {
  artifact: ArtifactRecord
  selected: boolean
  onSelect: () => void
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
        className={`mb-1.5 w-full rounded-xl p-3 text-left transition-colors ${selected ? "bg-primary/10" : "hover:bg-muted/60"}`}
        onClick={() => void resource.run(resource.primary)}
      >
        <div className="flex items-start justify-between gap-2">
          <span className="line-clamp-2 text-sm font-medium">{artifact.title}</span>
          <Badge className="shrink-0">v{artifact.currentVersion}</Badge>
        </div>
        <div className="mt-2 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
          <span>{artifact.kind.replaceAll("_", " ")}</span>
          <span>·</span>
          <span>{artifact.privacy.replaceAll("_", " ")}</span>
          {artifact.capabilities?.executableContent === true && (
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
  const [exportAudience, setExportAudience] = useState("")
  const [exportGuard, setExportGuard] = useState<DomainArtifactExportGuardReport | null>(null)
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
      if (selectedId && !rows.some((artifact) => artifact.id === selectedId)) {
        setSelectedId(null)
        setSelected(null)
        setVersions([])
      }
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("artifacts.loadFailed", "Failed to load Artifacts"))
    } finally {
      setLoading(false)
    }
  }, [kind, offset, selectedId, state, t])

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
        setExportGuard(null)
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
    if (!selectedTarget) return
    setBusy("export")
    try {
      const outcome = await artifactResource.run("download", { artifactFormat: exportFormat })
      if (outcome === "executed") {
        toast.success(t("artifacts.exportReady", "Artifact exported"))
      }
    } finally {
      setBusy(null)
    }
  }

  const runExportReview = async () => {
    if (!selected) return
    if (!exportAudience.trim()) {
      toast.error(t("artifacts.audienceRequired", "Enter the intended delivery audience"))
      return
    }
    if (!window.confirm(t("artifacts.redactionConfirm", "Confirm that sensitive fields and included sources have been reviewed for this audience?"))) return
    setBusy("export-review")
    try {
      const guard = await getTransport().reviewArtifactExport(selected.id, exportAudience.trim())
      setExportGuard(guard)
      if (guard.status === "passed") {
        toast.success(t("artifacts.exportReviewPassed", "Export review passed"))
      } else {
        toast.error(guard.blockers.join("; ") || t("artifacts.exportReviewBlocked", "Export review still has blockers"))
      }
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
    if (!selected || !window.confirm(t("artifacts.archiveConfirm", "Archive this Artifact?"))) return
    setBusy("archive")
    try {
      await getTransport().archiveArtifact(selected.id)
      setSelected(null)
      setSelectedId(null)
      await loadList()
    } finally {
      setBusy(null)
    }
  }

  const runDelete = async () => {
    if (!selected || !window.confirm(t("artifacts.deleteConfirm", "Permanently delete this Artifact and all versions?"))) return
    setBusy("delete")
    try {
      await getTransport().deleteArtifact(selected.id)
      setSelected(null)
      setSelectedId(null)
      await loadList()
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col bg-background">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border-soft px-4">
        <Button variant="ghost" size="icon" onClick={onBack} aria-label={t("common.back", "Back")}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <PackageOpen className="h-5 w-5 text-primary" />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-sm font-semibold">{t("artifacts.title", "Artifacts")}</h1>
          <p className="truncate text-xs text-muted-foreground">
            {t("artifacts.subtitle", "Versioned local reports, dashboards, and explainers")}
          </p>
        </div>
        <Button variant="ghost" size="icon" onClick={() => void loadList()} disabled={loading}>
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[340px] shrink-0 flex-col border-r border-border-soft">
          <div className="grid grid-cols-2 gap-2 border-b border-border-soft p-3">
            <SearchInput
              className="col-span-2 h-9"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("artifacts.search", "Search title, project, or Agent")}
            />
            <Select value={kind} onValueChange={(value) => { setKind(value); setOffset(0) }}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="all">{t("artifacts.allKinds", "All kinds")}</SelectItem>
                {KINDS.map((value) => <SelectItem key={value} value={value}>{value.replaceAll("_", " ")}</SelectItem>)}
              </SelectContent>
            </Select>
            <Select value={state} onValueChange={(value) => { setState(value); setOffset(0) }}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="active">{t("artifacts.active", "Active")}</SelectItem>
                <SelectItem value="archived">{t("artifacts.archived", "Archived")}</SelectItem>
                <SelectItem value="all">{t("artifacts.allStates", "All states")}</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {loading ? (
              <div className="flex h-32 items-center justify-center"><Loader2 className="h-5 w-5 animate-spin text-muted-foreground" /></div>
            ) : filtered.length === 0 ? (
              <div className="flex h-40 flex-col items-center justify-center gap-2 text-center text-muted-foreground">
                <PackageOpen className="h-7 w-7" />
                <p className="text-sm">{t("artifacts.empty", "No Artifacts found")}</p>
              </div>
            ) : filtered.map((artifact) => (
              <ArtifactListRow
                key={artifact.id}
                artifact={artifact}
                selected={selectedId === artifact.id}
                onSelect={() => setSelectedId(artifact.id)}
              />
            ))}
          </div>
          <div className="flex items-center justify-between border-t border-border-soft p-2">
            <Button variant="ghost" size="sm" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}>
              <ChevronLeft className="mr-1 h-4 w-4" />{t("common.previous", "Previous")}
            </Button>
            <span className="text-xs text-muted-foreground">{offset + 1}–{offset + artifacts.length}</span>
            <Button variant="ghost" size="sm" disabled={artifacts.length < PAGE_SIZE} onClick={() => setOffset(offset + PAGE_SIZE)}>
              {t("common.next", "Next")}<ChevronRight className="ml-1 h-4 w-4" />
            </Button>
          </div>
        </aside>

        {!selected ? (
          <main className="flex min-w-0 flex-1 items-center justify-center text-muted-foreground">
            <div className="text-center"><PackageOpen className="mx-auto mb-3 h-10 w-10" /><p>{t("artifacts.selectHint", "Select an Artifact to inspect it")}</p></div>
          </main>
        ) : (
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="flex flex-wrap items-center gap-2 border-b border-border-soft px-3 py-2">
              <div className="mr-auto min-w-0">
                <h2 className="truncate text-sm font-semibold">{selected.title}</h2>
                <p className="text-xs text-muted-foreground">
                  {selected.kind.replaceAll("_", " ")} · v{selected.currentVersion} · {selected.sourceCount} {t("artifacts.sources", "sources")}
                </p>
              </div>
              <Button variant="outline" size="sm" disabled={busy !== null} onClick={() => void runVerify()}>
                {busy === "verify" ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : <ShieldCheck className="mr-1.5 h-4 w-4" />}
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
              <Select value={exportFormat} onValueChange={(value) => setExportFormat(value as ArtifactExportFormat)}>
                <SelectTrigger className="h-8 w-[112px]"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="html">HTML</SelectItem>
                  <SelectItem value="zip">ZIP</SelectItem>
                  <SelectItem value="markdown">Markdown</SelectItem>
                  <SelectItem value="pdf">PDF</SelectItem>
                </SelectContent>
              </Select>
              <Button size="sm" disabled={busy !== null} onClick={() => void runExport()}>
                {busy === "export" ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : <Download className="mr-1.5 h-4 w-4" />}
                {t("artifacts.export", "Export")}
              </Button>
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

            <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_280px]">
              <div className="min-h-0 bg-white dark:bg-surface-app">
                <ArtifactViewer
                  artifactId={selected.id}
                  projectPath={selected.projectPath}
                  title={selected.title}
                  refreshKey={refreshKey}
                />
              </div>
              <aside className="min-h-0 overflow-y-auto border-l border-border-soft p-3">
                <div className="mb-4 rounded-xl bg-muted/40 p-3 text-xs">
                  <div className="mb-2 flex items-center gap-2 font-medium"><FileCheck2 className="h-4 w-4" />{t("artifacts.deliveryStatus", "Delivery status")}</div>
                  <dl className="space-y-1.5 text-muted-foreground">
                    <div className="flex justify-between gap-3"><dt>{t("artifacts.privacy", "Privacy")}</dt><dd className="text-right text-foreground">{selected.privacy.replaceAll("_", " ")}</dd></div>
                    <div className="flex justify-between gap-3"><dt>{t("artifacts.analysisStatus", "Analysis")}</dt><dd className="text-right text-foreground">{selected.analysisStatus ?? "—"}</dd></div>
                    <div className="flex justify-between gap-3"><dt>{t("artifacts.verification", "Verification")}</dt><dd className="text-right text-foreground">{selected.verification?.status ?? "not run"}</dd></div>
                    <div className="flex justify-between gap-3"><dt>{t("artifacts.contentMode", "Content")}</dt><dd className="text-right text-foreground">{selected.capabilities?.executableContent === true ? t("artifacts.executable", "executable") : t("artifacts.static", "static")}</dd></div>
                    <div className="flex justify-between gap-3"><dt>SHA-256</dt><dd className="max-w-[130px] truncate text-right font-mono text-foreground" data-ha-title-tip={selected.currentHash}>{selected.currentHash || "legacy / pending"}</dd></div>
                  </dl>
                </div>

                <div className="mb-4 rounded-xl border border-border-soft p-3 text-xs">
                  <div className="mb-2 flex items-center gap-2 font-medium"><FileCheck2 className="h-4 w-4" />{t("artifacts.sourceContext", "Sources & quality")}</div>
                  <p className="mb-2 text-muted-foreground">
                    {t("artifacts.qualityChecks", "Quality checks")}: {selected.evidenceSummary?.data_quality_checked ?? 0}
                  </p>
                  {selected.sourceSummaries.length === 0 ? (
                    <p className="text-muted-foreground">{t("artifacts.noSources", "No canonical sources recorded")}</p>
                  ) : (
                    <div className="space-y-2">
                      {selected.sourceSummaries.map((source, index) => (
                        <div key={source.id || `${source.label}-${index}`} className="rounded-lg bg-muted/40 p-2">
                          <p className="truncate font-medium" data-ha-title-tip={source.label}>{source.label}</p>
                          <p className="mt-0.5 text-[10px] text-muted-foreground">{source.sourceType} · {source.accessScope}</p>
                          {source.sha256 && <p className="mt-0.5 truncate font-mono text-[9px] text-muted-foreground" data-ha-title-tip={source.sha256}>{source.sha256}</p>}
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                <div className="mb-4 rounded-xl border border-border-soft p-3 text-xs">
                  <div className="mb-2 flex items-center gap-2 font-medium"><UserCheck className="h-4 w-4" />{t("artifacts.exportReview", "Export review")}</div>
                  <p className="mb-2 text-muted-foreground">
                    {t("artifacts.exportReviewHint", "Required for shareable, sensitive, private-source, or connector-backed packages.")}
                  </p>
                  <Input
                    className="h-8"
                    value={exportAudience}
                    onChange={(event) => setExportAudience(event.target.value)}
                    placeholder={t("artifacts.audiencePlaceholder", "Intended audience")}
                  />
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-2 w-full"
                    disabled={busy !== null}
                    onClick={() => void runExportReview()}
                  >
                    {busy === "export-review" ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : <ShieldCheck className="mr-1.5 h-4 w-4" />}
                    {t("artifacts.confirmRedaction", "Confirm redaction & review")}
                  </Button>
                  {exportGuard && (
                    <p className={`mt-2 ${exportGuard.status === "passed" ? "text-emerald-600" : "text-destructive"}`}>
                      {t("artifacts.guardStatus", "Guard")}: {exportGuard.status}
                    </p>
                  )}
                </div>

                <div className="mb-2 flex items-center gap-2 text-xs font-semibold"><History className="h-4 w-4" />{t("artifacts.versionHistory", "Version history")}</div>
                <div className="space-y-2">
                  {versions.map((version) => (
                    <div key={version.versionNumber} className="rounded-lg border border-border-soft p-2.5 text-xs">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-medium">v{version.versionNumber}</span>
                        {version.versionNumber === selected.currentVersion ? <Badge>{t("artifacts.current", "Current")}</Badge> : (
                          <Button variant="ghost" size="sm" className="h-7 px-2" disabled={busy !== null} onClick={() => void runRestore(version.versionNumber)}>
                            {busy === `restore-${version.versionNumber}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="mr-1 h-3.5 w-3.5" />}
                            {t("artifacts.restore", "Restore")}
                          </Button>
                        )}
                      </div>
                      <p className="mt-1 line-clamp-2 text-muted-foreground">{version.message ?? version.payloadKind}</p>
                      <p className="mt-1 text-[10px] text-muted-foreground">{new Date(version.createdAt).toLocaleString()}</p>
                    </div>
                  ))}
                </div>
              </aside>
            </div>
          </main>
        )}
      </div>
    </div>
  )
}
