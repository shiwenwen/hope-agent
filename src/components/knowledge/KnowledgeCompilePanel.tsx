import {
  Check,
  Clock3,
  FileText,
  Loader2,
  Play,
  RefreshCw,
  Sparkles,
  X,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import {
  buildUnifiedRows,
  buildVisibleRowItems,
  isUnifiedRowChanged,
} from "@/components/chat/diff-panel/diffLayout"
import { UnifiedDiffView } from "@/components/chat/diff-panel/UnifiedDiffView"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type {
  CompileProposal,
  CompileProposalStatus,
  CompileRun,
  CompileRunStatus,
} from "@/types/knowledge"
import {
  knowledgeCompileErrorDetail,
  knowledgeCompileOperationErrorToast,
} from "./knowledgeCompileFeedback"

interface KnowledgeCompilePanelProps {
  kbId: string | null
  open: boolean
  onOpenChange: (open: boolean) => void
  sourceIds: string[]
  requestToken: number
  onAfterApply?: () => void
  onAfterRun?: () => void
}

export default function KnowledgeCompilePanel({
  kbId,
  open,
  onOpenChange,
  sourceIds,
  requestToken,
  onAfterApply,
  onAfterRun,
}: KnowledgeCompilePanelProps) {
  const { t } = useTranslation()
  const [runs, setRuns] = useState<CompileRun[]>([])
  const [proposals, setProposals] = useState<CompileProposal[]>([])
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [selectedProposalId, setSelectedProposalId] = useState<number | null>(null)
  const [loadingRuns, setLoadingRuns] = useState(false)
  const [loadingProposals, setLoadingProposals] = useState(false)
  const [starting, setStarting] = useState(false)
  const [canceling, setCanceling] = useState(false)
  const [busyProposalId, setBusyProposalId] = useState<number | null>(null)
  const lastRequestTokenRef = useRef(0)
  const sourceIdsKey = useMemo(() => sourceIds.join("\n"), [sourceIds])

  const loadRuns = useCallback(async () => {
    if (!kbId) {
      setRuns([])
      setSelectedRunId(null)
      return
    }
    setLoadingRuns(true)
    try {
      const list = await getTransport().call<CompileRun[]>("kb_compile_runs_list_cmd", { kbId })
      setRuns(list)
      setSelectedRunId((prev) =>
        prev && list.some((run) => run.id === prev) ? prev : (list[0]?.id ?? null),
      )
    } catch (e) {
      logger.warn("knowledge", "KnowledgeCompilePanel::loadRuns", "load failed", e)
      const failureToast = knowledgeCompileOperationErrorToast("loadRuns", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setLoadingRuns(false)
    }
  }, [kbId, t])

  const loadProposals = useCallback(
    async (runId: string | null = selectedRunId) => {
      if (!kbId) {
        setProposals([])
        setSelectedProposalId(null)
        return
      }
      setLoadingProposals(true)
      try {
        const args: Record<string, unknown> = { kbId }
        if (runId) args.runId = runId
        const list = await getTransport().call<CompileProposal[]>(
          "kb_compile_proposals_list_cmd",
          args,
        )
        setProposals(list)
        setSelectedProposalId((prev) => {
          if (prev && list.some((proposal) => proposal.id === prev)) return prev
          return (
            list.find((proposal) => proposal.status === "draft")?.id ??
            list[0]?.id ??
            null
          )
      })
    } catch (e) {
      logger.warn("knowledge", "KnowledgeCompilePanel::loadProposals", "load failed", e)
      const failureToast = knowledgeCompileOperationErrorToast("loadProposals", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setLoadingProposals(false)
    }
    },
    [kbId, selectedRunId, t],
  )

  const startCompile = useCallback(
    async (ids: string[]) => {
      if (!kbId || ids.length === 0 || starting) return
      setStarting(true)
      try {
        const run = await getTransport().call<CompileRun>("kb_compile_start_cmd", {
          kbId,
          input: { sourceIds: ids, strategy: null },
        })
        setSelectedRunId(run.id)
        if (run.status === "completed") {
          toast.success(
            t("knowledge.compile.started", "Generated {{n}} note suggestion(s)", {
              n: run.proposalCount,
            }),
          )
        } else if (run.status === "cancelled") {
          toast.message(t("knowledge.compile.cancelled", "Source-to-note run cancelled"))
        } else if (run.status === "failed") {
          const failureToast = knowledgeCompileOperationErrorToast("runFailed", t, run.error)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
        }
        await loadRuns()
        await loadProposals(run.id)
        onAfterRun?.()
      } catch (e) {
        logger.warn("knowledge", "KnowledgeCompilePanel::start", "compile failed", e)
        const failureToast = knowledgeCompileOperationErrorToast("startCompile", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setStarting(false)
      }
    },
    [kbId, loadProposals, loadRuns, onAfterRun, starting, t],
  )

  const refresh = useCallback(async () => {
    await loadRuns()
    await loadProposals()
  }, [loadProposals, loadRuns])

  const cancelRun = useCallback(async () => {
    if (!kbId || !selectedRunId || canceling) return
    setCanceling(true)
    try {
      const run = await getTransport().call<CompileRun>("kb_compile_run_cancel_cmd", {
        kbId,
        runId: selectedRunId,
      })
      setRuns((items) => items.map((item) => (item.id === run.id ? run : item)))
      toast.message(t("knowledge.compile.cancelled", "Source-to-note run cancelled"))
    } catch (e) {
      logger.warn("knowledge", "KnowledgeCompilePanel::cancel", "cancel failed", e)
      const failureToast = knowledgeCompileOperationErrorToast("cancelRun", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setCanceling(false)
    }
  }, [canceling, kbId, selectedRunId, t])

  const decideProposal = useCallback(
    async (proposal: CompileProposal, approve: boolean) => {
      if (!kbId || busyProposalId != null) return
      setBusyProposalId(proposal.id)
      try {
        await getTransport().call<CompileProposal>(
          approve ? "kb_compile_proposal_approve_cmd" : "kb_compile_proposal_reject_cmd",
          { kbId, id: proposal.id },
        )
        toast.success(
          approve
            ? t("knowledge.compile.applied", "Proposal applied")
            : t("knowledge.compile.rejected", "Proposal rejected"),
        )
        await loadRuns()
        await loadProposals(selectedRunId)
        if (approve) onAfterApply?.()
      } catch (e) {
        logger.warn("knowledge", "KnowledgeCompilePanel::decide", "decision failed", e)
        await loadProposals(selectedRunId)
        const failureToast = knowledgeCompileOperationErrorToast(
          approve ? "applyProposal" : "rejectProposal",
          t,
          e,
        )
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setBusyProposalId(null)
      }
    },
    [busyProposalId, kbId, loadProposals, loadRuns, onAfterApply, selectedRunId, t],
  )

  useEffect(() => {
    if (!open) return
    void loadRuns()
  }, [loadRuns, open])

  useEffect(() => {
    if (!open) return
    void loadProposals(selectedRunId)
  }, [loadProposals, open, selectedRunId])

  useEffect(() => {
    if (!open) return
    return getTransport().listen("knowledge:changed", () => void refresh())
  }, [open, refresh])

  useEffect(() => {
    if (!open) return
    if (!starting && !runs.some((run) => run.status === "running")) return
    const timer = window.setInterval(() => void refresh(), 2_000)
    return () => window.clearInterval(timer)
  }, [open, refresh, runs, starting])

  useEffect(() => {
    if (!open || !kbId || sourceIds.length === 0) return
    if (requestToken === 0 || requestToken === lastRequestTokenRef.current) return
    lastRequestTokenRef.current = requestToken
    void startCompile(sourceIds)
  }, [kbId, open, requestToken, sourceIds, sourceIdsKey, startCompile])

  const selectedRun = useMemo(
    () => runs.find((run) => run.id === selectedRunId) ?? null,
    [runs, selectedRunId],
  )
  const selectedProposal = useMemo(
    () => proposals.find((proposal) => proposal.id === selectedProposalId) ?? null,
    [proposals, selectedProposalId],
  )
  const draftCount = proposals.filter((proposal) => proposal.status === "draft").length
  const canStart = !!kbId && sourceIds.length > 0 && !starting

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[82vh] max-w-6xl grid-rows-none flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="border-b border-border-soft/60 px-4 py-3 pr-12">
          <div className="flex min-w-0 items-start justify-between gap-3">
            <div className="min-w-0">
              <DialogTitle className="flex items-center gap-2 text-base">
                <Sparkles className="h-4 w-4 text-primary" />
                {t("knowledge.compile.title", "Organize into notes")}
              </DialogTitle>
              <DialogDescription className="mt-1">
                {t(
                  "knowledge.compile.description",
                  "AI drafts note changes from selected sources. Review the diff before anything is written.",
                )}
              </DialogDescription>
            </div>
            <div className="flex shrink-0 items-center gap-1.5 pr-6">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-8 gap-1.5"
                disabled={!canStart}
                onClick={() => void startCompile(sourceIds)}
              >
                {starting ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Play className="h-3.5 w-3.5" />
                )}
                {t("knowledge.compile.startSelected", "Organize {{count}}", {
                  count: sourceIds.length,
                })}
              </Button>
              <IconTip label={t("knowledge.compile.refresh", "Refresh")} side="bottom">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  disabled={loadingRuns || loadingProposals}
                  onClick={() => void refresh()}
                >
                  <RefreshCw
                    className={cn(
                      "h-3.5 w-3.5",
                      (loadingRuns || loadingProposals) && "animate-spin",
                    )}
                  />
                </Button>
              </IconTip>
            </div>
          </div>
        </DialogHeader>

        <div className="grid min-h-0 flex-1 grid-cols-[260px_minmax(0,1fr)]">
          <aside className="flex min-h-0 flex-col border-r border-border-soft/60">
            <div className="border-b border-border-soft/60 px-3 py-2">
              <div className="flex items-center justify-between gap-2">
                <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  {t("knowledge.compile.runs", "Runs")}
                </span>
                {loadingRuns ? <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" /> : null}
              </div>
            </div>
            <div className="max-h-44 overflow-auto border-b border-border-soft/60 p-2">
              {runs.length === 0 && !loadingRuns ? (
                <div className="px-2 py-5 text-center text-[11px] text-muted-foreground">
                  {t("knowledge.compile.noRuns", "No source-to-note runs yet.")}
                </div>
              ) : (
                <div className="space-y-1">
                  {runs.map((run) => (
                    <button
                      key={run.id}
                      type="button"
                      className={cn(
                        "flex w-full min-w-0 flex-col gap-1 rounded-md px-2 py-1.5 text-left hover:bg-accent",
                        selectedRunId === run.id && "bg-accent",
                      )}
                      onClick={() => setSelectedRunId(run.id)}
                    >
                      <span className="flex items-center gap-1.5">
                        <StatusPill status={run.status} />
                        <span className="truncate text-[11px] text-muted-foreground">
                          {formatDateTime(run.createdAt)}
                        </span>
                      </span>
                      <span className="truncate text-xs">
                        {t("knowledge.compile.runSources", "{{count}} source(s)", {
                          count: run.sourceIds.length,
                        })}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>

            <div className="flex min-h-0 flex-1 flex-col">
              <div className="flex items-center justify-between border-b border-border-soft/60 px-3 py-2">
                <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  {t("knowledge.compile.proposals", "Proposals")}
                </span>
                <span className="rounded-full bg-muted px-1.5 text-[10px] text-muted-foreground">
                  {draftCount}
                </span>
              </div>
              <div className="min-h-0 flex-1 overflow-auto p-2">
                {proposals.length === 0 && !loadingProposals ? (
                  <div className="px-2 py-8 text-center text-[11px] text-muted-foreground">
                    {t("knowledge.compile.noProposals", "No proposals for this run.")}
                  </div>
                ) : null}
                {loadingProposals ? (
                  <div className="flex items-center justify-center py-8 text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                  </div>
                ) : (
                  <div className="space-y-1">
                    {proposals.map((proposal) => (
                      <button
                        key={proposal.id}
                        type="button"
                        className={cn(
                          "flex w-full min-w-0 flex-col gap-1 rounded-md px-2 py-1.5 text-left hover:bg-accent",
                          selectedProposalId === proposal.id && "bg-accent",
                        )}
                        onClick={() => setSelectedProposalId(proposal.id)}
                      >
                        <span className="flex min-w-0 items-center gap-1.5">
                          <ProposalStatusPill status={proposal.status} />
                          <span className="truncate text-xs font-medium">{proposal.title}</span>
                        </span>
                        <span className="truncate text-[11px] text-muted-foreground">
                          {proposalPath(proposal)}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </aside>

          <section className="flex min-h-0 min-w-0 flex-col">
            {selectedRun ? (
              <div className="flex items-center justify-between gap-2 border-b border-border-soft/60 px-4 py-2">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <StatusPill status={selectedRun.status} />
                    <span className="truncate text-sm font-medium">
                      {selectedRun.summary ||
                        t("knowledge.compile.runPending", "Source-to-note run pending")}
                    </span>
                  </div>
                  {selectedRun.error ? (
                    <div className="mt-1 truncate text-[11px] text-destructive">
                      {knowledgeCompileErrorDetail(selectedRun.error) ?? selectedRun.error}
                    </div>
                  ) : (
                    <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                      <Clock3 className="h-3 w-3" />
                      <span>{formatDateTime(selectedRun.updatedAt)}</span>
                      {selectedRun.modelLabel ? <span>{selectedRun.modelLabel}</span> : null}
                    </div>
                  )}
                </div>
                {selectedRun.status === "running" ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1.5"
                    disabled={canceling}
                    onClick={() => void cancelRun()}
                  >
                    {canceling ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <X className="h-3.5 w-3.5" />
                    )}
                    {t("knowledge.compile.cancel", "Cancel")}
                  </Button>
                ) : null}
              </div>
            ) : null}

            {selectedProposal ? (
              <>
                <div className="flex items-center justify-between gap-3 border-b border-border-soft/60 px-4 py-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                      <span className="truncate text-sm font-medium">{selectedProposal.title}</span>
                      <ProposalStatusPill status={selectedProposal.status} />
                    </div>
                    <div className="mt-1 truncate text-[11px] text-muted-foreground">
                      {selectedProposal.detail || proposalPath(selectedProposal)}
                    </div>
                    {selectedProposal.error ? (
                      <div className="mt-1 truncate text-[11px] text-destructive">
                        {knowledgeCompileErrorDetail(selectedProposal.error) ??
                          selectedProposal.error}
                      </div>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-7 gap-1.5"
                      disabled={busyProposalId != null || selectedProposal.status !== "draft"}
                      onClick={() => void decideProposal(selectedProposal, false)}
                    >
                      <X className="h-3.5 w-3.5" />
                      {t("knowledge.compile.reject", "Reject")}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      className="h-7 gap-1.5"
                      disabled={busyProposalId != null || selectedProposal.status !== "draft"}
                      onClick={() => void decideProposal(selectedProposal, true)}
                    >
                      {busyProposalId === selectedProposal.id ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Check className="h-3.5 w-3.5" />
                      )}
                      {t("knowledge.compile.apply", "Apply")}
                    </Button>
                  </div>
                </div>
                <div className="min-h-0 flex-1 overflow-auto bg-muted/10">
                  <ProposalDiff proposal={selectedProposal} />
                </div>
              </>
            ) : (
              <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 text-center text-muted-foreground">
                <Sparkles className="h-7 w-7" />
                <div className="text-sm font-medium text-foreground/80">
                  {starting
                    ? t("knowledge.compile.running", "Organizing sources into notes…")
                    : t("knowledge.compile.empty", "Select a proposal to review")}
                </div>
                <div className="max-w-sm text-xs leading-relaxed">
                  {t(
                    "knowledge.compile.emptyHint",
                    "Generated proposals stay here until you apply or reject them.",
                  )}
                </div>
              </div>
            )}
          </section>
        </div>
      </DialogContent>
    </Dialog>
  )
}

export function ProposalDiff({ proposal }: { proposal: CompileProposal }) {
  const rows = useMemo(
    () => buildUnifiedRows(proposal.beforeText ?? "", proposal.afterText ?? ""),
    [proposal.afterText, proposal.beforeText],
  )
  const items = useMemo(
    () =>
      buildVisibleRowItems(rows, {
        collapseContext: true,
        expandedFoldIds: new Set(),
        isChanged: isUnifiedRowChanged,
      }),
    [rows],
  )

  return (
    <UnifiedDiffView
      items={items}
      omittedItemCount={0}
      onToggleFold={() => {}}
      onRenderAll={() => {}}
      onCopyLocation={() => {}}
      onOpenLocation={() => {}}
    />
  )
}

function StatusPill({ status }: { status: CompileRunStatus }) {
  return <Pill className={statusClass(status)}>{status}</Pill>
}

function ProposalStatusPill({ status }: { status: CompileProposalStatus }) {
  return <Pill className={proposalStatusClass(status)}>{status}</Pill>
}

function Pill({ className, children }: { className?: string; children: string }) {
  return (
    <span className={cn("rounded-full px-1.5 py-0.5 text-[10px] font-medium", className)}>
      {children}
    </span>
  )
}

function statusClass(status: CompileRunStatus): string {
  switch (status) {
    case "completed":
      return "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "failed":
      return "bg-destructive/10 text-destructive"
    case "cancelled":
      return "bg-muted text-muted-foreground"
    case "running":
    default:
      return "bg-primary/10 text-primary"
  }
}

function proposalStatusClass(status: CompileProposalStatus): string {
  switch (status) {
    case "applied":
      return "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "failed":
      return "bg-destructive/10 text-destructive"
    case "rejected":
      return "bg-muted text-muted-foreground"
    case "draft":
    default:
      return "bg-primary/10 text-primary"
  }
}

function proposalPath(proposal: CompileProposal): string {
  const action = proposal.action
  switch (action.op) {
    case "append_link":
      return action.from_path
    case "create_moc":
    case "create_note":
    case "patch_note":
    case "set_frontmatter":
      return action.path
    default:
      return proposal.detail
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
