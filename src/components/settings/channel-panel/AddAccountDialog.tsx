import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
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
import { Check, Loader2, AlertCircle, ArrowLeft } from "lucide-react"
import { logger } from "@/lib/logger"
import ChannelIcon from "@/components/common/ChannelIcon"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import AllowlistTagInput from "./AllowlistTagInput"
import WeChatConnectSection from "./WeChatConnectSection"
import TelegramGroupChannelConfig from "./TelegramGroupConfig"
import SaveErrorBanner from "./SaveErrorBanner"
import { defaultWeChatLabel, parseChannelSaveError } from "./utils"
import type {
  AgentInfo,
  ChannelPluginInfo,
  TelegramGroupConfig,
  TelegramChannelConfig,
  WeChatConnection,
} from "./types"

export default function AddAccountDialog({
  open,
  onOpenChange,
  plugins,
  agents,
  onAdded,
  initialChannelId,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  plugins: ChannelPluginInfo[]
  agents: AgentInfo[]
  onAdded: () => void
  initialChannelId?: string
}) {
  const { t } = useTranslation()
  const [step, setStep] = useState<"select" | "configure">("select")
  const [channelId, setChannelId] = useState("")
  const [label, setLabel] = useState("")

  // Sync initialChannelId when dialog opens
  useEffect(() => {
    if (open && initialChannelId) {
      setChannelId(initialChannelId)
      setStep("configure")
    }
  }, [open, initialChannelId])
  const [token, setToken] = useState("")
  // Slack-specific
  const [slackBotToken, setSlackBotToken] = useState("")
  const [slackAppToken, setSlackAppToken] = useState("")
  // Feishu-specific
  const [feishuAppId, setFeishuAppId] = useState("")
  const [feishuAppSecret, setFeishuAppSecret] = useState("")
  const [feishuDomain, setFeishuDomain] = useState("feishu")
  // QQ Bot-specific
  const [qqAppId, setQqAppId] = useState("")
  const [qqClientSecret, setQqClientSecret] = useState("")
  // IRC-specific
  const [ircServer, setIrcServer] = useState("")
  const [ircPort, setIrcPort] = useState("6697")
  const [ircTls, setIrcTls] = useState(true)
  const [ircNick, setIrcNick] = useState("")
  const [ircPassword, setIrcPassword] = useState("")
  const [ircNickservPassword, setIrcNickservPassword] = useState("")
  const [ircChannels, setIrcChannels] = useState("")
  // Signal-specific
  const [signalAccount, setSignalAccount] = useState("")
  // WhatsApp-specific
  const [whatsappBaseUrl, setWhatsappBaseUrl] = useState("")
  const [whatsappToken, setWhatsappToken] = useState("")
  // Google Chat-specific
  const [gchatCredentialsJson, setGchatCredentialsJson] = useState("")
  const [gchatWebhookUrl, setGchatWebhookUrl] = useState("")
  // LINE-specific
  const [lineAccessToken, setLineAccessToken] = useState("")
  const [lineChannelSecret, setLineChannelSecret] = useState("")
  const [lineWebhookUrl, setLineWebhookUrl] = useState("")
  const [agentId, setAgentId] = useState("")
  const [dmPolicy, setDmPolicy] = useState("open")
  const [userAllowlist, setUserAllowlist] = useState<string[]>([])
  const [allowlistInput, setAllowlistInput] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [validating, setValidating] = useState(false)
  const [validationResult, setValidationResult] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)
  const [wechatConnection, setWeChatConnection] = useState<WeChatConnection | null>(null)

  const selectedPlugin = plugins.find((p) => p.meta.id === channelId)
  const selectedAgent = agents.find((agent) => agent.id === agentId)

  const handleSelectChannel = (id: string) => {
    setChannelId(id)
    setStep("configure")
  }

  const handleBack = () => {
    setStep("select")
  }

  const buildCredentials = () => {
    switch (channelId) {
      case "slack":
        return { botToken: slackBotToken.trim(), appToken: slackAppToken.trim() }
      case "feishu":
        return { appId: feishuAppId.trim(), appSecret: feishuAppSecret.trim(), domain: feishuDomain }
      case "qqbot":
        return { appId: qqAppId.trim(), clientSecret: qqClientSecret.trim() }
      case "wechat":
        return {
          token: wechatConnection?.botToken ?? "",
          remoteAccountId: wechatConnection?.remoteAccountId ?? null,
          userId: wechatConnection?.userId ?? null,
        }
      case "irc":
        return {
          server: ircServer.trim(), port: parseInt(ircPort) || 6697, tls: ircTls,
          nick: ircNick.trim(), password: ircPassword.trim() || null,
          nickservPassword: ircNickservPassword.trim() || null,
          channels: ircChannels.trim() || null,
        }
      case "signal":
        return { account: signalAccount.trim() }
      case "imessage":
        return {}
      case "whatsapp":
        return { baseUrl: whatsappBaseUrl.trim(), token: whatsappToken.trim() || null }
      case "googlechat":
        return { credentialsJson: gchatCredentialsJson.trim(), webhookBaseUrl: gchatWebhookUrl.trim() || null }
      case "line":
        return { channelAccessToken: lineAccessToken.trim(), channelSecret: lineChannelSecret.trim(), webhookBaseUrl: lineWebhookUrl.trim() || null }
      default:
        return { token: token.trim() }
    }
  }

  const canValidate = () => {
    switch (channelId) {
      case "slack": return !!slackBotToken.trim()
      case "feishu": return !!feishuAppId.trim() && !!feishuAppSecret.trim()
      case "qqbot": return !!qqAppId.trim() && !!qqClientSecret.trim()
      case "wechat": return false
      case "irc": return !!ircServer.trim() && !!ircNick.trim()
      case "signal": return !!signalAccount.trim()
      case "imessage": return true
      case "whatsapp": return !!whatsappBaseUrl.trim()
      case "googlechat": return !!gchatCredentialsJson.trim()
      case "line": return !!lineAccessToken.trim() && !!lineChannelSecret.trim()
      default: return !!token.trim()
    }
  }

  const canSave = () => {
    if (!label.trim() || saving) return false
    switch (channelId) {
      case "slack": return !!slackBotToken.trim() && !!slackAppToken.trim()
      case "feishu": return !!feishuAppId.trim() && !!feishuAppSecret.trim()
      case "qqbot": return !!qqAppId.trim() && !!qqClientSecret.trim()
      case "wechat": return !!wechatConnection
      case "irc": return !!ircServer.trim() && !!ircNick.trim()
      case "signal": return !!signalAccount.trim()
      case "imessage": return true
      case "whatsapp": return !!whatsappBaseUrl.trim()
      case "googlechat": return !!gchatCredentialsJson.trim()
      case "line": return !!lineAccessToken.trim() && !!lineChannelSecret.trim()
      default: return !!token.trim()
    }
  }

  const handleValidate = async () => {
    if (!canValidate()) return
    setValidating(true)
    setValidationResult(null)
    setValidationError(null)
    try {
      const botName = await getTransport().call<string>("channel_validate_credentials", {
        channelId,
        credentials: buildCredentials(),
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
    if (!canSave()) return

    setSaving(true)
    setSaveError(null)
    try {
      const credentials = buildCredentials()

      const settings = channelId === "wechat"
        ? {
            transport: "longpoll",
            baseUrl: wechatConnection?.baseUrl ?? "",
          }
        : {}

      await getTransport().call("channel_add_account", {
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
      setSlackBotToken("")
      setSlackAppToken("")
      setFeishuAppId("")
      setFeishuAppSecret("")
      setFeishuDomain("feishu")
      setQqAppId("")
      setQqClientSecret("")
      setAgentId("")
      setDmPolicy("open")
      setUserAllowlist([])
      setAllowlistInput("")
      setGroupPolicy("open")
      setGroups({})
      setChannels({})
      setValidationResult(null)
      setValidationError(null)
      setSaveError(null)
      setWeChatConnection(null)
      onAdded()
    } catch (e) {
      logger.error("channel", "ChannelPanel", "Failed to add channel account", e)
      setSaveError(parseChannelSaveError(e, t))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => {
      if (!v) {
        setStep("select")
        setChannelId("")
        setSaveError(null)
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
                <Button
                  key={p.meta.id}
                  variant="outline"
                  onClick={() => handleSelectChannel(p.meta.id)}
                  className="h-auto justify-start gap-3 rounded-lg p-4 text-left font-normal hover:border-primary hover:bg-accent"
                >
                  <ChannelIcon channelId={p.meta.id} className="h-8 w-8" />
                  <div className="min-w-0">
                    <div className="font-medium">{t(`channels.pluginName_${p.meta.id}`, p.meta.displayName)}</div>
                    <div className="text-xs text-muted-foreground truncate">{t(`channels.pluginDesc_${p.meta.id}`, p.meta.description)}</div>
                  </div>
                </Button>
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
                  <DialogTitle>{t(`channels.pluginName_${channelId}`, selectedPlugin?.meta.displayName ?? channelId)}</DialogTitle>
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

              {/* Discord: single Bot Token */}
              {channelId === "discord" && (
                <div className="space-y-2">
                  <Label>Bot Token</Label>
                  <div className="flex gap-2">
                    <Input
                      type="password"
                      placeholder="MTIzNDU2Nzg5MDEyMzQ1Njc4OQ..."
                      value={token}
                      onChange={(e) => { setToken(e.target.value); setValidationResult(null); setValidationError(null) }}
                      onBlur={() => { if (token.trim() && !validationResult && !validating) handleValidate() }}
                      className="flex-1"
                    />
                    <Button variant="outline" size="sm" onClick={handleValidate} disabled={!token.trim() || validating}>
                      {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                    </Button>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.discordTokenHint", "Create a Bot at Discord Developer Portal and copy the token")}</p>
                </div>
              )}

              {/* Slack: Bot Token + App Token */}
              {channelId === "slack" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>Bot Token (xoxb-...)</Label>
                    <Input
                      type="password"
                      placeholder="xoxb-..."
                      value={slackBotToken}
                      onChange={(e) => { setSlackBotToken(e.target.value); setValidationResult(null); setValidationError(null) }}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>App Token (xapp-...)</Label>
                    <div className="flex gap-2">
                      <Input
                        type="password"
                        placeholder="xapp-..."
                        value={slackAppToken}
                        onChange={(e) => { setSlackAppToken(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.slackTokenHint", "Enable Socket Mode in your Slack App settings to get the App Token")}</p>
                </div>
              )}

              {/* Feishu: App ID + App Secret + Domain */}
              {channelId === "feishu" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>App ID</Label>
                    <Input
                      placeholder="cli_xxx"
                      value={feishuAppId}
                      onChange={(e) => { setFeishuAppId(e.target.value); setValidationResult(null); setValidationError(null) }}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>App Secret</Label>
                    <div className="flex gap-2">
                      <Input
                        type="password"
                        value={feishuAppSecret}
                        onChange={(e) => { setFeishuAppSecret(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("channels.feishuDomain", "Domain")}</Label>
                    <Select value={feishuDomain} onValueChange={setFeishuDomain}>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="feishu">Feishu (飞书)</SelectItem>
                        <SelectItem value="lark">Lark (International)</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.feishuTokenHint", "Create a Bot in Feishu Open Platform and get App ID / App Secret")}</p>
                </div>
              )}

              {/* QQ Bot: App ID + Client Secret */}
              {channelId === "qqbot" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>App ID</Label>
                    <Input
                      placeholder="102xxx"
                      value={qqAppId}
                      onChange={(e) => { setQqAppId(e.target.value); setValidationResult(null); setValidationError(null) }}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Client Secret</Label>
                    <div className="flex gap-2">
                      <Input
                        type="password"
                        value={qqClientSecret}
                        onChange={(e) => { setQqClientSecret(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.qqbotTokenHint", "Create a Bot at QQ Open Platform (q.qq.com) and get credentials")}</p>
                </div>
              )}

              {/* IRC: Server + Port + TLS + Nick + Password + Channels */}
              {channelId === "irc" && (
                <div className="space-y-3">
                  <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-2">
                      <Label>{t("channels.ircServer", "Server")}</Label>
                      <Input
                        placeholder="irc.libera.chat"
                        value={ircServer}
                        onChange={(e) => { setIrcServer(e.target.value); setValidationResult(null); setValidationError(null) }}
                      />
                    </div>
                    <div className="space-y-2">
                      <Label>{t("channels.ircPort", "Port")}</Label>
                      <Input
                        type="number"
                        placeholder="6697"
                        value={ircPort}
                        onChange={(e) => { setIrcPort(e.target.value); setValidationResult(null); setValidationError(null) }}
                      />
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <Switch checked={ircTls} onCheckedChange={setIrcTls} />
                    <Label>TLS</Label>
                  </div>
                  <div className="space-y-2">
                    <Label>Nick</Label>
                    <div className="flex gap-2">
                      <Input
                        placeholder="mybot"
                        value={ircNick}
                        onChange={(e) => { setIrcNick(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-2">
                      <Label>{t("channels.ircPassword", "Server Password")}</Label>
                      <Input type="password" value={ircPassword} onChange={(e) => setIrcPassword(e.target.value)} />
                    </div>
                    <div className="space-y-2">
                      <Label>{t("channels.ircNickserv", "NickServ Password")}</Label>
                      <Input type="password" value={ircNickservPassword} onChange={(e) => setIrcNickservPassword(e.target.value)} />
                    </div>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("channels.ircChannels", "Auto-join Channels")}</Label>
                    <Input placeholder="#channel1,#channel2" value={ircChannels} onChange={(e) => setIrcChannels(e.target.value)} />
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.ircHint", "Connect to any IRC server with optional TLS encryption")}</p>
                </div>
              )}

              {/* Signal: Phone number */}
              {channelId === "signal" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>{t("channels.signalAccount", "Phone Number")}</Label>
                    <div className="flex gap-2">
                      <Input
                        placeholder="+1234567890"
                        value={signalAccount}
                        onChange={(e) => { setSignalAccount(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.signalHint", "Requires signal-cli installed and registered locally. Run 'signal-cli link' or 'signal-cli register' first.")}</p>
                </div>
              )}

              {/* iMessage: minimal config */}
              {channelId === "imessage" && (
                <div className="space-y-3">
                  <div className="flex gap-2 items-center">
                    <Button variant="outline" size="sm" onClick={handleValidate} disabled={validating}>
                      {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                    </Button>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.imessageHint", "macOS only. Requires the imsg CLI tool installed and in your PATH.")}</p>
                </div>
              )}

              {/* WhatsApp: Bridge URL + Token */}
              {channelId === "whatsapp" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>{t("channels.whatsappBaseUrl", "Bridge Service URL")}</Label>
                    <div className="flex gap-2">
                      <Input
                        placeholder="http://localhost:3000"
                        value={whatsappBaseUrl}
                        onChange={(e) => { setWhatsappBaseUrl(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("channels.whatsappBridgeToken", "Auth Token (optional)")}</Label>
                    <Input type="password" value={whatsappToken} onChange={(e) => setWhatsappToken(e.target.value)} />
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.whatsappHint", "Requires a WhatsApp bridge service running. Point to its HTTP API URL.")}</p>
                </div>
              )}

              {/* Google Chat: Service Account JSON + Webhook URL */}
              {channelId === "googlechat" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>{t("channels.gchatCredentials", "Service Account JSON")}</Label>
                    <Textarea
                      rows={4}
                      placeholder='{"type": "service_account", "project_id": "...", ...}'
                      value={gchatCredentialsJson}
                      onChange={(e) => { setGchatCredentialsJson(e.target.value); setValidationResult(null); setValidationError(null) }}
                      className="font-mono text-xs"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>{t("channels.webhookUrl", "Public Webhook URL")}</Label>
                    <Input
                      placeholder="https://your-domain.com"
                      value={gchatWebhookUrl}
                      onChange={(e) => setGchatWebhookUrl(e.target.value)}
                    />
                    <p className="text-xs text-muted-foreground">{t("channels.webhookUrlHint", "Desktop apps need a public URL (e.g. ngrok) to receive webhooks")}</p>
                  </div>
                  <div className="flex gap-2">
                    <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                      {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                    </Button>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.gchatHint", "Requires a Google Workspace service account with Chat API enabled")}</p>
                </div>
              )}

              {/* LINE: Channel Access Token + Channel Secret + Webhook URL */}
              {channelId === "line" && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>Channel Access Token</Label>
                    <Input
                      type="password"
                      value={lineAccessToken}
                      onChange={(e) => { setLineAccessToken(e.target.value); setValidationResult(null); setValidationError(null) }}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Channel Secret</Label>
                    <div className="flex gap-2">
                      <Input
                        type="password"
                        value={lineChannelSecret}
                        onChange={(e) => { setLineChannelSecret(e.target.value); setValidationResult(null); setValidationError(null) }}
                        onBlur={() => { if (canValidate() && !validationResult && !validating) handleValidate() }}
                        className="flex-1"
                      />
                      <Button variant="outline" size="sm" onClick={handleValidate} disabled={!canValidate() || validating}>
                        {validating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("channels.testConnection")}
                      </Button>
                    </div>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("channels.webhookUrl", "Public Webhook URL")}</Label>
                    <Input
                      placeholder="https://your-domain.com"
                      value={lineWebhookUrl}
                      onChange={(e) => setLineWebhookUrl(e.target.value)}
                    />
                    <p className="text-xs text-muted-foreground">{t("channels.webhookUrlHint", "Desktop apps need a public URL (e.g. ngrok) to receive webhooks")}</p>
                  </div>
                  {validationResult && <div className="flex items-center gap-1 text-sm text-green-600"><Check className="h-3.5 w-3.5" />{validationResult}</div>}
                  {validationError && <div className="flex items-center gap-1 text-sm text-destructive"><AlertCircle className="h-3.5 w-3.5" />{validationError}</div>}
                  <p className="text-xs text-muted-foreground">{t("channels.lineHint", "Get credentials from LINE Developers Console (Messaging API channel)")}</p>
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
                    {selectedAgent ? (
                      <AgentSelectDisplay agent={selectedAgent} />
                    ) : (
                      <SelectValue placeholder={t("channels.boundAgentDefault")} />
                    )}
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">{t("channels.boundAgentDefault")}</SelectItem>
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

            <SaveErrorBanner message={saveError} />

            <DialogFooter>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t("common.cancel")}
              </Button>
              <Button
                onClick={handleSave}
                disabled={!canSave()}
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
