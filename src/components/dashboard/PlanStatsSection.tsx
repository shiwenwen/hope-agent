import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  BarChart,
  Bar,
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
} from "recharts"
import { ClipboardList, CheckCircle2, Timer, Activity } from "lucide-react"
import { cn } from "@/lib/utils"
import type { PlanStats } from "./types"
import { chartName, chartNumber, formatNumber } from "./types"

interface PlanStatsSectionProps {
  data: PlanStats | null
  loading: boolean
  agentNameMap?: Record<string, string>
  onDrillDownAgent?: (agentId: string) => void
}

export default function PlanStatsSection({
  data,
  loading,
  agentNameMap,
  onDrillDownAgent,
}: PlanStatsSectionProps) {
  const { t } = useTranslation()

  const stateBuckets = useMemo(() => {
    if (!data) return []
    const d = data.stateDistribution
    return [
      { key: "planning", label: t("plans.badge.planning"), value: d.planning, color: "#f59e0b" },
      { key: "review", label: t("plans.badge.review"), value: d.review, color: "#a855f7" },
      { key: "executing", label: t("plans.badge.executing"), value: d.executing, color: "#3b82f6" },
      { key: "completed", label: t("plans.badge.completed"), value: d.completed, color: "#10b981" },
      { key: "archived", label: t("plans.badge.archived"), value: d.off, color: "#64748b" },
    ]
  }, [data, t])

  if (loading && !data) {
    return (
      <div className="py-12 flex items-center justify-center text-muted-foreground text-sm">
        {t("common.loading")}
      </div>
    )
  }

  if (!data || data.total === 0) {
    return (
      <div className="py-12 flex flex-col items-center justify-center text-muted-foreground text-sm">
        <ClipboardList className="h-8 w-8 mb-3 opacity-30" />
        <span>{t("dashboard.plans.empty")}</span>
      </div>
    )
  }

  const completionPct = Math.round(data.completionRate * 100)
  const avgDurationMinutes =
    data.avgExecutionDurationSecs != null ? data.avgExecutionDurationSecs / 60 : null

  return (
    <div className="space-y-4 mt-4">
      <div className="grid grid-cols-1 md:grid-cols-4 gap-3">
        <StatCard
          icon={<ClipboardList className="h-4 w-4" />}
          label={t("dashboard.plans.total")}
          value={formatNumber(data.total)}
        />
        <StatCard
          icon={<CheckCircle2 className="h-4 w-4" />}
          label={t("dashboard.plans.completionRate")}
          value={`${completionPct}%`}
          hint={`${formatNumber(data.stateDistribution.completed)} / ${formatNumber(data.total)}`}
        />
        <StatCard
          icon={<Activity className="h-4 w-4" />}
          label={t("dashboard.plans.active")}
          value={formatNumber(
            data.stateDistribution.planning +
              data.stateDistribution.review +
              data.stateDistribution.executing,
          )}
        />
        <StatCard
          icon={<Timer className="h-4 w-4" />}
          label={t("dashboard.plans.avgExecution")}
          value={
            avgDurationMinutes != null
              ? avgDurationMinutes >= 60
                ? `${(avgDurationMinutes / 60).toFixed(1)}h`
                : `${avgDurationMinutes.toFixed(1)}m`
              : "—"
          }
          hint={
            data.sampledDurationCount > 0
              ? t("dashboard.plans.sampleSize", { count: data.sampledDurationCount })
              : undefined
          }
        />
      </div>

      <div className="rounded-lg border border-border/70 bg-card p-4">
        <h3 className="text-sm font-medium mb-3">{t("dashboard.plans.stateDistribution")}</h3>
        <StateStackBar buckets={stateBuckets} total={data.total} />
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div className="rounded-lg border border-border/70 bg-card p-4">
          <h3 className="text-sm font-medium mb-3">{t("dashboard.plans.byAgent")}</h3>
          {data.byAgent.length === 0 ? (
            <EmptyChart text={t("dashboard.plans.empty")} />
          ) : (
            <ResponsiveContainer width="100%" height={220}>
              <BarChart
                data={data.byAgent.map((b) => ({
                  agent: agentNameMap?.[b.agentId] ?? b.agentId,
                  agentId: b.agentId,
                  total: b.total,
                  completed: b.completed,
                }))}
                layout="vertical"
                margin={{ top: 4, right: 16, left: 16, bottom: 0 }}
              >
                <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
                <XAxis type="number" stroke="hsl(var(--muted-foreground))" fontSize={11} />
                <YAxis
                  type="category"
                  dataKey="agent"
                  stroke="hsl(var(--muted-foreground))"
                  fontSize={11}
                  width={110}
                  tickFormatter={chartName}
                />
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--card))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "6px",
                    fontSize: "12px",
                  }}
                />
                <Bar
                  dataKey={(d: { total: number }) => chartNumber(d.total)}
                  fill="#3b82f6"
                  name={t("dashboard.plans.total")}
                  radius={[0, 4, 4, 0]}
                  onClick={(d) => {
                    // Recharts payload lives at `d.payload`; the row's row datum
                    // contains the agentId we attached in the mapper above.
                    const payload =
                      (d as { payload?: { agentId?: string } } | undefined)?.payload
                    if (onDrillDownAgent && payload?.agentId) {
                      onDrillDownAgent(payload.agentId)
                    }
                  }}
                />
                <Bar
                  dataKey={(d: { completed: number }) => chartNumber(d.completed)}
                  fill="#10b981"
                  name={t("dashboard.plans.byAgentCompleted")}
                  radius={[0, 4, 4, 0]}
                />
              </BarChart>
            </ResponsiveContainer>
          )}
        </div>

        <div className="rounded-lg border border-border/70 bg-card p-4">
          <h3 className="text-sm font-medium mb-3">{t("dashboard.plans.byProject")}</h3>
          {data.byProject.length === 0 ? (
            <EmptyChart text={t("dashboard.plans.empty")} />
          ) : (
            <div className="space-y-1.5 max-h-[220px] overflow-y-auto pr-1">
              {data.byProject.map((b) => {
                const pct = data.total > 0 ? (b.total / data.total) * 100 : 0
                return (
                  <div key={b.projectId ?? "__none"} className="flex items-center gap-2 text-xs">
                    <span className="w-32 truncate text-muted-foreground">
                      {b.projectId ?? t("dashboard.plans.noProject")}
                    </span>
                    <div className="flex-1 h-2 bg-secondary/50 rounded overflow-hidden">
                      <div
                        className="h-full bg-blue-500"
                        style={{ width: `${Math.min(100, pct)}%` }}
                      />
                    </div>
                    <span className="w-12 text-right tabular-nums">
                      {b.total}
                      {b.completed > 0 && (
                        <span className="text-green-600 ml-1">/{b.completed}</span>
                      )}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>
      </div>

      <div className="rounded-lg border border-border/70 bg-card p-4">
        <h3 className="text-sm font-medium mb-3">{t("dashboard.plans.creationTrend")}</h3>
        <ResponsiveContainer width="100%" height={200}>
          <LineChart
            data={data.creationTrend}
            margin={{ top: 8, right: 16, left: 0, bottom: 0 }}
          >
            <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
            <XAxis
              dataKey="date"
              stroke="hsl(var(--muted-foreground))"
              fontSize={10}
              tickFormatter={(v: string) => v.slice(5)}
            />
            <YAxis stroke="hsl(var(--muted-foreground))" fontSize={11} allowDecimals={false} />
            <RechartsTooltip
              contentStyle={{
                backgroundColor: "hsl(var(--card))",
                border: "1px solid hsl(var(--border))",
                borderRadius: "6px",
                fontSize: "12px",
              }}
            />
            <Line
              type="monotone"
              dataKey="created"
              stroke="#3b82f6"
              strokeWidth={2}
              dot={false}
              name={t("dashboard.plans.created")}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  )
}

function StatCard({
  icon,
  label,
  value,
  hint,
}: {
  icon: React.ReactNode
  label: string
  value: string
  hint?: string
}) {
  return (
    <div className="rounded-lg border border-border/70 bg-card p-3">
      <div className="flex items-center gap-2 text-xs text-muted-foreground mb-1">
        {icon}
        <span>{label}</span>
      </div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[11px] text-muted-foreground mt-0.5">{hint}</div>}
    </div>
  )
}

function StateStackBar({
  buckets,
  total,
}: {
  buckets: { key: string; label: string; value: number; color: string }[]
  total: number
}) {
  return (
    <div className="space-y-2">
      <div className="flex h-3 rounded overflow-hidden bg-secondary/30">
        {buckets
          .filter((b) => b.value > 0)
          .map((b) => (
            <div
              key={b.key}
              className="h-full"
              style={{
                width: `${(b.value / total) * 100}%`,
                backgroundColor: b.color,
              }}
              title={`${b.label}: ${b.value}`}
            />
          ))}
      </div>
      <div className="grid grid-cols-2 sm:grid-cols-5 gap-2 text-xs">
        {buckets.map((b) => (
          <div key={b.key} className={cn("flex items-center gap-1.5", b.value === 0 && "opacity-50")}>
            <span
              className="h-2 w-2 rounded-sm shrink-0"
              style={{ backgroundColor: b.color }}
            />
            <span className="text-muted-foreground">{b.label}</span>
            <span className="font-medium tabular-nums ml-auto">{b.value}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function EmptyChart({ text }: { text: string }) {
  return (
    <div className="h-[220px] flex items-center justify-center text-muted-foreground text-xs">
      {text}
    </div>
  )
}
