import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Activity,
  CheckCircle2,
  GitBranch,
  Layers3,
  Loader2,
  Play,
  RefreshCw,
  ShieldAlert,
  Sparkles,
} from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  CodingBenchmarkCenterReport,
  CodingEvalReleaseGateReport,
  CodingEvalGoldTaskPackReport,
  CodingLearningGeneralizationReport,
} from "@/lib/transport"
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
const DAY_MS = 24 * 60 * 60 * 1000

interface LearningTabProps {
  filter: DashboardFilter
}

function releaseGateWindowDays(filter: DashboardFilter, fallbackDays: number): number {
  if (!filter.startDate) return fallbackDays
  const start = Date.parse(filter.startDate)
  if (!Number.isFinite(start)) return fallbackDays
  return Math.max(1, Math.min(180, Math.ceil((Date.now() - start) / DAY_MS)))
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
  const [benchmark, setBenchmark] = useState<CodingBenchmarkCenterReport | null>(null)
  const [releaseGate, setReleaseGate] = useState<CodingEvalReleaseGateReport | null>(null)
  const [generalization, setGeneralization] =
    useState<CodingLearningGeneralizationReport | null>(null)
  const [benchmarkRunning, setBenchmarkRunning] = useState(false)
  const [benchmarkError, setBenchmarkError] = useState<string | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    setBenchmarkError(null)
    try {
      const [ov, tl, ts, rs, ci, bc, rg, gen] = await Promise.all([
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
        getTransport().call<CodingBenchmarkCenterReport>("get_coding_benchmark_center", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
            limit: 12,
          },
        }),
        getTransport().call<CodingEvalReleaseGateReport>("evaluate_coding_eval_release_gate", {
          input: {
            windowDays: releaseGateWindowDays(filter, windowDays),
          },
        }),
        getTransport().call<CodingLearningGeneralizationReport>(
          "evaluate_coding_learning_generalization",
          {
            input: {
              windowDays: releaseGateWindowDays(filter, windowDays),
            },
          },
        ),
      ])
      setOverview(ov)
      setTimeline(tl ?? [])
      setTopSkills(ts ?? [])
      setRecall(rs)
      setCoding(ci)
      setBenchmark(bc)
      setReleaseGate(rg)
      setGeneralization(gen)
    } catch (e) {
      logger.error("dashboard", "LearningTab::load", "Failed to load learning data", e)
    } finally {
      setLoading(false)
    }
  }, [filter, windowDays])

  const runBenchmark = useCallback(async () => {
    setBenchmarkRunning(true)
    setBenchmarkError(null)
    try {
      await getTransport().call<CodingEvalGoldTaskPackReport>("run_coding_eval_gold_task_pack", {
        input: {
          executionMode: "fixture_patch",
          baselineKind: "deterministic_mock",
          label: "Benchmark Center deterministic run",
          sourceType: "benchmark_center",
          sourceId: "phase6.1",
          recordEvalRuns: true,
          recordPackRun: true,
          evaluateGoal: true,
        },
      })
      await reload()
    } catch (e) {
      setBenchmarkError(e instanceof Error ? e.message : String(e))
      logger.error("dashboard", "LearningTab::runBenchmark", "Failed to run benchmark pack", e)
    } finally {
      setBenchmarkRunning(false)
    }
  }, [reload])

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

      <CodingImprovementSection
        coding={coding}
        benchmark={benchmark}
        releaseGate={releaseGate}
        generalization={generalization}
        benchmarkRunning={benchmarkRunning}
        benchmarkError={benchmarkError}
        onRunBenchmark={runBenchmark}
      />

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

function CodingImprovementSection({
  coding,
  benchmark,
  releaseGate,
  generalization,
  benchmarkRunning,
  benchmarkError,
  onRunBenchmark,
}: {
  coding: CodingImprovementDashboard | null
  benchmark: CodingBenchmarkCenterReport | null
  releaseGate: CodingEvalReleaseGateReport | null
  generalization: CodingLearningGeneralizationReport | null
  benchmarkRunning: boolean
  benchmarkError: string | null
  onRunBenchmark: () => void
}) {
  const { t } = useTranslation()
  const overview = coding?.overview
  const recentTimeline = coding?.timeline.slice(-10).reverse() ?? []
  const failureModes = [...(coding?.topFailures ?? []), ...(coding?.toolCallFailures ?? [])]
  const maxTimelineValue = Math.max(
    1,
    ...recentTimeline.map(
      (p) =>
        p.completedWorkflows +
        p.blockedWorkflows +
        p.failedWorkflows +
        p.evalPassed +
        p.evalFailed +
        p.evalPackPassed +
        p.evalPackFailed +
        p.strategyImproved +
        p.strategyRegressed +
        p.strategyMixed +
        Math.abs(p.validationViolationDelta) +
        Math.abs(p.scopeCreepDelta) +
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

      <div className="grid grid-cols-2 md:grid-cols-4 xl:grid-cols-8 gap-3">
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
          icon={Layers3}
          label={t("dashboard.learning.packHealth", { defaultValue: "Pack" })}
          value={formatPct(overview?.evalPackPassRate)}
          hint={`${overview?.passedEvalPackRuns ?? 0}/${overview?.evalPackRuns ?? 0}`}
        />
        <InsightCard
          icon={Activity}
          label={t("dashboard.learning.strategyEffects", {
            defaultValue: "Strategy",
          })}
          value={overview?.strategyEffectRuns ?? 0}
          hint={`+${overview?.improvedStrategyEffects ?? 0} / -${overview?.regressedStrategyEffects ?? 0}`}
        />
        <InsightCard
          icon={Sparkles}
          label={t("dashboard.learning.toolCalls", { defaultValue: "Tool calls" })}
          value={overview?.missingToolCallRuns ?? 0}
          hint={t("dashboard.learning.missingToolCalls", {
            defaultValue: "missing calls",
          })}
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

      <BenchmarkCenterPanel
        report={benchmark}
        running={benchmarkRunning}
        error={benchmarkError}
        onRun={onRunBenchmark}
      />

      <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
        <ReleaseGatePanel report={releaseGate} />
        <GeneralizationPanel report={generalization} />
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
                  packRate={project.evalPackPassRate}
                  strategyRegressions={project.regressedStrategyEffects}
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
          {failureModes.length ? (
            <div className="space-y-2">
              {failureModes.map((failure) => (
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

      <div className="grid grid-cols-1 xl:grid-cols-3 gap-3">
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
                  point.evalPackPassed +
                  point.evalPackFailed +
                  point.strategyImproved +
                  point.strategyRegressed +
                  point.strategyMixed +
                  Math.abs(point.validationViolationDelta) +
                  Math.abs(point.scopeCreepDelta) +
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
            {t("dashboard.learning.latestStrategyEffects", {
              defaultValue: "Latest strategy effects",
            })}
          </h4>
          {coding?.latestStrategyEffects.length ? (
            <div className="space-y-2 max-h-[220px] overflow-y-auto">
              {coding.latestStrategyEffects.map((effect) => (
                <div
                  key={effect.id}
                  className="text-xs border-b border-border/20 pb-2 last:border-0 last:pb-0"
                >
                  <div className="flex items-center gap-2 mb-1 min-w-0">
                    <span className={`px-1.5 py-0.5 rounded text-[10px] ${verdictTone(effect.verdict)}`}>
                      {effect.verdict}
                    </span>
                    <span className="font-medium truncate flex-1">{effect.strategyType}</span>
                    <span className="text-[10px] text-muted-foreground tabular-nums">
                      {new Date(effect.createdAt).toLocaleDateString()}
                    </span>
                  </div>
                  <p className="text-[10px] text-muted-foreground truncate">
                    {effect.baselineLabel} -&gt; {effect.candidateLabel}
                  </p>
                  <div className="mt-1 flex flex-wrap gap-1.5">
                    <MetricPill
                      label="P"
                      value={formatSignedPct(effect.passRateDelta)}
                      tone={deltaTone(effect.passRateDelta)}
                    />
                    <MetricPill
                      label="S"
                      value={formatSignedPct(effect.averageScoreDelta)}
                      tone={deltaTone(effect.averageScoreDelta)}
                    />
                    <MetricPill
                      label="V"
                      value={formatSignedCount(effect.validationViolationDelta)}
                      tone={inverseDeltaTone(effect.validationViolationDelta)}
                    />
                    <MetricPill
                      label="C"
                      value={formatSignedCount(effect.scopeCreepDelta)}
                      tone={inverseDeltaTone(effect.scopeCreepDelta)}
                    />
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyLine label={t("dashboard.learning.noStrategyEffects", {
              defaultValue: "No strategy effects",
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

function BenchmarkCenterPanel({
  report,
  running,
  error,
  onRun,
}: {
  report: CodingBenchmarkCenterReport | null
  running: boolean
  error: string | null
  onRun: () => void
}) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []
  const recentRuns = report?.runs.slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("dashboard.learning.benchmarkCenter", {
              defaultValue: "Benchmark center",
            })}
          </h4>
          <span
            className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
          >
            {report?.status ?? "loading"}
          </span>
        </div>
        <Button size="sm" variant="outline" className="h-7 gap-1.5" onClick={onRun} disabled={running}>
          {running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
          <span className="text-xs">
            {t("dashboard.learning.runBenchmark", { defaultValue: "Run" })}
          </span>
        </Button>
      </div>
      {report ? (
        <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1.15fr)_minmax(220px,0.85fr)] gap-3">
          <div className="flex flex-wrap gap-1.5 content-start">
            <MetricPill label="RN" value={report.summary.totalRuns} />
            <MetricPill
              label="PR"
              value={formatPct(report.summary.runPassRate)}
              tone={report.summary.failedRuns > 0 ? "warn" : "accent"}
            />
            <MetricPill
              label="CS"
              value={formatPct(report.summary.casePassRate)}
              tone={report.summary.failedCases > 0 ? "warn" : "accent"}
            />
            <MetricPill
              label="EM"
              value={report.summary.externalModelRuns}
              tone={report.summary.externalModelRuns > 0 ? "accent" : "muted"}
            />
          </div>
          <div className="min-w-0 space-y-2">
            {recentRuns.length ? (
              recentRuns.map((run) => (
                <div
                  key={run.id}
                  className="flex flex-wrap items-center gap-2 text-xs border-b border-border/20 pb-1.5 last:border-0 last:pb-0"
                >
                  <span className={`px-1.5 py-0.5 rounded text-[10px] ${releaseGateTone(run.status)}`}>
                    {run.status}
                  </span>
                  <span className="font-medium truncate max-w-48">
                    {run.label ?? run.baselineKind}
                  </span>
                  <span className="text-muted-foreground tabular-nums">
                    {run.passedCases}/{run.passedCases + run.failedCases}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {new Date(run.createdAt).toLocaleDateString()}
                  </span>
                  {run.failedCasesSummary[0] && (
                    <span className="text-[10px] text-muted-foreground truncate basis-full">
                      {run.failedCasesSummary[0]}
                    </span>
                  )}
                </div>
              ))
            ) : (
              <EmptyLine
                label={t("dashboard.learning.noBenchmarkRuns", {
                  defaultValue: "No benchmark runs",
                })}
              />
            )}
          </div>
          <div className="min-w-0 space-y-2">
            <div className="flex flex-wrap gap-1.5">
              {report.baselines.slice(0, 3).map((baseline) => (
                <MetricPill
                  key={baseline.baselineKind}
                  label={baseline.baselineKind === "external_model" ? "EX" : "DT"}
                  value={`${baseline.passedRuns}/${baseline.runs}`}
                  tone={baseline.failedRuns > 0 ? "warn" : "accent"}
                />
              ))}
            </div>
            {attentionChecks.length ? (
              <div className="flex flex-wrap gap-1.5">
                {attentionChecks.map((check) => (
                  <span
                    key={check.name}
                    className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                    title={`${check.expected} · ${check.actual}`}
                  >
                    {check.name}: {check.actual}
                  </span>
                ))}
              </div>
            ) : (
              <span className="text-[10px] text-muted-foreground">
                {t("dashboard.learning.benchmarkClean", {
                  defaultValue: "Benchmark checks passed",
                })}
              </span>
            )}
          </div>
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.benchmarkLoading", {
            defaultValue: "Loading benchmark center",
          })}
        />
      )}
      {error && (
        <p className="mt-2 text-[10px] text-destructive line-clamp-2" title={error}>
          {t("dashboard.learning.benchmarkRunFailed", {
            defaultValue: "Run failed: {{message}}",
            message: error,
          })}
        </p>
      )}
    </div>
  )
}

function ReleaseGatePanel({ report }: { report: CodingEvalReleaseGateReport | null }) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.releaseGate", { defaultValue: "Release gate" })}
        </h4>
        <span
          className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
        >
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1fr)] gap-3">
          <div className="flex flex-wrap gap-1.5">
            <MetricPill label="PK" value={formatPct(report.summary.packPassRate)} />
            <MetricPill
              label="ST"
              value={report.summary.regressedStrategyEffects}
              tone={report.summary.regressedStrategyEffects > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="TC"
              value={report.summary.missingToolCallRuns}
              tone={report.summary.missingToolCallRuns > 0 ? "warn" : "muted"}
            />
            <MetricPill
              label="EX"
              value={report.summary.externalModelPackRuns}
              tone={
                report.thresholds.requireExternalModelPack &&
                report.summary.externalModelPackRuns === 0
                  ? "warn"
                  : "muted"
              }
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5 min-w-0 xl:justify-end">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground xl:text-right">
              {t("dashboard.learning.releaseGateClean", { defaultValue: "All checks passed" })}
            </span>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.releaseGateLoading", {
            defaultValue: "Loading release gate",
          })}
        />
      )}
    </div>
  )
}

function GeneralizationPanel({
  report,
}: {
  report: CodingLearningGeneralizationReport | null
}) {
  const { t } = useTranslation()
  const attentionChecks =
    report?.checks.filter((check) => check.status !== "passed").slice(0, 4) ?? []

  return (
    <div className="border border-border/60 rounded-lg p-4 min-w-0">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t("dashboard.learning.generalizationGate", {
            defaultValue: "Generalization gate",
          })}
        </h4>
        <span
          className={`px-2 py-1 rounded text-[10px] font-medium ${releaseGateTone(report?.status)}`}
        >
          {report?.status ?? "loading"}
        </span>
      </div>
      {report ? (
        <div className="grid grid-cols-1 xl:grid-cols-[auto_minmax(0,1fr)] gap-3">
          <div className="flex flex-wrap gap-1.5">
            <MetricPill label="PR" value={`${report.summary.passedProjects}/${report.summary.projectsEvaluated}`} />
            <MetricPill
              label="LR"
              value={report.summary.totalPromotedLearning}
              tone={report.summary.totalPromotedLearning > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="PK"
              value={report.summary.totalPackRuns}
              tone={report.summary.projectsWithPackRuns > 0 ? "accent" : "muted"}
            />
            <MetricPill
              label="RG"
              value={report.summary.regressedProjects}
              tone={report.summary.regressedProjects > 0 ? "warn" : "muted"}
            />
          </div>
          {attentionChecks.length ? (
            <div className="flex flex-wrap gap-1.5 min-w-0 xl:justify-end">
              {attentionChecks.map((check) => (
                <span
                  key={check.name}
                  className={`max-w-full truncate rounded px-1.5 py-0.5 text-[10px] ${releaseGateCheckTone(check.status)}`}
                  title={`${check.expected} · ${check.actual}`}
                >
                  {check.name}: {check.actual}
                </span>
              ))}
            </div>
          ) : (
            <span className="text-[10px] text-muted-foreground xl:text-right">
              {t("dashboard.learning.generalizationClean", {
                defaultValue: "Cross-project checks passed",
              })}
            </span>
          )}
        </div>
      ) : (
        <EmptyLine
          label={t("dashboard.learning.generalizationLoading", {
            defaultValue: "Loading generalization gate",
          })}
        />
      )}
    </div>
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
  packRate,
  strategyRegressions,
  blockers,
  candidates,
}: {
  name: string
  projectId: string | null
  workflowRate: number | null
  evalRate: number | null
  packRate: number | null
  strategyRegressions: number
  blockers: number
  candidates: number
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-2 text-xs sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
      <div className="min-w-0">
        <div className="font-medium truncate">{name}</div>
        {projectId && <div className="text-[10px] text-muted-foreground truncate">{projectId}</div>}
      </div>
      <div className="flex flex-wrap gap-1.5 sm:justify-end">
        <MetricPill label="WF" value={formatPct(workflowRate)} />
        <MetricPill label="EV" value={formatPct(evalRate)} />
        <MetricPill label="PK" value={formatPct(packRate)} />
        <MetricPill
          label="ST"
          value={strategyRegressions}
          tone={strategyRegressions > 0 ? "warn" : "muted"}
        />
        <MetricPill label="B" value={blockers} tone={blockers > 0 ? "warn" : "muted"} />
        <MetricPill label="Q" value={candidates} tone={candidates > 0 ? "accent" : "muted"} />
      </div>
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

function formatSignedPct(value: number): string {
  const pct = Math.round(value * 100)
  return `${pct > 0 ? "+" : ""}${pct}%`
}

function formatSignedCount(value: number): string {
  return `${value > 0 ? "+" : ""}${value}`
}

function deltaTone(value: number): "muted" | "warn" | "accent" {
  if (value > 0) return "accent"
  if (value < 0) return "warn"
  return "muted"
}

function inverseDeltaTone(value: number): "muted" | "warn" | "accent" {
  if (value < 0) return "accent"
  if (value > 0) return "warn"
  return "muted"
}

function releaseGateTone(status?: string | null): string {
  switch (status) {
    case "passed":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "failed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    case "insufficient_data":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}

function releaseGateCheckTone(status: string): string {
  return status === "failed"
    ? "bg-red-500/10 text-red-600 dark:text-red-400"
    : "bg-amber-500/10 text-amber-700 dark:text-amber-300"
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

function verdictTone(verdict: string): string {
  switch (verdict) {
    case "improved":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "regressed":
    case "mixed":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
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
