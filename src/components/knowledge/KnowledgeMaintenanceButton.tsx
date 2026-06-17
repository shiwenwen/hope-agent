import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Stethoscope,
  Unlink2,
  CircleOff,
  FileText,
  Sparkles,
  Check,
  X,
  Loader2,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { BrokenLink, MaintenanceProposal, MaintenanceReport, Note } from "@/types/knowledge"

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
  const [running, setRunning] = useState(false)
  const [busyId, setBusyId] = useState<number | null>(null)
  const rootRef = useRef<HTMLDivElement>(null)

  // Async load — setState lands only after the `await`, so the effect that calls
  // it stays cascade-free (mirrors KnowledgeJobsButton's loader). The badge/panel
  // gate display on `kbId`, so a null space shows nothing until the next load.
  const refresh = useCallback(async () => {
    if (!kbId) return
    try {
      const b = await getTransport().call<BrokenLink[]>("kb_broken_links_cmd", { kbId })
      const o = await getTransport().call<Note[]>("kb_orphans_cmd", { kbId })
      const p = await getTransport().call<MaintenanceProposal[]>("kb_maintenance_list_cmd", {
        kbId,
        status: "draft",
      })
      setBroken(b)
      setOrphans(o)
      setProposals(p)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceButton::refresh", "load failed", e)
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
          note: report.note,
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
      toast.error(t("knowledge.maintenance.runFailed", "Couldn't run maintenance"))
    } finally {
      setRunning(false)
    }
  }, [running, refresh, t])

  const decide = useCallback(
    async (id: number, approve: boolean) => {
      if (busyId != null) return
      setBusyId(id)
      try {
        await getTransport().call(
          approve ? "kb_maintenance_approve_cmd" : "kb_maintenance_reject_cmd",
          { id },
        )
        await refresh()
      } catch (e) {
        logger.warn("knowledge", "KnowledgeMaintenanceButton::decide", "decision failed", e)
        toast.error(
          approve
            ? t("knowledge.maintenance.applyFailed", "Couldn't apply proposal")
            : t("knowledge.maintenance.rejectFailed", "Couldn't reject proposal"),
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
    } finally {
      setBusyId(null)
    }
  }, [kbId, busyId, refresh])

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

  const issueCount = broken.length + orphans.length + proposals.length
  const hasBadge = kbId != null && (broken.length > 0 || proposals.length > 0)

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

      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 w-[340px] rounded-lg border border-border bg-popover shadow-lg">
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
            </div>
          </div>
          <div className="max-h-[360px] overflow-y-auto p-2">
            {issueCount === 0 ? (
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
                              onClick={() => decide(p.id, true)}
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
                              onClick={() => decide(p.id, false)}
                            >
                              <X className="h-3.5 w-3.5" />
                            </Button>
                          </IconTip>
                        </div>
                      </div>
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
        </div>
      )}
    </div>
  )
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
