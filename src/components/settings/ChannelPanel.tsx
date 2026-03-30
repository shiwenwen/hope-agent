import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import { IconTip } from "@/components/ui/tooltip"
import {
  Plus,
  Play,
  Square,
  Trash2,
  RefreshCw,
  Check,
  Loader2,
  AlertCircle,
  MessageCircle,
} from "lucide-react"
import { logger } from "@/lib/logger"

interface ChannelAccountConfig {
  id: string
  channelId: string
  label: string
  enabled: boolean
  credentials: Record<string, unknown>
  settings: Record<string, unknown>
  security: {
    dmPolicy: string
    groupAllowlist: string[]
    userAllowlist: string[]
    adminIds: string[]
  }
}

interface ChannelHealth {
  isRunning: boolean
  lastProbe: string | null
  probeOk: boolean | null
  error: string | null
  uptimeSecs: number | null
  botName: string | null
}

interface ChannelPluginInfo {
  meta: {
    id: string
    displayName: string
    description: string
    version: string
  }
  capabilities: {
    chatTypes: string[]
    supportsPolls: boolean
    supportsReactions: boolean
    supportsEdit: boolean
    supportsMedia: string[]
    supportsTyping: boolean
    maxMessageLength: number | null
  }
}

export default function ChannelPanel() {
  const { t } = useTranslation()
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [plugins, setPlugins] = useState<ChannelPluginInfo[]>([])
  const [healthMap, setHealthMap] = useState<Record<string, ChannelHealth>>({})
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [loading, setLoading] = useState(true)

  const loadData = useCallback(async () => {
    try {
      const [accountList, pluginList, healthList] = await Promise.all([
        invoke<ChannelAccountConfig[]>("channel_list_accounts"),
        invoke<ChannelPluginInfo[]>("channel_list_plugins"),
        invoke<[string, ChannelHealth][]>("channel_health_all"),
      ])
      setAccounts(accountList)
      setPlugins(pluginList)
      const hMap: Record<string, ChannelHealth> = {}
      for (const [id, health] of healthList) {
        hMap[id] = health
      }
      setHealthMap(hMap)
    } catch (e) {
      logger.error("Failed to load channel data", e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()
    // Poll health every 10s
    const interval = setInterval(async () => {
      try {
        const healthList = await invoke<[string, ChannelHealth][]>("channel_health_all")
        const hMap: Record<string, ChannelHealth> = {}
        for (const [id, health] of healthList) {
          hMap[id] = health
        }
        setHealthMap(hMap)
      } catch {
        // ignore
      }
    }, 10000)
    return () => clearInterval(interval)
  }, [loadData])

  const handleStart = async (accountId: string) => {
    try {
      await invoke("channel_start_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("Failed to start channel account", e)
    }
  }

  const handleStop = async (accountId: string) => {
    try {
      await invoke("channel_stop_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("Failed to stop channel account", e)
    }
  }

  const handleRemove = async (accountId: string) => {
    try {
      await invoke("channel_remove_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("Failed to remove channel account", e)
    }
  }

  const handleToggleEnabled = async (account: ChannelAccountConfig) => {
    try {
      await invoke("channel_update_account", {
        accountId: account.id,
        enabled: !account.enabled,
      })
      await loadData()
    } catch (e) {
      logger.error("Failed to toggle channel account", e)
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
        <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
          <MessageCircle className="h-12 w-12 mb-4 opacity-30" />
          <p className="text-sm">{t("channels.noAccounts")}</p>
          <Button
            variant="outline"
            size="sm"
            className="mt-4"
            onClick={() => setShowAddDialog(true)}
          >
            <Plus className="h-4 w-4 mr-1" />
            {t("channels.addFirst")}
          </Button>
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
                    <span className="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
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
        onOpenChange={setShowAddDialog}
        plugins={plugins}
        onAdded={() => {
          setShowAddDialog(false)
          loadData()
        }}
      />
    </div>
  )
}

function AddAccountDialog({
  open,
  onOpenChange,
  plugins,
  onAdded,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  plugins: ChannelPluginInfo[]
  onAdded: () => void
}) {
  const { t } = useTranslation()
  const [channelId, setChannelId] = useState("telegram")
  const [label, setLabel] = useState("")
  const [token, setToken] = useState("")
  const [dmPolicy, setDmPolicy] = useState("open")
  const [saving, setSaving] = useState(false)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)

  const handleValidate = async () => {
    if (!token.trim()) return
    setValidating(true)
    setValidationResult(null)
    setValidationError(null)
    try {
      const botName = await invoke<string>("channel_validate_credentials", {
        channelId,
        credentials: { token: token.trim() },
      })
      setValidationResult(botName)
      if (!label.trim()) {
        setLabel(botName)
      }
    } catch (e) {
      setValidationError(String(e))
    } finally {
      setValidating(false)
    }
  }

  const handleSave = async () => {
    if (!token.trim() || !label.trim()) return
    setSaving(true)
    try {
      await invoke("channel_add_account", {
        channelId,
        label: label.trim(),
        credentials: { token: token.trim() },
        settings: { transport: "polling" },
        security: {
          dmPolicy,
          groupAllowlist: [],
          userAllowlist: [],
          adminIds: [],
        },
      })
      // Reset form
      setLabel("")
      setToken("")
      setDmPolicy("open")
      setValidationResult(null)
      setValidationError(null)
      onAdded()
    } catch (e) {
      logger.error("Failed to add channel account", e)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("channels.addAccount")}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Channel Type */}
          <div className="space-y-2">
            <Label>{t("channels.channelType")}</Label>
            <Select value={channelId} onValueChange={setChannelId}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {plugins.map((p) => (
                  <SelectItem key={p.meta.id} value={p.meta.id}>
                    {p.meta.displayName}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Bot Token (Telegram-specific) */}
          {channelId === "telegram" && (
            <div className="space-y-2">
              <Label>Bot Token</Label>
              <div className="flex gap-2">
                <Input
                  type="password"
                  placeholder="123456:ABC-DEF..."
                  value={token}
                  onChange={(e) => {
                    setToken(e.target.value)
                    setValidationResult(null)
                    setValidationError(null)
                  }}
                  className="flex-1"
                />
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleValidate}
                  disabled={!token.trim() || validating}
                >
                  {validating ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    t("channels.testConnection")
                  )}
                </Button>
              </div>
              {validationResult && (
                <div className="flex items-center gap-1 text-sm text-green-600">
                  <Check className="h-3.5 w-3.5" />
                  {validationResult}
                </div>
              )}
              {validationError && (
                <div className="flex items-center gap-1 text-sm text-destructive">
                  <AlertCircle className="h-3.5 w-3.5" />
                  {validationError}
                </div>
              )}
              <p className="text-xs text-muted-foreground">
                {t("channels.telegramTokenHint")}
              </p>
            </div>
          )}

          {/* Label */}
          <div className="space-y-2">
            <Label>{t("channels.accountLabel")}</Label>
            <Input
              placeholder={t("channels.accountLabelPlaceholder")}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>

          {/* DM Policy */}
          <div className="space-y-2">
            <Label>{t("channels.dmPolicy")}</Label>
            <Select value={dmPolicy} onValueChange={setDmPolicy}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="open">{t("channels.dmPolicyOpen")}</SelectItem>
                <SelectItem value="allowlist">{t("channels.dmPolicyAllowlist")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleSave}
            disabled={!token.trim() || !label.trim() || saving}
          >
            {saving ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : null}
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`
}
