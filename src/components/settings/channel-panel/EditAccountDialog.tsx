import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
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
import { Switch } from "@/components/ui/switch"
import { Check, Loader2, AlertCircle } from "lucide-react"
import { logger } from "@/lib/logger"
import ChannelIcon from "@/components/common/ChannelIcon"
import {
  AgentSelectDisplay,
  INHERIT_AGENT_SENTINEL,
  InheritAgentSelectDisplay,
} from "@/components/common/AgentSelectDisplay"
import AllowlistTagInput from "./AllowlistTagInput"
import WeChatConnectSection from "./WeChatConnectSection"
import TelegramGroupChannelConfig from "./TelegramGroupConfig"
import SaveErrorBanner from "./SaveErrorBanner"
import { getWeChatConnectionFromAccount, parseChannelSaveError } from "./utils"
import type {
  AgentInfo,
  ChannelAccountConfig,
  ChannelPluginInfo,
  ImReplyMode,
  TelegramGroupConfig,
  TelegramChannelConfig,
  WeChatConnection,
} from "./types"
import {
  IM_REPLY_MODE_DEFAULT,
  channelSupportsStreamPreview,
  readImReplyMode,
} from "./types"

export default function EditAccountDialog({
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
  const [autoApproveTools, setAutoApproveTools] = useState(false)
  const [imReplyMode, setImReplyMode] = useState<ImReplyMode>(IM_REPLY_MODE_DEFAULT)
  const [groups, setGroups] = useState<Record<string, TelegramGroupConfig>>({})
  const [channels, setChannels] = useState<Record<string, TelegramChannelConfig>>({})
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)
  const [wechatConnection, setWeChatConnection] = useState<WeChatConnection | null>(null)
  const selectedAgent = agents.find((agent) => agent.id === agentId)
  const selectedPlugin = account
    ? plugins.find((p) => p.meta.id === account.channelId)
    : undefined
  const channelSupportsStreaming = channelSupportsStreamPreview(selectedPlugin)

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
      setAutoApproveTools(account.autoApproveTools ?? false)
      setImReplyMode(readImReplyMode(account))
      setValidationResult(null)
      setValidationError(null)
      setSaveError(null)
      setWeChatConnection(getWeChatConnectionFromAccount(account))
    }
  }, [account])

  const handleValidate = async () => {
    if (!token.trim() || !account) return
    setValidating(true)
    setValidationResult(null)
    setValidationError(null)
    try {
      const botName = await getTransport().call<string>("channel_validate_credentials", {
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
    setSaveError(null)
    try {
      const params: Record<string, unknown> = {
        accountId: account.id,
        label: label.trim(),
        agentId: agentId || "",  // empty string = clear to default
        autoApproveTools,
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
      // Start from existing settings so we never wipe sibling keys (transport,
      // baseUrl, etc.) when the user only touched the IM reply mode toggle.
      const settingsBase = {
        ...((account.settings as Record<string, unknown> | null | undefined) ?? {}),
        imReplyMode,
      }
      if (account.channelId === "wechat") {
        if (wechatConnection) {
          params.credentials = {
            token: wechatConnection.botToken,
            remoteAccountId: wechatConnection.remoteAccountId ?? null,
            userId: wechatConnection.userId ?? null,
          }
          params.settings = {
            ...settingsBase,
            transport: "longpoll",
            baseUrl: wechatConnection.baseUrl,
          }
        } else {
          params.settings = settingsBase
        }
      } else {
        if (token.trim() !== originalToken) {
          params.credentials = { token: token.trim() }
        }
        params.settings = settingsBase
      }
      await getTransport().call("channel_update_account", params)
      onSaved()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to update channel account", e)
      setSaveError(parseChannelSaveError(e, t))
    } finally {
      setSaving(false)
    }
  }

  if (!account) return null

  return (
    <Dialog open={open} onOpenChange={(v) => {
      if (!v) setSaveError(null)
      onOpenChange(v)
    }}>
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
              <span>{t(`channels.pluginName_${account.channelId}`, plugins.find((p) => p.meta.id === account.channelId)?.meta.displayName ?? account.channelId)}</span>
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
            <Select
              value={agentId || INHERIT_AGENT_SENTINEL}
              onValueChange={(v) => setAgentId(v === INHERIT_AGENT_SENTINEL ? "" : v)}
            >
              <SelectTrigger>
                {selectedAgent ? (
                  <AgentSelectDisplay agent={selectedAgent} />
                ) : (
                  <InheritAgentSelectDisplay label={t("channels.boundAgentDefault")} />
                )}
              </SelectTrigger>
              <SelectContent>
                <SelectItem
                  value={INHERIT_AGENT_SENTINEL}
                  textValue={t("channels.boundAgentDefault")}
                >
                  {t("channels.boundAgentDefault")}
                </SelectItem>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id} textValue={a.name}>
                    <AgentSelectDisplay agent={a} />
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

          {/* Auto Approve Tools */}
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label>{t("channels.autoApproveTools")}</Label>
              <p className="text-xs text-muted-foreground">
                {t("channels.autoApproveToolsHint")}
              </p>
            </div>
            <Switch
              checked={autoApproveTools}
              onCheckedChange={setAutoApproveTools}
            />
          </div>

          {/* IM Reply Mode — three modes, all channels honor the same
              setting. `preview` only has streaming-specific behavior on
              channels that advertise a preview transport; non-streaming
              channels silently degrade `preview` → `final`. */}
          <div className="space-y-2">
            <Label>{t("channels.imReplyMode")}</Label>
            <Select
              value={imReplyMode}
              onValueChange={(v) => setImReplyMode(v as ImReplyMode)}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="split">
                  {t("channels.imReplyModeSplit")}
                </SelectItem>
                <SelectItem value="final">
                  {t("channels.imReplyModeFinal")}
                </SelectItem>
                <SelectItem value="preview">
                  {t("channels.imReplyModePreview")}
                </SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {(() => {
                if (imReplyMode === "preview" && !channelSupportsStreaming) {
                  return t("channels.imReplyModePreviewDegrades")
                }
                switch (imReplyMode) {
                  case "split":
                    return t("channels.imReplyModeSplitHint")
                  case "final":
                    return t("channels.imReplyModeFinalHint")
                  case "preview":
                    return t("channels.imReplyModePreviewHint")
                }
              })()}
            </p>
          </div>
        </div>

        <SaveErrorBanner message={saveError} />

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
