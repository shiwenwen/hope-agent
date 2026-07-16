import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Clock3,
  ExternalLink,
  Flag,
  GitBranch,
  ListChecks,
  RefreshCw,
  Repeat2,
} from "lucide-react"
import {
  Line,
  LineChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  XAxis,
  YAxis,
} from "recharts"

import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { ProjectMeta } from "@/types/project"
import type {
  AttentionItem,
  ControlPlaneDashboard,
  ControlPlaneTrendPoint,
  DurationMetric,
  NamedCount,
  RatioMetric,
} from "./types"

export const UNASSIGNED_PROJECT = "__unassigned__"

interface ControlPlaneSectionProps {
  data: ControlPlaneDashboard | null
  loading: boolean
  projectId: string | null
  onProjectChange: (projectId: string | null) => void
  onOpenPlanHistory?: () => void
  onOpenAttention?: (item: AttentionItem) => void
  initialSection?: "overview" | "goals" | "workflows" | "loops" | "progress"
}

function percent(metric: RatioMetric): string {
  return metric.rate == null ? "—" : `${(metric.rate * 100).toFixed(1)}%`
}

function formatSeconds(seconds: number | null): string {
  if (seconds == null) return "—"
  if (seconds < 60) return `${Math.round(seconds)}s`
  if (seconds < 3600) return `${(seconds / 60).toFixed(1)}m`
  if (seconds < 86400) return `${(seconds / 3600).toFixed(1)}h`
  return `${(seconds / 86400).toFixed(1)}d`
}

function humanize(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (char) => char.toUpperCase())
}

export default function ControlPlaneSection({
  data,
  loading,
  projectId,
  onProjectChange,
  onOpenPlanHistory,
  onOpenAttention,
  initialSection = "overview",
}: ControlPlaneSectionProps) {
  const { t } = useTranslation()
  const [section, setSection] = useState(initialSection)
  const [projects, setProjects] = useState<ProjectMeta[]>([])

  useEffect(() => {
    getTransport()
      .call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true })
      .then(setProjects)
      .catch(() => setProjects([]))
  }, [])

  const projectNames = useMemo(
    () => new Map(projects.map((project) => [project.id, project.name])),
    [projects],
  )

  if (loading && !data) {
    return (
      <div className="mt-4 grid gap-3 md:grid-cols-4">
        {Array.from({ length: 8 }, (_, index) => (
          <div key={index} className="h-24 animate-pulse rounded-xl bg-muted" />
        ))}
      </div>
    )
  }

  if (!data) {
    return (
      <div className="mt-4 rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
        {t("dashboard.controlPlane.empty")}
      </div>
    )
  }

  return (
    <div className="mt-4 space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card/60 p-3">
        <div>
          <div className="text-sm font-medium">{t("dashboard.controlPlane.title")}</div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t("dashboard.controlPlane.scopeHint")}
          </div>
        </div>
        <Select
          value={projectId ?? "__all__"}
          onValueChange={(value) => onProjectChange(value === "__all__" ? null : value)}
        >
          <SelectTrigger className="h-8 w-48 text-xs">
            <SelectValue placeholder={t("dashboard.controlPlane.allProjects")} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t("dashboard.controlPlane.allProjects")}</SelectItem>
            <SelectItem value={UNASSIGNED_PROJECT}>
              {t("dashboard.controlPlane.unassignedProject")}
            </SelectItem>
            {projects.map((project) => (
              <SelectItem key={project.id} value={project.id}>
                {project.name}
                {project.archived ? ` · ${t("dashboard.controlPlane.archived")}` : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <Tabs value={section} onValueChange={(value) => setSection(value as typeof section)}>
        <TabsList className="h-auto flex-wrap justify-start">
          <TabsTrigger value="overview">
            {t("dashboard.controlPlane.sections.overview")}
          </TabsTrigger>
          <TabsTrigger value="goals">Goal</TabsTrigger>
          <TabsTrigger value="workflows">Workflow</TabsTrigger>
          <TabsTrigger value="loops">Loop</TabsTrigger>
          <TabsTrigger value="progress">
            {t("dashboard.controlPlane.sections.planTask")}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-4">
          <div className="grid gap-3 md:grid-cols-5">
            <MetricCard
              icon={Flag}
              label={t("dashboard.controlPlane.metrics.goalAcceptance")}
              value={percent(data.summary.goalAcceptance)}
              hint={`${data.summary.goalAcceptance.numerator} / ${data.summary.goalAcceptance.denominator}`}
              tone="emerald"
            />
            <MetricCard
              icon={GitBranch}
              label={t("dashboard.controlPlane.metrics.workflowCompletion")}
              value={percent(data.summary.workflowCompletion)}
              hint={`${data.summary.workflowCompletion.numerator} / ${data.summary.workflowCompletion.denominator}`}
              tone="blue"
            />
            <MetricCard
              icon={Repeat2}
              label={t("dashboard.controlPlane.metrics.loopStrongProgress")}
              value={percent(data.summary.loopStrongProgress)}
              hint={`${data.summary.loopStrongProgress.numerator} / ${data.summary.loopStrongProgress.denominator}`}
              tone="violet"
            />
            <MetricCard
              icon={ListChecks}
              label={t("dashboard.controlPlane.metrics.taskCompletion")}
              value={percent(data.summary.taskCohortCompletion)}
              hint={`${data.summary.taskCohortCompletion.numerator} / ${data.summary.taskCohortCompletion.denominator}`}
              tone="amber"
            />
            <MetricCard
              icon={AlertTriangle}
              label={t("dashboard.controlPlane.metrics.attention")}
              value={String(data.summary.attentionCount)}
              hint={t("dashboard.controlPlane.currentSnapshot")}
              tone={data.summary.attentionCount > 0 ? "red" : "slate"}
            />
          </div>

          <div className="grid gap-4 xl:grid-cols-[1.2fr_1fr]">
            <TrendChart
              title={t("dashboard.controlPlane.goalTrend")}
              data={data.goals.trend}
              lines={[
                { key: "resolved", label: t("dashboard.controlPlane.resolved"), color: "#64748b" },
                { key: "accepted", label: t("dashboard.controlPlane.accepted"), color: "#10b981" },
              ]}
            />
            <AttentionList
              items={data.attention.items}
              total={data.attention.total}
              onOpen={onOpenAttention}
            />
          </div>
        </TabsContent>

        <TabsContent value="goals" className="space-y-4">
          <div className="grid gap-3 md:grid-cols-3">
            <MetricCard
              icon={CheckCircle2}
              label={t("dashboard.controlPlane.metrics.requiredCriteria")}
              value={percent(data.goals.requiredCriteria)}
              hint={t("dashboard.controlPlane.auditSamples", {
                count: data.goals.auditedGoalCount,
              })}
              tone="emerald"
            />
            <DurationCard
              label={t("dashboard.controlPlane.metrics.goalP50")}
              duration={data.goals.acceptedDuration}
            />
            <MetricCard
              icon={Flag}
              label={t("dashboard.controlPlane.metrics.goalAcceptance")}
              value={percent(data.goals.acceptance)}
              hint={`${data.goals.acceptance.numerator} / ${data.goals.acceptance.denominator}`}
              tone="blue"
            />
          </div>
          <DistributionGrid
            groups={[
              [t("dashboard.controlPlane.currentStates"), data.goals.currentStates],
              [t("dashboard.controlPlane.closureOutcomes"), data.goals.closureOutcomes],
              [t("dashboard.controlPlane.domains"), data.goals.domains],
            ]}
          />
          <TrendChart
            title={t("dashboard.controlPlane.goalTrend")}
            data={data.goals.trend}
            lines={[
              { key: "resolved", label: t("dashboard.controlPlane.resolved"), color: "#64748b" },
              { key: "accepted", label: t("dashboard.controlPlane.accepted"), color: "#10b981" },
            ]}
          />
        </TabsContent>

        <TabsContent value="workflows" className="space-y-4">
          <div className="grid gap-3 md:grid-cols-5">
            <MetricCard
              icon={CheckCircle2}
              label={t("dashboard.controlPlane.metrics.workflowCompletion")}
              value={percent(data.workflows.completion)}
              hint={`${data.workflows.completion.numerator} / ${data.workflows.completion.denominator}`}
              tone="emerald"
            />
            <MetricCard
              icon={AlertTriangle}
              label={t("dashboard.controlPlane.metrics.opFailure")}
              value={percent(data.workflows.opFailure)}
              hint={`${data.workflows.opFailure.numerator} / ${data.workflows.opFailure.denominator}`}
              tone="red"
            />
            <DurationCard
              label={t("dashboard.controlPlane.metrics.workflowP50")}
              duration={data.workflows.duration}
            />
            <MetricCard
              icon={Flag}
              label={t("dashboard.controlPlane.metrics.goalBinding")}
              value={percent(data.workflows.goalBinding)}
              hint={`${data.workflows.goalBinding.numerator} / ${data.workflows.goalBinding.denominator}`}
              tone="blue"
            />
            <MetricCard
              icon={AlertTriangle}
              label={t("dashboard.controlPlane.metrics.approvalTrigger")}
              value={percent(data.workflows.approvalTrigger)}
              hint={`${data.workflows.approvalTrigger.numerator} / ${data.workflows.approvalTrigger.denominator}`}
              tone="amber"
            />
          </div>
          <DistributionGrid
            groups={[
              [t("dashboard.controlPlane.currentStates"), data.workflows.currentStates],
              [t("dashboard.controlPlane.kinds"), data.workflows.kinds],
              [t("dashboard.controlPlane.origins"), data.workflows.origins],
            ]}
          />
          <TrendChart
            title={t("dashboard.controlPlane.workflowTrend")}
            data={data.workflows.trend}
            lines={[
              {
                key: "completed",
                label: t("dashboard.controlPlane.decidedRuns"),
                color: "#3b82f6",
              },
            ]}
          />
        </TabsContent>

        <TabsContent value="loops" className="space-y-4">
          <div className="grid gap-3 md:grid-cols-4">
            <MetricCard
              icon={Activity}
              label={t("dashboard.controlPlane.metrics.loopStrongProgress")}
              value={percent(data.loops.strongProgress)}
              hint={`${data.loops.strongProgress.numerator} / ${data.loops.strongProgress.denominator}`}
              tone="violet"
            />
            <MetricCard
              icon={AlertTriangle}
              label={t("dashboard.controlPlane.metrics.noProgress")}
              value={percent(data.loops.noProgress)}
              hint={`${data.loops.noProgress.numerator} / ${data.loops.noProgress.denominator}`}
              tone="red"
            />
            <MetricCard
              icon={Repeat2}
              label={t("dashboard.controlPlane.metrics.blockedSchedules")}
              value={String(data.loops.currentBlockedSchedules)}
              hint={t("dashboard.controlPlane.currentSnapshot")}
              tone="amber"
            />
            <DurationCard
              label={t("dashboard.controlPlane.metrics.loopP50")}
              duration={data.loops.duration}
            />
          </div>
          <DistributionGrid
            groups={[
              [t("dashboard.controlPlane.progressStates"), data.loops.progressStates],
              [t("dashboard.controlPlane.triggerKinds"), data.loops.triggerKinds],
              [t("dashboard.controlPlane.strategies"), data.loops.strategies],
              [t("dashboard.controlPlane.currentStates"), data.loops.currentStates],
            ]}
          />
          <TrendChart
            title={t("dashboard.controlPlane.loopTrend")}
            data={data.loops.trend}
            lines={[
              {
                key: "completed",
                label: t("dashboard.controlPlane.evaluatedRuns"),
                color: "#8b5cf6",
              },
            ]}
          />
        </TabsContent>

        <TabsContent value="progress" className="space-y-4">
          <div className="flex justify-end">
            <Button variant="outline" size="sm" onClick={onOpenPlanHistory}>
              <ExternalLink className="mr-2 h-3.5 w-3.5" />
              {t("dashboard.controlPlane.openPlanHistory")}
            </Button>
          </div>
          <div className="grid gap-4 xl:grid-cols-2">
            <ProgressPanel
              title="Task"
              completion={data.tasks.cohortCompletion}
              currentValue={data.tasks.currentBacklog}
              currentLabel={t("dashboard.controlPlane.metrics.currentBacklog")}
              duration={data.tasks.duration}
              states={data.tasks.currentStates}
              trend={data.tasks.trend}
            />
            <ProgressPanel
              title="Plan"
              completion={data.plans.cohortCompletion}
              currentValue={data.plans.activeNow}
              currentLabel={t("dashboard.controlPlane.metrics.activePlans")}
              duration={data.plans.duration}
              states={data.plans.currentStates}
              trend={data.plans.trend}
            />
          </div>
          <DistributionGrid
            groups={[
              [t("dashboard.controlPlane.planByAgent"), data.plans.byAgent],
              [
                t("dashboard.controlPlane.planByProject"),
                data.plans.byProject.map((item) => ({
                  ...item,
                  key:
                    item.key === "unassigned"
                      ? t("dashboard.controlPlane.unassignedProject")
                      : (projectNames.get(item.key) ?? item.key),
                })),
              ],
            ]}
          />
          <div className="rounded-xl border bg-muted/20 p-3 text-xs text-muted-foreground">
            {t("dashboard.controlPlane.noFunnelHint")}
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}

function MetricCard({
  icon: Icon,
  label,
  value,
  hint,
  tone,
}: {
  icon: React.ElementType
  label: string
  value: string
  hint?: string
  tone: "emerald" | "blue" | "violet" | "amber" | "red" | "slate"
}) {
  const toneClass = {
    emerald: "bg-emerald-500/10 text-emerald-600",
    blue: "bg-blue-500/10 text-blue-600",
    violet: "bg-violet-500/10 text-violet-600",
    amber: "bg-amber-500/10 text-amber-600",
    red: "bg-red-500/10 text-red-600",
    slate: "bg-slate-500/10 text-slate-600",
  }[tone]
  return (
    <div className="rounded-xl border bg-card p-3">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className={cn("rounded-md p-1.5", toneClass)}>
          <Icon className="h-3.5 w-3.5" />
        </span>
        <span className="truncate">{label}</span>
      </div>
      <div className="mt-2 text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="mt-1 text-[11px] text-muted-foreground">{hint}</div>}
    </div>
  )
}

function DurationCard({ label, duration }: { label: string; duration: DurationMetric }) {
  const { t } = useTranslation()
  const coverage =
    duration.eligibleCount > 0
      ? Math.round((duration.sampleCount / duration.eligibleCount) * 100)
      : null
  return (
    <MetricCard
      icon={Clock3}
      label={label}
      value={formatSeconds(duration.p50Secs)}
      hint={
        coverage == null
          ? t("dashboard.controlPlane.noExactSamples")
          : t("dashboard.controlPlane.coverage", {
              samples: duration.sampleCount,
              eligible: duration.eligibleCount,
              percent: coverage,
            })
      }
      tone="slate"
    />
  )
}

function DistributionGrid({ groups }: { groups: [string, NamedCount[]][] }) {
  return (
    <div className={cn("grid gap-4", groups.length >= 3 ? "lg:grid-cols-3" : "lg:grid-cols-2")}>
      {groups.map(([title, items]) => (
        <DistributionCard key={title} title={title} items={items} />
      ))}
    </div>
  )
}

function DistributionCard({ title, items }: { title: string; items: NamedCount[] }) {
  const { t } = useTranslation()
  const max = Math.max(1, ...items.map((item) => item.count))
  return (
    <div className="rounded-xl border bg-card p-4">
      <h3 className="mb-3 text-sm font-medium">{title}</h3>
      {items.length === 0 ? (
        <div className="py-8 text-center text-xs text-muted-foreground">
          {t("dashboard.controlPlane.empty")}
        </div>
      ) : (
        <div className="space-y-2">
          {items.map((item) => (
            <div
              key={item.key}
              className="grid grid-cols-[minmax(0,1fr)_3fr_auto] items-center gap-2 text-xs"
            >
              <span className="truncate text-muted-foreground">
                {t(`dashboard.controlPlane.states.${item.key}`, {
                  defaultValue: humanize(item.key),
                })}
              </span>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary/70"
                  style={{ width: `${Math.max(3, (item.count / max) * 100)}%` }}
                />
              </div>
              <span className="tabular-nums">{item.count}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function TrendChart({
  title,
  data,
  lines,
}: {
  title: string
  data: ControlPlaneTrendPoint[]
  lines: { key: string; label: string; color: string }[]
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-xl border bg-card p-4">
      <h3 className="mb-3 text-sm font-medium">{title}</h3>
      {data.length === 0 ? (
        <div className="flex h-[210px] items-center justify-center text-xs text-muted-foreground">
          {t("dashboard.controlPlane.empty")}
        </div>
      ) : (
        <ResponsiveContainer width="100%" height={210}>
          <LineChart data={data} margin={{ top: 8, right: 12, left: -16, bottom: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
            <XAxis dataKey="date" fontSize={10} tickFormatter={(value: string) => value.slice(5)} />
            <YAxis allowDecimals={false} fontSize={10} />
            <RechartsTooltip
              contentStyle={{
                background: "hsl(var(--card))",
                border: "1px solid hsl(var(--border))",
                borderRadius: 8,
                fontSize: 12,
              }}
            />
            {lines.map((line) => (
              <Line
                key={line.key}
                type="monotone"
                dataKey={line.key}
                name={line.label}
                stroke={line.color}
                strokeWidth={2}
                dot={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      )}
    </div>
  )
}

function AttentionList({
  items,
  total,
  onOpen,
}: {
  items: AttentionItem[]
  total: number
  onOpen?: (item: AttentionItem) => void
}) {
  const { t } = useTranslation()
  const attentionLabel = (group: "attentionKinds" | "states" | "attentionReasons", value: string) =>
    t(`dashboard.controlPlane.${group}.${value}`, { defaultValue: humanize(value) })
  return (
    <div className="rounded-xl border bg-card p-4">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-medium">{t("dashboard.controlPlane.attentionTitle")}</h3>
        <span className="rounded-full bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700">
          {total}
        </span>
      </div>
      {items.length === 0 ? (
        <div className="flex h-[210px] flex-col items-center justify-center text-xs text-muted-foreground">
          <CheckCircle2 className="mb-2 h-7 w-7 text-emerald-500/60" />
          {t("dashboard.controlPlane.noAttention")}
        </div>
      ) : (
        <div className="max-h-[260px] space-y-1.5 overflow-y-auto">
          {items.map((item) => (
            <button
              key={`${item.kind}:${item.id}`}
              type="button"
              className="flex w-full items-start gap-2 rounded-lg p-2 text-left hover:bg-secondary/40"
              onClick={() => onOpen?.(item)}
            >
              <AlertTriangle
                className={cn(
                  "mt-0.5 h-3.5 w-3.5 shrink-0",
                  item.severity === "critical" ? "text-red-500" : "text-amber-500",
                )}
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-xs font-medium">{item.title}</span>
                <span className="mt-0.5 block truncate text-[11px] text-muted-foreground">
                  {attentionLabel("attentionKinds", item.kind)} ·{" "}
                  {attentionLabel("states", item.status)} ·{" "}
                  {attentionLabel("attentionReasons", item.reason)}
                </span>
              </span>
              <ExternalLink className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function ProgressPanel({
  title,
  completion,
  currentValue,
  currentLabel,
  duration,
  states,
  trend,
}: {
  title: string
  completion: RatioMetric
  currentValue: number
  currentLabel: string
  duration: DurationMetric
  states: NamedCount[]
  trend: ControlPlaneTrendPoint[]
}) {
  const { t } = useTranslation()
  return (
    <div className="space-y-3 rounded-xl border bg-card p-4">
      <h3 className="text-sm font-semibold">{title}</h3>
      <div className="grid grid-cols-3 gap-2">
        <div className="rounded-lg bg-muted/50 p-2">
          <div className="text-[10px] text-muted-foreground">
            {t("dashboard.controlPlane.metrics.cohortCompletion")}
          </div>
          <div className="mt-1 text-lg font-semibold">{percent(completion)}</div>
        </div>
        <div className="rounded-lg bg-muted/50 p-2">
          <div className="text-[10px] text-muted-foreground">{currentLabel}</div>
          <div className="mt-1 text-lg font-semibold">{currentValue}</div>
        </div>
        <div className="rounded-lg bg-muted/50 p-2">
          <div className="text-[10px] text-muted-foreground">P50</div>
          <div className="mt-1 text-lg font-semibold">{formatSeconds(duration.p50Secs)}</div>
        </div>
      </div>
      <DistributionCard title={t("dashboard.controlPlane.currentStates")} items={states} />
      <TrendChart
        title={t("dashboard.controlPlane.creationTrend")}
        data={trend}
        lines={[{ key: "created", label: t("dashboard.controlPlane.created"), color: "#3b82f6" }]}
      />
      <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
        <RefreshCw className="h-3 w-3" />
        {duration.eligibleCount > 0
          ? t("dashboard.controlPlane.coverageShort", {
              samples: duration.sampleCount,
              eligible: duration.eligibleCount,
            })
          : t("dashboard.controlPlane.noExactSamples")}
      </div>
    </div>
  )
}
