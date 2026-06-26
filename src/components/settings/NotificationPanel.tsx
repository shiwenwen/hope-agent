import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import {
  loadStartupNotificationConfig,
  saveStartupNotificationConfig,
  type StartupNotificationConfig,
} from "@/lib/startup-notification"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select"
import { logger } from "@/lib/logger"
import {
  saveNotificationConfig,
  loadNotificationConfig,
  type NotificationConfig,
} from "@/lib/notifications"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import type { AgentConfig } from "./types"

import type { AgentInfo as BaseAgentInfo } from "@/types/chat"

interface AgentInfo extends BaseAgentInfo {
  notifyOnComplete?: boolean | null
}

export default function NotificationPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<NotificationConfig | null>(null)
  const [startupConfig, setStartupConfig] = useState<StartupNotificationConfig | null>(null)
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [saving, setSaving] = useState(false)
  const [savingStartup, setSavingStartup] = useState(false)

  // Load global config + startup config + agents in parallel — the
  // three calls are independent. Startup config tolerates failure (old
  // backend without the route) by leaving its toggle hidden.
  const loadData = useCallback(async () => {
    const [cfgResult, startupResult, agentsResult] = await Promise.allSettled([
      loadNotificationConfig(),
      loadStartupNotificationConfig(),
      getTransport().call<BaseAgentInfo[]>("list_agents"),
    ])
    if (cfgResult.status === "fulfilled") {
      setConfig(cfgResult.value)
    } else {
      logger.error("settings", "NotificationPanel::load", "Failed to load config", cfgResult.reason)
    }
    if (startupResult.status === "fulfilled") {
      setStartupConfig(startupResult.value)
    } else {
      logger.error("settings", "NotificationPanel::loadStartup", "Failed to load startup config", startupResult.reason)
    }
    if (agentsResult.status !== "fulfilled") {
      return
    }
    const agentsWithNotify = await Promise.all(
      agentsResult.value.map(async (a) => {
        try {
          const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", { id: a.id })
          return { ...a, notifyOnComplete: agentConfig.notifyOnComplete ?? null }
        } catch {
          return { ...a, notifyOnComplete: null }
        }
      }),
    )
    setAgents(agentsWithNotify)
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  const handleGlobalToggle = async (enabled: boolean) => {
    if (!config) return
    const newConfig = { ...config, enabled }
    setConfig(newConfig)
    setSaving(true)
    try {
      await saveNotificationConfig(newConfig)
    } catch (e) {
      logger.error("settings", "NotificationPanel::save", "Failed to save config", e)
    } finally {
      setSaving(false)
    }
  }

  const handleShowChatContentToggle = async (showChatContent: boolean) => {
    if (!config) return
    const newConfig = { ...config, showChatContent }
    setConfig(newConfig)
    setSaving(true)
    try {
      await saveNotificationConfig(newConfig)
    } catch (e) {
      logger.error("settings", "NotificationPanel::saveContentPreview", "Failed to save config", e)
    } finally {
      setSaving(false)
    }
  }

  const handleBackgroundJobToggle = async (notifyOnBackgroundJobComplete: boolean) => {
    if (!config) return
    const newConfig = { ...config, notifyOnBackgroundJobComplete }
    setConfig(newConfig)
    setSaving(true)
    try {
      await saveNotificationConfig(newConfig)
    } catch (e) {
      logger.error("settings", "NotificationPanel::saveBgJob", "Failed to save config", e)
    } finally {
      setSaving(false)
    }
  }

  const handleStartupToggle = async (enabled: boolean) => {
    if (!startupConfig) return
    const next = { ...startupConfig, enabled }
    setStartupConfig(next)
    setSavingStartup(true)
    try {
      await saveStartupNotificationConfig(next)
    } catch (e) {
      logger.error("settings", "NotificationPanel::saveStartup", "Failed to save startup config", e)
    } finally {
      setSavingStartup(false)
    }
  }

  const handleAgentNotify = async (agentId: string, value: string) => {
    const notifyValue = value === "default" ? null : value === "on"
    try {
      const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", { id: agentId })
      const updated = { ...agentConfig, notifyOnComplete: notifyValue }
      await getTransport().call("save_agent_config_cmd", { id: agentId, config: updated })
      setAgents((prev) =>
        prev.map((a) => (a.id === agentId ? { ...a, notifyOnComplete: notifyValue } : a)),
      )
    } catch (e) {
      logger.error("settings", "NotificationPanel::saveAgent", "Failed to save agent config", e)
    }
  }

  if (!config) return null

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* Global Toggle */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium">{t("notification.globalToggle")}</h3>
            <p className="text-xs text-muted-foreground mt-0.5">{t("notification.globalDesc")}</p>
          </div>
          <Switch checked={config.enabled} onCheckedChange={handleGlobalToggle} disabled={saving} />
        </div>
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium">{t("notification.showChatContentToggle")}</h3>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("notification.showChatContentDesc")}
            </p>
          </div>
          <Switch
            checked={config.showChatContent !== false}
            onCheckedChange={handleShowChatContentToggle}
            disabled={saving}
          />
        </div>
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium">
              {t("notification.backgroundJobToggle", "后台任务完成提醒")}
            </h3>
            <p className="text-xs text-muted-foreground mt-0.5">
              {t("notification.backgroundJobDesc", "后台任务跑完时在窗口处于后台时弹桌面通知")}
            </p>
          </div>
          <Switch
            checked={config.notifyOnBackgroundJobComplete !== false}
            onCheckedChange={handleBackgroundJobToggle}
            disabled={saving}
          />
        </div>
      </div>

      {/* IM Startup Back-Online Notice */}
      {startupConfig && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="text-sm font-medium">{t("notification.startupToggle")}</h3>
              <p className="text-xs text-muted-foreground mt-0.5">{t("notification.startupDesc")}</p>
            </div>
            <Switch
              checked={startupConfig.enabled}
              onCheckedChange={handleStartupToggle}
              disabled={savingStartup}
            />
          </div>
        </div>
      )}

      {/* Agent Notifications */}
      <div className="space-y-3">
        <div>
          <h3 className="text-sm font-medium">{t("notification.agentSection")}</h3>
          <p className="text-xs text-muted-foreground mt-0.5">{t("notification.agentDesc")}</p>
        </div>
        <div className="space-y-2">
          {agents.map((agent) => (
            <div key={agent.id} className="flex items-center justify-between py-1.5">
              <AgentSelectDisplay agent={agent} className="text-sm" />
              <Select
                value={
                  agent.notifyOnComplete === true
                    ? "on"
                    : agent.notifyOnComplete === false
                      ? "off"
                      : "default"
                }
                onValueChange={(v) => handleAgentNotify(agent.id, v)}
              >
                <SelectTrigger className="w-[120px] h-8">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="default">{t("notification.default")}</SelectItem>
                  <SelectItem value="on">{t("notification.on")}</SelectItem>
                  <SelectItem value="off">{t("notification.off")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          ))}
        </div>
      </div>

      {/* Cron note */}
      <p className="text-xs text-muted-foreground">{t("notification.cronNote")}</p>
    </div>
  )
}
