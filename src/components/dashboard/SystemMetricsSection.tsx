import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  PieChart,
  Pie,
  Cell,
} from "recharts"
import {
  Cpu,
  MemoryStick,
  Network,
  HardDrive,
  Monitor,
  Server,
  Clock,
  ArrowDownToLine,
  ArrowUpFromLine,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { SystemMetrics } from "./types"
import { formatBytes, formatUptime } from "./types"

interface SystemMetricsSectionProps {
  data: SystemMetrics | null
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

function MetricCard({
  icon: Icon,
  label,
  value,
  subValue,
  colorClass,
  bgClass,
}: {
  icon: React.ElementType
  label: string
  value: string
  subValue?: string
  colorClass: string
  bgClass: string
}) {
  return (
    <div className="flex items-center gap-3 p-3 rounded-lg bg-muted/50">
      <div
        className={cn(
          "h-9 w-9 rounded-full flex items-center justify-center shrink-0",
          bgClass,
        )}
      >
        <Icon className={cn("h-4 w-4", colorClass)} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold truncate">{value}</div>
        <div className="text-[11px] text-muted-foreground truncate">
          {label}
        </div>
        {subValue && (
          <div className="text-[10px] text-muted-foreground/70 truncate">
            {subValue}
          </div>
        )}
      </div>
    </div>
  )
}

const MEM_USED_COLOR = "hsl(var(--chart-1))"
const MEM_AVAILABLE_COLOR = "hsl(var(--chart-2))"
const SWAP_USED_COLOR = "hsl(var(--chart-3))"
const SWAP_FREE_COLOR = "hsl(var(--chart-4))"

const CPU_COLORS = [
  "hsl(var(--chart-1))",
  "hsl(var(--chart-2))",
  "hsl(var(--chart-3))",
  "hsl(var(--chart-4))",
  "hsl(var(--chart-5))",
  "#10b981",
  "#6366f1",
  "#f59e0b",
  "#ec4899",
  "#8b5cf6",
  "#14b8a6",
  "#f97316",
]

const SystemMetricsSection = React.memo(function SystemMetricsSection({
  data,
  loading,
}: SystemMetricsSectionProps) {
  const { t } = useTranslation()

  const cpuBarData = useMemo(() => {
    if (!data) return []
    return data.cpuCores.map((core, i) => ({
      name: `${i}`,
      usage: Math.round(core.usagePercent * 10) / 10,
    }))
  }, [data])

  const memPieData = useMemo(() => {
    if (!data) return []
    return [
      {
        name: t("dashboard.system.memUsed"),
        value: data.memory.usedBytes,
        color: MEM_USED_COLOR,
      },
      {
        name: t("dashboard.system.memAvailable"),
        value: data.memory.availableBytes,
        color: MEM_AVAILABLE_COLOR,
      },
    ]
  }, [data, t])

  const swapPieData = useMemo(() => {
    if (!data || data.memory.swapTotalBytes === 0) return []
    return [
      {
        name: t("dashboard.system.swapUsed"),
        value: data.memory.swapUsedBytes,
        color: SWAP_USED_COLOR,
      },
      {
        name: t("dashboard.system.swapFree"),
        value: data.memory.swapTotalBytes - data.memory.swapUsedBytes,
        color: SWAP_FREE_COLOR,
      },
    ]
  }, [data, t])

  const networkBarData = useMemo(() => {
    if (!data) return []
    return data.networks.slice(0, 10).map((iface) => ({
      name: iface.name.length > 12 ? iface.name.slice(0, 12) + "..." : iface.name,
      fullName: iface.name,
      received: iface.receivedBytes,
      transmitted: iface.transmittedBytes,
    }))
  }, [data])

  if (loading && !data) {
    return (
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mt-4">
        <SectionSkeleton height={360} />
        <SectionSkeleton height={360} />
        <SectionSkeleton height={360} />
        <SectionSkeleton height={360} />
      </div>
    )
  }

  if (!data) return null

  return (
    <div className="space-y-6 mt-4">
      {/* System info cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <MetricCard
          icon={Monitor}
          label={t("dashboard.system.osName")}
          value={data.osName}
          colorClass="text-blue-500"
          bgClass="bg-blue-500/10"
        />
        <MetricCard
          icon={Server}
          label={t("dashboard.system.hostName")}
          value={data.hostName}
          colorClass="text-indigo-500"
          bgClass="bg-indigo-500/10"
        />
        <MetricCard
          icon={Clock}
          label={t("dashboard.system.uptime")}
          value={formatUptime(data.uptimeSecs)}
          colorClass="text-green-500"
          bgClass="bg-green-500/10"
        />
        <MetricCard
          icon={Cpu}
          label={t("dashboard.system.cpuCores")}
          value={`${data.cpuCount} ${t("dashboard.system.cores")}`}
          subValue={`${data.cpuGlobalUsage.toFixed(1)}% ${t("dashboard.system.usage")}`}
          colorClass="text-amber-500"
          bgClass="bg-amber-500/10"
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* CPU Usage per Core */}
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <Cpu className="h-4 w-4 text-amber-500" />
              {t("dashboard.system.cpuUsage")}
            </h3>
            <span className="text-sm font-semibold text-amber-500">
              {data.cpuGlobalUsage.toFixed(1)}%
            </span>
          </div>
          <ResponsiveContainer width="100%" height={240}>
            <BarChart data={cpuBarData} margin={{ left: -10 }}>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="hsl(var(--border))"
                vertical={false}
              />
              <XAxis
                dataKey="name"
                tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                axisLine={false}
                tickLine={false}
              />
              <YAxis
                domain={[0, 100]}
                tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                axisLine={false}
                tickLine={false}
                tickFormatter={(v) => `${v}%`}
              />
              <RechartsTooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--popover))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "8px",
                  fontSize: "12px",
                }}
                formatter={(value: number) => [`${value}%`, t("dashboard.system.usage")]}
                labelFormatter={(label) => `Core ${label}`}
              />
              <Bar dataKey="usage" radius={[3, 3, 0, 0]}>
                {cpuBarData.map((_, i) => (
                  <Cell
                    key={i}
                    fill={CPU_COLORS[i % CPU_COLORS.length]}
                  />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>

        {/* Memory Usage */}
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <MemoryStick className="h-4 w-4 text-purple-500" />
              {t("dashboard.system.memoryUsage")}
            </h3>
            <span className="text-sm font-semibold text-purple-500">
              {data.memory.usagePercent.toFixed(1)}%
            </span>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <MetricCard
              icon={HardDrive}
              label={t("dashboard.system.memTotal")}
              value={formatBytes(data.memory.totalBytes)}
              colorClass="text-blue-500"
              bgClass="bg-blue-500/10"
            />
            <MetricCard
              icon={HardDrive}
              label={t("dashboard.system.memUsed")}
              value={formatBytes(data.memory.usedBytes)}
              colorClass="text-red-500"
              bgClass="bg-red-500/10"
            />
          </div>

          {/* Memory pie */}
          <div className="flex items-center justify-center gap-6">
            <div className="relative">
              <ResponsiveContainer width={150} height={150}>
                <PieChart>
                  <Pie
                    data={memPieData}
                    cx="50%"
                    cy="50%"
                    innerRadius={45}
                    outerRadius={65}
                    dataKey="value"
                    startAngle={90}
                    endAngle={-270}
                  >
                    {memPieData.map((entry, i) => (
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
                    formatter={(value: number) => [formatBytes(value)]}
                  />
                </PieChart>
              </ResponsiveContainer>
              <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                <div className="text-center">
                  <div className="text-sm font-bold">
                    {data.memory.usagePercent.toFixed(1)}%
                  </div>
                  <div className="text-[9px] text-muted-foreground">RAM</div>
                </div>
              </div>
            </div>

            {swapPieData.length > 0 && (
              <div className="relative">
                <ResponsiveContainer width={150} height={150}>
                  <PieChart>
                    <Pie
                      data={swapPieData}
                      cx="50%"
                      cy="50%"
                      innerRadius={45}
                      outerRadius={65}
                      dataKey="value"
                      startAngle={90}
                      endAngle={-270}
                    >
                      {swapPieData.map((entry, i) => (
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
                      formatter={(value: number) => [formatBytes(value)]}
                    />
                  </PieChart>
                </ResponsiveContainer>
                <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                  <div className="text-center">
                    <div className="text-sm font-bold">
                      {data.memory.swapUsagePercent.toFixed(1)}%
                    </div>
                    <div className="text-[9px] text-muted-foreground">Swap</div>
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Legend */}
          <div className="flex justify-center gap-4 text-[11px] text-muted-foreground">
            <div className="flex items-center gap-1">
              <div className="w-2 h-2 rounded-full" style={{ backgroundColor: MEM_USED_COLOR }} />
              {t("dashboard.system.memUsed")}
            </div>
            <div className="flex items-center gap-1">
              <div className="w-2 h-2 rounded-full" style={{ backgroundColor: MEM_AVAILABLE_COLOR }} />
              {t("dashboard.system.memAvailable")}
            </div>
            {swapPieData.length > 0 && (
              <>
                <div className="flex items-center gap-1">
                  <div className="w-2 h-2 rounded-full" style={{ backgroundColor: SWAP_USED_COLOR }} />
                  {t("dashboard.system.swapUsed")}
                </div>
                <div className="flex items-center gap-1">
                  <div className="w-2 h-2 rounded-full" style={{ backgroundColor: SWAP_FREE_COLOR }} />
                  {t("dashboard.system.swapFree")}
                </div>
              </>
            )}
          </div>
        </div>

        {/* Network Traffic */}
        <div className="bg-card border rounded-xl p-4 space-y-3 lg:col-span-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <Network className="h-4 w-4 text-cyan-500" />
              {t("dashboard.system.networkTraffic")}
            </h3>
            <div className="flex items-center gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1">
                <ArrowDownToLine className="h-3 w-3 text-green-500" />
                {formatBytes(data.totalReceivedBytes)}
              </span>
              <span className="flex items-center gap-1">
                <ArrowUpFromLine className="h-3 w-3 text-blue-500" />
                {formatBytes(data.totalTransmittedBytes)}
              </span>
            </div>
          </div>

          {networkBarData.length > 0 ? (
            <ResponsiveContainer width="100%" height={280}>
              <BarChart
                data={networkBarData}
                layout="vertical"
                margin={{ left: 10, right: 20 }}
              >
                <CartesianGrid
                  strokeDasharray="3 3"
                  stroke="hsl(var(--border))"
                  horizontal={false}
                />
                <XAxis
                  type="number"
                  tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                  axisLine={false}
                  tickLine={false}
                  tickFormatter={(v) => formatBytes(v)}
                />
                <YAxis
                  type="category"
                  dataKey="name"
                  width={100}
                  tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
                  axisLine={false}
                  tickLine={false}
                />
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: "hsl(var(--popover))",
                    border: "1px solid hsl(var(--border))",
                    borderRadius: "8px",
                    fontSize: "12px",
                  }}
                  formatter={(value: number, name: string) => [
                    formatBytes(value),
                    name === "received"
                      ? t("dashboard.system.received")
                      : t("dashboard.system.transmitted"),
                  ]}
                  labelFormatter={(_label, payload) =>
                    payload?.[0]?.payload?.fullName ?? _label
                  }
                />
                <Bar
                  dataKey="received"
                  fill="hsl(var(--chart-2))"
                  name={t("dashboard.system.received")}
                  radius={[0, 3, 3, 0]}
                  barSize={14}
                />
                <Bar
                  dataKey="transmitted"
                  fill="hsl(var(--chart-1))"
                  name={t("dashboard.system.transmitted")}
                  radius={[0, 3, 3, 0]}
                  barSize={14}
                />
              </BarChart>
            </ResponsiveContainer>
          ) : (
            <div className="flex items-center justify-center h-40 text-sm text-muted-foreground">
              {t("dashboard.system.noNetworkData")}
            </div>
          )}

          {/* Network interface table */}
          {data.networks.length > 0 && (
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b text-muted-foreground">
                    <th className="text-left py-2 px-2 font-medium">
                      {t("dashboard.system.interface")}
                    </th>
                    <th className="text-right py-2 px-2 font-medium">
                      {t("dashboard.system.received")}
                    </th>
                    <th className="text-right py-2 px-2 font-medium">
                      {t("dashboard.system.transmitted")}
                    </th>
                    <th className="text-right py-2 px-2 font-medium">
                      {t("dashboard.system.total")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {data.networks.slice(0, 10).map((iface) => (
                    <tr key={iface.name} className="border-b border-border/50">
                      <td className="py-1.5 px-2 font-medium truncate max-w-[200px]">
                        {iface.name}
                      </td>
                      <td className="py-1.5 px-2 text-right text-green-600">
                        {formatBytes(iface.receivedBytes)}
                      </td>
                      <td className="py-1.5 px-2 text-right text-blue-600">
                        {formatBytes(iface.transmittedBytes)}
                      </td>
                      <td className="py-1.5 px-2 text-right">
                        {formatBytes(iface.receivedBytes + iface.transmittedBytes)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </div>
  )
})

export default SystemMetricsSection
