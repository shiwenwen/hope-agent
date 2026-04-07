import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { IconTip } from "@/components/ui/tooltip"
import { Plus, Play, Square, Trash2, Loader2, Pencil } from "lucide-react"
import { logger } from "@/lib/logger"
import ChannelIcon from "@/components/common/ChannelIcon"
import AgentAvatar from "./AgentAvatar"
import AddAccountDialog from "./AddAccountDialog"
import EditAccountDialog from "./EditAccountDialog"
import { formatUptime } from "./utils"
import type {
  ChannelAccountConfig,
  ChannelPluginInfo,
  ChannelHealth,
  AgentInfo,
} from "./types"

export default function ChannelPanel() {
  const { t } = useTranslation()
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [plugins, setPlugins] = useState<ChannelPluginInfo[]>([])
  const [healthMap, setHealthMap] = useState<Record<string, ChannelHealth>>({})
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [addInitialChannel, setAddInitialChannel] = useState<string | undefined>()
  const [editingAccount, setEditingAccount] = useState<ChannelAccountConfig | null>(null)
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [loading, setLoading] = useState(true)

  const loadData = useCallback(async () => {
    try {
      const [accountList, pluginList, healthList, agentList] = await Promise.all([
        getTransport().call<ChannelAccountConfig[]>("channel_list_accounts"),
        getTransport().call<ChannelPluginInfo[]>("channel_list_plugins"),
        getTransport().call<[string, ChannelHealth][]>("channel_health_all"),
        getTransport().call<AgentInfo[]>("list_agents"),
      ])
      setAccounts(accountList)
      // Prioritize commonly-used channels at the top of selection grid
      const priorityOrder = ["wechat", "telegram", "feishu", "qq_bot", "discord"]
      const sorted = [...pluginList].sort((a, b) => {
        const ai = priorityOrder.indexOf(a.meta.id)
        const bi = priorityOrder.indexOf(b.meta.id)
        if (ai !== -1 && bi !== -1) return ai - bi
        if (ai !== -1) return -1
        if (bi !== -1) return 1
        return 0
      })
      setPlugins(sorted)
      setAgents(agentList)
      const hMap: Record<string, ChannelHealth> = {}
      for (const [id, health] of healthList) {
        hMap[id] = health
      }
      setHealthMap(hMap)
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to load channel data", e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    let aborted = false
    loadData()
    // Poll health every 10s
    const interval = setInterval(async () => {
      try {
        const healthList = await getTransport().call<[string, ChannelHealth][]>("channel_health_all")
        if (aborted) return
        const hMap: Record<string, ChannelHealth> = {}
        for (const [id, health] of healthList) {
          hMap[id] = health
        }
        setHealthMap(hMap)
      } catch {
        // ignore
      }
    }, 10000)
    return () => { aborted = true; clearInterval(interval) }
  }, [loadData])

  const handleStart = async (accountId: string) => {
    try {
      await getTransport().call("channel_start_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to start channel account", e)
    }
  }

  const handleStop = async (accountId: string) => {
    try {
      await getTransport().call("channel_stop_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to stop channel account", e)
    }
  }

  const handleRemove = async (accountId: string) => {
    try {
      await getTransport().call("channel_remove_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to remove channel account", e)
    }
  }

  const handleToggleEnabled = async (account: ChannelAccountConfig) => {
    try {
      await getTransport().call("channel_update_account", {
        accountId: account.id,
        enabled: !account.enabled,
      })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to toggle channel account", e)
    }
  }

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold">{t("channels.title")}</h2>
          <p className="text-sm text-muted-foreground">{t("channels.description")}</p>
        </div>
        <Button size="sm" onClick={() => setShowAddDialog(true)}>
          <Plus className="h-4 w-4 mr-1" />
          {t("channels.addAccount")}
        </Button>
      </div>

      {/* Account List */}
      {accounts.length === 0 ? (
        <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
          {plugins.map((p) => (
            <button
              key={p.meta.id}
              onClick={() => {
                setAddInitialChannel(p.meta.id)
                setShowAddDialog(true)
              }}
              className="flex items-center gap-3 p-4 rounded-lg border border-border hover:border-primary hover:bg-accent transition-colors text-left cursor-pointer"
            >
              <ChannelIcon channelId={p.meta.id} className="h-8 w-8" />
              <div className="min-w-0">
                <div className="font-medium text-sm">{t(`channels.pluginName_${p.meta.id}`, p.meta.displayName)}</div>
                <div className="text-xs text-muted-foreground truncate">{t(`channels.pluginDesc_${p.meta.id}`, p.meta.description)}</div>
              </div>
            </button>
          ))}
        </div>
      ) : (
        <div className="space-y-3">
          {accounts.map((account) => {
            const health = healthMap[account.id]
            const isRunning = health?.isRunning ?? false

            return (
              <div
                key={account.id}
                className="flex items-center gap-4 p-4 rounded-lg border bg-card"
              >
                {/* Status dot */}
                <div
                  className={`h-2.5 w-2.5 rounded-full shrink-0 ${
                    isRunning
                      ? "bg-green-500"
                      : account.enabled
                        ? "bg-yellow-500"
                        : "bg-zinc-400"
                  }`}
                />

                {/* Info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium truncate">{account.label}</span>
                    <span className="inline-flex items-center gap-1 text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
                      <ChannelIcon channelId={account.channelId} className="h-3 w-3" />
                      {account.channelId}
                    </span>
                  </div>
                  <div className="text-xs text-muted-foreground mt-0.5">
                    {isRunning
                      ? `${t("channels.running")}${health?.uptimeSecs ? ` · ${formatUptime(health.uptimeSecs)}` : ""}`
                      : account.enabled
                        ? t("channels.starting")
                        : t("channels.stopped")}
                    {health?.botName && ` · ${health.botName}`}
                    {account.agentId && (() => {
                      const agent = agents.find(a => a.id === account.agentId)
                      return agent ? (
                        <span className="inline-flex items-center gap-1 ml-1">· <AgentAvatar agent={agent} /> {agent.name}</span>
                      ) : ` · ${account.agentId}`
                    })()}
                    {health?.error && (
                      <span className="text-destructive ml-1">· {health.error}</span>
                    )}
                  </div>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1 shrink-0">
                  <Switch
                    checked={account.enabled}
                    onCheckedChange={() => handleToggleEnabled(account)}
                  />
                  {account.enabled && !isRunning && (
                    <IconTip label={t("channels.start")}>
                      <Button variant="ghost" size="icon" onClick={() => handleStart(account.id)}>
                        <Play className="h-4 w-4" />
                      </Button>
                    </IconTip>
                  )}
                  {isRunning && (
                    <IconTip label={t("channels.stop")}>
                      <Button variant="ghost" size="icon" onClick={() => handleStop(account.id)}>
                        <Square className="h-4 w-4" />
                      </Button>
                    </IconTip>
                  )}
                  <IconTip label={t("channels.edit")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setEditingAccount(account)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                  </IconTip>
                  <IconTip label={t("channels.remove")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => handleRemove(account.id)}
                    >
                      <Trash2 className="h-4 w-4 text-destructive" />
                    </Button>
                  </IconTip>
                </div>
              </div>
            )
          })}
        </div>
      )}

      {/* Add Account Dialog */}
      <AddAccountDialog
        open={showAddDialog}
        onOpenChange={(v) => {
          setShowAddDialog(v)
          if (!v) setAddInitialChannel(undefined)
        }}
        plugins={plugins}
        agents={agents}
        onAdded={() => {
          setShowAddDialog(false)
          setAddInitialChannel(undefined)
          loadData()
        }}
        initialChannelId={addInitialChannel}
      />

      {/* Edit Account Dialog */}
      <EditAccountDialog
        open={!!editingAccount}
        onOpenChange={(open) => { if (!open) setEditingAccount(null) }}
        account={editingAccount}
        plugins={plugins}
        agents={agents}
        onSaved={() => {
          setEditingAccount(null)
          loadData()
        }}
      />
    </div>
  )
}
