import { useEffect, useState } from "react"
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
  inlineResultBytes: number
  retentionSecs: number
  orphanGraceSecs: number
  jobStatusMaxWaitSecs: number
}

const DEFAULT_CONFIG: AsyncToolsConfig = {
  enabled: true,
  autoBackgroundSecs: 30,
  maxJobSecs: 1800,
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

  const persist = async (next: AsyncToolsConfig) => {
    try {
      await getTransport().call("save_async_tools_config", { config: next })
      setSavedSnapshot(JSON.stringify(next))
    } catch (e) {
      logger.error("settings", "AsyncToolsPanel::save", "Failed to save async tools config", e)
    }
  }

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

  const commitIfChanged = (next: AsyncToolsConfig) => {
    if (JSON.stringify(next) !== savedSnapshot) {
      void persist(next)
    }
  }

  const handleEnabledChange = (enabled: boolean) => {
    const next = { ...config, enabled }
    setConfig(next)
    commitIfChanged(next)
  }

  type NumericKey =
    | "autoBackgroundSecs"
    | "maxJobSecs"
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
              <div className="text-sm font-medium">{t("settings.asyncToolsAutoBackground")}</div>
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
                onChange={(e) => updateNumber("autoBackgroundSecs", 0)(Number(e.target.value))}
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
              <div className="text-sm font-medium">{t("settings.asyncToolsJobStatusMaxWait")}</div>
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
                onChange={(e) => updateNumber("jobStatusMaxWaitSecs", 1)(Number(e.target.value))}
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
                  updateNumber(
                    "retentionSecs",
                    0,
                  )(Number.isFinite(days) ? Math.max(0, Math.round(days)) * 86400 : 0)
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
                  updateNumber(
                    "orphanGraceSecs",
                    0,
                  )(Number.isFinite(hours) ? Math.max(0, Math.round(hours)) * 3600 : 0)
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
                onChange={(e) => updateNumber("inlineResultBytes", 0)(Number(e.target.value))}
                onBlur={commitNumber("inlineResultBytes", 0)}
                className="w-28 h-8 text-sm text-right"
              />
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {t("settings.bytes")}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
