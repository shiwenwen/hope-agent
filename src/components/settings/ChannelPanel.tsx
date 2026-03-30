import { useState, useEffect, useCallback, useRef, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
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
  Pencil,
  X,
  Bot,
} from "lucide-react"
import { logger } from "@/lib/logger"
import ChannelIcon from "@/components/common/ChannelIcon"

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
  const [channelId, setChannelId] = useState("telegram")
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
        agentId: agentId || null,
        credentials: { token: token.trim() },
        settings: { transport: "polling" },
        security: {
          dmPolicy,
          groupAllowlist: [],
          userAllowlist,
          adminIds: [],
        },
      })
      // Reset form
      setLabel("")
      setToken("")
      setAgentId("")
      setDmPolicy("open")
      setUserAllowlist([])
      setAllowlistInput("")
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

function EditAccountDialog({
  open,
  onOpenChange,
  account,
  agents,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  account: ChannelAccountConfig | null
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
  const [saving, setSaving] = useState(false)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)

  // Populate form when account changes
  useEffect(() => {
    if (account) {
      setLabel(account.label)
      setToken((account.credentials as Record<string, string>).token ?? "")
      setAgentId(account.agentId ?? "")
      setDmPolicy(account.security.dmPolicy)
      setUserAllowlist([...account.security.userAllowlist])
      setAllowlistInput("")
      setValidationResult(null)
      setValidationError(null)
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
        },
      }
      // Only send credentials if token was changed
      const originalToken = (account.credentials as Record<string, string>).token ?? ""
      if (token.trim() !== originalToken) {
        params.credentials = { token: token.trim() }
      }
      await invoke("channel_update_account", params)
      onSaved()
    } catch (e) {
      logger.error("Failed to update channel account", e)
    } finally {
      setSaving(false)
    }
  }

  if (!account) return null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("channels.editAccount")}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Channel Type (read-only) */}
          <div className="space-y-2">
            <Label>{t("channels.channelType")}</Label>
            <Input value={account.channelId} disabled />
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
