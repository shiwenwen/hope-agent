import { useRef, useState } from "react"
import { Clock, FolderOpen, ShieldAlert, ShieldCheck } from "lucide-react"
import { useTranslation } from "react-i18next"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import { approvalBarsAllowAlways, isStrictApprovalReason } from "@/components/chat/approvalPolicy"
import { Button } from "@/components/ui/button"
import { formatRemaining, useCountdownRemainingSec } from "@/lib/countdown"
import { cn } from "@/lib/utils"

interface PetApprovalCardProps {
  request: ApprovalRequest
  onRespond: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => void | Promise<void>
  queuePosition?: { current: number; total: number }
  measuring?: boolean
}

export function PetApprovalCard({
  request,
  onRespond,
  queuePosition,
  measuring,
}: PetApprovalCardProps) {
  const { t } = useTranslation()
  const [responding, setResponding] = useState(false)
  const respondingRef = useRef(false)
  const remaining = useCountdownRemainingSec(request.local_timeout_at_ms ?? null)
  const timedOut = remaining !== null && remaining <= 0
  const strict = isStrictApprovalReason(request.reason?.kind)
  const allowAlways = !request.incognito && !approvalBarsAllowAlways(request.reason?.kind)

  const respond = async (response: "allow_once" | "allow_always" | "deny") => {
    if (respondingRef.current || timedOut || measuring) return
    respondingRef.current = true
    setResponding(true)
    try {
      await onRespond(request.request_id, response)
    } finally {
      respondingRef.current = false
      setResponding(false)
    }
  }

  return (
    <section
      aria-label={t("approval.title", { defaultValue: "Approval needed" })}
      aria-hidden={measuring || undefined}
      className="w-[360px] rounded-[18px] border border-border/80 bg-popover/95 p-2.5 text-popover-foreground shadow-lg backdrop-blur-md"
    >
      <header className="flex min-h-5 items-center gap-1.5 text-xs">
        {strict ? (
          <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-destructive" />
        ) : (
          <ShieldCheck className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
        )}
        <h2 className="truncate font-semibold">
          {t("approval.title", { defaultValue: "Approval needed" })}
        </h2>
        {request.reason && (
          <span
            className={cn(
              "truncate text-[10px]",
              strict ? "text-destructive" : "text-muted-foreground",
            )}
          >
            ·{" "}
            {t(`approval.reasons.${request.reason.kind}.title`, {
              defaultValue: request.reason.kind,
            })}
          </span>
        )}
        {queuePosition && queuePosition.total > 1 && (
          <span className="shrink-0 text-[10px] text-muted-foreground">
            · {queuePosition.current}/{queuePosition.total}
          </span>
        )}
        {remaining !== null && (
          <span
            className={cn(
              "ml-auto flex shrink-0 items-center gap-1 text-[10px] tabular-nums text-muted-foreground",
              remaining <= 10 && "text-destructive",
            )}
          >
            <Clock className="h-3 w-3" />
            {formatRemaining(Math.max(0, remaining))}
          </span>
        )}
      </header>

      {request.reason?.detail && (
        <p className="mt-1 line-clamp-1 text-[10px] text-muted-foreground">
          {request.reason.detail}
        </p>
      )}

      <div className="mt-1.5 rounded-lg bg-secondary/70 px-2 py-1.5">
        <pre className="max-h-16 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-4 text-foreground">
          {request.command}
        </pre>
        {request.cwd && (
          <div className="mt-1 flex items-center gap-1 text-[9px] text-muted-foreground">
            <FolderOpen className="h-3 w-3 shrink-0" />
            <span className="truncate font-mono">{request.cwd}</span>
          </div>
        )}
      </div>

      <footer className="mt-2 flex items-center justify-end gap-1.5">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={responding || timedOut || measuring}
          onClick={() => void respond("deny")}
          className="h-7 rounded-full px-2.5 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
        >
          {t("approval.deny", { defaultValue: "Deny" })}
        </Button>
        {allowAlways && (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={responding || timedOut || measuring}
            onClick={() => void respond("allow_always")}
            className="h-7 rounded-full px-2.5 text-xs"
          >
            {t("approval.allowAlways", { defaultValue: "Always allow" })}
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          disabled={responding || timedOut || measuring}
          onClick={() => void respond("allow_once")}
          className="h-7 rounded-full px-2.5 text-xs"
        >
          {t("approval.allowOnce", { defaultValue: "Allow once" })}
        </Button>
      </footer>
    </section>
  )
}
