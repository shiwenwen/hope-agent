import { Ghost, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { INCOGNITO_TOGGLE_ON_CLASSES } from "./incognitoStyles"

interface IncognitoToggleProps {
  sessionId: string | null
  enabled: boolean
  saving?: boolean
  onChange: (enabled: boolean) => void
}

export default function IncognitoToggle({
  sessionId,
  enabled,
  saving = false,
  onChange,
}: IncognitoToggleProps) {
  const { t } = useTranslation()

  return (
    <IconTip
      label={
        sessionId
          ? t("chat.incognito")
          : t("chat.incognitoPreset")
      }
    >
      <button
        type="button"
        disabled={saving}
        onClick={() => onChange(!enabled)}
        className={cn(
          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap disabled:cursor-wait disabled:opacity-70",
          enabled
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
