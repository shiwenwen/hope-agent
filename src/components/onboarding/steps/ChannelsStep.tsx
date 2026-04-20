import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import ChannelIcon from "@/components/common/ChannelIcon"
import AddAccountDialog from "@/components/settings/channel-panel/AddAccountDialog"
import type {
  AgentInfo,
  ChannelPluginInfo,
} from "@/components/settings/channel-panel/types"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface ChannelsStepProps {
  /**
   * Footer "Configure in Settings" link — finishes the wizard and
   * lands the user in Settings → Channels for the full panel. Cards
   * themselves now open the Add dialog inline without leaving the
   * onboarding flow.
   */
  onJumpToSettings: () => void
}

interface ChannelMeta {
  id: string
  /** Whether the channel is supported on the current OS. */
  supported: boolean
}

function buildChannels(): ChannelMeta[] {
  const isMac = typeof navigator !== "undefined" && navigator.platform.includes("Mac")
  return [
    { id: "telegram", supported: true },
    { id: "discord", supported: true },
    { id: "slack", supported: true },
    { id: "feishu", supported: true },
    { id: "googlechat", supported: true },
    { id: "line", supported: true },
    { id: "qqbot", supported: true },
    { id: "whatsapp", supported: true },
    { id: "wechat", supported: true },
    { id: "signal", supported: true },
    { id: "irc", supported: true },
    { id: "imessage", supported: isMac },
  ]
}

/**
 * Step 9 — IM channel discovery.
 *
 * Visual parity with Settings → Channels empty state: same 2×N card
 * grid, same `ChannelIcon` brand icons, same `pluginName_*` /
 * `pluginDesc_*` i18n keys for labels. Clicking a card opens the
 * Settings AddAccountDialog inline — same dialog, same credential
 * flow — so the user never leaves the wizard. The footer link still
 * jumps to the full Settings panel for power users.
 */
export function ChannelsStep({ onJumpToSettings }: ChannelsStepProps) {
  const { t } = useTranslation()
  const channels = buildChannels()
  const [plugins, setPlugins] = useState<ChannelPluginInfo[]>([])
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [addOpen, setAddOpen] = useState(false)
  const [addInitialChannel, setAddInitialChannel] = useState<string | undefined>()

  useEffect(() => {
    void (async () => {
      try {
        const [pluginList, agentList] = await Promise.all([
          getTransport().call<ChannelPluginInfo[]>("channel_list_plugins"),
          getTransport().call<AgentInfo[]>("list_agents"),
        ])
        setPlugins(pluginList)
        setAgents(agentList)
      } catch (e) {
        logger.warn("onboarding", "ChannelsStep::load", "failed to load plugins/agents", e)
      }
    })()
  }, [])

  function openAddFor(channelId: string) {
    setAddInitialChannel(channelId)
    setAddOpen(true)
  }

  return (
    <div className="px-6 py-6 space-y-5 max-w-2xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.channels.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.channels.subtitle")}</p>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
        {channels.map((c) => {
          const disabled = !c.supported
          return (
            <button
              key={c.id}
              type="button"
              disabled={disabled}
              onClick={() => !disabled && openAddFor(c.id)}
              className={`flex items-center gap-3 p-4 rounded-lg border text-left transition-colors ${
                disabled
                  ? "border-border/60 opacity-50 cursor-not-allowed"
                  : "border-border hover:border-primary hover:bg-accent cursor-pointer"
              }`}
            >
              <ChannelIcon channelId={c.id} className="h-8 w-8 shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="font-medium text-sm truncate">
                  {t(`channels.pluginName_${c.id}`, c.id)}
                </div>
                <div className="text-xs text-muted-foreground truncate">
                  {disabled
                    ? t("onboarding.channels.macOnly")
                    : t(`channels.pluginDesc_${c.id}`, "")}
                </div>
              </div>
            </button>
          )
        })}
      </div>

      <div className="flex flex-col items-center gap-1 pt-2">
        <button
          type="button"
          onClick={onJumpToSettings}
          className="text-xs text-primary hover:underline"
        >
          {t("onboarding.channels.configureInSettings")}
        </button>
        <p className="text-xs text-muted-foreground text-center">
          {t("onboarding.channels.hint")}
        </p>
      </div>

      <AddAccountDialog
        open={addOpen}
        onOpenChange={(v) => {
          setAddOpen(v)
          if (!v) setAddInitialChannel(undefined)
        }}
        plugins={plugins}
        agents={agents}
        initialChannelId={addInitialChannel}
        onAdded={() => {
          setAddOpen(false)
          setAddInitialChannel(undefined)
        }}
      />
    </div>
  )
}
