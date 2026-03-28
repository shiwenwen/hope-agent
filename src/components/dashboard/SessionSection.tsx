import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
  Legend,
} from "recharts"
import type { DashboardSessionData } from "./types"
import { formatNumber } from "./types"

const PIE_COLORS = [
  "#6366f1",
  "#10b981",
  "#f59e0b",
  "#ef4444",
  "#8b5cf6",
  "#06b6d4",
  "#ec4899",
  "#14b8a6",
  "#f97316",
]

interface SessionSectionProps {
  data: DashboardSessionData | null
  loading: boolean
  agentNameMap: Record<string, string>
  onDrillDown: (agentId: string) => void
}

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

const SessionSection = React.memo(function SessionSection({
  data,
  loading,
  agentNameMap,
  onDrillDown,
}: SessionSectionProps) {
  const resolveAgent = (id: string) => agentNameMap[id] || id
  const { t } = useTranslation()

  const pieData = useMemo(() => {
    if (!data?.byAgent) return []
    return [...data.byAgent]
      .sort((a, b) => b.sessionCount - a.sessionCount)
      .slice(0, 9)
      .map((a) => ({
        name: resolveAgent(a.agentId),
        agentId: a.agentId,
        value: a.sessionCount,
      }))
  }, [data?.byAgent, agentNameMap]) // eslint-disable-line react-hooks/exhaustive-deps

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
      {/* Trend line chart */}
      <div className="bg-card border rounded-xl p-4">
        <h3 className="text-sm font-medium mb-4">
          {t("dashboard.session.trend")}
        </h3>
        {data.trend.length === 0 ? (
          <div className="flex items-center justify-center h-[300px] text-sm text-muted-foreground">
            {t("dashboard.noData")}
          </div>
        ) : (
          <ResponsiveContainer width="100%" height={300}>
            <LineChart data={data.trend}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                dataKey="date"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
              />
              <YAxis
                yAxisId="left"
                tick={{ fontSize: 12 }}
                className="fill-muted-foreground"
                tickFormatter={(v: number) => formatNumber(v)}
              />
              <YAxis
                yAxisId="right"
                orientation="right"
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
                formatter={(value: number, name: string) => [
                  formatNumber(value),
                  name === "sessionCount"
                    ? t("dashboard.session.sessions")
                    : t("dashboard.session.messages"),
                ]}
              />
              <Legend
                formatter={(value: string) =>
                  value === "sessionCount"
                    ? t("dashboard.session.sessions")
                    : t("dashboard.session.messages")
                }
              />
              <Line
                yAxisId="left"
                type="monotone"
                dataKey="sessionCount"
                stroke="#6366f1"
                strokeWidth={2}
                dot={false}
              />
              <Line
                yAxisId="right"
                type="monotone"
                dataKey="messageCount"
                stroke="#10b981"
                strokeWidth={2}
                dot={false}
              />
            </LineChart>
          </ResponsiveContainer>
        )}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Agent distribution pie chart */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.session.byAgent")}
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
                  onClick={(entry) => onDrillDown(entry.agentId)}
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
                  formatter={(value: number) => [
                    formatNumber(value),
                    t("dashboard.session.sessions"),
                  ]}
                />
              </PieChart>
            </ResponsiveContainer>
          )}
        </div>

        {/* Agent ranking table */}
        <div className="bg-card border rounded-xl p-4">
          <h3 className="text-sm font-medium mb-4">
            {t("dashboard.session.agentRanking")}
          </h3>
          <div className="overflow-auto max-h-[300px]">
            <div className="grid grid-cols-4 gap-2 text-xs font-medium text-muted-foreground pb-2 border-b">
              <div>{t("dashboard.session.agent")}</div>
              <div className="text-right">
                {t("dashboard.session.sessions")}
              </div>
              <div className="text-right">
                {t("dashboard.session.messages")}
              </div>
              <div className="text-right">{t("dashboard.session.tokens")}</div>
            </div>
            {data.byAgent.length === 0 ? (
              <div className="py-8 text-center text-sm text-muted-foreground">
                {t("dashboard.noData")}
              </div>
            ) : (
              data.byAgent.map((agent) => (
                <div
                  key={agent.agentId}
                  className="grid grid-cols-4 gap-2 text-xs py-2 border-b border-border/50 hover:bg-muted/50 cursor-pointer"
                  onClick={() => onDrillDown(agent.agentId)}
                >
                  <div className="truncate font-medium">{resolveAgent(agent.agentId)}</div>
                  <div className="text-right">
                    {formatNumber(agent.sessionCount)}
                  </div>
                  <div className="text-right">
                    {formatNumber(agent.messageCount)}
                  </div>
                  <div className="text-right">
                    {formatNumber(agent.totalTokens)}
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  )
})

export default SessionSection
