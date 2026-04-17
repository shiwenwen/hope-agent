import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import ContextCompactPanel from "@/components/settings/ContextCompactPanel"
import CrossSessionPanel from "@/components/settings/CrossSessionPanel"
import { invalidateThinkingExpandCache } from "@/components/chat/thinkingCache"

interface ChatConfig {
  autoSendPending: boolean
  autoExpandThinking: boolean
}

export default function ChatSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ChatConfig>({ autoSendPending: true, autoExpandThinking: true })
  const [narrationEnabled, setNarrationEnabled] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    Promise.all([
      getTransport().call<{ autoSendPending?: boolean; autoExpandThinking?: boolean }>("get_user_config"),
      getTransport().call<boolean>("get_tool_call_narration_enabled"),
    ])
      .then(([cfg, narration]) => {
        setConfig({
          autoSendPending: cfg.autoSendPending !== false,
          autoExpandThinking: cfg.autoExpandThinking !== false,
        })
        setNarrationEnabled(narration === true)
      })
      .catch((e: unknown) => logger.error("settings", "ChatSettingsPanel::load", "Failed to load chat config", e))
      .finally(() => setLoaded(true))
  }, [])

  async function toggle(key: keyof ChatConfig) {
    const updated = { ...config, [key]: !config[key] }
    setConfig(updated)
    try {
      const full = await getTransport().call<Record<string, unknown>>("get_user_config")
      await getTransport().call("save_user_config", { config: { ...full, ...updated } })
      if (key === "autoExpandThinking") {
        invalidateThinkingExpandCache()
      }
    } catch (e) {
      logger.error("settings", "ChatSettingsPanel::save", "Failed to save chat config", e)
    }
  }

  async function toggleNarration() {
    const next = !narrationEnabled
    setNarrationEnabled(next)
    try {
      await getTransport().call("set_tool_call_narration_enabled", { enabled: next })
    } catch (e) {
      setNarrationEnabled(!next)
      logger.error("settings", "ChatSettingsPanel::saveNarration", "Failed to save narration toggle", e)
    }
  }

  if (!loaded) return null

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs defaultValue="basic" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="basic">{t("settings.tabChatBasic")}</TabsTrigger>
            <TabsTrigger value="cross-session">{t("settings.tabCrossSession")}</TabsTrigger>
            <TabsTrigger value="context-compact">{t("settings.tabContextCompact")}</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="basic" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full space-y-6 pt-4">
            <div
              className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
              onClick={() => toggle("autoSendPending")}
            >
              <div className="space-y-0.5">
                <div className="text-sm font-medium">{t("settings.chatAutoSend")}</div>
                <div className="text-xs text-muted-foreground">{t("settings.chatAutoSendDesc")}</div>
              </div>
              <Switch
                checked={config.autoSendPending}
                onCheckedChange={() => toggle("autoSendPending")}
              />
            </div>

            <div
              className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
              onClick={() => toggle("autoExpandThinking")}
            >
              <div className="space-y-0.5">
                <div className="text-sm font-medium">{t("settings.chatAutoExpandThinking")}</div>
                <div className="text-xs text-muted-foreground">{t("settings.chatAutoExpandThinkingDesc")}</div>
              </div>
              <Switch
                checked={config.autoExpandThinking}
                onCheckedChange={() => toggle("autoExpandThinking")}
              />
            </div>

            <div
              className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
              onClick={toggleNarration}
            >
              <div className="space-y-0.5">
                <div className="text-sm font-medium">{t("settings.toolCallNarration")}</div>
                <div className="text-xs text-muted-foreground">{t("settings.toolCallNarrationDesc")}</div>
              </div>
              <Switch
                checked={narrationEnabled}
                onCheckedChange={toggleNarration}
              />
            </div>
          </div>
        </TabsContent>

        <TabsContent value="cross-session" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full pt-4">
            <CrossSessionPanel />
          </div>
        </TabsContent>

        <TabsContent value="context-compact" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full pt-4">
            <ContextCompactPanel />
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
