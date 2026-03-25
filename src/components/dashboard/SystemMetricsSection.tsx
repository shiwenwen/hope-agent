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
  HardDrive,
  Monitor,
  Server,
  Clock,
  Hash,
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

const MEM_RSS_COLOR = "#ef4444" // red-500
const MEM_FREE_COLOR = "#3b82f6" // blue-500

/** Get CPU usage color based on percentage */
function getCpuColor(percent: number): string {
  if (percent < 30) return "#10b981" // green
  if (percent < 60) return "#f59e0b" // amber
  if (percent < 80) return "#f97316" // orange
  return "#ef4444" // red
}

const SystemMetricsSection = React.memo(function SystemMetricsSection({
  data,
  loading,
}: SystemMetricsSectionProps) {
  const { t } = useTranslation()

  const cpuBarData = useMemo(() => {
    if (!data) return []
    // Normalize: process CPU can exceed 100% on multi-core
    const normalizedCpu = Math.min(data.processCpuPercent, data.cpuCount * 100)
    return [
      {
        name: "OpenComputer",
        usage: Math.round(normalizedCpu * 10) / 10,
        fill: getCpuColor(normalizedCpu / data.cpuCount),
      },
    ]
  }, [data])

  const memPieData = useMemo(() => {
    if (!data) return []
    return [
      {
        name: t("dashboard.system.processRss"),
        value: data.memory.rssBytes,
        color: MEM_RSS_COLOR,
      },
      {
        name: t("dashboard.system.systemFree"),
        value: Math.max(0, data.memory.systemTotalBytes - data.memory.rssBytes),
        color: MEM_FREE_COLOR,
      },
    ]
  }, [data, t])

  const diskBarData = useMemo(() => {
    if (!data) return []
    return [
      {
        name: t("dashboard.system.diskRead"),
        value: data.diskIo.readBytes,
        fill: "#3b82f6",
      },
      {
        name: t("dashboard.system.diskWrite"),
        value: data.diskIo.writtenBytes,
        fill: "#ef4444",
      },
    ]
  }, [data, t])

  if (loading && !data) {
    return (
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 mt-4">
        <SectionSkeleton height={300} />
        <SectionSkeleton height={300} />
      </div>
    )
  }

  if (!data) return null

  const normalizedCpu = Math.min(data.processCpuPercent, data.cpuCount * 100)

  return (
    <div className="space-y-6 mt-4">
      {/* Process & system info cards */}
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-3">
        <MetricCard
          icon={Hash}
          label={t("dashboard.system.pid")}
          value={`PID ${data.pid}`}
          colorClass="text-violet-500"
          bgClass="bg-violet-500/10"
        />
        <MetricCard
          icon={Clock}
          label={t("dashboard.system.processUptime")}
          value={formatUptime(data.processUptimeSecs)}
          colorClass="text-green-500"
          bgClass="bg-green-500/10"
        />
        <MetricCard
          icon={Cpu}
          label={t("dashboard.system.cpuUsage")}
          value={`${normalizedCpu.toFixed(1)}%`}
          subValue={`${data.cpuCount} ${t("dashboard.system.cores")}`}
          colorClass="text-amber-500"
          bgClass="bg-amber-500/10"
        />
        <MetricCard
          icon={MemoryStick}
          label={t("dashboard.system.memRss")}
          value={formatBytes(data.memory.rssBytes)}
          subValue={`${data.memory.rssPercent.toFixed(2)}% ${t("dashboard.system.ofSystem")}`}
          colorClass="text-purple-500"
          bgClass="bg-purple-500/10"
        />
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
          subValue={`${t("dashboard.system.systemUptime")}: ${formatUptime(data.systemUptimeSecs)}`}
          colorClass="text-indigo-500"
          bgClass="bg-indigo-500/10"
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* CPU Usage */}
        <div className="bg-card border rounded-xl p-4 space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <Cpu className="h-4 w-4 text-amber-500" />
              {t("dashboard.system.cpuUsage")}
            </h3>
            <span className="text-sm font-semibold" style={{ color: getCpuColor(normalizedCpu / data.cpuCount) }}>
              {normalizedCpu.toFixed(1)}%
            </span>
          </div>

          {/* Visual gauge bar */}
          <div className="space-y-2">
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span>0%</span>
              <span>{data.cpuCount * 100}% ({data.cpuCount} {t("dashboard.system.cores")})</span>
            </div>
            <div className="h-6 bg-muted rounded-full overflow-hidden">
              <div
                className="h-full rounded-full transition-all duration-500"
                style={{
                  width: `${Math.min((normalizedCpu / (data.cpuCount * 100)) * 100, 100)}%`,
                  backgroundColor: getCpuColor(normalizedCpu / data.cpuCount),
                }}
              />
            </div>
            <p className="text-xs text-muted-foreground text-center">
              {t("dashboard.system.cpuDesc", {
                percent: normalizedCpu.toFixed(1),
                cores: data.cpuCount,
              })}
            </p>
          </div>
        </div>

        {/* Memory Usage */}
        <div className="bg-card border rounded-xl p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <MemoryStick className="h-4 w-4 text-purple-500" />
              {t("dashboard.system.memoryUsage")}
            </h3>
            <span className="text-sm font-semibold text-purple-500">
              {formatBytes(data.memory.rssBytes)}
            </span>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <MetricCard
              icon={MemoryStick}
              label={t("dashboard.system.processRss")}
              value={formatBytes(data.memory.rssBytes)}
              colorClass="text-red-500"
              bgClass="bg-red-500/10"
            />
            <MetricCard
              icon={HardDrive}
              label={t("dashboard.system.virtualMem")}
              value={formatBytes(data.memory.virtualBytes)}
              colorClass="text-orange-500"
              bgClass="bg-orange-500/10"
            />
          </div>

          {/* Memory donut: RSS vs system total */}
          <div className="flex items-center justify-center">
            <div className="relative">
              <ResponsiveContainer width={180} height={180}>
                <PieChart>
                  <Pie
                    data={memPieData}
                    cx="50%"
                    cy="50%"
                    innerRadius={55}
                    outerRadius={80}
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
                      color: "hsl(var(--popover-foreground))",
                    }}
                    formatter={(value: number) => [formatBytes(value)]}
                  />
                </PieChart>
              </ResponsiveContainer>
              <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                <div className="text-center">
                  <div className="text-sm font-bold">
                    {data.memory.rssPercent.toFixed(2)}%
                  </div>
                  <div className="text-[9px] text-muted-foreground">
                    {t("dashboard.system.ofSystem")}
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div className="flex justify-center gap-4 text-[11px] text-muted-foreground">
            <div className="flex items-center gap-1">
              <div className="w-2 h-2 rounded-full" style={{ backgroundColor: MEM_RSS_COLOR }} />
              {t("dashboard.system.processRss")}
            </div>
            <div className="flex items-center gap-1">
              <div className="w-2 h-2 rounded-full" style={{ backgroundColor: MEM_FREE_COLOR }} />
              {t("dashboard.system.systemFree")}
            </div>
          </div>
          <p className="text-xs text-muted-foreground text-center">
            {t("dashboard.system.memTotal")}: {formatBytes(data.memory.systemTotalBytes)}
          </p>
        </div>

        {/* Disk I/O */}
        <div className="bg-card border rounded-xl p-4 space-y-3 lg:col-span-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <HardDrive className="h-4 w-4 text-cyan-500" />
              {t("dashboard.system.diskIO")}
            </h3>
            <div className="flex items-center gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1">
                <ArrowDownToLine className="h-3 w-3 text-green-500" />
                {t("dashboard.system.diskRead")}: {formatBytes(data.diskIo.readBytes)}
              </span>
              <span className="flex items-center gap-1">
                <ArrowUpFromLine className="h-3 w-3 text-blue-500" />
                {t("dashboard.system.diskWrite")}: {formatBytes(data.diskIo.writtenBytes)}
              </span>
            </div>
          </div>

          <ResponsiveContainer width="100%" height={120}>
            <BarChart data={diskBarData} layout="vertical" margin={{ left: 10, right: 20 }}>
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
                width={60}
                tick={{ fontSize: 11, fill: "hsl(var(--muted-foreground))" }}
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
                formatter={(value: number) => [formatBytes(value)]}
              />
              <Bar dataKey="value" radius={[0, 4, 4, 0]} barSize={20}>
                {diskBarData.map((entry, i) => (
                  <Cell key={i} fill={entry.fill} />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>
    </div>
  )
})

export default SystemMetricsSection
