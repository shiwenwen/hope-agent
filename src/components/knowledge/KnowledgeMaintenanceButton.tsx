import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertTriangle,
  BarChart3,
  Stethoscope,
  Unlink2,
  CircleOff,
  FileText,
  RefreshCw,
  Sparkles,
  Check,
  X,
  Loader2,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { knowledgeMaintenanceSkipReasonLabel } from "./knowledgeMaintenanceLabels"
import type {
  BrokenLink,
  MaintenanceProposal,
  MaintenanceReport,
  Note,
  KnowledgeEvidenceCoverage,
  KnowledgeEvidenceRebuildResult,
  SchemaIssue,
  SchemaIssueKind,
} from "@/types/knowledge"
import { knowledgeMaintenanceErrorToast } from "./knowledgeMaintenanceFeedback"

interface Props {
  /** The active knowledge space; the panel is empty/disabled when null. */
  kbId: string | null
  /** Open a note (optionally scrolling to a 1-based line) — closes the panel. */
  onOpenNote: (path: string, line?: number) => void
}

/** Drop the `.md`/`.markdown` extension for display. */
function stem(rel: string): string {
  return rel.replace(/\.(md|markdown)$/i, "")
}

/**
 * Top-right "maintenance" entry for the Knowledge view: a stethoscope icon that
 * pulses when the active space has broken links, opening a floating panel that
 * lists every broken (dangling) `[[ ]]` link and every orphan note (no links).
 * Click a row to jump to it. Owner plane (`kb_broken_links_cmd` /
 * `kb_orphans_cmd`); refreshes on open + on `knowledge:changed`.
 */
export default function KnowledgeMaintenanceButton({ kbId, onOpenNote }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [broken, setBroken] = useState<BrokenLink[]>([])
  const [orphans, setOrphans] = useState<Note[]>([])
  const [proposals, setProposals] = useState<MaintenanceProposal[]>([])
  const [schemaIssues, setSchemaIssues] = useState<SchemaIssue[]>([])
  const [evidenceCoverage, setEvidenceCoverage] = useState<KnowledgeEvidenceCoverage | null>(null)
  const [running, setRunning] = useState(false)
  const [rebuildingEvidence, setRebuildingEvidence] = useState(false)
  const [busyId, setBusyId] = useState<number | null>(null)
  const rootRef = useRef<HTMLDivElement>(null)

  // Async load — setState lands only after the `await`, so the effect that calls
  // it stays cascade-free (mirrors KnowledgeActivityButton's loader). The badge/panel
  // gate display on `kbId`, so a null space shows nothing until the next load.
  const refresh = useCallback(async () => {
    if (!kbId) return
    try {
      const [b, o, p] = await Promise.all([
        getTransport().call<BrokenLink[]>("kb_broken_links_cmd", { kbId }),
        getTransport().call<Note[]>("kb_orphans_cmd", { kbId }),
        getTransport().call<MaintenanceProposal[]>("kb_maintenance_list_cmd", {
          kbId,
          status: "draft",
        }),
      ])
      setBroken(b)
      setOrphans(o)
      setProposals(p)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::refresh", "core load failed", e)
    }

    try {
      const s = await getTransport().call<SchemaIssue[]>("kb_schema_issues_cmd", { kbId })
      setSchemaIssues(s)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::refresh", "schema load failed", e)
      setSchemaIssues([])
    }

    try {
      const coverage = await getTransport().call<KnowledgeEvidenceCoverage>(
        "kb_evidence_coverage_cmd",
        { kbId },
      )
      setEvidenceCoverage(coverage)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::refresh", "evidence load failed", e)
      setEvidenceCoverage(null)
    }
  }, [kbId])

  // Run one maintenance cycle now (generates proposals across all spaces).
  const runNow = useCallback(async () => {
    if (running) return
    setRunning(true)
    try {
      const report = await getTransport().call<MaintenanceReport>("kb_maintenance_run_cmd", {})
      if (report.note) {
        toast.message(t("knowledge.maintenance.cycleSkipped", "Maintenance skipped: {{note}}", {
          note: knowledgeMaintenanceSkipReasonLabel(t, report.note),
        }))
      } else {
        toast.success(
          t("knowledge.maintenance.cycleDone", "Generated {{n}} proposal(s)", {
            n: report.generated,
          }),
        )
      }
      await refresh()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::runNow", "run failed", e)
      const failureToast = knowledgeMaintenanceErrorToast("runNow", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRunning(false)
    }
  }, [running, refresh, t])

  const rebuildEvidence = useCallback(async () => {
    if (!kbId || rebuildingEvidence) return
    setRebuildingEvidence(true)
    try {
      const result = await getTransport().call<KnowledgeEvidenceRebuildResult>(
        "kb_evidence_rebuild_cmd",
        { kbId },
      )
      toast.success(
        t("knowledge.maintenance.evidenceRebuilt", {
          defaultValue: "Rebuilt evidence index for {{notes}} notes, {{refs}} refs, {{claims}} claims",
          notes: result.scannedCount,
          refs: result.indexedRefCount,
          claims: result.indexedClaimCount,
        }),
      )
      await refresh()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::rebuildEvidence", "failed", e)
      const failureToast = knowledgeMaintenanceErrorToast("rebuildEvidence", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRebuildingEvidence(false)
    }
  }, [kbId, rebuildingEvidence, refresh, t])

  const decide = useCallback(
    async (proposal: MaintenanceProposal, approve: boolean) => {
      if (busyId != null) return
      const id = proposal.id
      setBusyId(id)
      try {
        await getTransport().call(
          approve ? "kb_maintenance_approve_cmd" : "kb_maintenance_reject_cmd",
          { id },
        )
        if (approve && proposal.kind === "source_compile") {
          toast.success(
            t(
              "knowledge.maintenance.compileQueued",
              "Compile review generated. Check the source compile panel for diffs.",
            ),
          )
        }
        await refresh()
      } catch (e) {
        logger.warn("knowledge", "KnowledgeMaintenanceButton::decide", "decision failed", e)
        const failureToast = knowledgeMaintenanceErrorToast(
          approve ? "applyProposal" : "rejectProposal",
          t,
          e,
        )
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setBusyId(null)
      }
    },
    [busyId, refresh, t],
  )

  const rejectAll = useCallback(async () => {
    if (!kbId || busyId != null) return
    setBusyId(-1)
    try {
      await getTransport().call("kb_maintenance_reject_all_cmd", { kbId })
      await refresh()
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::rejectAll", "failed", e)
      const failureToast = knowledgeMaintenanceErrorToast("rejectAll", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setBusyId(null)
    }
  }, [kbId, busyId, refresh, t])

  // Keep the badge fresh as the space mutates (links resolve/break on edits).
  // `refresh` only setStates after an `await` (a microtask, not a synchronous
  // cascade), so the set-state-in-effect lint doesn't fire here.
  useEffect(() => {
    void refresh()
    const un = getTransport().listen("knowledge:changed", refresh)
    return un
  }, [refresh])

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false)
    }
    document.addEventListener("mousedown", onDown)
    document.addEventListener("keydown", onKey)
    return () => {
      document.removeEventListener("mousedown", onDown)
      document.removeEventListener("keydown", onKey)
    }
  }, [open])

  const evidenceIssueCount = evidenceCoverage
    ? evidenceCoverage.notesMissingEvidence +
      evidenceCoverage.staleRefCount +
      evidenceCoverage.missingRefCount
    : 0
  const issueCount =
    broken.length + orphans.length + proposals.length + schemaIssues.length + evidenceIssueCount
  const hasBadge =
    kbId != null &&
    (broken.length > 0 ||
      proposals.length > 0 ||
      schemaIssues.length > 0 ||
      evidenceIssueCount > 0)

  const jump = useCallback(
    (path: string, line?: number) => {
      setOpen(false)
      onOpenNote(path, line)
    },
    [onOpenNote],
  )

  return (
    <div ref={rootRef} className="relative">
      <IconTip label={t("knowledge.maintenance.tooltip", "Maintenance")} side="bottom">
        <Button
          variant="ghost"
          size="icon"
          className="relative h-8 w-8"
          disabled={!kbId}
          onClick={() => {
            if (!open) void refresh()
            setOpen((v) => !v)
          }}
        >
          <Stethoscope className="h-4 w-4" />
          {hasBadge && (
            <span className="absolute right-1 top-1 flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-500/70" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-amber-500" />
            </span>
          )}
        </Button>
      </IconTip>

      <FloatingMenu
        open={open}
        positionClassName="top-full right-0 mt-1.5"
        originClassName="origin-top-right"
        className="ha-menu-from-top w-[340px] overflow-hidden p-0"
        onEscapeKeyDown={() => setOpen(false)}
      >
          <div className="flex items-center justify-between border-b border-border-soft/60 px-3 py-2">
            <span className="text-xs font-medium">
              {t("knowledge.maintenance.title", "Maintenance")}
            </span>
            <div className="flex items-center gap-1">
              {proposals.length > 0 && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 px-1.5 text-[11px] text-muted-foreground"
                  disabled={busyId != null}
                  onClick={rejectAll}
                >
                  {t("knowledge.maintenance.rejectAll", "Reject all")}
                </Button>
              )}
              <IconTip
                label={t("knowledge.maintenance.runNowTip", "Scan now for maintenance suggestions")}
              >
                <Button
                  variant="outline"
                  size="sm"
                  className="h-6 px-2 text-[11px]"
                  disabled={running}
                  onClick={runNow}
                >
                  {running ? (
                    <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                  ) : (
                    <Sparkles className="mr-1 h-3 w-3" />
                  )}
                  {t("knowledge.maintenance.runNow", "Scan")}
                </Button>
              </IconTip>
              <IconTip
                label={t(
                  "knowledge.maintenance.rebuildEvidenceTip",
                  "Rebuild the derived evidence index",
                )}
              >
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6"
                  disabled={rebuildingEvidence}
                  onClick={rebuildEvidence}
                >
                  {rebuildingEvidence ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3 w-3" />
                  )}
                </Button>
              </IconTip>
            </div>
          </div>
          <div className="max-h-[360px] overflow-y-auto p-2">
            {issueCount === 0 && !evidenceCoverage ? (
              <div className="flex flex-col items-center justify-center gap-1.5 px-4 py-8 text-center">
                <Stethoscope className="h-6 w-6 text-muted-foreground/70" />
                <span className="text-xs font-medium">
                  {t("knowledge.maintenance.healthy", "Everything looks connected.")}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {t(
                    "knowledge.maintenance.healthyHint",
                    "No broken links or orphan notes in this space.",
                  )}
                </span>
              </div>
            ) : (
              <div className="space-y-3">
                {evidenceCoverage && (
                  <EvidenceCoverageCard
                    coverage={evidenceCoverage}
                    rebuilding={rebuildingEvidence}
                    onRebuild={rebuildEvidence}
                  />
                )}
                {proposals.length > 0 && (
                  <Section
                    icon={<Sparkles className="h-3.5 w-3.5 text-primary" />}
                    label={t("knowledge.maintenance.proposals", "Suggestions")}
                    count={proposals.length}
                  >
                    {proposals.map((p) => (
                      <div
                        key={p.id}
                        className="flex items-start gap-1.5 rounded-md px-2 py-1.5 hover:bg-accent"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1">
                            <span className="rounded bg-primary/10 px-1 text-[10px] font-medium text-primary">
                              {t(`knowledge.maintenance.kind.${p.kind}`, p.kind)}
                            </span>
                            <span className="truncate text-xs">{p.title}</span>
                          </div>
                          {p.detail && (
                            <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">
                              {p.detail}
                            </span>
                          )}
                        </div>
                        <div className="flex shrink-0 items-center gap-0.5">
                          <IconTip label={t("knowledge.maintenance.approve", "Apply")}>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-6 w-6 text-emerald-600"
                              disabled={busyId != null}
                              onClick={() => decide(p, true)}
                            >
                              {busyId === p.id ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Check className="h-3.5 w-3.5" />
                              )}
                            </Button>
                          </IconTip>
                          <IconTip label={t("knowledge.maintenance.reject", "Dismiss")}>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-6 w-6 text-muted-foreground"
                              disabled={busyId != null}
                              onClick={() => decide(p, false)}
                            >
                              <X className="h-3.5 w-3.5" />
                            </Button>
                          </IconTip>
                        </div>
                      </div>
                    ))}
                  </Section>
                )}
                {schemaIssues.length > 0 && (
                  <Section
                    icon={<AlertTriangle className="h-3.5 w-3.5 text-amber-500" />}
                    label={t("knowledge.maintenance.schemaIssues", "Schema issues")}
                    count={schemaIssues.length}
                  >
                    {schemaIssues.map((issue, i) => (
                      <button
                        key={`${issue.relPath}:${issue.kind}:${i}`}
                        type="button"
                        onClick={() => jump(issue.relPath)}
                        className="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-accent"
                      >
                        <span className="flex min-w-0 items-center gap-1">
                          <span className="rounded bg-amber-500/10 px-1 font-mono text-[10px] text-amber-700 dark:text-amber-300">
                            {schemaIssueKindLabel(t, issue.kind)}
                          </span>
                          <span className="truncate text-xs">{stem(issue.relPath)}</span>
                        </span>
                        <span className="line-clamp-2 text-[11px] text-muted-foreground">
                          {issue.detail}
                        </span>
                      </button>
                    ))}
                  </Section>
                )}
                {broken.length > 0 && (
                  <Section
                    icon={<Unlink2 className="h-3.5 w-3.5 text-amber-500" />}
                    label={t("knowledge.maintenance.brokenLinks", "Broken links")}
                    count={broken.length}
                  >
                    {broken.map((b, i) => (
                      <button
                        key={`${b.srcRelPath}:${b.srcStartLine}:${b.srcStartCol}:${i}`}
                        type="button"
                        onClick={() => jump(b.srcRelPath, b.srcStartLine)}
                        className="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-accent"
                      >
                        <span className="flex items-center gap-1 text-xs">
                          <span className="rounded bg-amber-500/10 px-1 font-mono text-[11px] text-amber-700 dark:text-amber-300">
                            [[{b.targetRef}]]
                          </span>
                        </span>
                        <span className="truncate text-[11px] text-muted-foreground">
                          {t("knowledge.maintenance.inNote", "in")} {stem(b.srcRelPath)}
                        </span>
                      </button>
                    ))}
                  </Section>
                )}
                {orphans.length > 0 && (
                  <Section
                    icon={<CircleOff className="h-3.5 w-3.5 text-muted-foreground" />}
                    label={t("knowledge.maintenance.orphans", "Orphan notes")}
                    count={orphans.length}
                  >
                    {orphans.map((o) => (
                      <button
                        key={o.relPath}
                        type="button"
                        onClick={() => jump(o.relPath)}
                        className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left hover:bg-accent"
                      >
                        <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <span className="truncate text-xs">{o.title}</span>
                      </button>
                    ))}
                  </Section>
                )}
              </div>
            )}
          </div>
      </FloatingMenu>
    </div>
  )
}

function schemaIssueKindLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: SchemaIssueKind,
): string {
  switch (kind) {
    case "missing_evidence":
      return t("knowledge.maintenance.schemaIssueKind.missingEvidence", "Missing evidence")
    case "stale_source":
      return t("knowledge.maintenance.schemaIssueKind.staleSource", "Stale source")
    case "schema_violation":
      return t("knowledge.maintenance.schemaIssueKind.schemaViolation", "Schema violation")
    case "conflicting_claim":
      return t("knowledge.maintenance.schemaIssueKind.conflictingClaim", "Conflicting claim")
    case "unfiled_open_question":
      return t(
        "knowledge.maintenance.schemaIssueKind.unfiledOpenQuestion",
        "Unfiled open question",
      )
  }
}

function Section({
  icon,
  label,
  count,
  children,
}: {
  icon: React.ReactNode
  label: string
  count: number
  children: React.ReactNode
}) {
  return (
    <div>
      <div className="flex items-center gap-1.5 px-2 pb-1 text-[11px] font-medium text-muted-foreground">
        {icon}
        <span>{label}</span>
        <span className="rounded-full bg-muted px-1.5 text-[10px]">{count}</span>
      </div>
      <div className="space-y-0.5">{children}</div>
    </div>
  )
}

function EvidenceCoverageCard({
  coverage,
  rebuilding,
  onRebuild,
}: {
  coverage: KnowledgeEvidenceCoverage
  rebuilding: boolean
  onRebuild: () => void
}) {
  const { t } = useTranslation()
  const pct = Math.round(Math.max(0, Math.min(1, coverage.coverageScore)) * 100)
  const hasIssues =
    coverage.notesMissingEvidence > 0 ||
    coverage.staleRefCount > 0 ||
    coverage.missingRefCount > 0

  return (
    <div className="rounded-md border border-border-soft/60 bg-muted/20 p-2 text-xs">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-medium">
            <BarChart3 className="h-3.5 w-3.5 text-primary" />
            <span>{t("knowledge.maintenance.evidenceCoverage", "Evidence coverage")}</span>
            {hasIssues ? (
              <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700 dark:text-amber-300">
                {t("knowledge.maintenance.needsReview", "Needs review")}
              </span>
            ) : null}
          </div>
          <div className="mt-1 text-[11px] text-muted-foreground">
            {t("knowledge.maintenance.evidenceCoverageDetail", {
              defaultValue:
                "{{pct}}% · {{claimsWithEvidence}}/{{claims}} claims · {{notesWithEvidence}}/{{notes}} notes",
              pct,
              claimsWithEvidence: coverage.claimsWithEvidence,
              claims: coverage.claimCount,
              notesWithEvidence: coverage.notesWithEvidence,
              notes: coverage.compiledNoteCount,
            })}
          </div>
        </div>
        <IconTip label={t("knowledge.maintenance.rebuildEvidenceTip", "Rebuild the derived evidence index")}>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0"
            disabled={rebuilding}
            onClick={onRebuild}
          >
            {rebuilding ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <RefreshCw className="h-3 w-3" />
            )}
          </Button>
        </IconTip>
      </div>
      <div className="mt-2 grid grid-cols-3 gap-1 text-[10px] text-muted-foreground">
        <Metric
          label={t("knowledge.maintenance.missingEvidence", "Missing evidence")}
          value={coverage.notesMissingEvidence}
          warn={coverage.notesMissingEvidence > 0}
        />
        <Metric
          label={t("knowledge.maintenance.staleRefs", "Stale refs")}
          value={coverage.staleRefCount}
          warn={coverage.staleRefCount > 0}
        />
        <Metric
          label={t("knowledge.maintenance.missingRefs", "Missing refs")}
          value={coverage.missingRefCount}
          warn={coverage.missingRefCount > 0}
        />
      </div>
    </div>
  )
}

function Metric({ label, value, warn }: { label: string; value: number; warn?: boolean }) {
  return (
    <div className="rounded bg-background/60 px-2 py-1">
      <div className={warn ? "font-medium text-amber-700 dark:text-amber-300" : "font-medium"}>
        {value}
      </div>
      <div className="truncate">{label}</div>
    </div>
  )
}
