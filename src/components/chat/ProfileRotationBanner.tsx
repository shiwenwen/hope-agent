import { useTranslation } from "react-i18next"
import { RotateCw } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ProfileRotationEvent } from "@/types/chat"
import { failoverReasonKey } from "./failoverReason"

/** Inline auth-profile rotation banner — compact amber chip. Mirrors the
 *  visual language of {@link FallbackBanner} (same provider-failover family)
 *  but doesn't open a details popover: rotation events are point-in-time,
 *  not failure-with-context. */
export default function ProfileRotationBanner({ event }: { event: ProfileRotationEvent }) {
  const { t } = useTranslation()
  const toLabel = event.to_profile?.trim() || "—"
  const reason = event.reason?.trim()
    ? t(`chat.${failoverReasonKey(event.reason)}`)
    : ""

  return (
    <div
      className={cn(
        "mb-1.5 inline-flex max-w-full items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px]",
        "border-amber-500/25 bg-amber-500/[0.07] text-muted-foreground",
      )}
    >
      <RotateCw className="h-3 w-3 shrink-0 text-amber-500" />
      <span className="shrink-0 font-medium text-foreground/75">
        {t("chat.profileRotationTitle")}
      </span>
      <span className="shrink-0 opacity-30">·</span>
      <span className="truncate font-semibold text-foreground/90">{toLabel}</span>
      {reason && (
        <>
          <span className="shrink-0 opacity-30">·</span>
          <span className="shrink-0 opacity-60">{reason}</span>
        </>
      )}
    </div>
  )
}
