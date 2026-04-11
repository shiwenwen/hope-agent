import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  LineChart,
  Line,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  ReferenceLine,
} from "recharts"
import {
  Activity,
  Flame,
  Clock4,
  TrendingUp,
  Trophy,
  Cpu,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { DashboardInsights } from "./types"
import { formatNumber, formatCost, formatDuration } from "./types"

interface InsightsSectionProps {
  data: DashboardInsights | null
  loading: boolean
  onDrillDownSession?: (sessionId: string) => void
  onDrillDownModel?: (modelId: string) => void
}

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

// ── Health Score Gauge ──────────────────────────────────────────

function HealthGauge({ score, status }: { score: number; status: string }) {
  const { t } = useTranslation()
  const clamped = Math.max(0, Math.min(100, score))
  // Ring: stroke-dasharray 251.2 circumference for r=40
  const circumference = 2 * Math.PI * 40
  const offset = circumference * (1 - clamped / 100)
  const color =
    status === "excellent"
      ? "#10b981"
      : status === "good"
        ? "#3b82f6"
        : status === "warning"
          ? "#f59e0b"
          : "#ef4444"
  return (
    <div className="flex flex-col items-center justify-center gap-2">
      <div className="relative h-[120px] w-[120px]">
        <svg className="h-full w-full -rotate-90" viewBox="0 0 100 100">
          <circle
            cx="50"
            cy="50"
            r="40"
            fill="none"
            className="stroke-muted"
            strokeWidth="8"
          />
          <circle
            cx="50"
            cy="50"
            r="40"
            fill="none"
            stroke={color}
            strokeWidth="8"
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            className="transition-[stroke-dashoffset] duration-700 ease-out"
          />
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <div className="text-2xl font-bold" style={{ color }}>
            {clamped}
          </div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
            {t(`dashboard.insights.status.${status}`)}
          </div>
        </div>
      </div>
    </div>
  )
}

// ── Activity Heatmap (7×24) ────────────────────────────────────

function Heatmap({
  cells,
  max,
}: {
  cells: { weekday: number; hour: number; messageCount: number }[]
  max: number
}) {
  const { t } = useTranslation()
  // Build a 7×24 grid
  const grid = useMemo(() => {
    const g: number[][] = Array.from({ length: 7 }, () => Array(24).fill(0))
    for (const c of cells) {
      if (c.weekday >= 0 && c.weekday <= 6 && c.hour >= 0 && c.hour <= 23) {
        g[c.weekday][c.hour] = c.messageCount
      }
    }
    return g
  }, [cells])

  const weekdayKeys = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
  const safeMax = Math.max(max, 1)

  function cellColor(v: number): string {
    if (v === 0) return "var(--color-muted)"
    const intensity = Math.min(1, v / safeMax)
    // interpolate from light to strong purple
    const alpha = 0.12 + 0.78 * intensity
    return `rgba(139, 92, 246, ${alpha.toFixed(3)})`
  }

  return (
    <div className="space-y-2">
      <div className="flex gap-1 pl-8">
        {Array.from({ length: 24 }).map((_, h) => (
          <div
            key={h}
            className="flex-1 text-center text-[9px] text-muted-foreground"
          >
            {h % 3 === 0 ? h : ""}
          </div>
        ))}
      </div>
      {grid.map((row, wd) => (
        <div key={wd} className="flex items-center gap-1">
          <div className="w-7 shrink-0 text-right text-[10px] text-muted-foreground">
            {t(`dashboard.insights.weekday.${weekdayKeys[wd]}`)}
          </div>
          <div className="flex flex-1 gap-1">
            {row.map((v, h) => (
              <div
                key={h}
                className="group relative flex-1 aspect-square rounded-sm border border-border/30 transition-transform hover:scale-110 hover:z-10"
                style={{ backgroundColor: cellColor(v) }}
              >
                <div className="pointer-events-none absolute bottom-full left-1/2 mb-1 hidden -translate-x-1/2 whitespace-nowrap rounded-md bg-popover px-2 py-1 text-[10px] text-popover-foreground shadow-md group-hover:block z-20">
                  {t(`dashboard.insights.weekday.${weekdayKeys[wd]}`)} {h.toString().padStart(2, "0")}:00 · {formatNumber(v)}
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
      <div className="flex items-center justify-end gap-2 pt-2 text-[10px] text-muted-foreground">
        <span>{t("dashboard.insights.less")}</span>
        {[0, 0.25, 0.5, 0.75, 1].map((i) => (
          <div
            key={i}
            className="h-3 w-3 rounded-sm border border-border/30"
            style={{ backgroundColor: cellColor(i * safeMax) }}
          />
        ))}
        <span>{t("dashboard.insights.more")}</span>
      </div>
    </div>
  )
}

// ── Main Section ────────────────────────────────────────────────

const InsightsSection = React.memo(function InsightsSection({
  data,
  loading,
  onDrillDownSession,
  onDrillDownModel,
}: InsightsSectionProps) {
  const { t } = useTranslation()

  if (loading && !data) {
    return (
      <div className="space-y-6 mt-4">
        <div className="grid grid-cols-1 lg:grid-cols-[320px_1fr] gap-6">
          <SectionSkeleton height={260} />
          <SectionSkeleton height={260} />
        </div>
        <SectionSkeleton height={280} />
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <SectionSkeleton height={280} />
          <SectionSkeleton height={280} />
        </div>
      </div>
    )
  }

  if (!data) return null

  const costChartData = data.costTrend.points.map((p) => ({
    date: p.date,
    cost: Number(p.costUsd.toFixed(4)),
  }))

  const hourlyData = data.hourly.buckets.map((b) => ({
    hour: `${b.hour.toString().padStart(2, "0")}:00`,
    messageCount: b.messageCount,
    sessionCount: b.sessionCount,
  }))

  return (
    <div className="space-y-6 mt-4">
      {/* Row 1: Health gauge + Key KPI stats */}
      <div className="grid grid-cols-1 lg:grid-cols-[320px_1fr] gap-6">
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <h3 className="text-sm font-medium flex items-center gap-2">
            <Activity className="h-4 w-4 text-emerald-500" />
            {t("dashboard.insights.healthScore")}
          </h3>
          <HealthGauge score={data.health.score} status={data.health.status} />
          <div className="grid grid-cols-2 gap-2 text-[11px]">
            <div className="rounded-md bg-muted/40 p-2">
              <div className="text-muted-foreground">
                {t("dashboard.insights.logErrorRate")}
              </div>
              <div className="font-semibold">
                {data.health.logErrorRatePercent.toFixed(2)}%
              </div>
            </div>
            <div className="rounded-md bg-muted/40 p-2">
              <div className="text-muted-foreground">
                {t("dashboard.insights.toolErrorRate")}
              </div>
              <div className="font-semibold">
                {data.health.toolErrorRatePercent.toFixed(2)}%
              </div>
            </div>
            <div className="rounded-md bg-muted/40 p-2">
              <div className="text-muted-foreground">
                {t("dashboard.insights.cronSuccessRate")}
              </div>
              <div className="font-semibold">
                {data.health.cronSuccessRatePercent.toFixed(1)}%
              </div>
            </div>
            <div className="rounded-md bg-muted/40 p-2">
              <div className="text-muted-foreground">
                {t("dashboard.insights.subagentSuccessRate")}
              </div>
              <div className="font-semibold">
                {data.health.subagentSuccessRatePercent.toFixed(1)}%
              </div>
            </div>
          </div>
        </div>

        {/* Cost trend chart */}
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between flex-wrap gap-2">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <TrendingUp className="h-4 w-4 text-amber-500" />
              {t("dashboard.insights.costTrend")}
            </h3>
            <div className="flex items-center gap-3 text-[11px] text-muted-foreground">
              <span>
                {t("dashboard.insights.total")}:{" "}
                <span className="font-semibold text-foreground">
                  {formatCost(data.costTrend.totalCostUsd)}
                </span>
              </span>
              <span>
                {t("dashboard.insights.avgDaily")}:{" "}
                <span className="font-semibold text-foreground">
                  {formatCost(data.costTrend.avgDailyCostUsd)}
                </span>
              </span>
              {data.costTrend.peakDay && (
                <span>
                  {t("dashboard.insights.peak")}:{" "}
                  <span className="font-semibold text-foreground">
                    {data.costTrend.peakDay} · {formatCost(data.costTrend.peakCostUsd)}
                  </span>
                </span>
              )}
            </div>
          </div>
          {costChartData.length === 0 ? (
            <div className="flex items-center justify-center h-[220px] text-sm text-muted-foreground">
              {t("dashboard.noData")}
            </div>
          ) : (
            <ResponsiveContainer width="100%" height={220}>
              <LineChart data={costChartData} margin={{ top: 10, right: 16, left: 0, bottom: 0 }}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
                <XAxis
                  dataKey="date"
                  tick={{ fontSize: 11 }}
                  className="fill-muted-foreground"
                />
                <YAxis
                  tick={{ fontSize: 11 }}
                  className="fill-muted-foreground"
                  tickFormatter={(v: number) => `$${v.toFixed(2)}`}
                />
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "var(--color-popover)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "8px",
                    fontSize: "12px",
                    color: "var(--color-popover-foreground)",
                  }}
                  formatter={(value: number) => [formatCost(value), t("dashboard.insights.cost")]}
                />
                <ReferenceLine
                  y={data.costTrend.avgDailyCostUsd}
                  stroke="#94a3b8"
                  strokeDasharray="4 4"
                  strokeWidth={1}
                />
                <Line
                  type="monotone"
                  dataKey="cost"
                  stroke="#f59e0b"
                  strokeWidth={2}
                  dot={{ r: 3 }}
                  activeDot={{ r: 5 }}
                />
              </LineChart>
            </ResponsiveContainer>
          )}
        </div>
      </div>

      {/* Row 2: Heatmap */}
      <div className="bg-card border rounded-xl p-4 space-y-3">
        <div className="flex items-center justify-between flex-wrap gap-2">
          <h3 className="text-sm font-medium flex items-center gap-2">
            <Flame className="h-4 w-4 text-rose-500" />
            {t("dashboard.insights.heatmap")}
          </h3>
          <span className="text-[11px] text-muted-foreground">
            {t("dashboard.insights.totalMessages")}:{" "}
            <span className="font-semibold text-foreground">{formatNumber(data.heatmap.total)}</span>
          </span>
        </div>
        {data.heatmap.total === 0 ? (
          <div className="flex items-center justify-center h-[220px] text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <Heatmap cells={data.heatmap.cells} max={data.heatmap.maxValue} />
        )}
      </div>

      {/* Row 3: Hourly distribution + Top sessions */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between flex-wrap gap-2">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <Clock4 className="h-4 w-4 text-indigo-500" />
              {t("dashboard.insights.hourly")}
            </h3>
            {data.hourly.peakHour != null && (
              <span className="text-[11px] text-muted-foreground">
                {t("dashboard.insights.peakHour")}:{" "}
                <span className="font-semibold text-foreground">
                  {data.hourly.peakHour.toString().padStart(2, "0")}:00
                </span>
              </span>
            )}
          </div>
          <ResponsiveContainer width="100%" height={240}>
            <BarChart data={hourlyData} margin={{ top: 10, right: 16, left: 0, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                dataKey="hour"
                tick={{ fontSize: 10 }}
                interval={2}
                className="fill-muted-foreground"
              />
              <YAxis
                tick={{ fontSize: 11 }}
                tickFormatter={(v: number) => formatNumber(v)}
                className="fill-muted-foreground"
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "var(--color-popover)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                  fontSize: "12px",
                  color: "var(--color-popover-foreground)",
                }}
                formatter={(value: number, name: string) => [
                  formatNumber(value),
                  name === "messageCount"
                    ? t("dashboard.insights.messages")
                    : t("dashboard.insights.sessions"),
                ]}
              />
              <Bar dataKey="messageCount" fill="#6366f1" radius={[4, 4, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>

        {/* Top sessions ranking */}
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <h3 className="text-sm font-medium flex items-center gap-2">
            <Trophy className="h-4 w-4 text-yellow-500" />
            {t("dashboard.insights.topSessions")}
          </h3>
          <div className="overflow-auto max-h-[260px]">
            {data.topSessions.length === 0 ? (
              <div className="py-8 text-center text-sm text-muted-foreground">
                {t("dashboard.noData")}
              </div>
            ) : (
              <div className="space-y-1.5">
                {data.topSessions.map((s, idx) => {
                  const rankColor =
                    idx === 0
                      ? "bg-yellow-500 text-white"
                      : idx === 1
                        ? "bg-gray-400 text-white"
                        : idx === 2
                          ? "bg-orange-500 text-white"
                          : "bg-muted text-muted-foreground"
                  return (
                    <div
                      key={s.id}
                      className={cn(
                        "group flex items-center gap-2 rounded-md p-2 text-xs",
                        onDrillDownSession ? "cursor-pointer hover:bg-muted/50" : "",
                      )}
                      onClick={() => onDrillDownSession?.(s.id)}
                    >
                      <div
                        className={cn(
                          "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[10px] font-bold",
                          rankColor,
                        )}
                      >
                        {idx + 1}
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-medium">
                          {s.title || s.id.slice(0, 8)}
                        </div>
                        <div className="truncate text-[10px] text-muted-foreground">
                          {s.modelId ?? "unknown"} · {formatNumber(s.messageCount)}{" "}
                          {t("dashboard.insights.msgs")}
                        </div>
                      </div>
                      <div className="text-right shrink-0">
                        <div className="font-semibold">{formatNumber(s.totalTokens)}</div>
                        <div className="text-[10px] text-muted-foreground">
                          {formatCost(s.estimatedCostUsd)}
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Row 4: Model efficiency */}
      <div className="bg-card border rounded-xl p-4 space-y-3">
        <h3 className="text-sm font-medium flex items-center gap-2">
          <Cpu className="h-4 w-4 text-cyan-500" />
          {t("dashboard.insights.modelEfficiency")}
        </h3>
        {data.modelEfficiency.length === 0 ? (
          <div className="py-8 text-center text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <div className="overflow-auto">
            <div className="grid grid-cols-[1fr_100px_100px_110px_110px_90px] gap-2 border-b pb-2 text-xs font-medium text-muted-foreground">
              <div>{t("dashboard.insights.model")}</div>
              <div className="text-right">{t("dashboard.insights.messages")}</div>
              <div className="text-right">{t("dashboard.insights.tokens")}</div>
              <div className="text-right">{t("dashboard.insights.tokensPerMsg")}</div>
              <div className="text-right">{t("dashboard.insights.costPer1k")}</div>
              <div className="text-right">{t("dashboard.insights.ttft")}</div>
            </div>
            {data.modelEfficiency.map((m) => (
              <div
                key={`${m.modelId}-${m.providerName}`}
                className={cn(
                  "grid grid-cols-[1fr_100px_100px_110px_110px_90px] gap-2 py-2 text-xs border-b border-border/40",
                  onDrillDownModel ? "cursor-pointer hover:bg-muted/40" : "",
                )}
                onClick={() => onDrillDownModel?.(m.modelId)}
              >
                <div className="min-w-0">
                  <div className="truncate font-medium">{m.modelId}</div>
                  <div className="truncate text-[10px] text-muted-foreground">
                    {m.providerName}
                  </div>
                </div>
                <div className="text-right">{formatNumber(m.messageCount)}</div>
                <div className="text-right">{formatNumber(m.totalTokens)}</div>
                <div className="text-right">{m.avgTokensPerMessage.toFixed(0)}</div>
                <div className="text-right">
                  ${m.avgCostPer1kTokens.toFixed(4)}
                </div>
                <div className="text-right">
                  {m.avgTtftMs != null ? formatDuration(m.avgTtftMs) : "-"}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
})

export default InsightsSection
