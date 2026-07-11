import { useTranslation } from "react-i18next"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"
import type { FallbackEvent } from "@/types/chat"

/** Map backend FailoverReason snake_case to i18n key suffix */
const REASON_KEYS: Record<string, string> = {
  rate_limit: "reasonRateLimit",
  overloaded: "reasonOverloaded",
  timeout: "reasonTimeout",
  auth: "reasonAuth",
  billing: "reasonBilling",
  model_not_found: "reasonModelNotFound",
  context_overflow: "reasonContextOverflow",
  unknown: "reasonUnknown",
}

export default function FallbackDetailsPopover({
  event,
  open,
}: {
  event: FallbackEvent
  open: boolean
}) {
  const { t } = useTranslation()

  const reasonKey = event.reason ? REASON_KEYS[event.reason] || REASON_KEYS["unknown"] : null
  const reasonText = reasonKey ? t(`chat.${reasonKey}`) : null

  return (
    <FloatingMenu
      open={open}
      positionClassName="top-full left-0 mt-1.5"
      originClassName="origin-top-left"
      className="ha-menu-from-top min-w-[280px] p-2.5"
    >
      <div className="space-y-1.5 text-xs">
        {event.from_model && (
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">{t("chat.fallbackFrom")}</span>
            <span className="font-medium text-foreground truncate max-w-[140px]">
              {event.from_model}
            </span>
          </div>
        )}
        <div className="flex items-center justify-between gap-3">
          <span className="text-muted-foreground">{t("chat.fallbackTo")}</span>
          <span className="font-medium text-foreground truncate max-w-[140px]">{event.model}</span>
        </div>
        {reasonText && (
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">{t("chat.fallbackReason")}</span>
            <span
              className={cn(
                "font-medium",
                event.reason === "rate_limit" ||
                  event.reason === "overloaded" ||
                  event.reason === "timeout"
                  ? "text-amber-600 dark:text-amber-400"
                  : "text-red-500 dark:text-red-400",
              )}
            >
              {reasonText}
            </span>
          </div>
        )}
        {event.attempt != null && event.total != null && (
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">{t("chat.fallbackProgress")}</span>
            <span className="font-medium text-foreground tabular-nums">
              {event.attempt} / {event.total}
            </span>
          </div>
        )}
        {event.error && (
          <>
            <div className="border-t border-border" />
            <div>
              <span className="text-muted-foreground">{t("chat.fallbackError")}</span>
              <div className="mt-0.5 px-2 py-1 rounded bg-muted/50 text-muted-foreground font-mono text-[10px] leading-relaxed break-all max-h-[80px] overflow-y-auto">
                {event.error}
              </div>
            </div>
          </>
        )}
      </div>
    </FloatingMenu>
  )
}
