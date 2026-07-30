import { useEffect, useState } from "react"
import { ArrowRight, FastForward, LoaderCircle, RefreshCw } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { ModelRecoveryEvent } from "@/types/chat"
import { failoverReasonKey } from "./failoverReason"

type RecoveryAction = "skip_wait" | "switch_model"

interface RecoveryControlResult {
  applied: boolean
  reason?: string
}

interface ModelRecoveryBannerProps {
  event: ModelRecoveryEvent
  sessionId?: string | null
}

export default function ModelRecoveryBanner({ event, sessionId }: ModelRecoveryBannerProps) {
  const { t } = useTranslation()
  const isChainRetry = event.type === "model_chain_retry"
  const reason = t(`chat.${failoverReasonKey(event.reason)}`)
  const model = event.model?.trim()
  const delayMs = Math.max(0, event.delay_ms ?? 0)
  const [deadline] = useState(() => Date.now() + delayMs)
  const [remainingMs, setRemainingMs] = useState(delayMs)
  const [pendingAction, setPendingAction] = useState<RecoveryAction | null>(null)
  const [actionApplied, setActionApplied] = useState(false)
  const Icon = isChainRetry ? RefreshCw : LoaderCircle

  useEffect(() => {
    if (delayMs <= 0) return

    const timer = window.setInterval(() => {
      const next = Math.max(0, deadline - Date.now())
      setRemainingMs(next)
      if (next === 0) window.clearInterval(timer)
    }, 200)
    return () => window.clearInterval(timer)
  }, [deadline, delayMs])

  const remainingSeconds = Math.max(0, Math.ceil(remainingMs / 1000))
  const progress = delayMs > 0 ? (remainingMs / delayMs) * 100 : 0
  const canControl = !!sessionId && !!event.recovery_id && remainingMs > 0 && !actionApplied

  const applyAction = async (action: RecoveryAction) => {
    if (!sessionId || !event.recovery_id || pendingAction) return
    setPendingAction(action)
    try {
      const result = await getTransport().call<RecoveryControlResult>("control_model_recovery", {
        sessionId,
        recoveryId: event.recovery_id,
        action,
      })
      if (result.applied) {
        setActionApplied(true)
        setRemainingMs(0)
      } else {
        // The wait already elapsed or a newer recovery step replaced it.
        // Retiring this card avoids presenting an action that can no longer
        // affect the active request.
        setRemainingMs(0)
      }
    } catch {
      // Keep the still-live action available after a transient transport
      // failure; the countdown remains the source of truth for expiry.
    } finally {
      setPendingAction(null)
    }
  }

  return (
    <div
      className={cn(
        "mb-1.5 inline-flex max-w-full flex-col gap-1.5 rounded-lg border px-2.5 py-1.5 text-[11px]",
        "border-blue-500/25 bg-blue-500/[0.07] text-muted-foreground",
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <Icon
          className={cn(
            "h-3 w-3 shrink-0 text-blue-500",
            remainingMs > 0 && !actionApplied && "animate-spin",
          )}
        />
        <span className="shrink-0 font-medium text-foreground/75">
          {t(isChainRetry ? "chat.modelRecoveryRoundTitle" : "chat.modelRetryTitle")}
        </span>
        {model && (
          <>
            <span className="shrink-0 opacity-30">·</span>
            <span className="truncate font-semibold text-foreground/90">{model}</span>
          </>
        )}
        {event.attempt != null && event.total != null && (
          <span className="shrink-0 tabular-nums text-[10px] opacity-60">
            {event.attempt}/{event.total}
          </span>
        )}
        <span className="shrink-0 opacity-30">·</span>
        <span className="truncate opacity-65">{reason}</span>
        <span className="shrink-0 tabular-nums opacity-55">{remainingSeconds}s</span>
      </div>

      {delayMs > 0 && (
        <Progress value={progress} className="h-1 bg-blue-500/10 [&>div]:bg-blue-500/70" />
      )}

      {canControl && (
        <div className="flex items-center justify-end gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-[10px] text-blue-600 hover:bg-blue-500/10 hover:text-blue-700 dark:text-blue-400 dark:hover:text-blue-300"
            disabled={pendingAction != null}
            onClick={() => void applyAction("skip_wait")}
          >
            {pendingAction === "skip_wait" ? (
              <LoaderCircle className="mr-1 h-3 w-3 animate-spin" />
            ) : (
              <FastForward className="mr-1 h-3 w-3" />
            )}
            {t(isChainRetry ? "chat.recoveryStartNow" : "chat.recoverySkipWait")}
          </Button>
          {!isChainRetry && event.can_switch_model && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-[10px] text-foreground/70 hover:bg-blue-500/10 hover:text-foreground"
              disabled={pendingAction != null}
              onClick={() => void applyAction("switch_model")}
            >
              {pendingAction === "switch_model" ? (
                <LoaderCircle className="mr-1 h-3 w-3 animate-spin" />
              ) : (
                <ArrowRight className="mr-1 h-3 w-3" />
              )}
              {t("chat.recoverySwitchModel")}
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
