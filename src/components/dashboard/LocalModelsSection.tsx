import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  AreaChart,
  Area,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip as RechartsTooltip,
  ResponsiveContainer,
  Cell,
} from "recharts"
import {
  Activity,
  AlertCircle,
  Cpu,
  Gauge,
  HardDrive,
  Layers,
  MemoryStick,
  Monitor,
  PackageOpen,
  Server,
  Timer,
  Zap,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { cn } from "@/lib/utils"
import MetricCard from "@/components/common/MetricCard"
import type { SettingsSection } from "@/components/settings/types"
import type {
  HardwareInfo,
  LocalOllamaModel,
  OllamaStatus,
} from "@/types/local-llm"
import type { LocalModelJobSnapshot } from "@/types/local-model-jobs"
import {
  isLocalModelJobActive,
  isLocalModelJobVisible,
  localModelJobPercent,
} from "@/types/local-model-jobs"
import type { DashboardLocalModelUsage } from "./types"
import { chartName, chartNumber, formatDashboardBytes, formatNumber } from "./types"

interface LocalModelsSectionProps {
  loading: boolean
  ollama: OllamaStatus | null
  ollamaVersion: string | null
  hardware: HardwareInfo | null
  models: LocalOllamaModel[] | null
  usage: DashboardLocalModelUsage | null
  jobs: LocalModelJobSnapshot[] | null
  onOpenSettings?: (section?: SettingsSection) => void
}

function mbToBytes(mb: number): number {
  return mb * 1024 * 1024
}

function SectionSkeleton({ height }: { height: number }) {
  return (
    <div
      className="w-full bg-muted animate-pulse rounded-lg"
      style={{ height }}
    />
  )
}

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n))
}

interface BadgeProps {
  children: React.ReactNode
  className?: string
}

function Badge({ children, className }: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-1.5 py-0.5 text-[10px] font-medium",
        className,
      )}
    >
      {children}
    </span>
  )
}

/**
 * Parse `expires_at` (RFC3339) into a human-friendly "在 5 分钟后卸载" string.
 * Returns null when the timestamp is missing, malformed, or already past.
 */
function formatExpiresIn(
  expiresAt: string | null | undefined,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string | null {
  if (!expiresAt) return null
  const ts = Date.parse(expiresAt)
  if (!Number.isFinite(ts)) return null
  const deltaSec = Math.round((ts - Date.now()) / 1000)
  if (deltaSec <= 0) return t("dashboard.localModels.running.unloadingSoon")
  if (deltaSec < 60)
    return t("dashboard.localModels.running.expiresIn", {
      duration: `${deltaSec}s`,
    })
  if (deltaSec < 3600)
    return t("dashboard.localModels.running.expiresIn", {
      duration: `${Math.floor(deltaSec / 60)}m`,
    })
  return t("dashboard.localModels.running.expiresIn", {
    duration: `${Math.floor(deltaSec / 3600)}h${Math.floor((deltaSec % 3600) / 60)}m`,
  })
}

const PHASE_COLORS: Record<string, { text: string; bg: string }> = {
  running: { text: "text-emerald-600", bg: "bg-emerald-500/10" },
  installed: { text: "text-amber-600", bg: "bg-amber-500/10" },
  "not-installed": { text: "text-red-600", bg: "bg-red-500/10" },
}

const LocalModelsSection = React.memo(function LocalModelsSection({
  loading,
  ollama,
  ollamaVersion,
  hardware,
  models,
  usage,
  jobs,
  onOpenSettings,
}: LocalModelsSectionProps) {
  const { t } = useTranslation()

  const runningModels = useMemo(
    () => (models ?? []).filter((m) => m.running),
    [models],
  )
  const installedModels = models ?? []
  const visibleJobs = useMemo(
    () => (jobs ?? []).filter(isLocalModelJobVisible),
    [jobs],
  )

  const totalVramBytes = useMemo(
    () =>
      runningModels.reduce((sum, m) => sum + (m.sizeVramBytes ?? 0), 0),
    [runningModels],
  )
  const budgetBytes = hardware ? mbToBytes(hardware.budgetMb) : 0
  const vramPct = budgetBytes > 0
    ? clamp((totalVramBytes / budgetBytes) * 100, 0, 100)
    : 0

  const trendData = useMemo(
    () =>
      (usage?.trend ?? []).map((p) => ({
        date: p.date,
        inputTokens: p.inputTokens,
        outputTokens: p.outputTokens,
      })),
    [usage],
  )

  const byModelData = useMemo(
    () =>
      (usage?.byModel ?? []).map((row) => ({
        modelId: row.modelId,
        providerName: row.providerName,
        totalTokens: row.inputTokens + row.outputTokens,
        callCount: row.callCount,
        errorRate:
          row.callCount > 0 ? (row.errorCount / row.callCount) * 100 : 0,
        errorCount: row.errorCount,
        avgTtftMs: row.avgTtftMs,
      })),
    [usage],
  )

  if (loading && !ollama && !hardware && !models && !usage) {
    return (
      <div className="space-y-4 mt-4">
        <SectionSkeleton height={64} />
        <SectionSkeleton height={120} />
        <SectionSkeleton height={240} />
        <SectionSkeleton height={300} />
      </div>
    )
  }

  const phase = ollama?.phase ?? "not-installed"
  const phaseColor = PHASE_COLORS[phase] ?? PHASE_COLORS["not-installed"]
  const phaseLabel = t(`dashboard.localModels.ollamaPhases.${phase}`)

  return (
    <div className="space-y-4 mt-4">
      {/* Block A — Ollama status bar */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3">
        <div className="flex items-center gap-3 min-w-0">
          <div
            className={`h-8 w-8 rounded-full flex items-center justify-center shrink-0 ${phaseColor.bg}`}
          >
            <Server className={`h-4 w-4 ${phaseColor.text}`} />
          </div>
          <div className="min-w-0">
            <div className="flex items-baseline gap-2 flex-wrap">
              <span className="text-sm font-semibold">
                {t("dashboard.localModels.ollamaStatus")}
              </span>
              <span className={`text-sm font-medium ${phaseColor.text}`}>
                {phaseLabel}
              </span>
              {ollamaVersion && (
                <span className="text-xs text-muted-foreground">
                  v{ollamaVersion}
                </span>
              )}
            </div>
            {ollama?.baseUrl && (
              <div className="text-[11px] text-muted-foreground truncate">
                {ollama.baseUrl}
              </div>
            )}
          </div>
        </div>
        <Button
          size="sm"
          variant="outline"
          onClick={() => onOpenSettings?.("modelConfig")}
        >
          {t("dashboard.localModels.openInSettings")}
        </Button>
      </div>

      {/* Block B — Hardware budget */}
      {hardware && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
          <MetricCard
            icon={MemoryStick}
            label={t("dashboard.localModels.hardware.totalMemory")}
            value={formatDashboardBytes(mbToBytes(hardware.totalMemoryMb))}
            subValue={`${t("dashboard.localModels.hardware.available")}: ${formatDashboardBytes(
              mbToBytes(hardware.availableMemoryMb),
            )}`}
            colorClass="text-purple-500"
            bgClass="bg-purple-500/10"
          />
          <MetricCard
            icon={Monitor}
            label={t("dashboard.localModels.hardware.gpu")}
            value={hardware.gpu?.name ?? t("dashboard.localModels.hardware.integratedGpu")}
            subValue={
              hardware.gpu?.vramMb != null
                ? `${t("dashboard.localModels.hardware.vram")}: ${formatDashboardBytes(mbToBytes(hardware.gpu.vramMb))}`
                : undefined
            }
            colorClass="text-blue-500"
            bgClass="bg-blue-500/10"
          />
          <MetricCard
            icon={Gauge}
            label={t("dashboard.localModels.hardware.budget")}
            value={formatDashboardBytes(mbToBytes(hardware.budgetMb))}
            subValue={t(
              `dashboard.localModels.hardware.budgetSource.${hardware.budgetSource}`,
            )}
            colorClass="text-emerald-500"
            bgClass="bg-emerald-500/10"
          />
          <MetricCard
            icon={Layers}
            label={t("dashboard.localModels.installed.title")}
            value={formatNumber(installedModels.length)}
            subValue={
              runningModels.length > 0
                ? t("dashboard.localModels.running.summary", {
                    count: runningModels.length,
                  })
                : undefined
            }
            colorClass="text-amber-500"
            bgClass="bg-amber-500/10"
          />
        </div>
      )}

      {/* Block C — Running models (read-only) */}
      <div className="rounded-xl border bg-card p-4">
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-sm font-medium flex items-center gap-2">
            <Cpu className="h-4 w-4 text-emerald-500" />
            {t("dashboard.localModels.running.title")}
          </h3>
          {runningModels.length > 0 && budgetBytes > 0 && (
            <span className="text-xs text-muted-foreground tabular-nums">
              {formatDashboardBytes(totalVramBytes)} /{" "}
              {formatDashboardBytes(budgetBytes)}
            </span>
          )}
        </div>

        {runningModels.length === 0 ? (
          <div className="py-6 flex flex-col items-center text-muted-foreground text-sm">
            <Cpu className="h-7 w-7 mb-2 opacity-30" />
            <span>{t("dashboard.localModels.running.empty")}</span>
          </div>
        ) : (
          <div className="space-y-3">
            {budgetBytes > 0 && (
              <Progress value={vramPct} className="h-2" />
            )}
            <div className="space-y-2">
              {runningModels.map((m) => {
                const expires = formatExpiresIn(m.expiresAt, t)
                return (
                  <div
                    key={m.id}
                    className="flex items-center justify-between gap-3 rounded-lg bg-muted/40 px-3 py-2 text-sm"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="font-medium truncate">{m.name}</div>
                      <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
                        {m.sizeVramBytes != null && (
                          <span className="tabular-nums">
                            {t("dashboard.localModels.running.vramUsed")}:{" "}
                            {formatDashboardBytes(m.sizeVramBytes)}
                          </span>
                        )}
                        {m.contextWindow != null && (
                          <span className="tabular-nums">
                            {t("dashboard.localModels.running.context")}:{" "}
                            {formatNumber(m.contextWindow)}
                          </span>
                        )}
                        {expires && (
                          <span className="flex items-center gap-1">
                            <Timer className="h-3 w-3" />
                            {expires}
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                )
              })}
            </div>
          </div>
        )}
      </div>

      {/* Block D — Installed models (read-only grid) */}
      {installedModels.length > 0 && (
        <div className="rounded-xl border bg-card p-4">
          <h3 className="text-sm font-medium flex items-center gap-2 mb-3">
            <PackageOpen className="h-4 w-4 text-blue-500" />
            {t("dashboard.localModels.installed.title")}
          </h3>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {installedModels.map((m) => (
              <div
                key={m.id}
                className="rounded-lg border bg-background/40 p-3 space-y-2"
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="font-medium text-sm truncate">{m.name}</div>
                    <div className="text-[11px] text-muted-foreground truncate">
                      {m.details?.family ??
                        t("dashboard.localModels.installed.unknownFamily")}
                      {m.sizeBytes != null
                        ? ` · ${formatDashboardBytes(m.sizeBytes)}`
                        : ""}
                      {m.contextWindow != null
                        ? ` · ctx ${formatNumber(m.contextWindow)}`
                        : ""}
                    </div>
                  </div>
                  {m.usage.running && (
                    <Badge className="bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/20">
                      <Activity className="h-3 w-3 mr-1" />
                      {t("dashboard.localModels.installed.badges.running")}
                    </Badge>
                  )}
                </div>
                <div className="flex flex-wrap gap-1">
                  {m.usage.activeModel && (
                    <Badge className="bg-blue-500/10 text-blue-700 dark:text-blue-300 border-blue-500/20">
                      {t("dashboard.localModels.installed.badges.active")}
                    </Badge>
                  )}
                  {m.usage.fallbackModel && (
                    <Badge className="border-border text-muted-foreground">
                      {t("dashboard.localModels.installed.badges.fallback")}
                    </Badge>
                  )}
                  {m.usage.providerModel && !m.usage.activeModel && (
                    <Badge className="border-border text-muted-foreground">
                      {t("dashboard.localModels.installed.badges.provider")}
                    </Badge>
                  )}
                  {(m.usage.embeddingConfig || m.usage.embeddingModel) && (
                    <Badge className="border-border text-muted-foreground">
                      {t("dashboard.localModels.installed.badges.embedding")}
                    </Badge>
                  )}
                  {m.capabilities.map((cap) => (
                    <Badge
                      key={cap}
                      className="border-border text-muted-foreground"
                    >
                      {cap}
                    </Badge>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Block E — Usage statistics */}
      <div className="rounded-xl border bg-card p-4 space-y-4">
        <div className="flex items-center gap-2">
          <Zap className="h-4 w-4 text-violet-500" />
          <h3 className="text-sm font-medium">
            {t("dashboard.localModels.usage.title")}
          </h3>
        </div>

        {(() => {
          if (usage == null) return null
          if (usage.localProviderNames.length === 0) {
            return (
              <div className="py-8 flex flex-col items-center gap-2 text-muted-foreground text-sm">
                <AlertCircle className="h-7 w-7 opacity-30" />
                <span>{t("dashboard.localModels.empty.noProvider")}</span>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onOpenSettings?.("modelConfig")}
                >
                  {t("dashboard.localModels.empty.goToSettings")}
                </Button>
              </div>
            )
          }
          if (usage.totalCalls === 0) {
            return (
              <div className="py-8 flex flex-col items-center gap-2 text-muted-foreground text-sm">
                <Zap className="h-7 w-7 opacity-30" />
                <span>{t("dashboard.localModels.usage.empty")}</span>
              </div>
            )
          }
          return (
            <>
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
              <MetricCard
                icon={Zap}
                label={t("dashboard.localModels.usage.totalCalls")}
                value={formatNumber(usage.totalCalls)}
                colorClass="text-violet-500"
                bgClass="bg-violet-500/10"
              />
              <MetricCard
                icon={HardDrive}
                label={t("dashboard.localModels.usage.totalTokens")}
                value={formatNumber(
                  usage.totalInputTokens + usage.totalOutputTokens,
                )}
                subValue={`${formatNumber(usage.totalInputTokens)} in · ${formatNumber(usage.totalOutputTokens)} out`}
                colorClass="text-blue-500"
                bgClass="bg-blue-500/10"
              />
              <MetricCard
                icon={Timer}
                label={t("dashboard.localModels.usage.avgTtft")}
                value={
                  usage.avgTtftMs != null
                    ? `${Math.round(usage.avgTtftMs)}ms`
                    : "—"
                }
                colorClass="text-amber-500"
                bgClass="bg-amber-500/10"
              />
            </div>

            {trendData.length > 1 && (
              <div>
                <div className="text-xs text-muted-foreground mb-2">
                  {t("dashboard.localModels.usage.trend")}
                </div>
                <ResponsiveContainer width="100%" height={180}>
                  <AreaChart
                    data={trendData}
                    margin={{ top: 5, right: 16, left: 0, bottom: 0 }}
                  >
                    <defs>
                      <linearGradient id="localInGrad" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="#3b82f6" stopOpacity={0.45} />
                        <stop offset="100%" stopColor="#3b82f6" stopOpacity={0} />
                      </linearGradient>
                      <linearGradient id="localOutGrad" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="#a855f7" stopOpacity={0.45} />
                        <stop offset="100%" stopColor="#a855f7" stopOpacity={0} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" />
                    <XAxis
                      dataKey="date"
                      tick={{ fontSize: 10 }}
                      stroke="var(--color-muted-foreground)"
                      tickFormatter={(v: string) => v.slice(5)}
                    />
                    <YAxis
                      tick={{ fontSize: 10 }}
                      stroke="var(--color-muted-foreground)"
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
                        chartName(name) === "inputTokens"
                          ? t("dashboard.token.inputTokens")
                          : t("dashboard.token.outputTokens"),
                      ]}
                    />
                    <Area
                      type="monotone"
                      dataKey="inputTokens"
                      stroke="#3b82f6"
                      strokeWidth={2}
                      fill="url(#localInGrad)"
                    />
                    <Area
                      type="monotone"
                      dataKey="outputTokens"
                      stroke="#a855f7"
                      strokeWidth={2}
                      fill="url(#localOutGrad)"
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            )}

            {byModelData.length > 0 && (
              <div>
                <div className="text-xs text-muted-foreground mb-2">
                  {t("dashboard.localModels.usage.byModel")}
                </div>
                <ResponsiveContainer width="100%" height={Math.max(160, byModelData.length * 36)}>
                  <BarChart
                    data={byModelData}
                    layout="vertical"
                    margin={{ top: 4, right: 16, left: 16, bottom: 0 }}
                  >
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" />
                    <XAxis
                      type="number"
                      tick={{ fontSize: 10 }}
                      stroke="var(--color-muted-foreground)"
                      tickFormatter={(v: number) => formatNumber(v)}
                    />
                    <YAxis
                      type="category"
                      dataKey="modelId"
                      tick={{ fontSize: 11 }}
                      stroke="var(--color-muted-foreground)"
                      width={140}
                    />
                    <RechartsTooltip
                      contentStyle={{
                        backgroundColor: "var(--color-popover)",
                        border: "1px solid var(--color-border)",
                        borderRadius: "8px",
                        fontSize: "12px",
                        color: "var(--color-popover-foreground)",
                      }}
                      formatter={(value) => [formatNumber(chartNumber(value))]}
                    />
                    <Bar dataKey="totalTokens" radius={[0, 4, 4, 0]}>
                      {byModelData.map((row, idx) => (
                        <Cell
                          key={`${row.modelId}-${idx}`}
                          fill={row.errorRate > 5 ? "#ef4444" : "#8b5cf6"}
                        />
                      ))}
                    </Bar>
                  </BarChart>
                </ResponsiveContainer>
                <div className="mt-2 space-y-1">
                  {byModelData
                    .filter((row) => row.errorRate > 5)
                    .map((row) => (
                      <div
                        key={`err-${row.modelId}`}
                        className="text-[11px] text-red-600 dark:text-red-400 flex items-center gap-1"
                      >
                        <AlertCircle className="h-3 w-3" />
                        {t("dashboard.localModels.usage.errorBadge", {
                          model: row.modelId,
                          rate: row.errorRate.toFixed(1),
                          errors: row.errorCount,
                          calls: row.callCount,
                        })}
                      </div>
                    ))}
                </div>
              </div>
            )}
            </>
          )
        })()}
      </div>

      {/* Block F — Background jobs */}
      {visibleJobs.length > 0 && (
        <div className="rounded-xl border bg-card p-4">
          <h3 className="text-sm font-medium flex items-center gap-2 mb-3">
            <Activity className="h-4 w-4 text-amber-500" />
            {t("dashboard.localModels.jobs.title")}
          </h3>
          <div className="space-y-2">
            {visibleJobs.map((job) => {
              const pct = localModelJobPercent(job)
              const active = isLocalModelJobActive(job)
              return (
                <div
                  key={job.jobId}
                  className="rounded-lg bg-muted/40 px-3 py-2 text-sm space-y-1.5"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <div className="font-medium truncate">
                        {job.displayName || job.modelId}
                      </div>
                      <div className="text-[11px] text-muted-foreground truncate">
                        {t(`dashboard.localModels.jobs.kind.${job.kind}`, {
                          defaultValue: job.kind,
                        })}
                        {" · "}
                        {job.phase}
                        {!active && (
                          <span className="ml-1">({job.status})</span>
                        )}
                      </div>
                    </div>
                    {pct != null && (
                      <span className="text-[11px] tabular-nums text-muted-foreground shrink-0">
                        {pct.toFixed(0)}%
                      </span>
                    )}
                  </div>
                  {pct != null && (
                    <Progress value={pct} className="h-1.5" />
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
})

export default LocalModelsSection
