import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"

/**
 * Inline marker for a text/thinking block whose backing row was left in
 * `streaming` / `orphaned` state by a previous (crashed) run. Caller
 * controls indent / size via `className`.
 */
export default function InterruptedMark({ className }: { className?: string }) {
  const { t } = useTranslation()
  return (
    <div className={cn("mt-1 text-xs text-muted-foreground italic", className)}>
      {t("chat.interrupted_partial")}
    </div>
  )
}
