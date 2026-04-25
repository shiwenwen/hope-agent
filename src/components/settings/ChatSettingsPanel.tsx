import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import ContextCompactPanel from "@/components/settings/ContextCompactPanel"
import AwarenessPanel from "@/components/settings/AwarenessPanel"
import { invalidateThinkingExpandCache } from "@/components/chat/thinkingCache"
import { Check, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"

interface ChatConfig {
  autoSendPending: boolean
  autoExpandThinking: boolean
}

interface SessionTitleConfig {
  enabled: boolean
  providerId: string | null
  modelId: string | null
}

interface ProviderOption {
  id: string
  name: string
  models: { id: string; name: string }[]
  enabled?: boolean
}

export default function ChatSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ChatConfig>({ autoSendPending: true, autoExpandThinking: true })
  const [narrationEnabled, setNarrationEnabled] = useState(false)
  const [sessionTitleConfig, setSessionTitleConfig] = useState<SessionTitleConfig>({
    enabled: false,
    providerId: null,
    modelId: null,
  })
  const [sessionTitleSavedJson, setSessionTitleSavedJson] = useState("")
  const [sessionTitleSaving, setSessionTitleSaving] = useState(false)
  const [sessionTitleSaveStatus, setSessionTitleSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [providers, setProviders] = useState<ProviderOption[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    Promise.all([
      getTransport().call<{ autoSendPending?: boolean; autoExpandThinking?: boolean }>("get_user_config"),
      getTransport().call<boolean>("get_tool_call_narration_enabled"),
      getTransport().call<SessionTitleConfig>("get_session_title_config"),
      getTransport().call<ProviderOption[]>("get_providers"),
    ])
      .then(([cfg, narration, sessionTitle, providerList]) => {
        setConfig({
          autoSendPending: cfg.autoSendPending !== false,
          autoExpandThinking: cfg.autoExpandThinking !== false,
        })
        setNarrationEnabled(narration === true)
        setSessionTitleConfig({
          enabled: sessionTitle.enabled === true,
          providerId: sessionTitle.providerId ?? null,
          modelId: sessionTitle.modelId ?? null,
        })
        setSessionTitleSavedJson(JSON.stringify({
          enabled: sessionTitle.enabled === true,
          providerId: sessionTitle.providerId ?? null,
          modelId: sessionTitle.modelId ?? null,
        }))
        setProviders(providerList.filter((p) => p.enabled !== false && p.models.length > 0))
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

  const sessionTitleDirty = JSON.stringify(sessionTitleConfig) !== sessionTitleSavedJson

  function updateSessionTitleConfig(patch: Partial<SessionTitleConfig>) {
    setSessionTitleConfig((prev) => ({ ...prev, ...patch }))
  }

  async function saveSessionTitleConfig() {
    setSessionTitleSaving(true)
    try {
      await getTransport().call("save_session_title_config", { config: sessionTitleConfig })
      setSessionTitleSavedJson(JSON.stringify(sessionTitleConfig))
      setSessionTitleSaveStatus("saved")
      setTimeout(() => setSessionTitleSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ChatSettingsPanel::saveSessionTitle", "Failed to save session title config", e)
      setSessionTitleSaveStatus("failed")
      setTimeout(() => setSessionTitleSaveStatus("idle"), 2000)
    } finally {
      setSessionTitleSaving(false)
    }
  }

  function handleSessionTitleModel(value: string) {
    if (value === "__chat__") {
      updateSessionTitleConfig({ providerId: null, modelId: null })
      return
    }
    const [providerId, modelId] = value.split("::", 2)
    updateSessionTitleConfig({ providerId, modelId })
  }

  if (!loaded) return null

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs defaultValue="basic" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="basic">{t("settings.tabChatBasic")}</TabsTrigger>
            <TabsTrigger value="awareness">{t("settings.tabAwareness")}</TabsTrigger>
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

            <div className="rounded-lg bg-secondary/30">
              <div className="flex items-center justify-between px-3 py-3 gap-3">
                <div className="space-y-0.5 flex-1 min-w-0">
                  <div className="text-sm font-medium">{t("settings.sessionTitle")}</div>
                  <div className="text-xs text-muted-foreground">{t("settings.sessionTitleDesc")}</div>
                </div>
                <Switch
                  checked={sessionTitleConfig.enabled}
                  onCheckedChange={(enabled) => updateSessionTitleConfig({ enabled })}
                />
              </div>
              {sessionTitleConfig.enabled && (
                <div className="px-3 pb-3 pt-2 border-t border-border/30 space-y-3">
                  <div className="flex items-center gap-2">
                    <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                      {t("settings.sessionTitleModel")}
                    </label>
                    <Select
                      value={
                        sessionTitleConfig.providerId && sessionTitleConfig.modelId
                          ? `${sessionTitleConfig.providerId}::${sessionTitleConfig.modelId}`
                          : "__chat__"
                      }
                      onValueChange={handleSessionTitleModel}
                    >
                      <SelectTrigger className="h-8 text-xs flex-1">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__chat__">{t("settings.sessionTitleUseChatModel")}</SelectItem>
                        {providers.map((prov) =>
                          prov.models.map((m) => (
                            <SelectItem key={`${prov.id}::${m.id}`} value={`${prov.id}::${m.id}`}>
                              {prov.name} / {m.name}
                            </SelectItem>
                          )),
                        )}
                      </SelectContent>
                    </Select>
                  </div>
                  <p className="text-[10px] text-muted-foreground/60">
                    {sessionTitleConfig.providerId && sessionTitleConfig.modelId
                      ? t("settings.sessionTitleCustomModelHint")
                      : t("settings.sessionTitleChatModelHint")}
                  </p>
                </div>
              )}
              <div className="px-3 pb-3">
                <Button
                  variant="default"
                  size="sm"
                  onClick={saveSessionTitleConfig}
                  disabled={(!sessionTitleDirty && sessionTitleSaveStatus === "idle") || sessionTitleSaving}
                  className={cn(
                    sessionTitleSaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
                    sessionTitleSaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
                  )}
                >
                  {sessionTitleSaving ? (
                    <span className="flex items-center gap-1.5">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      {t("common.saving")}
                    </span>
                  ) : sessionTitleSaveStatus === "saved" ? (
                    <span className="flex items-center gap-1.5">
                      <Check className="h-3.5 w-3.5" />
                      {t("common.saved")}
                    </span>
                  ) : sessionTitleSaveStatus === "failed" ? (
                    t("common.saveFailed")
                  ) : (
                    t("common.save")
                  )}
                </Button>
              </div>
            </div>
          </div>
        </TabsContent>

        <TabsContent value="awareness" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="w-full pt-4">
            <AwarenessPanel />
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
