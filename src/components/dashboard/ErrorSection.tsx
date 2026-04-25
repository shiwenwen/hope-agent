import React from "react"
import { useTranslation } from "react-i18next"
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  BarChart,
  Bar,
  Legend,
} from "recharts"
import { AlertTriangle, AlertOctagon, Percent } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DashboardErrorData } from "./types"
import { chartName, chartNumber, formatNumber } from "./types"

interface ErrorSectionProps {
  data: DashboardErrorData | null
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

function SummaryCard({
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
    <div className="bg-card border rounded-xl p-4 flex items-center gap-3">
      <div
        className={cn(
          "h-9 w-9 rounded-full flex items-center justify-center shrink-0",
          bgClass,
        )}
      >
        <Icon className={cn("h-4.5 w-4.5", colorClass)} />
      </div>
      <div className="min-w-0">
        <div className="text-xl font-bold truncate">{value}</div>
        <div className="text-xs text-muted-foreground truncate">{label}</div>
      </div>
    </div>
  )
}

const ErrorSection = React.memo(function ErrorSection({
  data,
  loading,
}: ErrorSectionProps) {
  const { t } = useTranslation()

  if (loading && !data) {
    return (
      <div className="space-y-6 mt-4">
        <div className="grid grid-cols-3 gap-4">
          <SectionSkeleton height={80} />
          <SectionSkeleton height={80} />
          <SectionSkeleton height={80} />
        </div>
        <SectionSkeleton height={300} />
        <SectionSkeleton height={300} />
      </div>
    )
  }

  if (!data) return null

  const totalEvents = data.totalErrors + data.totalWarnings
  const errorRate =
    totalEvents > 0
      ? ((data.totalErrors / totalEvents) * 100).toFixed(1)
      : "0.0"

  const sortedCategories = [...data.byCategory].sort(
    (a, b) => b.count - a.count,
  )

  return (
    <div className="space-y-6 mt-4">
      {/* Summary cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <SummaryCard
          icon={AlertOctagon}
          label={t("dashboard.error.totalErrors")}
          value={formatNumber(data.totalErrors)}
          colorClass="text-red-500"
          bgClass="bg-red-500/10"
        />
        <SummaryCard
          icon={AlertTriangle}
          label={t("dashboard.error.totalWarnings")}
          value={formatNumber(data.totalWarnings)}
          colorClass="text-amber-500"
          bgClass="bg-amber-500/10"
        />
        <SummaryCard
          icon={Percent}
          label={t("dashboard.error.errorRate")}
          value={`${errorRate}%`}
          colorClass="text-orange-500"
          bgClass="bg-orange-500/10"
        />
      </div>

      {/* Error/warn trend area chart */}
      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">
          {t("dashboard.error.trend")}
        </h3>
        {data.trend.length === 0 ? (
          <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <ResponsiveContainer width="100%" height={300}>
            <AreaChart data={data.trend}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                dataKey="date"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
              />
              <YAxis
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
                tickFormatter={(v: number) => formatNumber(v)}
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "var(--color-popover)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                  fontSize: "12px",
                color: "var(--color-popover-foreground)",
                }}
                formatter={(value, name) => [
                  formatNumber(chartNumber(value)),
                  chartName(name) === "errorCount"
                    ? t("dashboard.error.errors")
                    : t("dashboard.error.warnings"),
                ]}
              />
              <Legend
                formatter={(value: string) =>
                  value === "errorCount"
                    ? t("dashboard.error.errors")
                    : t("dashboard.error.warnings")
                }
              />
              <Area
                type="monotone"
                dataKey="errorCount"
                stackId="1"
                stroke="#ef4444"
                fill="#ef4444"
                fillOpacity={0.3}
              />
              <Area
                type="monotone"
                dataKey="warnCount"
                stackId="1"
                stroke="#f59e0b"
                fill="#f59e0b"
                fillOpacity={0.3}
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>

      {/* Category bar chart */}
      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">
          {t("dashboard.error.byCategory")}
        </h3>
        {sortedCategories.length === 0 ? (
          <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <ResponsiveContainer
            width="100%"
            height={Math.max(200, sortedCategories.length * 32)}
          >
            <BarChart
              data={sortedCategories}
              layout="vertical"
              margin={{ left: 100 }}
            >
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                type="number"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
                tickFormatter={(v: number) => formatNumber(v)}
              />
              <YAxis
                type="category"
                dataKey="category"
                tick={{ fontSize: 11 }}
                width={100}
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
                formatter={(value) => [
                  formatNumber(chartNumber(value)),
                  t("dashboard.error.count"),
                ]}
              />
              <Bar dataKey="count" fill="#ef4444" radius={[0, 4, 4, 0]} fillOpacity={0.8} />
            </BarChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  )
})

export default ErrorSection
