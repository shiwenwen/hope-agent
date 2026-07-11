import React, { useState, useMemo, useCallback } from "react"
import { useTranslation } from "react-i18next"
import {
  AreaChart,
  Area,
  LineChart,
  Line,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
} from "recharts"
import { ChevronDown, ChevronUp, ArrowUpDown } from "lucide-react"
import { Button } from "@/components/ui/button"
import type { DashboardTokenData } from "./types"
import { chartName, chartNumber, formatNumber, formatCost, formatDuration, humanizeDomain } from "./types"

type OperationSortKey =
  | "operation"
  | "domain"
  | "callCount"
  | "inputTokens"
  | "outputTokens"
  | "estimatedCostUsd"
type SortDir = "asc" | "desc"

function OperationSortIndicator({
  column,
  sortKey,
  sortDir,
}: {
  column: OperationSortKey
  sortKey: OperationSortKey
  sortDir: SortDir
}) {
  if (sortKey !== column) return <ArrowUpDown className="h-3 w-3 ml-1 opacity-40" />
  return sortDir === "asc" ? (
    <ChevronUp className="h-3 w-3 ml-1" />
  ) : (
    <ChevronDown className="h-3 w-3 ml-1" />
  )
}

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
  onDrillDownOperation: (operation: string) => void
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
  onDrillDownOperation,
}: TokenUsageSectionProps) {
  const { t } = useTranslation()
  const [operationsExpanded, setOperationsExpanded] = useState(false)
  const [selectedDomain, setSelectedDomain] = useState<string | null>(null)
  const [opSortKey, setOpSortKey] = useState<OperationSortKey>("inputTokens")
  const [opSortDir, setOpSortDir] = useState<SortDir>("desc")
  const [prevData, setPrevData] = useState(data)

  const domainLabel = useCallback(
    (domain: string) => t(`dashboard.operationDomain.${domain}`, humanizeDomain(domain)),
    [t],
  )

  // `selectedDomain` is a local drill-down filter that doesn't touch the
  // global `DashboardFilter` (per design, clicking a domain bar only filters
  // the operations table below it). But `data` itself DOES change whenever
  // an unrelated global filter changes and the parent refetches — if the new
  // dataset has no rows under the previously-selected domain, the table
  // would silently render "no data" for a scope that actually has usage.
  // Reset the local selection whenever the underlying dataset changes, using
  // React's "adjust state during render" pattern (not an effect) so the
  // reset lands in the same commit as the new data instead of a follow-up
  // render.
  if (data !== prevData) {
    setPrevData(data)
    setSelectedDomain(null)
  }

  const domainData = useMemo(() => {
    if (!data?.byDomain) return []
    return [...data.byDomain].sort(
      (a, b) => b.inputTokens + b.outputTokens - (a.inputTokens + a.outputTokens),
    )
  }, [data])

  const visibleOperations = useMemo(() => {
    const ops = data?.byOperation ?? []
    return selectedDomain ? ops.filter((o) => o.domain === selectedDomain) : ops
  }, [data, selectedDomain])

  const sortedOperations = useMemo(() => {
    const sorted = [...visibleOperations].sort((a, b) => {
      const aVal = a[opSortKey]
      const bVal = b[opSortKey]
      if (typeof aVal === "string" && typeof bVal === "string") {
        return opSortDir === "asc" ? aVal.localeCompare(bVal) : bVal.localeCompare(aVal)
      }
      return opSortDir === "asc"
        ? (aVal as number) - (bVal as number)
        : (bVal as number) - (aVal as number)
    })
    return sorted
  }, [visibleOperations, opSortKey, opSortDir])

  const handleOpSort = useCallback(
    (key: OperationSortKey) => {
      if (opSortKey === key) {
        setOpSortDir((d) => (d === "asc" ? "desc" : "asc"))
      } else {
        setOpSortKey(key)
        setOpSortDir("desc")
      }
    },
    [opSortKey],
  )

  const ttftData = !data?.trend ? [] : data.trend.filter((t) => t.avgTtftMs != null)

  const pieData = (() => {
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
  })()

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
                  backgroundColor: "var(--color-popover)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                  fontSize: "12px",
                color: "var(--color-popover-foreground)",
                }}
                labelStyle={{ color: "var(--color-foreground)" }}
                formatter={(value, name) => [
                  formatNumber(chartNumber(value)),
                  chartName(name) === "inputTokens"
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

      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">{t("dashboard.token.byType")}</h3>
        <div className="overflow-auto">
          <div className="grid grid-cols-8 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b min-w-[760px]">
            <div>{t("dashboard.token.type")}</div>
            <div className="text-right">{t("dashboard.token.calls")}</div>
            <div className="text-right">{t("dashboard.token.input")}</div>
            <div className="text-right">{t("dashboard.token.output")}</div>
            <div className="text-right">{t("dashboard.token.cacheWrite")}</div>
            <div className="text-right">{t("dashboard.token.cacheRead")}</div>
            <div className="text-right">{t("dashboard.token.cost")}</div>
            <div className="text-right">{t("dashboard.token.avgDuration")}</div>
          </div>
          {(data.byKind ?? []).length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              {t("dashboard.noData")}
            </div>
          ) : (
            data.byKind.map((row) => (
              <div
                key={row.kind}
                className="grid grid-cols-8 gap-2 text-xs py-2 border-b border-border/50 min-w-[760px]"
              >
                <div className="truncate font-medium">
                  {t(`dashboard.usageKind.${row.kind}`, row.kind)}
                </div>
                <div className="text-right">{formatNumber(row.callCount)}</div>
                <div className="text-right">{formatNumber(row.inputTokens)}</div>
                <div className="text-right">{formatNumber(row.outputTokens)}</div>
                <div className="text-right">{formatNumber(row.cacheCreationInputTokens)}</div>
                <div className="text-right">{formatNumber(row.cacheReadInputTokens)}</div>
                <div className="text-right">{formatCost(row.estimatedCostUsd)}</div>
                <div className="text-right">
                  {row.avgDurationMs != null ? formatDuration(row.avgDurationMs) : "-"}
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* Domain (coarse purpose) bar chart */}
      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">{t("dashboard.token.byDomain")}</h3>
        {domainData.length === 0 ? (
          <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <ResponsiveContainer width="100%" height={Math.max(300, domainData.length * 28)}>
            <BarChart
              data={domainData}
              layout="vertical"
              margin={{ left: 100 }}
              onClick={(e) => {
                const domain = (e as { activeLabel?: string } | null)?.activeLabel ?? null
                setSelectedDomain((prev) => (prev === domain ? null : domain))
              }}
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
                dataKey="domain"
                tick={{ fontSize: 11 }}
                width={100}
                className="fill-muted-foreground"
                tickFormatter={domainLabel}
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "var(--color-popover)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "8px",
                  fontSize: "12px",
                  color: "var(--color-popover-foreground)",
                }}
                labelFormatter={(name) => domainLabel(chartName(name))}
                formatter={(value, name) => [
                  formatNumber(chartNumber(value)),
                  chartName(name) === "inputTokens"
                    ? t("dashboard.token.input")
                    : t("dashboard.token.output"),
                ]}
              />
              <Bar dataKey="inputTokens" stackId="t" fill="#8b5cf6" className="cursor-pointer" />
              <Bar dataKey="outputTokens" stackId="t" fill="#06b6d4" className="cursor-pointer" />
            </BarChart>
          </ResponsiveContainer>
        )}
      </div>

      {/* Operation (purpose tag) drill-down table */}
      <div className="bg-card border rounded-xl p-4">
        <div className="flex items-center justify-between mb-3 gap-2">
          <h3 className="text-sm font-medium">
            {t("dashboard.token.byOperation")}
            {selectedDomain && (
              <button
                className="ml-2 text-xs font-normal text-primary hover:underline"
                onClick={() => setSelectedDomain(null)}
              >
                {domainLabel(selectedDomain)} ×
              </button>
            )}
          </h3>
          <Button
            variant="ghost"
            size="sm"
            className="text-xs h-7"
            onClick={() => setOperationsExpanded((v) => !v)}
          >
            {operationsExpanded ? t("dashboard.tool.collapse") : t("dashboard.tool.expand")}
            {operationsExpanded ? (
              <ChevronUp className="h-3 w-3 ml-1" />
            ) : (
              <ChevronDown className="h-3 w-3 ml-1" />
            )}
          </Button>
        </div>

        {operationsExpanded &&
          (sortedOperations.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              {t("dashboard.noData")}
            </div>
          ) : (
            <div className="overflow-auto">
              <div className="grid grid-cols-6 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b min-w-[640px]">
                <button
                  className="flex items-center text-left"
                  onClick={() => handleOpSort("operation")}
                >
                  {t("dashboard.token.operation")}
                  <OperationSortIndicator column="operation" sortKey={opSortKey} sortDir={opSortDir} />
                </button>
                <button
                  className="flex items-center text-left"
                  onClick={() => handleOpSort("domain")}
                >
                  {t("dashboard.token.domain")}
                  <OperationSortIndicator column="domain" sortKey={opSortKey} sortDir={opSortDir} />
                </button>
                <button
                  className="flex items-center justify-end"
                  onClick={() => handleOpSort("callCount")}
                >
                  {t("dashboard.tool.calls")}
                  <OperationSortIndicator column="callCount" sortKey={opSortKey} sortDir={opSortDir} />
                </button>
                <button
                  className="flex items-center justify-end"
                  onClick={() => handleOpSort("inputTokens")}
                >
                  {t("dashboard.token.input")}
                  <OperationSortIndicator column="inputTokens" sortKey={opSortKey} sortDir={opSortDir} />
                </button>
                <button
                  className="flex items-center justify-end"
                  onClick={() => handleOpSort("outputTokens")}
                >
                  {t("dashboard.token.output")}
                  <OperationSortIndicator column="outputTokens" sortKey={opSortKey} sortDir={opSortDir} />
                </button>
                <button
                  className="flex items-center justify-end"
                  onClick={() => handleOpSort("estimatedCostUsd")}
                >
                  {t("dashboard.token.cost")}
                  <OperationSortIndicator
                    column="estimatedCostUsd"
                    sortKey={opSortKey}
                    sortDir={opSortDir}
                  />
                </button>
              </div>
              {sortedOperations.map((row) => (
                <div
                  key={row.operation}
                  className="grid grid-cols-6 gap-2 text-xs py-2 border-b border-border/50 hover:bg-muted/50 min-w-[640px]"
                >
                  <button
                    className="truncate text-left font-mono hover:underline"
                    onClick={() => onDrillDownOperation(row.operation)}
                    title={row.operation}
                  >
                    {row.operation}
                  </button>
                  <div className="truncate text-muted-foreground">{domainLabel(row.domain)}</div>
                  <div className="text-right">{formatNumber(row.callCount)}</div>
                  <div className="text-right">{formatNumber(row.inputTokens)}</div>
                  <div className="text-right">{formatNumber(row.outputTokens)}</div>
                  <div className="text-right">{formatCost(row.estimatedCostUsd)}</div>
                </div>
              ))}
            </div>
          ))}
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
                    `${name} (${((percent ?? 0) * 100).toFixed(0)}%)`
                  }
                  labelLine={{ strokeWidth: 1 }}
                  onClick={(entry) => {
                    const name = chartName((entry as { name?: unknown }).name)
                    if (name && name !== t("dashboard.token.other")) {
                      onDrillDown(name)
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
                    backgroundColor: "var(--color-popover)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "8px",
                    fontSize: "12px",
                  color: "var(--color-popover-foreground)",
                  }}
                  formatter={(value) => [formatNumber(chartNumber(value)), t("dashboard.token.tokensUnit")]}
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
            <div className="grid grid-cols-6 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b">
              <div>{t("dashboard.token.model")}</div>
              <div>{t("dashboard.token.provider")}</div>
              <div className="text-right">{t("dashboard.token.input")}</div>
              <div className="text-right">{t("dashboard.token.output")}</div>
              <div className="text-right">{t("dashboard.token.cost")}</div>
              <div className="text-right">{t("dashboard.token.ttft")}</div>
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
                    className="grid grid-cols-6 gap-2 text-xs py-2 border-b border-border/50 hover:bg-muted/50"
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
                    <div className="text-right">
                      {m.avgTtftMs != null ? formatDuration(m.avgTtftMs) : "-"}
                    </div>
                  </div>
                ))}
                {/* Totals row */}
                <div className="grid grid-cols-6 gap-2 text-xs py-2 font-semibold">
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
                  <div />
                </div>
              </>
            )}
          </div>
        </div>
      </div>

      {/* TTFT trend chart */}
      {ttftData.length > 0 && (
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">{t("dashboard.token.ttftTrend")}</h3>
          <ResponsiveContainer width="100%" height={250}>
            <LineChart data={ttftData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                dataKey="date"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
              />
              <YAxis
                tick={{ fontSize: 12 }}
                tickFormatter={(v: number) => formatDuration(v)}
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
                labelStyle={{ color: "var(--color-foreground)" }}
                formatter={(value) => [
                  formatDuration(chartNumber(value)),
                  t("dashboard.token.avgTtft"),
                ]}
              />
              <Line
                type="monotone"
                dataKey="avgTtftMs"
                stroke="#eab308"
                strokeWidth={2}
                dot={{ r: 3 }}
                activeDot={{ r: 5 }}
              />
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
    </div>
  )
})

export default TokenUsageSection
