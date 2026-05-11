import { useTranslation } from "react-i18next"
import { PlayCircle, AlertTriangle } from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import type { RoundLimitReachedEvent } from "@/types/chat"

interface RoundLimitReachedBannerProps {
  event: RoundLimitReachedEvent
  onResume?: (message: string) => void
}

export default function RoundLimitReachedBanner({
  event,
  onResume,
}: RoundLimitReachedBannerProps) {
  const { t } = useTranslation()
  const maxRounds = typeof event.max_rounds === "number" ? event.max_rounds : undefined

  return (
    <div
      className={cn(
        "flex w-full max-w-[85%] items-start gap-3 rounded-lg border px-3.5 py-3",
        "border-amber-500/25 bg-amber-500/[0.07]",
      )}
    >
      <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium text-foreground">
          {t("chat.roundLimitReachedTitle")}
        </div>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          {maxRounds != null
            ? t("chat.roundLimitReachedBody", { count: maxRounds })
            : t("chat.roundLimitReachedBodyNoCount")}
        </p>
        <Button
          size="sm"
          variant="outline"
          disabled={!onResume}
          onClick={() => onResume?.(t("chat.roundLimitResumeMessage"))}
          className="mt-2.5 gap-1.5 border-amber-500/40 bg-amber-500/[0.04] text-amber-700 hover:bg-amber-500/15 dark:text-amber-300"
        >
          <PlayCircle className="h-3.5 w-3.5" />
          {t("chat.roundLimitResumeButton")}
        </Button>
      </div>
    </div>
  )
}
