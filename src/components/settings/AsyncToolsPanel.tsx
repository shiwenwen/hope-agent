import { useCallback, useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

interface AsyncToolsConfig {
  enabled: boolean
  autoBackgroundSecs: number
  maxJobSecs: number
  maxConcurrentJobs: number
  maxConcurrentJobsPerSession: number
  retryEnabled: boolean
  maxRetryAttempts: number
  completionMergeWindowSecs: number
  outputTailBytes: number
  maxQueuedJobs: number
  wakeupMaxDelaySecs: number
  wakeupMaxPendingPerSession: number
  inlineResultBytes: number
  retentionSecs: number
  orphanGraceSecs: number
  jobStatusMaxWaitSecs: number
}

const DEFAULT_CONFIG: AsyncToolsConfig = {
  enabled: true,
  autoBackgroundSecs: 30,
  maxJobSecs: 0,
  maxConcurrentJobs: 8,
  maxConcurrentJobsPerSession: 6,
  retryEnabled: false,
  maxRetryAttempts: 3,
  completionMergeWindowSecs: 3,
  outputTailBytes: 8192,
  maxQueuedJobs: 256,
  wakeupMaxDelaySecs: 86400,
  wakeupMaxPendingPerSession: 5,
  inlineResultBytes: 4096,
  retentionSecs: 30 * 86400,
  orphanGraceSecs: 24 * 3600,
  jobStatusMaxWaitSecs: 7200,
}

export default function AsyncToolsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<AsyncToolsConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [loaded, setLoaded] = useState(false)

  const persist = useCallback(async (next: AsyncToolsConfig) => {
    try {
      await getTransport().call("save_async_tools_config", { config: next })
      setSavedSnapshot(JSON.stringify(next))
    } catch (e) {
      logger.error("settings", "AsyncToolsPanel::save", "Failed to save async tools config", e)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<AsyncToolsConfig>("get_async_tools_config")
      .then((cfg) => {
        if (cancelled) return
        const merged = { ...DEFAULT_CONFIG, ...cfg }
        setConfig(merged)
        setSavedSnapshot(JSON.stringify(merged))
        setLoaded(true)
      })
      .catch((e: unknown) => {
        logger.error("settings", "AsyncToolsPanel::load", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const commitIfChanged = useCallback(
    (next: AsyncToolsConfig) => {
      if (JSON.stringify(next) !== savedSnapshot) {
        void persist(next)
      }
    },
    [persist, savedSnapshot],
  )

  const handleEnabledChange = (enabled: boolean) => {
    const next = { ...config, enabled }
    setConfig(next)
    commitIfChanged(next)
  }

  const handleRetryEnabledChange = (retryEnabled: boolean) => {
    const next = { ...config, retryEnabled }
    setConfig(next)
    commitIfChanged(next)
  }

  type NumericKey =
    | "autoBackgroundSecs"
    | "maxJobSecs"
    | "maxConcurrentJobs"
    | "maxConcurrentJobsPerSession"
    | "maxRetryAttempts"
    | "completionMergeWindowSecs"
    | "outputTailBytes"
    | "maxQueuedJobs"
    | "wakeupMaxDelaySecs"
    | "wakeupMaxPendingPerSession"
    | "inlineResultBytes"
    | "retentionSecs"
    | "orphanGraceSecs"
    | "jobStatusMaxWaitSecs"

  const updateNumber = (key: NumericKey, min: number) => (raw: number) => {
    const clamped = Number.isFinite(raw) ? Math.max(min, Math.round(raw)) : min
    setConfig((prev) => ({ ...prev, [key]: clamped }))
  }

  const commitNumber = (key: NumericKey, min: number) => () => {
    setConfig((prev) => {
      const clamped = Math.max(min, Math.round(prev[key]))
      const next = { ...prev, [key]: clamped }
      commitIfChanged(next)
      return next
    })
  }

  if (!loaded) return null

  const disabled = !config.enabled
  const disabledCls = disabled ? "opacity-50 pointer-events-none" : ""

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <p className="text-xs text-muted-foreground px-3 mb-4">{t("settings.asyncToolsDesc")}</p>

      <div className="space-y-6">
        {/* Master switch */}
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.asyncToolsEnabled")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.asyncToolsEnabledDesc")}
            </div>
          </div>
          <Switch checked={config.enabled} onCheckedChange={handleEnabledChange} />
        </div>

        <div className={cn("space-y-6", disabledCls)}>
          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsAutoBackground")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsAutoBackgroundDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={10}
                value={config.autoBackgroundSecs}
                onChange={(e) =>
                  updateNumber("autoBackgroundSecs", 0)(Number(e.target.value))
                }
                onBlur={commitNumber("autoBackgroundSecs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.seconds")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.asyncToolsMaxJob")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsMaxJobDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={60}
                value={config.maxJobSecs}
                onChange={(e) => updateNumber("maxJobSecs", 0)(Number(e.target.value))}
                onBlur={commitNumber("maxJobSecs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.seconds")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsMergeWindow", "完成合并窗口")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsMergeWindowDesc",
                  "同会话多个后台任务在此窗口内完成时合并为一轮注入（省计费 turn）；0 关闭。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1}
                value={config.completionMergeWindowSecs}
                onChange={(e) =>
                  updateNumber("completionMergeWindowSecs", 0)(Number(e.target.value))
                }
                onBlur={commitNumber("completionMergeWindowSecs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.seconds")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsMaxConcurrent")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsMaxConcurrentDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1}
                value={config.maxConcurrentJobs}
                onChange={(e) =>
                  updateNumber("maxConcurrentJobs", 0)(Number(e.target.value))
                }
                onBlur={commitNumber("maxConcurrentJobs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsMaxConcurrentPerSession", "每会话并发上限")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsMaxConcurrentPerSessionDesc",
                  "单个会话（或 IM 群聊）最多同时运行的后台任务数；超出的请求即使全局仍有空位也会排队，避免单个会话占满所有槽位。0 表示不限制。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1}
                value={config.maxConcurrentJobsPerSession}
                onChange={(e) =>
                  updateNumber("maxConcurrentJobsPerSession", 0)(Number(e.target.value))
                }
                onBlur={commitNumber("maxConcurrentJobsPerSession", 0)}
                className="w-24 h-8 text-sm text-right"
              />
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsMaxQueued", "排队上限")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsMaxQueuedDesc",
                  "并发槽位占满后，新的后台任务进入等待队列的最大长度；超出则直接拒绝（让模型稍后重试或同步运行）。每个排队任务会占用内存，故有上限。范围 1–4096。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={4096}
                step={1}
                value={config.maxQueuedJobs}
                onChange={(e) => updateNumber("maxQueuedJobs", 1)(Number(e.target.value))}
                onBlur={commitNumber("maxQueuedJobs", 1)}
                className="w-24 h-8 text-sm text-right"
              />
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsRetryEnabled", "失败自动重试")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsRetryEnabledDesc",
                  "后台任务因网络抖动等瞬时错误失败时自动退避重试。仅对无副作用、可重入的工具（如联网搜索）生效；exec、图像生成等有副作用 / 计费的工具永不自动重试。",
                )}
              </div>
            </div>
            <Switch checked={config.retryEnabled} onCheckedChange={handleRetryEnabledChange} />
          </div>

          <div
            className={cn(
              "flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors",
              !config.retryEnabled && "opacity-50 pointer-events-none",
            )}
          >
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsMaxRetryAttempts", "最大尝试次数")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsMaxRetryAttemptsDesc",
                  "含首次执行的总尝试次数（1 = 不重试）。重试间隔按 500ms 起指数退避。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={10}
                step={1}
                value={config.maxRetryAttempts}
                onChange={(e) =>
                  updateNumber("maxRetryAttempts", 1)(Number(e.target.value))
                }
                onBlur={commitNumber("maxRetryAttempts", 1)}
                className="w-24 h-8 text-sm text-right"
              />
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsJobStatusMaxWait")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsJobStatusMaxWaitDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                step={60}
                value={config.jobStatusMaxWaitSecs}
                onChange={(e) =>
                  updateNumber("jobStatusMaxWaitSecs", 1)(Number(e.target.value))
                }
                onBlur={commitNumber("jobStatusMaxWaitSecs", 1)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.seconds")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.asyncToolsRetention")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsRetentionDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1}
                value={Math.round(config.retentionSecs / 86400)}
                onChange={(e) => {
                  const days = Number(e.target.value)
                  updateNumber("retentionSecs", 0)(
                    Number.isFinite(days) ? Math.max(0, Math.round(days)) * 86400 : 0,
                  )
                }}
                onBlur={commitNumber("retentionSecs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.days")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.asyncToolsOrphanGrace")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsOrphanGraceDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1}
                value={Math.round(config.orphanGraceSecs / 3600)}
                onChange={(e) => {
                  const hours = Number(e.target.value)
                  updateNumber("orphanGraceSecs", 0)(
                    Number.isFinite(hours) ? Math.max(0, Math.round(hours)) * 3600 : 0,
                  )
                }}
                onBlur={commitNumber("orphanGraceSecs", 0)}
                className="w-24 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.hours")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">{t("settings.asyncToolsInlineBytes")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.asyncToolsInlineBytesDesc")}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={0}
                step={1024}
                value={config.inlineResultBytes}
                onChange={(e) =>
                  updateNumber("inlineResultBytes", 0)(Number(e.target.value))
                }
                onBlur={commitNumber("inlineResultBytes", 0)}
                className="w-28 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.bytes")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsOutputTail", "运行输出留存")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsOutputTailDesc",
                  "后台 exec 任务运行期间留存的最新输出字节数，供 job_status 实时查看（判断「还在跑」还是「卡住」）。越大越清晰、越占内存。范围 256B–1MB。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={256}
                max={1048576}
                step={1024}
                value={config.outputTailBytes}
                onChange={(e) => updateNumber("outputTailBytes", 256)(Number(e.target.value))}
                onBlur={commitNumber("outputTailBytes", 256)}
                className="w-28 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.bytes")}</span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsWakeupMaxDelay", "定时唤醒最长延迟")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsWakeupMaxDelayDesc",
                  "代理用 schedule_wakeup 自我定时唤醒的最长延迟上限；更长的周期任务应交给定时任务（cron）。下限固定 10 秒。范围 10 秒–7 天。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={10}
                max={604800}
                step={60}
                value={config.wakeupMaxDelaySecs}
                onChange={(e) => updateNumber("wakeupMaxDelaySecs", 10)(Number(e.target.value))}
                onBlur={commitNumber("wakeupMaxDelaySecs", 10)}
                className="w-28 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.seconds")}
              </span>
            </div>
          </div>

          <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
            <div className="space-y-0.5 pr-4">
              <div className="text-sm font-medium">
                {t("settings.asyncToolsWakeupMaxPending", "每会话定时唤醒上限")}
              </div>
              <div className="text-xs text-muted-foreground">
                {t(
                  "settings.asyncToolsWakeupMaxPendingDesc",
                  "单个会话最多可同时挂起的定时唤醒数；超出直接拒绝（不排队），防止代理自我排程出大量计费轮次。范围 1–100。",
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                min={1}
                max={100}
                step={1}
                value={config.wakeupMaxPendingPerSession}
                onChange={(e) =>
                  updateNumber("wakeupMaxPendingPerSession", 1)(Number(e.target.value))
                }
                onBlur={commitNumber("wakeupMaxPendingPerSession", 1)}
                className="w-24 h-8 text-sm text-right"
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
