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
  onChange: (enabled: boolean) => void
}

export default function IncognitoToggle({
  sessionId,
  enabled,
  saving = false,
  disabledReason,
  onChange,
}: IncognitoToggleProps) {
  const { t } = useTranslation()
  const disabled = disabledReason !== undefined

  const tooltip = disabled
    ? t(DISABLED_REASON_KEY[disabledReason] ?? "chat.incognitoMutuallyExclusive")
    : t(sessionId ? "chat.incognito" : "chat.incognitoPreset")

  return (
    <IconTip label={tooltip}>
      <button
        type="button"
        disabled={saving || disabled}
        onClick={() => onChange(!enabled)}
        className={cn(
          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50",
          saving && "disabled:cursor-wait disabled:opacity-70",
          enabled && !disabled
            ? INCOGNITO_TOGGLE_ON_CLASSES
            : "text-muted-foreground hover:text-foreground",
        )}
      >
        {saving ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <Ghost className="h-3.5 w-3.5" />
        )}
        <span>{t("chat.incognito")}</span>
      </button>
    </IconTip>
  )
}
