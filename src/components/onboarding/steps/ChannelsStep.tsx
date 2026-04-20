import { useTranslation } from "react-i18next"
import {
  MessageSquare,
  Send,
  Hash,
  Phone,
  Mail,
  Apple,
  Bot,
  Megaphone,
  Music2,
  Radio,
  Users,
  Sparkle,
} from "lucide-react"

interface ChannelsStepProps {
  /** Settings panel opener used by the "Configure in Settings" CTA. */
  onOpenSettings: () => void
}

interface ChannelMeta {
  id: string
  labelKey: string
  icon: React.ReactNode
  /** Whether the channel is supported on the current OS. */
  supported: boolean
}

function buildChannels(): ChannelMeta[] {
  const isMac = typeof navigator !== "undefined" && navigator.platform.includes("Mac")
  return [
    { id: "telegram", labelKey: "onboarding.channels.items.telegram", icon: <Send className="h-4 w-4" />, supported: true },
    { id: "discord", labelKey: "onboarding.channels.items.discord", icon: <Hash className="h-4 w-4" />, supported: true },
    { id: "slack", labelKey: "onboarding.channels.items.slack", icon: <Sparkle className="h-4 w-4" />, supported: true },
    { id: "feishu", labelKey: "onboarding.channels.items.feishu", icon: <Users className="h-4 w-4" />, supported: true },
    { id: "googlechat", labelKey: "onboarding.channels.items.googleChat", icon: <MessageSquare className="h-4 w-4" />, supported: true },
    { id: "line", labelKey: "onboarding.channels.items.line", icon: <MessageSquare className="h-4 w-4" />, supported: true },
    { id: "qqbot", labelKey: "onboarding.channels.items.qqbot", icon: <Bot className="h-4 w-4" />, supported: true },
    { id: "whatsapp", labelKey: "onboarding.channels.items.whatsapp", icon: <Phone className="h-4 w-4" />, supported: true },
    { id: "wechat", labelKey: "onboarding.channels.items.wechat", icon: <Megaphone className="h-4 w-4" />, supported: true },
    { id: "signal", labelKey: "onboarding.channels.items.signal", icon: <Music2 className="h-4 w-4" />, supported: true },
    { id: "irc", labelKey: "onboarding.channels.items.irc", icon: <Radio className="h-4 w-4" />, supported: true },
    { id: "imessage", labelKey: "onboarding.channels.items.imessage", icon: <Apple className="h-4 w-4" />, supported: isMac },
    { id: "email", labelKey: "onboarding.channels.items.email", icon: <Mail className="h-4 w-4" />, supported: true },
  ]
}

/**
 * Step 8 — IM channel discovery.
 *
 * This step deliberately does NOT embed the per-channel credential
 * forms. The original plan called for 12 bespoke dialogs, but those
 * already exist in the full Settings → Channels panel — duplicating
 * them here would ship ~2k lines of UI for a step most users skip. We
 * instead:
 *
 *   1. Make it obvious that connecting IM is optional and skippable.
 *   2. Show the full roster so users see what's possible.
 *   3. Route any click into the existing Channels settings panel via
 *      `onOpenSettings`, which preserves all current capability.
 *
 * When a user genuinely wants to connect a channel during onboarding,
 * they click any chip → land on the Channels panel → configure it →
 * close Settings → resume the wizard on Step 8 (state was persisted).
 */
export function ChannelsStep({ onOpenSettings }: ChannelsStepProps) {
  const { t } = useTranslation()
  const channels = buildChannels()

  return (
    <div className="px-6 py-6 space-y-5 max-w-2xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.channels.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.channels.subtitle")}</p>
      </div>

      <div className="grid gap-2 sm:grid-cols-3">
        {channels.map((c) => (
          <button
            key={c.id}
            type="button"
            disabled={!c.supported}
            onClick={onOpenSettings}
            className={`flex items-center gap-2 rounded-md border px-3 py-2 text-sm transition-colors ${
              c.supported
                ? "border-border hover:border-primary/40 hover:bg-primary/5"
                : "border-border/60 opacity-50 cursor-not-allowed"
            }`}
          >
            <span className="flex h-7 w-7 items-center justify-center rounded-md bg-muted text-muted-foreground">
              {c.icon}
            </span>
            <span className="flex-1 text-left">{t(c.labelKey)}</span>
            {!c.supported && (
              <span className="text-[10px] text-muted-foreground">
                {t("onboarding.channels.macOnly")}
              </span>
            )}
          </button>
        ))}
      </div>

      <p className="text-xs text-muted-foreground text-center">
        {t("onboarding.channels.hint")}
      </p>
    </div>
  )
}
