import { useState, useEffect, useCallback, useRef, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
import { QRCodeSVG } from "qrcode.react"
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
  Check,
  Loader2,
  AlertCircle,
  MessageCircle,
  Pencil,
  X,
  Bot,
  ArrowLeft,
} from "lucide-react"
import { logger } from "@/lib/logger"
import ChannelIcon from "@/components/common/ChannelIcon"

interface TelegramTopicConfig {
  requireMention?: boolean | null
  enabled?: boolean | null
  allowFrom: string[]
  agentId?: string | null
  systemPrompt?: string | null
}

interface TelegramGroupConfig {
  requireMention?: boolean | null
  groupPolicy?: string | null
  enabled?: boolean | null
  allowFrom: string[]
  agentId?: string | null
  systemPrompt?: string | null
  topics: Record<string, TelegramTopicConfig>
}

interface TelegramChannelConfig {
  requireMention?: boolean | null
  enabled?: boolean | null
  agentId?: string | null
  systemPrompt?: string | null
}

interface ChannelAccountConfig {
  id: string
  channelId: string
  label: string
  enabled: boolean
  agentId?: string | null
  credentials: Record<string, unknown>
  settings: Record<string, unknown>
  security: {
    dmPolicy: string
    groupAllowlist: string[]
    userAllowlist: string[]
    adminIds: string[]
    groupPolicy: string
    groups: Record<string, TelegramGroupConfig>
    channels: Record<string, TelegramChannelConfig>
  }
}

interface AgentInfo {
  id: string
  name: string
  emoji?: string | null
  avatar?: string | null
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

interface WeChatConnection {
  botToken: string
  baseUrl: string
  remoteAccountId?: string | null
  userId?: string | null
}

interface WeChatLoginStartResult {
  qrcodeUrl?: string | null
  sessionKey: string
  message: string
}

interface WeChatLoginWaitResult {
  connected: boolean
  status?: string | null
  botToken?: string | null
  remoteAccountId?: string | null
  baseUrl?: string | null
  userId?: string | null
  message: string
}

export default function ChannelPanel() {
  const { t } = useTranslation()
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [plugins, setPlugins] = useState<ChannelPluginInfo[]>([])
  const [healthMap, setHealthMap] = useState<Record<string, ChannelHealth>>({})
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [editingAccount, setEditingAccount] = useState<ChannelAccountConfig | null>(null)
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [loading, setLoading] = useState(true)

  const loadData = useCallback(async () => {
    try {
      const [accountList, pluginList, healthList, agentList] = await Promise.all([
        invoke<ChannelAccountConfig[]>("channel_list_accounts"),
        invoke<ChannelPluginInfo[]>("channel_list_plugins"),
        invoke<[string, ChannelHealth][]>("channel_health_all"),
        invoke<AgentInfo[]>("list_agents"),
      ])
      setAccounts(accountList)
      setPlugins(pluginList)
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
      logger.error("channel", "ChannelPanel", "Failed to start channel account", e)
    }
  }

  const handleStop = async (accountId: string) => {
    try {
      await invoke("channel_stop_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to stop channel account", e)
    }
  }

  const handleRemove = async (accountId: string) => {
    try {
      await invoke("channel_remove_account", { accountId })
      await loadData()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to remove channel account", e)
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
        onOpenChange={setShowAddDialog}
        plugins={plugins}
        agents={agents}
        onAdded={() => {
          setShowAddDialog(false)
          loadData()
        }}
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

function AgentAvatar({ agent, size = "sm" }: { agent: AgentInfo; size?: "sm" | "md" }) {
  const cls = size === "sm" ? "w-5 h-5 text-[10px]" : "w-6 h-6 text-xs"
  const iconCls = size === "sm" ? "h-3 w-3" : "h-3.5 w-3.5"
  return (
    <span className={`${cls} rounded-full bg-primary/15 flex items-center justify-center shrink-0 overflow-hidden`}>
      {agent.avatar ? (
        <img
          src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar}
          className="w-full h-full object-cover"
          alt=""
        />
      ) : agent.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className={`${iconCls} text-muted-foreground`} />
      )}
    </span>
  )
}

function getWeChatConnectionFromAccount(account: ChannelAccountConfig | null): WeChatConnection | null {
  if (!account || account.channelId !== "wechat") return null

  const credentials = account.credentials as Record<string, string | undefined>
  const settings = account.settings as Record<string, string | undefined>
  const botToken = credentials.token?.trim()
  const baseUrl = settings.baseUrl?.trim() || credentials.baseUrl?.trim()

  if (!botToken || !baseUrl) return null

  return {
    botToken,
    baseUrl,
    remoteAccountId: credentials.remoteAccountId ?? null,
    userId: credentials.userId ?? null,
  }
}

function defaultWeChatLabel(connection: WeChatConnection): string {
  const identity = connection.userId?.trim() || connection.remoteAccountId?.trim()
  return identity ? `WeChat ${identity}` : "WeChat"
}

function WeChatConnectSection({
  accountId,
  connection,
  onConnectionChange,
}: {
  accountId?: string
  connection: WeChatConnection | null
  onConnectionChange: (connection: WeChatConnection | null) => void
}) {
  const { t } = useTranslation()
  const [sessionKey, setSessionKey] = useState<string | null>(null)
  const [qrCodeUrl, setQrCodeUrl] = useState<string | null>(null)
  const [status, setStatus] = useState<"idle" | "wait" | "scanned" | "expired" | "connected" | "error">(
    connection ? "connected" : "idle",
  )
  const [message, setMessage] = useState<string | null>(null)
  const [connecting, setConnecting] = useState(false)
  const pollingRef = useRef(false)

  useEffect(() => {
    if (connection && !sessionKey && !qrCodeUrl) {
      setStatus("connected")
      if (!message) {
        setMessage(t("channels.wechatConnected"))
      }
      return
    }

    if (!sessionKey && status === "connected") {
      setStatus("idle")
      setMessage(null)
    }
  }, [connection, message, qrCodeUrl, sessionKey, status, t])

  useEffect(() => {
    if (!sessionKey) return

    let cancelled = false

    const poll = async () => {
      if (cancelled || pollingRef.current) return
      pollingRef.current = true

      try {
        const result = await invoke<WeChatLoginWaitResult>("channel_wechat_wait_login", {
          sessionKey,
          timeoutMs: 1500,
        })

        if (cancelled) return

        if (result.connected && result.botToken && result.baseUrl) {
          onConnectionChange({
            botToken: result.botToken,
            baseUrl: result.baseUrl,
            remoteAccountId: result.remoteAccountId ?? null,
            userId: result.userId ?? null,
          })
          setStatus("connected")
          setMessage(result.message)
          setSessionKey(null)
          return
        }

        if (result.status === "scanned") {
          setStatus("scanned")
        } else if (result.status === "expired") {
          setStatus("expired")
          setSessionKey(null)
        } else {
          setStatus("wait")
        }
        setMessage(result.message)
      } catch (error) {
        if (!cancelled) {
          setStatus("error")
          setMessage(String(error))
          setSessionKey(null)
        }
      } finally {
        pollingRef.current = false
      }
    }

    void poll()
    const timer = window.setInterval(() => {
      void poll()
    }, 2000)

    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [onConnectionChange, sessionKey])

  const handleStart = async () => {
    setConnecting(true)
    setMessage(null)
    setStatus("wait")

    try {
      const result = await invoke<WeChatLoginStartResult>("channel_wechat_start_login", {
        accountId: accountId ?? null,
      })
      console.log("[WeChat Login] start_login result:", {
        qrcodeUrl: result.qrcodeUrl ? `${result.qrcodeUrl.substring(0, 80)}... (${result.qrcodeUrl.length} chars)` : null,
        sessionKey: result.sessionKey,
        message: result.message,
      })
      setQrCodeUrl(result.qrcodeUrl ?? null)
      setSessionKey(result.sessionKey)
      setMessage(result.message)
    } catch (error) {
      setStatus("error")
      setMessage(String(error))
      setSessionKey(null)
    } finally {
      setConnecting(false)
    }
  }

  const identity = connection?.userId?.trim() || connection?.remoteAccountId?.trim()
  const statusText = status === "scanned"
    ? t("channels.wechatScannedHint")
    : status === "expired"
      ? t("channels.wechatExpiredHint")
      : status === "connected"
        ? t("channels.wechatConnected")
        : status === "error"
          ? message || t("common.saveFailed")
          : t("channels.wechatScanHint")

  return (
    <div className="space-y-3 rounded-lg border bg-card/60 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="space-y-1">
          <Label>{t("channels.wechatConnect")}</Label>
          <p className="text-xs text-muted-foreground">{t("channels.wechatConnectionHint")}</p>
        </div>
        <Button variant="outline" size="sm" onClick={handleStart} disabled={connecting}>
          {connecting ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
          {connection ? t("channels.wechatReconnect") : t("channels.wechatConnect")}
        </Button>
      </div>

      {connection && (
        <div className="flex items-center gap-1 text-sm text-green-600">
          <Check className="h-3.5 w-3.5" />
          {identity ? `${t("channels.wechatConnectedAs")} ${identity}` : t("channels.wechatConnected")}
        </div>
      )}

      {qrCodeUrl && status !== "connected" && (
        <div className="space-y-3">
          <div className="rounded-lg border bg-white p-3 flex justify-center">
            <QRCodeSVG value={qrCodeUrl} size={200} />
          </div>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.open(qrCodeUrl, "_blank", "noopener,noreferrer")}
            >
              {t("channels.wechatOpenQr")}
            </Button>
          </div>
        </div>
      )}

      <div className={`text-sm ${status === "error" ? "text-destructive" : "text-muted-foreground"}`}>
        {statusText}
        {message && status !== "error" ? <span className="ml-1">{message}</span> : null}
      </div>
    </div>
  )
}

function AddAccountDialog({
  open,
  onOpenChange,
  plugins,
  agents,
  onAdded,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  plugins: ChannelPluginInfo[]
  agents: AgentInfo[]
  onAdded: () => void
}) {
  const { t } = useTranslation()
  const [step, setStep] = useState<"select" | "configure">("select")
  const [channelId, setChannelId] = useState("")
  const [label, setLabel] = useState("")
  const [token, setToken] = useState("")
  const [agentId, setAgentId] = useState("")
  const [dmPolicy, setDmPolicy] = useState("open")
  const [userAllowlist, setUserAllowlist] = useState<string[]>([])
  const [allowlistInput, setAllowlistInput] = useState("")
  const [saving, setSaving] = useState(false)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)
  const [wechatConnection, setWeChatConnection] = useState<WeChatConnection | null>(null)

  const selectedPlugin = plugins.find((p) => p.meta.id === channelId)

  const handleSelectChannel = (id: string) => {
    setChannelId(id)
    setStep("configure")
  }

  const handleBack = () => {
    setStep("select")
  }

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

  // Group policy state
  const [groupPolicy, setGroupPolicy] = useState("open")
  const [groups, setGroups] = useState<Record<string, TelegramGroupConfig>>({})
  const [channels, setChannels] = useState<Record<string, TelegramChannelConfig>>({})

  useEffect(() => {
    if (channelId === "wechat" && wechatConnection && !label.trim()) {
      setLabel(defaultWeChatLabel(wechatConnection))
    }
  }, [channelId, label, wechatConnection])

  const handleSave = async () => {
    if (!label.trim()) return
    if (channelId === "telegram" && !token.trim()) return
    if (channelId === "wechat" && !wechatConnection) return

    setSaving(true)
    try {
      const credentials = channelId === "wechat"
        ? {
            token: wechatConnection?.botToken ?? "",
            remoteAccountId: wechatConnection?.remoteAccountId ?? null,
            userId: wechatConnection?.userId ?? null,
          }
        : { token: token.trim() }

      const settings = channelId === "wechat"
        ? {
            transport: "longpoll",
            baseUrl: wechatConnection?.baseUrl ?? "",
          }
        : { transport: "polling" }

      await invoke("channel_add_account", {
        channelId,
        label: label.trim(),
        agentId: agentId || null,
        credentials,
        settings,
        security: {
          dmPolicy,
          groupAllowlist: [],
          userAllowlist,
          adminIds: [],
          groupPolicy,
          groups,
          channels,
        },
      })
      // Reset form
      setStep("select")
      setChannelId("")
      setLabel("")
      setToken("")
      setAgentId("")
      setDmPolicy("open")
      setUserAllowlist([])
      setAllowlistInput("")
      setGroupPolicy("open")
      setGroups({})
      setChannels({})
      setValidationResult(null)
      setValidationError(null)
      setWeChatConnection(null)
      onAdded()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to add channel account", e)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => {
      if (!v) {
        setStep("select")
        setChannelId("")
      }
      onOpenChange(v)
    }}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        {step === "select" ? (
          <>
            <DialogHeader>
              <DialogTitle>{t("channels.selectChannel")}</DialogTitle>
            </DialogHeader>

            <div className="grid grid-cols-2 gap-3">
              {plugins.map((p) => (
                <button
                  key={p.meta.id}
                  onClick={() => handleSelectChannel(p.meta.id)}
                  className="flex items-center gap-3 p-4 rounded-lg border border-border hover:border-primary hover:bg-accent transition-colors text-left cursor-pointer"
                >
                  <ChannelIcon channelId={p.meta.id} className="h-8 w-8" />
                  <div className="min-w-0">
                    <div className="font-medium">{p.meta.displayName}</div>
                    <div className="text-xs text-muted-foreground truncate">{p.meta.description}</div>
                  </div>
                </button>
              ))}
            </div>

            <DialogFooter>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t("common.cancel")}
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <div className="flex items-center gap-2">
                <Button variant="ghost" size="icon" className="h-7 w-7" onClick={handleBack}>
                  <ArrowLeft className="h-4 w-4" />
                </Button>
                <div className="flex items-center gap-2">
                  <ChannelIcon channelId={channelId} className="h-5 w-5" />
                  <DialogTitle>{selectedPlugin?.meta.displayName ?? channelId}</DialogTitle>
                </div>
              </div>
            </DialogHeader>

            <div className="space-y-4">
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
                      onBlur={() => {
                        if (token.trim() && !validationResult && !validating) {
                          handleValidate()
                        }
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

              {channelId === "wechat" && (
                <WeChatConnectSection
                  connection={wechatConnection}
                  onConnectionChange={setWeChatConnection}
                />
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

              {/* Bound Agent */}
              <div className="space-y-2">
                <Label>{t("channels.boundAgent")}</Label>
                <Select value={agentId || "__none__"} onValueChange={(v) => setAgentId(v === "__none__" ? "" : v)}>
                  <SelectTrigger>
                    <SelectValue placeholder={t("channels.boundAgentDefault")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
                    {agents.map((a) => (
                      <SelectItem key={a.id} value={a.id}>
                        <span className="flex items-center gap-2">
                          <AgentAvatar agent={a} />
                          {a.name}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t("channels.boundAgentHint")}
                </p>
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

              {/* User Allowlist */}
              {dmPolicy === "allowlist" && (
                <AllowlistTagInput
                  tags={userAllowlist}
                  onTagsChange={setUserAllowlist}
                  inputValue={allowlistInput}
                  onInputChange={setAllowlistInput}
                />
              )}

              {/* Telegram-specific: Group & Channel Config */}
              {channelId === "telegram" && (
                <TelegramGroupChannelConfig
                  groupPolicy={groupPolicy}
                  onGroupPolicyChange={setGroupPolicy}
                  groups={groups}
                  onGroupsChange={setGroups}
                  channels={channels}
                  onChannelsChange={setChannels}
                  agents={agents}
                  t={t}
                />
              )}
            </div>

            <DialogFooter>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t("common.cancel")}
              </Button>
              <Button
                onClick={handleSave}
                disabled={
                  !label.trim()
                  || saving
                  || (channelId === "telegram" && !token.trim())
                  || (channelId === "wechat" && !wechatConnection)
                }
              >
                {saving ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : null}
                {t("common.save")}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

function EditAccountDialog({
  open,
  onOpenChange,
  account,
  plugins,
  agents,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  account: ChannelAccountConfig | null
  plugins: ChannelPluginInfo[]
  agents: AgentInfo[]
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const [label, setLabel] = useState("")
  const [token, setToken] = useState("")
  const [agentId, setAgentId] = useState("")
  const [dmPolicy, setDmPolicy] = useState("open")
  const [userAllowlist, setUserAllowlist] = useState<string[]>([])
  const [allowlistInput, setAllowlistInput] = useState("")
  const [groupPolicy, setGroupPolicy] = useState("open")
  const [groups, setGroups] = useState<Record<string, TelegramGroupConfig>>({})
  const [channels, setChannels] = useState<Record<string, TelegramChannelConfig>>({})
  const [saving, setSaving] = useState(false)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)
  const [wechatConnection, setWeChatConnection] = useState<WeChatConnection | null>(null)

  // Populate form when account changes
  useEffect(() => {
    if (account) {
      setLabel(account.label)
      setToken((account.credentials as Record<string, string>).token ?? "")
      setAgentId(account.agentId ?? "")
      setDmPolicy(account.security.dmPolicy)
      setUserAllowlist([...account.security.userAllowlist])
      setAllowlistInput("")
      setGroupPolicy(account.security.groupPolicy ?? "open")
      setGroups(account.security.groups ? { ...account.security.groups } : {})
      setChannels(account.security.channels ? { ...account.security.channels } : {})
      setValidationResult(null)
      setValidationError(null)
      setWeChatConnection(getWeChatConnectionFromAccount(account))
    }
  }, [account])

  const handleValidate = async () => {
    if (!token.trim() || !account) return
    setValidating(true)
    setValidationResult(null)
    setValidationError(null)
    try {
      const botName = await invoke<string>("channel_validate_credentials", {
        channelId: account.channelId,
        credentials: { token: token.trim() },
      })
      setValidationResult(botName)
    } catch (e) {
      setValidationError(String(e))
    } finally {
      setValidating(false)
    }
  }

  const handleSave = async () => {
    if (!account || !label.trim()) return
    setSaving(true)
    try {
      const params: Record<string, unknown> = {
        accountId: account.id,
        label: label.trim(),
        agentId: agentId || "",  // empty string = clear to default
        security: {
          dmPolicy,
          groupAllowlist: account.security.groupAllowlist,
          userAllowlist,
          adminIds: account.security.adminIds,
          groupPolicy,
          groups,
          channels,
        },
      }
      // Only send credentials if token was changed
      const originalToken = (account.credentials as Record<string, string>).token ?? ""
      if (account.channelId === "wechat") {
        if (wechatConnection) {
          params.credentials = {
            token: wechatConnection.botToken,
            remoteAccountId: wechatConnection.remoteAccountId ?? null,
            userId: wechatConnection.userId ?? null,
          }
          params.settings = {
            ...(account.settings as Record<string, unknown>),
            transport: "longpoll",
            baseUrl: wechatConnection.baseUrl,
          }
        }
      } else if (token.trim() !== originalToken) {
        params.credentials = { token: token.trim() }
      }
      await invoke("channel_update_account", params)
      onSaved()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to update channel account", e)
    } finally {
      setSaving(false)
    }
  }

  if (!account) return null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("channels.editAccount")}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Channel Type (read-only with logo) */}
          <div className="space-y-2">
            <Label>{t("channels.channelType")}</Label>
            <div className="flex items-center gap-2 h-9 px-3 rounded-md border border-input bg-muted text-sm">
              <ChannelIcon channelId={account.channelId} className="h-5 w-5" />
              <span>{plugins.find((p) => p.meta.id === account.channelId)?.meta.displayName ?? account.channelId}</span>
            </div>
          </div>

          {/* Bot Token */}
          {account.channelId === "telegram" && (
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
                  onBlur={() => {
                    if (token.trim() && !validationResult && !validating) {
                      handleValidate()
                    }
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
            </div>
          )}

          {account.channelId === "wechat" && (
            <WeChatConnectSection
              accountId={account.id}
              connection={wechatConnection}
              onConnectionChange={setWeChatConnection}
            />
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

          {/* Bound Agent */}
          <div className="space-y-2">
            <Label>{t("channels.boundAgent")}</Label>
            <Select value={agentId || "__none__"} onValueChange={(v) => setAgentId(v === "__none__" ? "" : v)}>
              <SelectTrigger>
                <SelectValue placeholder={t("channels.boundAgentDefault")} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    <span className="flex items-center gap-2">
                      <AgentAvatar agent={a} />
                      {a.name}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {t("channels.boundAgentHint")}
            </p>
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

          {/* User Allowlist */}
          {dmPolicy === "allowlist" && (
            <AllowlistTagInput
              tags={userAllowlist}
              onTagsChange={setUserAllowlist}
              inputValue={allowlistInput}
              onInputChange={setAllowlistInput}
            />
          )}

          {/* Telegram-specific: Group & Channel Config */}
          {account.channelId === "telegram" && (
            <TelegramGroupChannelConfig
              groupPolicy={groupPolicy}
              onGroupPolicyChange={setGroupPolicy}
              groups={groups}
              onGroupsChange={setGroups}
              channels={channels}
              onChannelsChange={setChannels}
              agents={agents}
              t={t}
            />
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleSave}
            disabled={!label.trim() || saving}
          >
            {saving ? <Loader2 className="h-4 w-4 animate-spin mr-1" /> : null}
            {t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function AllowlistTagInput({
  tags,
  onTagsChange,
  inputValue,
  onInputChange,
}: {
  tags: string[]
  onTagsChange: (tags: string[]) => void
  inputValue: string
  onInputChange: (value: string) => void
}) {
  const { t } = useTranslation()
  const inputRef = useRef<HTMLInputElement>(null)

  const addTag = (raw: string) => {
    const value = raw.trim()
    if (value && !tags.includes(value)) {
      onTagsChange([...tags, value])
    }
    onInputChange("")
  }

  const removeTag = (index: number) => {
    onTagsChange(tags.filter((_, i) => i !== index))
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault()
      addTag(inputValue)
    } else if (e.key === "Backspace" && !inputValue && tags.length > 0) {
      removeTag(tags.length - 1)
    }
  }

  const handlePaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData("text")
    if (text.includes(",") || text.includes("\n")) {
      e.preventDefault()
      const newTags = text
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean)
        .filter((s) => !tags.includes(s))
      if (newTags.length > 0) {
        onTagsChange([...tags, ...newTags])
      }
    }
  }

  return (
    <div className="space-y-2">
      <Label>{t("channels.userAllowlist")}</Label>
      <div
        className="flex flex-wrap gap-1.5 rounded-md border bg-background px-3 py-2 min-h-[38px] cursor-text"
        onClick={() => inputRef.current?.focus()}
      >
        {tags.map((tag, i) => (
          <span
            key={tag}
            className="inline-flex items-center gap-0.5 rounded bg-muted px-2 py-0.5 text-sm"
          >
            {tag}
            <button
              type="button"
              className="ml-0.5 rounded-full hover:bg-muted-foreground/20 p-0.5"
              onClick={(e) => {
                e.stopPropagation()
                removeTag(i)
              }}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          className="flex-1 min-w-[120px] bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          placeholder={tags.length === 0 ? t("channels.userAllowlistPlaceholder") : ""}
          value={inputValue}
          onChange={(e) => onInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onBlur={() => { if (inputValue.trim()) addTag(inputValue) }}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("channels.userAllowlistHint")}
      </p>
    </div>
  )
}

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`
}

// ── Telegram Group & Channel Configuration Component ──────────────

function TelegramGroupChannelConfig({
  groupPolicy,
  onGroupPolicyChange,
  groups,
  onGroupsChange,
  channels,
  onChannelsChange,
  agents,
  t,
}: {
  groupPolicy: string
  onGroupPolicyChange: (v: string) => void
  groups: Record<string, TelegramGroupConfig>
  onGroupsChange: (v: Record<string, TelegramGroupConfig>) => void
  channels: Record<string, TelegramChannelConfig>
  onChannelsChange: (v: Record<string, TelegramChannelConfig>) => void
  agents: AgentInfo[]
  t: (key: string) => string
}) {
  const [newGroupId, setNewGroupId] = useState("")
  const [newChannelId, setNewChannelId] = useState("")

  const addGroup = () => {
    const id = newGroupId.trim()
    if (!id || id in groups) return
    onGroupsChange({
      ...groups,
      [id]: {
        requireMention: null,
        enabled: true,
        allowFrom: [],
        agentId: null,
        systemPrompt: null,
        topics: {},
      },
    })
    setNewGroupId("")
  }

  const removeGroup = (id: string) => {
    const next = { ...groups }
    delete next[id]
    onGroupsChange(next)
  }

  const updateGroup = (id: string, patch: Partial<TelegramGroupConfig>) => {
    onGroupsChange({
      ...groups,
      [id]: { ...groups[id], ...patch },
    })
  }

  const addChannel = () => {
    const id = newChannelId.trim()
    if (!id || id in channels) return
    onChannelsChange({
      ...channels,
      [id]: { requireMention: null, enabled: true, agentId: null, systemPrompt: null },
    })
    setNewChannelId("")
  }

  const removeChannel = (id: string) => {
    const next = { ...channels }
    delete next[id]
    onChannelsChange(next)
  }

  const updateChannel = (id: string, patch: Partial<TelegramChannelConfig>) => {
    onChannelsChange({
      ...channels,
      [id]: { ...channels[id], ...patch },
    })
  }

  return (
    <>
      {/* Divider line */}
      <div className="border-t my-2" />

      {/* Group Policy */}
      <div className="space-y-2">
        <Label>{t("channels.groupPolicy")}</Label>
        <Select value={groupPolicy} onValueChange={onGroupPolicyChange}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="open">{t("channels.groupPolicyOpen")}</SelectItem>
            <SelectItem value="allowlist">{t("channels.groupPolicyAllowlist")}</SelectItem>
            <SelectItem value="disabled">{t("channels.groupPolicyDisabled")}</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {/* Group Configuration List */}
      {groupPolicy !== "disabled" && (
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <Label>{t("channels.groupConfig")}</Label>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("channels.groupConfigHint")}
              </p>
            </div>
          </div>

          {/* Existing groups */}
          <div className="space-y-2">
            {Object.entries(groups).map(([gId, gCfg]) => (
              <GroupConfigItem
                key={gId}
                groupId={gId}
                config={gCfg}
                agents={agents}
                onUpdate={(patch) => updateGroup(gId, patch)}
                onRemove={() => removeGroup(gId)}
                t={t}
              />
            ))}
          </div>

          {/* Add group */}
          <div className="flex gap-2">
            <Input
              placeholder={t("channels.groupIdPlaceholder")}
              value={newGroupId}
              onChange={(e) => setNewGroupId(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") addGroup() }}
              className="flex-1"
            />
            <Button variant="outline" size="sm" onClick={addGroup} disabled={!newGroupId.trim()}>
              <Plus className="h-4 w-4 mr-1" />
              {t("channels.addGroup")}
            </Button>
          </div>
        </div>
      )}

      {/* Divider */}
      <div className="border-t my-2" />

      {/* Channel Configuration List */}
      <div className="space-y-3">
        <div>
          <Label>{t("channels.channelConfig")}</Label>
          <p className="text-xs text-muted-foreground mt-0.5">
            {t("channels.channelConfigHint")}
          </p>
        </div>

        {/* Existing channels */}
        <div className="space-y-2">
          {Object.entries(channels).map(([cId, cCfg]) => (
            <div key={cId} className="rounded-lg border bg-card p-3 space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium font-mono">{cId}</span>
                <IconTip label={t("channels.removeConfig")}>
                  <button
                    type="button"
                    className="p-1 rounded hover:bg-muted"
                    onClick={() => removeChannel(cId)}
                  >
                    <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
                  </button>
                </IconTip>
              </div>
              <div className="flex items-center gap-4 flex-wrap">
                <div className="flex items-center gap-2">
                  <Label className="text-xs">{t("channels.channelEnabled")}</Label>
                  <Switch
                    checked={cCfg.enabled !== false}
                    onCheckedChange={(v) => updateChannel(cId, { enabled: v })}
                  />
                </div>
                <div className="flex items-center gap-2">
                  <Label className="text-xs">{t("channels.groupRequireMention")}</Label>
                  <Select
                    value={cCfg.requireMention === null || cCfg.requireMention === undefined ? "yes" : cCfg.requireMention ? "yes" : "no"}
                    onValueChange={(v) => updateChannel(cId, { requireMention: v === "yes" })}
                  >
                    <SelectTrigger className="h-7 text-xs w-20">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="yes">✓</SelectItem>
                      <SelectItem value="no">✗</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="flex-1 min-w-[160px]">
                  <Select
                    value={cCfg.agentId || "__none__"}
                    onValueChange={(v) => updateChannel(cId, { agentId: v === "__none__" ? null : v })}
                  >
                    <SelectTrigger className="h-8 text-xs">
                      <SelectValue placeholder={t("channels.boundAgentDefault")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
                      {agents.map((a) => (
                        <SelectItem key={a.id} value={a.id}>
                          <span className="flex items-center gap-2">
                            <AgentAvatar agent={a} />
                            {a.name}
                          </span>
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* Add channel */}
        <div className="flex gap-2">
          <Input
            placeholder={t("channels.channelIdPlaceholder")}
            value={newChannelId}
            onChange={(e) => setNewChannelId(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") addChannel() }}
            className="flex-1"
          />
          <Button variant="outline" size="sm" onClick={addChannel} disabled={!newChannelId.trim()}>
            <Plus className="h-4 w-4 mr-1" />
            {t("channels.addChannel")}
          </Button>
        </div>
      </div>
    </>
  )
}

// ── Single Group Config Item ──────────────────────────────────────

function GroupConfigItem({
  groupId,
  config,
  agents,
  onUpdate,
  onRemove,
  t,
}: {
  groupId: string
  config: TelegramGroupConfig
  agents: AgentInfo[]
  onUpdate: (patch: Partial<TelegramGroupConfig>) => void
  onRemove: () => void
  t: (key: string) => string
}) {
  const [expanded, setExpanded] = useState(false)

  const mentionLabel = groupId === "*" ? t("channels.groupIdWildcard") : groupId

  return (
    <div className="rounded-lg border bg-card p-3 space-y-2">
      {/* Header row */}
      <div className="flex items-center justify-between">
        <button
          type="button"
          className="flex items-center gap-2 text-sm font-medium hover:text-foreground transition-colors"
          onClick={() => setExpanded(!expanded)}
        >
          <span className={`transition-transform ${expanded ? "rotate-90" : ""}`}>▸</span>
          <span className="font-mono">{mentionLabel}</span>
        </button>
        <IconTip label={t("channels.removeConfig")}>
          <button
            type="button"
            className="p-1 rounded hover:bg-muted"
            onClick={onRemove}
          >
            <Trash2 className="h-3.5 w-3.5 text-muted-foreground" />
          </button>
        </IconTip>
      </div>

      {/* Compact inline controls */}
      <div className="flex items-center gap-4 flex-wrap">
        <div className="flex items-center gap-2">
          <Label className="text-xs">{t("channels.groupEnabled")}</Label>
          <Switch
            checked={config.enabled !== false}
            onCheckedChange={(v) => onUpdate({ enabled: v })}
          />
        </div>
        <div className="flex items-center gap-2">
          <Label className="text-xs">{t("channels.groupRequireMention")}</Label>
          <Select
            value={config.requireMention === null || config.requireMention === undefined ? "__inherit__" : config.requireMention ? "yes" : "no"}
            onValueChange={(v) => {
              if (v === "__inherit__") onUpdate({ requireMention: null })
              else onUpdate({ requireMention: v === "yes" })
            }}
          >
            <SelectTrigger className="h-7 text-xs w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__inherit__">{t("channels.groupRequireMentionInherit")}</SelectItem>
              <SelectItem value="yes">✓</SelectItem>
              <SelectItem value="no">✗</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex-1 min-w-[160px]">
          <Select
            value={config.agentId || "__none__"}
            onValueChange={(v) => onUpdate({ agentId: v === "__none__" ? null : v })}
          >
            <SelectTrigger className="h-7 text-xs">
              <SelectValue placeholder={t("channels.boundAgentDefault")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
              {agents.map((a) => (
                <SelectItem key={a.id} value={a.id}>
                  <span className="flex items-center gap-2">
                    <AgentAvatar agent={a} />
                    {a.name}
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Expanded details */}
      {expanded && (
        <div className="space-y-3 pt-2 border-t">
          {/* Allow from */}
          <div className="space-y-1">
            <Label className="text-xs">{t("channels.groupAllowFrom")}</Label>
            <Input
              placeholder={t("channels.groupAllowFromHint")}
              value={(config.allowFrom || []).join(", ")}
              onChange={(e) => {
                const ids = e.target.value
                  .split(/[,\n]/)
                  .map((s) => s.trim())
                  .filter(Boolean)
                onUpdate({ allowFrom: ids })
              }}
              className="text-xs h-8"
            />
          </div>

          {/* System prompt */}
          <div className="space-y-1">
            <Label className="text-xs">{t("channels.groupSystemPrompt")}</Label>
            <Input
              placeholder={t("channels.groupSystemPromptPlaceholder")}
              value={config.systemPrompt || ""}
              onChange={(e) => onUpdate({ systemPrompt: e.target.value || null })}
              className="text-xs h-8"
            />
          </div>
        </div>
      )}
    </div>
  )
}
