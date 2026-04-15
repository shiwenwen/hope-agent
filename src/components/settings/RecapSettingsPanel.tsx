import { useCallback, useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

interface RecapConfig {
  analysisAgent?: string | null
  defaultRangeDays: number
  maxSessionsPerReport: number
  facetConcurrency: number
  cacheRetentionDays: number
}

interface AgentItem {
  id: string
  name: string
  emoji?: string | null
}

const DEFAULT_CONFIG: RecapConfig = {
  analysisAgent: null,
  defaultRangeDays: 30,
  maxSessionsPerReport: 500,
  facetConcurrency: 4,
  cacheRetentionDays: 180,
}

const AGENT_DEFAULT_SENTINEL = "default"

export default function RecapSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<RecapConfig>(DEFAULT_CONFIG)
  const [savedSnapshot, setSavedSnapshot] = useState<string>("")
  const [agents, setAgents] = useState<AgentItem[]>([])
  const [loaded, setLoaded] = useState(false)

  const persist = useCallback(async (next: RecapConfig) => {
    try {
      await getTransport().call("save_recap_config", { config: next })
      setSavedSnapshot(JSON.stringify(next))
    } catch (e) {
      logger.error("settings", "RecapSettingsPanel::save", "Failed to save recap config", e)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<RecapConfig>("get_recap_config"),
      getTransport().call<AgentItem[]>("list_agents").catch(() => [] as AgentItem[]),
    ])
      .then(([cfg, agentList]) => {
        if (cancelled) return
        const merged = { ...DEFAULT_CONFIG, ...cfg }
        setConfig(merged)
        setSavedSnapshot(JSON.stringify(merged))
        setAgents(agentList)
        setLoaded(true)
      })
      .catch((e: unknown) => {
        logger.error("settings", "RecapSettingsPanel::load", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const commitIfChanged = useCallback(
    (next: RecapConfig) => {
      if (JSON.stringify(next) !== savedSnapshot) {
        void persist(next)
      }
    },
    [persist, savedSnapshot],
  )

  const updateNumber =
    (key: keyof Pick<
      RecapConfig,
      "defaultRangeDays" | "maxSessionsPerReport" | "facetConcurrency" | "cacheRetentionDays"
    >, min: number) =>
    (raw: number) => {
      const clamped = Number.isFinite(raw) ? Math.max(min, Math.round(raw)) : min
      setConfig((prev) => ({ ...prev, [key]: clamped }))
    }

  const commitNumber =
    (key: keyof Pick<
      RecapConfig,
      "defaultRangeDays" | "maxSessionsPerReport" | "facetConcurrency" | "cacheRetentionDays"
    >, min: number) =>
    () => {
      setConfig((prev) => {
        const clamped = Math.max(min, Math.round(prev[key]))
        const next = { ...prev, [key]: clamped }
        commitIfChanged(next)
        return next
      })
    }

  const handleAgentChange = (value: string) => {
    const next: RecapConfig = {
      ...config,
      analysisAgent: value === AGENT_DEFAULT_SENTINEL ? null : value,
    }
    setConfig(next)
    commitIfChanged(next)
  }

  if (!loaded) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-1">
        <p className="text-xs text-muted-foreground px-3">{t("settings.recapDesc")}</p>
      </div>

      <div className="mt-4 space-y-6">
        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapAnalysisAgent")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapAnalysisAgentDesc")}
            </div>
          </div>
          <Select
            value={config.analysisAgent ?? AGENT_DEFAULT_SENTINEL}
            onValueChange={handleAgentChange}
          >
            <SelectTrigger className="w-56 h-8 text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={AGENT_DEFAULT_SENTINEL}>
                {t("settings.recapAnalysisAgentDefault")}
              </SelectItem>
              {agents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>
                  {agent.emoji ? `${agent.emoji} ` : ""}
                  {agent.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapDefaultRangeDays")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapDefaultRangeDaysDesc")}
            </div>
          </div>
          <Input
            type="number"
            min={1}
            step={1}
            value={config.defaultRangeDays}
            onChange={(e) => updateNumber("defaultRangeDays", 1)(Number(e.target.value))}
            onBlur={commitNumber("defaultRangeDays", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapMaxSessions")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapMaxSessionsDesc")}
            </div>
          </div>
          <Input
            type="number"
            min={1}
            step={50}
            value={config.maxSessionsPerReport}
            onChange={(e) => updateNumber("maxSessionsPerReport", 1)(Number(e.target.value))}
            onBlur={commitNumber("maxSessionsPerReport", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapFacetConcurrency")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapFacetConcurrencyDesc")}
            </div>
          </div>
          <Input
            type="number"
            min={1}
            max={32}
            step={1}
            value={config.facetConcurrency}
            onChange={(e) => updateNumber("facetConcurrency", 1)(Number(e.target.value))}
            onBlur={commitNumber("facetConcurrency", 1)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>

        <div className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="space-y-0.5 pr-4">
            <div className="text-sm font-medium">{t("settings.recapCacheRetentionDays")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.recapCacheRetentionDaysDesc")}
            </div>
          </div>
          <Input
            type="number"
            min={0}
            step={30}
            value={config.cacheRetentionDays}
            onChange={(e) => updateNumber("cacheRetentionDays", 0)(Number(e.target.value))}
            onBlur={commitNumber("cacheRetentionDays", 0)}
            className="w-24 h-8 text-sm text-right"
          />
        </div>
      </div>
    </div>
  )
}
