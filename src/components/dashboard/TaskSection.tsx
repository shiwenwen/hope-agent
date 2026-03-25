import React from "react"
import { useTranslation } from "react-i18next"
import {
  PieChart,
  Pie,
  Cell,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
} from "recharts"
import {
  Clock,
  PlayCircle,
  CheckCircle2,
  XCircle,
  Timer,
  Coins,
  Bot,
  Skull,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { DashboardTaskData } from "./types"
import { formatNumber, formatDuration } from "./types"

interface TaskSectionProps {
  data: DashboardTaskData | null
  loading: boolean
}

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

function MiniCard({
  icon: Icon,
  label,
  value,
  colorClass,
  bgClass,
}: {
  icon: React.ElementType
  label: string
  value: string
  colorClass: string
  bgClass: string
}) {
  return (
    <div className="flex items-center gap-2 p-2 rounded-lg bg-muted/50">
      <div
        className={cn(
          "h-7 w-7 rounded-full flex items-center justify-center shrink-0",
          bgClass,
        )}
      >
        <Icon className={cn("h-3.5 w-3.5", colorClass)} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold truncate">{value}</div>
        <div className="text-[10px] text-muted-foreground truncate">
          {label}
        </div>
      </div>
    </div>
  )
}

const DONUT_SUCCESS = "#10b981"
const DONUT_FAIL = "#ef4444"
const DONUT_KILLED = "#f59e0b"
const DONUT_EMPTY = "hsl(var(--muted))"

const TaskSection = React.memo(function TaskSection({
  data,
  loading,
}: TaskSectionProps) {
  const { t } = useTranslation()

  if (loading && !data) {
    return (
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mt-4">
        <SectionSkeleton height={360} />
        <SectionSkeleton height={360} />
      </div>
    )
  }

  if (!data) return null

  const cronPieData = (() => {
    const { successRuns, failedRuns, totalRuns } = data.cron
    if (totalRuns === 0) return [{ name: t("dashboard.task.noRuns"), value: 1, color: DONUT_EMPTY }]
    return [
      { name: t("dashboard.task.success"), value: successRuns, color: DONUT_SUCCESS },
      { name: t("dashboard.task.failed"), value: failedRuns, color: DONUT_FAIL },
    ].filter((d) => d.value > 0)
  })()

  const cronSuccessRate =
    data.cron.totalRuns > 0
      ? ((data.cron.successRuns / data.cron.totalRuns) * 100).toFixed(1)
      : "0.0"

  const subagentPieData = (() => {
    const { completed, failed, killed, totalRuns } = data.subagent
    if (totalRuns === 0) return [{ name: t("dashboard.task.noRuns"), value: 1, color: DONUT_EMPTY }]
    return [
      { name: t("dashboard.task.completed"), value: completed, color: DONUT_SUCCESS },
      { name: t("dashboard.task.failed"), value: failed, color: DONUT_FAIL },
      { name: t("dashboard.task.killed"), value: killed, color: DONUT_KILLED },
    ].filter((d) => d.value > 0)
  })()

  const subagentCompletionRate =
    data.subagent.totalRuns > 0
      ? ((data.subagent.completed / data.subagent.totalRuns) * 100).toFixed(1)
      : "0.0"

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mt-4">
      {/* Cron stats */}
      <div className="bg-card border rounded-xl p-4 space-y-4">
        <h3 className="text-sm font-medium">
          {t("dashboard.task.cronTitle")}
        </h3>

        <div className="grid grid-cols-2 gap-2">
          <MiniCard
            icon={Clock}
            label={t("dashboard.task.totalJobs")}
            value={formatNumber(data.cron.totalJobs)}
            colorClass="text-blue-500"
            bgClass="bg-blue-500/10"
          />
          <MiniCard
            icon={PlayCircle}
            label={t("dashboard.task.activeJobs")}
            value={formatNumber(data.cron.activeJobs)}
            colorClass="text-green-500"
            bgClass="bg-green-500/10"
          />
          <MiniCard
            icon={CheckCircle2}
            label={t("dashboard.task.successRuns")}
            value={formatNumber(data.cron.successRuns)}
            colorClass="text-emerald-500"
            bgClass="bg-emerald-500/10"
          />
          <MiniCard
            icon={XCircle}
            label={t("dashboard.task.failedRuns")}
            value={formatNumber(data.cron.failedRuns)}
            colorClass="text-red-500"
            bgClass="bg-red-500/10"
          />
          <MiniCard
            icon={Timer}
            label={t("dashboard.task.avgDuration")}
            value={formatDuration(data.cron.avgDurationMs)}
            colorClass="text-amber-500"
            bgClass="bg-amber-500/10"
          />
        </div>

        {/* Success rate donut */}
        <div className="flex items-center justify-center">
          <div className="relative">
            <ResponsiveContainer width={180} height={180}>
              <PieChart>
                <Pie
                  data={cronPieData}
                  cx="50%"
                  cy="50%"
                  innerRadius={55}
                  outerRadius={80}
                  dataKey="value"
                  startAngle={90}
                  endAngle={-270}
                >
                  {cronPieData.map((entry, i) => (
                    <Cell key={i} fill={entry.color} />
                  ))}
                </Pie>
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    fontSize: "12px",
                  }}
                  formatter={(value: number) => [formatNumber(value)]}
                />
              </PieChart>
            </ResponsiveContainer>
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
              <div className="text-center">
                <div className="text-lg font-bold">{cronSuccessRate}%</div>
                <div className="text-[10px] text-muted-foreground">
                  {t("dashboard.task.successRate")}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Sub-agent stats */}
      <div className="bg-card border rounded-xl p-4 space-y-4">
        <h3 className="text-sm font-medium">
          {t("dashboard.task.subagentTitle")}
        </h3>

        <div className="grid grid-cols-2 gap-2">
          <MiniCard
            icon={Bot}
            label={t("dashboard.task.totalRuns")}
            value={formatNumber(data.subagent.totalRuns)}
            colorClass="text-indigo-500"
            bgClass="bg-indigo-500/10"
          />
          <MiniCard
            icon={CheckCircle2}
            label={t("dashboard.task.completed")}
            value={formatNumber(data.subagent.completed)}
            colorClass="text-green-500"
            bgClass="bg-green-500/10"
          />
          <MiniCard
            icon={XCircle}
            label={t("dashboard.task.failed")}
            value={formatNumber(data.subagent.failed)}
            colorClass="text-red-500"
            bgClass="bg-red-500/10"
          />
          <MiniCard
            icon={Skull}
            label={t("dashboard.task.killed")}
            value={formatNumber(data.subagent.killed)}
            colorClass="text-amber-500"
            bgClass="bg-amber-500/10"
          />
          <MiniCard
            icon={Coins}
            label={t("dashboard.task.tokens")}
            value={formatNumber(
              data.subagent.totalInputTokens +
                data.subagent.totalOutputTokens,
            )}
            colorClass="text-purple-500"
            bgClass="bg-purple-500/10"
          />
          <MiniCard
            icon={Timer}
            label={t("dashboard.task.avgDuration")}
            value={formatDuration(data.subagent.avgDurationMs)}
            colorClass="text-cyan-500"
            bgClass="bg-cyan-500/10"
          />
        </div>

        {/* Completion donut */}
        <div className="flex items-center justify-center">
          <div className="relative">
            <ResponsiveContainer width={180} height={180}>
              <PieChart>
                <Pie
                  data={subagentPieData}
                  cx="50%"
                  cy="50%"
                  innerRadius={55}
                  outerRadius={80}
                  dataKey="value"
                  startAngle={90}
                  endAngle={-270}
                >
                  {subagentPieData.map((entry, i) => (
                    <Cell key={i} fill={entry.color} />
                  ))}
                </Pie>
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    fontSize: "12px",
                  }}
                  formatter={(value: number) => [formatNumber(value)]}
                />
              </PieChart>
            </ResponsiveContainer>
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
              <div className="text-center">
                <div className="text-lg font-bold">
                  {subagentCompletionRate}%
                </div>
                <div className="text-[10px] text-muted-foreground">
                  {t("dashboard.task.completionRate")}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
})

export default TaskSection
