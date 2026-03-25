import React, { useState, useMemo, useCallback } from "react"
import { useTranslation } from "react-i18next"
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
} from "recharts"
import { ChevronDown, ChevronUp, ArrowUpDown } from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import type { ToolUsageStats } from "./types"
import { formatNumber, formatDuration } from "./types"

interface ToolUsageSectionProps {
  data: ToolUsageStats[] | null
  loading: boolean
}

type SortKey = "toolName" | "callCount" | "errorCount" | "avgDurationMs" | "totalDurationMs"
type SortDir = "asc" | "desc"

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

const ToolUsageSection = React.memo(function ToolUsageSection({
  data,
  loading,
}: ToolUsageSectionProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [sortKey, setSortKey] = useState<SortKey>("callCount")
  const [sortDir, setSortDir] = useState<SortDir>("desc")

  const frequencyData = useMemo(() => {
    if (!data) return []
    return [...data]
      .sort((a, b) => b.callCount - a.callCount)
      .slice(0, 15)
  }, [data])

  const durationData = useMemo(() => {
    if (!data) return []
    return [...data]
      .filter((d) => d.avgDurationMs > 0)
      .sort((a, b) => b.avgDurationMs - a.avgDurationMs)
      .slice(0, 15)
  }, [data])

  const sortedData = useMemo(() => {
    if (!data) return []
    const sorted = [...data].sort((a, b) => {
      const aVal = a[sortKey]
      const bVal = b[sortKey]
      if (typeof aVal === "string" && typeof bVal === "string") {
        return sortDir === "asc" ? aVal.localeCompare(bVal) : bVal.localeCompare(aVal)
      }
      return sortDir === "asc"
        ? (aVal as number) - (bVal as number)
        : (bVal as number) - (aVal as number)
    })
    return sorted
  }, [data, sortKey, sortDir])

  const handleSort = useCallback(
    (key: SortKey) => {
      if (sortKey === key) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"))
      } else {
        setSortKey(key)
        setSortDir("desc")
      }
    },
    [sortKey],
  )

  if (loading && !data) {
    return (
      <div className="space-y-6 mt-4">
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <SectionSkeleton height={400} />
          <SectionSkeleton height={400} />
        </div>
        <SectionSkeleton height={200} />
      </div>
    )
  }

  if (!data || data.length === 0) {
    return (
      <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground mt-4">
        {t("dashboard.noData")}
      </div>
    )
  }

  const SortIcon = ({ column }: { column: SortKey }) => {
    if (sortKey !== column) return <ArrowUpDown className="h-3 w-3 ml-1 opacity-40" />
    return sortDir === "asc" ? (
      <ChevronUp className="h-3 w-3 ml-1" />
    ) : (
      <ChevronDown className="h-3 w-3 ml-1" />
    )
  }

  return (
    <div className="space-y-6 mt-4">
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Tool frequency bar chart */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.tool.frequency")}
          </h3>
          <ResponsiveContainer width="100%" height={Math.max(300, frequencyData.length * 28)}>
            <BarChart data={frequencyData} layout="vertical" margin={{ left: 80 }}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                type="number"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
                tickFormatter={(v: number) => formatNumber(v)}
              />
              <YAxis
                type="category"
                dataKey="toolName"
                tick={{ fontSize: 11 }}
                width={80}
                className="fill-muted-foreground"
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--popover))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "8px",
                  fontSize: "12px",
                }}
                formatter={(value: number, name: string) => [
                  formatNumber(value),
                  name === "callCount"
                    ? t("dashboard.tool.calls")
                    : t("dashboard.tool.errors"),
                ]}
              />
              <Bar dataKey="callCount" fill="#3b82f6" radius={[0, 4, 4, 0]} />
              <Bar dataKey="errorCount" fill="#ef4444" radius={[0, 4, 4, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>

        {/* Avg duration bar chart */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.tool.avgDuration")}
          </h3>
          {durationData.length === 0 ? (
            <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
              {t("dashboard.noData")}
            </div>
          ) : (
            <ResponsiveContainer width="100%" height={Math.max(300, durationData.length * 28)}>
              <BarChart data={durationData} layout="vertical" margin={{ left: 80 }}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
                <XAxis
                  type="number"
                  tick={{ fontSize: 12 }}
                  className="fill-muted-foreground"
                  tickFormatter={(v: number) => formatDuration(v)}
                />
                <YAxis
                  type="category"
                  dataKey="toolName"
                  tick={{ fontSize: 11 }}
                  width={80}
                  className="fill-muted-foreground"
                />
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    fontSize: "12px",
                  }}
                  formatter={(value: number) => [
                    formatDuration(value),
                    t("dashboard.tool.avgDuration"),
                  ]}
                />
                <Bar dataKey="avgDurationMs" fill="#f59e0b" radius={[0, 4, 4, 0]} />
              </BarChart>
            </ResponsiveContainer>
          )}
        </div>
      </div>

      {/* Expandable details table */}
      <div className="bg-card border rounded-xl p-4">
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-medium">{t("dashboard.tool.details")}</h3>
          <Button
            variant="ghost"
            size="sm"
            className="text-xs h-7"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? t("dashboard.tool.collapse") : t("dashboard.tool.expand")}
            {expanded ? (
              <ChevronUp className="h-3 w-3 ml-1" />
            ) : (
              <ChevronDown className="h-3 w-3 ml-1" />
            )}
          </Button>
        </div>

        {expanded && (
          <div className="overflow-auto">
            {/* Header */}
            <div className="grid grid-cols-6 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b">
              <button
                className="flex items-center text-left"
                onClick={() => handleSort("toolName")}
              >
                {t("dashboard.tool.name")}
                <SortIcon column="toolName" />
              </button>
              <button
                className="flex items-center justify-end"
                onClick={() => handleSort("callCount")}
              >
                {t("dashboard.tool.calls")}
                <SortIcon column="callCount" />
              </button>
              <button
                className="flex items-center justify-end"
                onClick={() => handleSort("errorCount")}
              >
                {t("dashboard.tool.errors")}
                <SortIcon column="errorCount" />
              </button>
              <div className="text-right">{t("dashboard.tool.errorRate")}</div>
              <button
                className="flex items-center justify-end"
                onClick={() => handleSort("avgDurationMs")}
              >
                {t("dashboard.tool.avgMs")}
                <SortIcon column="avgDurationMs" />
              </button>
              <button
                className="flex items-center justify-end"
                onClick={() => handleSort("totalDurationMs")}
              >
                {t("dashboard.tool.totalMs")}
                <SortIcon column="totalDurationMs" />
              </button>
            </div>

            {/* Rows */}
            {sortedData.map((tool) => {
              const errorRate =
                tool.callCount > 0
                  ? ((tool.errorCount / tool.callCount) * 100).toFixed(1)
                  : "0.0"
              return (
                <div
                  key={tool.toolName}
                  className="grid grid-cols-6 gap-2 text-xs py-2 border-b border-border/50 hover:bg-muted/50"
                >
                  <div className="truncate font-medium">{tool.toolName}</div>
                  <div className="text-right">{formatNumber(tool.callCount)}</div>
                  <div
                    className={cn(
                      "text-right",
                      tool.errorCount > 0 && "text-red-500",
                    )}
                  >
                    {formatNumber(tool.errorCount)}
                  </div>
                  <div
                    className={cn(
                      "text-right",
                      parseFloat(errorRate) > 10 && "text-red-500",
                    )}
                  >
                    {errorRate}%
                  </div>
                  <div className="text-right">
                    {formatDuration(tool.avgDurationMs)}
                  </div>
                  <div className="text-right">
                    {formatDuration(tool.totalDurationMs)}
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
})

export default ToolUsageSection
