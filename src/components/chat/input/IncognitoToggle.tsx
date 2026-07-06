import { Ghost, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { INCOGNITO_TOGGLE_ON_CLASSES } from "./incognitoStyles"

export type IncognitoDisabledReason = "project" | "channel"

const DISABLED_REASON_KEY: Record<IncognitoDisabledReason, string> = {
  project: "chat.incognitoProjectExclusive",
  channel: "chat.incognitoChannelExclusive",
}

interface IncognitoToggleProps {
  sessionId: string | null
  enabled: boolean
  saving?: boolean
  disabledReason?: IncognitoDisabledReason
  variant?: "toolbar" | "titlebar"
  showLabel?: boolean
  onChange: (enabled: boolean) => void
}

export default function IncognitoToggle({
  sessionId,
  enabled,
  saving = false,
  disabledReason,
  variant = "toolbar",
  showLabel = true,
  onChange,
}: IncognitoToggleProps) {
  const { t } = useTranslation()
  const disabled = disabledReason !== undefined

  const tooltip = disabled
    ? t(DISABLED_REASON_KEY[disabledReason] ?? "chat.incognitoMutuallyExclusive")
    : enabled
      ? t("chat.incognitoDisable", { defaultValue: "Turn off incognito chat" })
      : t(sessionId ? "chat.incognito" : "chat.incognitoPreset")
  const titlebar = variant === "titlebar"

  return (
    <IconTip label={tooltip}>
      <button
        type="button"
        aria-label={showLabel ? t("chat.incognito") : tooltip}
        disabled={saving || disabled}
        onClick={() => onChange(!enabled)}
        className={cn(
          titlebar
            ? "pb-1.5 text-muted-foreground hover:text-foreground transition-colors disabled:cursor-not-allowed disabled:opacity-50"
            : "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50",
          saving && "disabled:cursor-wait disabled:opacity-70",
          enabled && !disabled
            ? titlebar
              ? "text-slate-700 hover:text-slate-900 dark:text-slate-200 dark:hover:text-white"
              : INCOGNITO_TOGGLE_ON_CLASSES
            : "text-muted-foreground hover:text-foreground",
        )}
      >
        {saving ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Ghost className="h-4 w-4" strokeWidth={1.75} />
        )}
        {showLabel && <span>{t("chat.incognito")}</span>}
      </button>
    </IconTip>
  )
}
