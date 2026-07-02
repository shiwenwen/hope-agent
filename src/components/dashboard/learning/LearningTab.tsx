import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Activity,
  CheckCircle2,
  GitBranch,
  Layers3,
  Loader2,
  RefreshCw,
  ShieldAlert,
  Sparkles,
} from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { CodingImprovementDashboard, DashboardFilter } from "../types"

interface LearningOverview {
  windowDays: number
  autoCreatedSkills: number
  userCreatedSkills: number
  skillsActivated: number
  skillsPatched: number
  skillsDiscarded: number
  skillsUsed: number
  recallHits: number
  recallSummaryUsed: number
  profileMemories: number
}

interface TimelinePoint {
  ts: number
  kind: string
  skillId?: string
  source?: string
}

interface SkillUsage {
  skillId: string
  usedCount: number
  lastUsedTs?: number
  createdSource?: string
}

interface RecallStats {
  hits: number
  summarized: number
  windowDays: number
}

const WINDOW_OPTIONS = [7, 14, 30, 60, 90]

interface LearningTabProps {
  filter: DashboardFilter
}

export default function LearningTab({ filter }: LearningTabProps) {
  const { t } = useTranslation()
  const [windowDays, setWindowDays] = useState(30)
  const [loading, setLoading] = useState(false)
  const [overview, setOverview] = useState<LearningOverview | null>(null)
  const [timeline, setTimeline] = useState<TimelinePoint[]>([])
  const [topSkills, setTopSkills] = useState<SkillUsage[]>([])
  const [recall, setRecall] = useState<RecallStats | null>(null)
  const [coding, setCoding] = useState<CodingImprovementDashboard | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    try {
      const [ov, tl, ts, rs, ci] = await Promise.all([
        getTransport().call<LearningOverview>("dashboard_learning_overview", {
          windowDays,
        }),
        getTransport().call<TimelinePoint[]>("dashboard_learning_timeline", {
          windowDays,
        }),
        getTransport().call<SkillUsage[]>("dashboard_top_skills", {
          windowDays,
          limit: 10,
        }),
        getTransport().call<RecallStats>("dashboard_recall_stats", {
          windowDays,
        }),
        getTransport().call<CodingImprovementDashboard>("dashboard_coding_improvement", {
          filter,
          limit: 8,
        }),
      ])
      setOverview(ov)
      setTimeline(tl ?? [])
      setTopSkills(ts ?? [])
      setRecall(rs)
      setCoding(ci)
    } catch (e) {
      logger.error("dashboard", "LearningTab::load", "Failed to load learning data", e)
    } finally {
      setLoading(false)
    }
  }, [filter, windowDays])

  useEffect(() => {
    reload()
  }, [reload])

  const totalRecall = (recall?.hits ?? 0) + (recall?.summarized ?? 0)
  const summaryPct = totalRecall > 0 ? Math.round(((recall?.summarized ?? 0) / totalRecall) * 100) : 0

  return (
    <div className="flex flex-col gap-4 mt-4">
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <h3 className="text-sm font-semibold flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-muted-foreground" />
            {t("dashboard.learning.title")}
          </h3>
          <p className="text-xs text-muted-foreground">{t("dashboard.learning.subtitle")}</p>
        </div>
        <div className="flex gap-2 items-center">
          <div className="flex gap-1">
            {WINDOW_OPTIONS.map((d) => (
              <Button
                key={d}
                size="sm"
                variant={windowDays === d ? "secondary" : "ghost"}
                className="text-xs h-7 px-2"
                onClick={() => setWindowDays(d)}
              >
                {t("dashboard.learning.daysN", { n: d })}
              </Button>
            ))}
          </div>
          <Button size="sm" variant="outline" onClick={reload} disabled={loading}>
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </Button>
        </div>
      </div>

      <CodingImprovementSection coding={coding} />

      {/* Overview cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <OverviewCard
          label={t("dashboard.learning.autoSkills")}
          value={overview?.autoCreatedSkills ?? 0}
          hint={t("dashboard.learning.userSkillsHint", {
            n: overview?.userCreatedSkills ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.activated")}
          value={overview?.skillsActivated ?? 0}
          hint={t("dashboard.learning.patchedHint", {
            n: overview?.skillsPatched ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.recallHits")}
          value={overview?.recallHits ?? 0}
          hint={t("dashboard.learning.recallSummaryHint", {
            n: overview?.recallSummaryUsed ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.profileMemories")}
          value={overview?.profileMemories ?? 0}
          hint={t("dashboard.learning.profileHint")}
        />
      </div>

      {/* Timeline */}
      <div className="border border-border/60 rounded-lg p-4">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
          {t("dashboard.learning.timeline")}
        </h4>
        {timeline.length === 0 ? (
          <div className="text-xs text-muted-foreground text-center py-6">
            {t("dashboard.learning.noEvents")}
          </div>
        ) : (
          <div className="space-y-1 max-h-[240px] overflow-y-auto">
            {timeline.slice().reverse().map((p, i) => (
              <div
                key={`${p.ts}-${i}`}
                className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
              >
                <span className="text-muted-foreground tabular-nums w-32 shrink-0">
                  {new Date(p.ts * 1000).toLocaleString()}
                </span>
                <span
                  className={`px-1.5 py-0.5 rounded text-[10px] font-medium shrink-0 ${kindColor(p.kind)}`}
                >
                  {t(`dashboard.learning.kind.${p.kind}`)}
                </span>
                {p.skillId && (
                  <span className="text-foreground font-medium truncate flex-1">
                    {p.skillId}
                  </span>
                )}
                {p.source && (
                  <span className="text-[10px] text-muted-foreground">{p.source}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Top skills */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.topSkills")}
          </h4>
          {topSkills.length === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noSkillUsage")}
            </div>
          ) : (
            <div className="space-y-1.5">
              {topSkills.map((s) => (
                <div
                  key={s.skillId}
                  className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
                >
                  <span className="flex-1 truncate font-medium">{s.skillId}</span>
                  <span className="text-muted-foreground tabular-nums">
                    {s.usedCount}× · {s.lastUsedTs ? new Date(s.lastUsedTs * 1000).toLocaleDateString() : "—"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Recall effectiveness */}
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.recallEffectiveness")}
          </h4>
          {totalRecall === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noRecall")}
            </div>
          ) : (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.recallHits")}</span>
                <span className="font-mono">{recall?.hits ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-emerald-500 transition-all"
                  style={{ width: `${100 - summaryPct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.summarized")}</span>
                <span className="font-mono">{recall?.summarized ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-sky-500 transition-all"
                  style={{ width: `${summaryPct}%` }}
                />
              </div>
              <div className="text-[10px] text-muted-foreground text-right pt-1">
                {t("dashboard.learning.summaryPct", { pct: summaryPct })}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function OverviewCard({
  label,
  value,
  hint,
}: {
  label: string
  value: number
  hint?: string
}) {
  return (
    <div className="border border-border/60 rounded-lg p-3 flex flex-col gap-1">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[10px] text-muted-foreground">{hint}</div>}
    </div>
  )
}

function CodingImprovementSection({ coding }: { coding: CodingImprovementDashboard | null }) {
  const { t } = useTranslation()
  const overview = coding?.overview
  const recentTimeline = coding?.timeline.slice(-10).reverse() ?? []
  const maxTimelineValue = Math.max(
    1,
    ...recentTimeline.map(
      (p) =>
        p.completedWorkflows +
        p.blockedWorkflows +
        p.failedWorkflows +
        p.evalPassed +
        p.evalFailed +
        p.proposalsCreated +
        p.proposalsApplied +
        p.proposalsPromoted +
        p.retroRecommendations,
    ),
  )

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.codingImprovement", {
            defaultValue: "Coding improvement",
          })}
        </h4>
        {coding?.generatedAt && (
          <span className="text-[10px] text-muted-foreground">
            {new Date(coding.generatedAt).toLocaleString()}
          </span>
        )}
      </div>

      <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
        <InsightCard
          icon={GitBranch}
          label={t("dashboard.learning.workflowHealth", {
            defaultValue: "Workflow",
          })}
          value={formatPct(overview?.workflowCompletionRate)}
          hint={`${overview?.completedWorkflows ?? 0}/${overview?.workflowRuns ?? 0}`}
        />
        <InsightCard
          icon={CheckCircle2}
          label={t("dashboard.learning.evalHealth", { defaultValue: "Eval" })}
          value={formatPct(overview?.evalSuccessRate)}
          hint={`${overview?.passedEvalRuns ?? 0}/${overview?.evalRuns ?? 0}`}
        />
        <InsightCard
          icon={ShieldAlert}
          label={t("dashboard.learning.blockers", { defaultValue: "Blockers" })}
          value={overview?.openReviewBlockers ?? 0}
          hint={t("dashboard.learning.verificationFailures", {
            defaultValue: "{{n}} verification",
            n: overview?.failedVerificationSteps ?? 0,
          })}
        />
        <InsightCard
          icon={Layers3}
          label={t("dashboard.learning.distillationQueue", {
            defaultValue: "Distillation",
          })}
          value={overview?.distillationCandidates ?? 0}
          hint={t("dashboard.learning.proposalHint", {
            defaultValue: "{{n}} drafts",
            n: overview?.draftProposals ?? 0,
          })}
        />
        <InsightCard
          icon={Activity}
          label={t("dashboard.learning.retros", { defaultValue: "Retros" })}
          value={overview?.retros ?? 0}
          hint={t("dashboard.learning.retroHint", {
            defaultValue: "{{n}} recommendations",
            n: overview?.retroRecommendations ?? 0,
          })}
        />
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-[1.3fr_1fr] gap-3">
        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <div className="flex items-center justify-between mb-3">
            <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("dashboard.learning.projectSignals", {
                defaultValue: "Project signals",
              })}
            </h4>
            <span className="text-[10px] text-muted-foreground tabular-nums">
              {coding?.byProject.length ?? 0}
            </span>
          </div>
          {coding?.byProject.length ? (
            <div className="space-y-2">
              {coding.byProject.map((project) => (
                <ProjectSignalRow
                  key={project.projectId ?? "__unassigned__"}
                  name={project.projectName ?? project.projectId ?? "Unassigned"}
                  projectId={project.projectId}
                  workflowRate={project.workflowCompletionRate}
                  evalRate={project.evalSuccessRate}
                  blockers={project.openReviewBlockers}
                  candidates={project.distillationCandidates}
                />
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noProjectSignals", {
              defaultValue: "No coding improvement signals",
            })} />
          )}
        </div>

        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.failureModes", { defaultValue: "Failure modes" })}
          </h4>
          {coding?.topFailures.length ? (
            <div className="space-y-2">
              {coding.topFailures.map((failure) => (
                <div
                  key={failure.category}
                  className="flex items-center gap-2 text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <span className={`h-2 w-2 rounded-full ${severityDot(failure.severity)}`} />
                  <span className="font-medium truncate flex-1">{failure.label}</span>
                  <span className="text-muted-foreground tabular-nums">{failure.count}</span>
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noFailureModes", {
              defaultValue: "No failure modes",
            })} />
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.improvementTimeline", {
              defaultValue: "Improvement timeline",
            })}
          </h4>
          {recentTimeline.length ? (
            <div className="space-y-2">
              {recentTimeline.map((point) => {
                const total =
                  point.completedWorkflows +
                  point.blockedWorkflows +
                  point.failedWorkflows +
                  point.evalPassed +
                  point.evalFailed +
                  point.proposalsCreated +
                  point.proposalsApplied +
                  point.proposalsPromoted +
                  point.retroRecommendations
                return (
                  <div key={point.date} className="flex items-center gap-3 text-xs">
                    <span className="w-20 text-muted-foreground tabular-nums">
                      {point.date}
                    </span>
                    <div className="h-2 flex-1 bg-secondary/40 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-emerald-500"
                        style={{ width: `${Math.max(4, (total / maxTimelineValue) * 100)}%` }}
                      />
                    </div>
                    <span className="w-8 text-right tabular-nums text-muted-foreground">
                      {total}
                    </span>
                  </div>
                )
              })}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noTimeline", {
              defaultValue: "No timeline data",
            })} />
          )}
        </div>

        <div className="border border-border/60 rounded-lg p-4 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.latestRetros", { defaultValue: "Latest retros" })}
          </h4>
          {coding?.latestRetros.length ? (
            <div className="space-y-2 max-h-[220px] overflow-y-auto">
              {coding.latestRetros.map((retro) => (
                <div
                  key={retro.id}
                  className="text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <div className="flex items-center gap-2 mb-1">
                    <span className={`px-1.5 py-0.5 rounded text-[10px] ${stateTone(retro.runState)}`}>
                      {retro.runState}
                    </span>
                    <span className="text-muted-foreground tabular-nums">
                      {new Date(retro.updatedAt).toLocaleDateString()}
                    </span>
                  </div>
                  <p className="text-foreground line-clamp-2">{retro.summary}</p>
                  {retro.recommendations[0] && (
                    <p className="text-[10px] text-muted-foreground mt-1 truncate">
                      {retro.recommendations[0].title}
                    </p>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noRetros", {
              defaultValue: "No retros",
            })} />
          )}
        </div>
      </div>
    </section>
  )
}

function InsightCard({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: LucideIcon
  label: string
  value: string | number
  hint?: string
}) {
  return (
    <div className="border border-border/60 rounded-lg p-3 flex flex-col gap-2 min-w-0">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{label}</span>
      </div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[10px] text-muted-foreground truncate">{hint}</div>}
    </div>
  )
}

function ProjectSignalRow({
  name,
  projectId,
  workflowRate,
  evalRate,
  blockers,
  candidates,
}: {
  name: string
  projectId: string | null
  workflowRate: number | null
  evalRate: number | null
  blockers: number
  candidates: number
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto_auto_auto_auto] gap-3 items-center text-xs">
      <div className="min-w-0">
        <div className="font-medium truncate">{name}</div>
        {projectId && <div className="text-[10px] text-muted-foreground truncate">{projectId}</div>}
      </div>
      <MetricPill label="WF" value={formatPct(workflowRate)} />
      <MetricPill label="EV" value={formatPct(evalRate)} />
      <MetricPill label="B" value={blockers} tone={blockers > 0 ? "warn" : "muted"} />
      <MetricPill label="Q" value={candidates} tone={candidates > 0 ? "accent" : "muted"} />
    </div>
  )
}

function MetricPill({
  label,
  value,
  tone = "muted",
}: {
  label: string
  value: string | number
  tone?: "muted" | "warn" | "accent"
}) {
  const toneClass =
    tone === "warn"
      ? "bg-red-500/10 text-red-600 dark:text-red-400"
      : tone === "accent"
        ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
        : "bg-secondary/40 text-muted-foreground"
  return (
    <span className={`inline-flex min-w-12 justify-center rounded px-1.5 py-0.5 tabular-nums ${toneClass}`}>
      {label}:{value}
    </span>
  )
}

function EmptyLine({ label }: { label: string }) {
  return <div className="text-xs text-muted-foreground text-center py-6">{label}</div>
}

function formatPct(value: number | null | undefined): string {
  return typeof value === "number" ? `${Math.round(value * 100)}%` : "—"
}

function severityDot(severity: string): string {
  switch (severity) {
    case "high":
      return "bg-red-500"
    case "medium":
      return "bg-amber-500"
    default:
      return "bg-muted-foreground/40"
  }
}

function stateTone(state: string): string {
  switch (state) {
    case "completed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "blocked":
    case "failed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function kindColor(kind: string): string {
  switch (kind) {
    case "skill_created":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "skill_activated":
      return "bg-sky-500/10 text-sky-600 dark:text-sky-400"
    case "skill_patched":
      return "bg-amber-500/10 text-amber-600 dark:text-amber-400"
    case "skill_discarded":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}
