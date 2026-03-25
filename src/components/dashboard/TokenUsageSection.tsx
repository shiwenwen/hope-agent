import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
} from "recharts"
import { cn } from "@/lib/utils"
import type { DashboardTokenData } from "./types"
import { formatNumber, formatCost } from "./types"

const PIE_COLORS = [
  "#8b5cf6",
  "#06b6d4",
  "#f59e0b",
  "#10b981",
  "#ef4444",
  "#ec4899",
  "#6366f1",
  "#14b8a6",
  "#f97316",
]

interface TokenUsageSectionProps {
  data: DashboardTokenData | null
  loading: boolean
  onDrillDown: (modelId: string) => void
}

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

const TokenUsageSection = React.memo(function TokenUsageSection({
  data,
  loading,
  onDrillDown,
}: TokenUsageSectionProps) {
  const { t } = useTranslation()

  const pieData = useMemo(() => {
    if (!data?.byModel) return []
    const sorted = [...data.byModel].sort(
      (a, b) => b.inputTokens + b.outputTokens - (a.inputTokens + a.outputTokens),
    )
    const top8 = sorted.slice(0, 8)
    const rest = sorted.slice(8)
    const result = top8.map((m) => ({
      name: m.modelId,
      value: m.inputTokens + m.outputTokens,
    }))
    if (rest.length > 0) {
      result.push({
        name: t("dashboard.token.other"),
        value: rest.reduce((acc, m) => acc + m.inputTokens + m.outputTokens, 0),
      })
    }
    return result
  }, [data?.byModel, t])

  if (loading && !data) {
    return (
      <div className="space-y-6 mt-4">
        <SectionSkeleton height={300} />
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <SectionSkeleton height={300} />
          <SectionSkeleton height={300} />
        </div>
      </div>
    )
  }

  if (!data) return null

  return (
    <div className="space-y-6 mt-4">
      {/* Trend area chart */}
      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">{t("dashboard.token.trend")}</h3>
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
                tickFormatter={(v: number) => formatNumber(v)}
                className="fill-muted-foreground"
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--popover))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "8px",
                  fontSize: "12px",
                }}
                labelStyle={{ color: "hsl(var(--foreground))" }}
                formatter={(value: number, name: string) => [
                  formatNumber(value),
                  name === "inputTokens"
                    ? t("dashboard.token.input")
                    : t("dashboard.token.output"),
                ]}
              />
              <Area
                type="monotone"
                dataKey="inputTokens"
                stackId="1"
                stroke="#8b5cf6"
                fill="#8b5cf6"
                fillOpacity={0.3}
              />
              <Area
                type="monotone"
                dataKey="outputTokens"
                stackId="1"
                stroke="#06b6d4"
                fill="#06b6d4"
                fillOpacity={0.3}
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Model distribution pie chart */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.token.byModel")}
          </h3>
          {pieData.length === 0 ? (
            <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
              {t("dashboard.noData")}
            </div>
          ) : (
            <ResponsiveContainer width="100%" height={300}>
              <PieChart>
                <Pie
                  data={pieData}
                  cx="50%"
                  cy="50%"
                  outerRadius={100}
                  dataKey="value"
                  label={({ name, percent }) =>
                    `${name} (${(percent * 100).toFixed(0)}%)`
                  }
                  labelLine={{ strokeWidth: 1 }}
                  onClick={(entry) => {
                    if (entry.name !== t("dashboard.token.other")) {
                      onDrillDown(entry.name)
                    }
                  }}
                  className="cursor-pointer"
                >
                  {pieData.map((_, i) => (
                    <Cell
                      key={i}
                      fill={PIE_COLORS[i % PIE_COLORS.length]}
                      fillOpacity={0.8}
                    />
                  ))}
                </Pie>
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    fontSize: "12px",
                  }}
                  formatter={(value: number) => [formatNumber(value), "tokens"]}
                />
              </PieChart>
            </ResponsiveContainer>
          )}
        </div>

        {/* Cost table */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.token.costTable")}
          </h3>
          <div className="overflow-auto max-h-[300px]">
            <div className="grid grid-cols-5 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b">
              <div>{t("dashboard.token.model")}</div>
              <div>{t("dashboard.token.provider")}</div>
              <div className="text-right">{t("dashboard.token.input")}</div>
              <div className="text-right">{t("dashboard.token.output")}</div>
              <div className="text-right">{t("dashboard.token.cost")}</div>
            </div>
            {data.byModel.length === 0 ? (
              <div className="py-8 text-center text-sm text-muted-foreground">
                {t("dashboard.noData")}
              </div>
            ) : (
              <>
                {data.byModel.map((m) => (
                  <div
                    key={m.modelId}
                    className="grid grid-cols-5 gap-2 text-xs py-2 border-b border-border/50 hover:bg-muted/50"
                  >
                    <div className="truncate font-medium">{m.modelId}</div>
                    <div className="truncate text-muted-foreground">
                      {m.providerName}
                    </div>
                    <div className="text-right">{formatNumber(m.inputTokens)}</div>
                    <div className="text-right">
                      {formatNumber(m.outputTokens)}
                    </div>
                    <div className="text-right">
                      {formatCost(m.estimatedCostUsd)}
                    </div>
                  </div>
                ))}
                {/* Totals row */}
                <div
                  className={cn(
                    "grid grid-cols-5 gap-2 text-xs py-2 font-semibold",
                  )}
                >
                  <div>{t("dashboard.token.total")}</div>
                  <div />
                  <div className="text-right">
                    {formatNumber(
                      data.byModel.reduce((a, m) => a + m.inputTokens, 0),
                    )}
                  </div>
                  <div className="text-right">
                    {formatNumber(
                      data.byModel.reduce((a, m) => a + m.outputTokens, 0),
                    )}
                  </div>
                  <div className="text-right">
                    {formatCost(data.totalCostUsd)}
                  </div>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  )
})

export default TokenUsageSection
