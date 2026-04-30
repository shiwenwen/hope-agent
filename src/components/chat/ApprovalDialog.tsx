import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ShieldAlert, ShieldCheck, FolderOpen, Clock } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"

export interface ApprovalRequest {
  request_id: string
  command: string
  cwd: string
  /**
   * Optional human-readable reason emitted by the engine when the approval
   * is forced by a protected-path / dangerous-command match. When set, the
   * dialog renders a red warning bar and disables the AllowAlways button.
   */
  reason?: {
    kind:
      | "edit_tool"
      | "edit_command"
      | "dangerous_command"
      | "protected_path"
      | "agent_custom_list"
      | "smart_judge"
    /** Pattern / path / rationale text to display. */
    detail?: string
  }
}

interface ApprovalDialogProps {
  requests: ApprovalRequest[]
  onRespond: (requestId: string, response: "allow_once" | "allow_always" | "deny") => void
}

const AUTO_DENY_DEFAULT_SECS = 300

export default function ApprovalDialog({ requests, onRespond }: ApprovalDialogProps) {
  const { t } = useTranslation()
  const [timeoutSecs, setTimeoutSecs] = useState<number | null>(null)
  const [autoAction, setAutoAction] = useState<"deny" | "proceed">("deny")
  const current = requests[0]
  const currentId = current?.request_id ?? null
  // Countdown: state is updated only inside the interval callback (not during
  // render), satisfying react-hooks/purity + react-hooks/set-state-in-effect.
  // The dialog shows `null` for ~1ms between mount and first tick — not
  // user-visible.
  const [remaining, setRemaining] = useState<number | null>(null)

  // Load the approval timeout once — the dialog only needs it for the
  // visual countdown; the actual timeout enforcement happens server-side.
  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<number>("get_approval_timeout").catch(() => AUTO_DENY_DEFAULT_SECS),
      getTransport()
        .call<"deny" | "proceed">("get_approval_timeout_action")
        .catch(() => "deny" as const),
    ]).then(([secs, action]) => {
      if (cancelled) return
      setTimeoutSecs(secs)
      setAutoAction(action)
    })
    return () => {
      cancelled = true
    }
  }, [])

  // Drive the countdown. setState lives inside the interval callback (not
  // the effect body), satisfying the new react-hooks/set-state-in-effect
  // lint. Stops itself when the timer hits zero.
  useEffect(() => {
    if (!currentId || timeoutSecs === null || timeoutSecs <= 0) return
    const startMs = Date.now()
    const total = timeoutSecs
    let id: number | null = null
    const tick = () => {
      const next = Math.max(0, total - Math.floor((Date.now() - startMs) / 1000))
      setRemaining(next)
      if (next <= 0 && id !== null) {
        window.clearInterval(id)
        id = null
      }
    }
    tick()
    id = window.setInterval(tick, 1000)
    return () => {
      if (id !== null) window.clearInterval(id)
    }
  }, [currentId, timeoutSecs])

  if (!current) return null

  const total = requests.length
  const reason = current.reason
  const isStrict = reason?.kind === "protected_path" || reason?.kind === "dangerous_command"

  return (
    <div className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm flex items-center justify-center">
      <div className="bg-card border border-border rounded-2xl shadow-xl max-w-md w-full mx-4 p-6 animate-in fade-in zoom-in-95 duration-200">
        {/* Header */}
        <div className="flex items-center gap-3 mb-4">
          <div
            className={`w-10 h-10 rounded-full flex items-center justify-center shrink-0 ${
              isStrict ? "bg-destructive/15 text-destructive" : "bg-amber-500/15 text-amber-500"
            }`}
          >
            {isStrict ? (
              <ShieldAlert className="h-5 w-5" />
            ) : (
              <ShieldCheck className="h-5 w-5" />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold text-foreground">{t("approval.title")}</h3>
            {total > 1 && (
              <span className="text-xs text-muted-foreground">
                {t("approval.queueIndicator", { current: 1, total })}
              </span>
            )}
          </div>
          {remaining !== null && (
            <CountdownRing
              remaining={remaining}
              total={timeoutSecs ?? AUTO_DENY_DEFAULT_SECS}
              autoAction={autoAction}
            />
          )}
        </div>

        {/* Reason banner (protected path / dangerous / edit / agent custom) */}
        {reason && (
          <ReasonBanner kind={reason.kind} detail={reason.detail} t={t} />
        )}

        {/* Working Directory */}
        <div className="mb-3">
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
            <FolderOpen className="h-3 w-3" />
            <span>{t("approval.workingDir")}</span>
          </div>
          <div className="text-xs text-foreground/70 font-mono bg-secondary/50 rounded-lg px-2.5 py-1.5 truncate">
            {current.cwd}
          </div>
        </div>

        {/* Command */}
        <div className="mb-5">
          <div className="text-xs text-muted-foreground mb-1">{t("approval.command")}</div>
          <pre className="text-sm text-foreground font-mono bg-secondary rounded-lg p-3 whitespace-pre-wrap break-all max-h-40 overflow-y-auto leading-relaxed">
            {current.command}
          </pre>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="text-red-400 hover:text-red-300 border-red-500/30 hover:border-red-500/50 hover:bg-red-500/10"
            onClick={() => onRespond(current.request_id, "deny")}
          >
            {t("approval.deny")}
          </Button>
          <div className="flex-1" />
          <Button
            variant="secondary"
            size="sm"
            onClick={() => onRespond(current.request_id, "allow_once")}
          >
            {t("approval.allowOnce")}
          </Button>
          <Button
            size="sm"
            onClick={() => onRespond(current.request_id, "allow_always")}
            disabled={isStrict}
            title={isStrict ? t("approval.allowAlwaysDisabled") : undefined}
          >
            {t("approval.allowAlways")}
          </Button>
        </div>
      </div>
    </div>
  )
}

// ── CountdownRing ───────────────────────────────────────────────────

function CountdownRing({
  remaining,
  total,
  autoAction,
}: {
  remaining: number
  total: number
  autoAction: "deny" | "proceed"
}) {
  const { t } = useTranslation()
  const ratio = total <= 0 ? 0 : Math.max(0, Math.min(1, remaining / total))
  const isUrgent = remaining <= 30
  const stroke = 3
  const size = 28
  const r = (size - stroke) / 2
  const c = 2 * Math.PI * r
  const offset = c * (1 - ratio)
  const color = isUrgent ? "stroke-destructive" : "stroke-primary"

  return (
    <div
      className="relative shrink-0"
      title={t("approval.countdownTooltip", {
        seconds: remaining,
        action: t(`approval.countdownAction.${autoAction}`),
      })}
    >
      <svg width={size} height={size} className="rotate-[-90deg]">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={stroke}
          className="stroke-secondary fill-none"
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={stroke}
          strokeDasharray={c}
          strokeDashoffset={offset}
          className={`${color} fill-none transition-[stroke-dashoffset]`}
          strokeLinecap="round"
        />
      </svg>
      <div className="absolute inset-0 flex items-center justify-center">
        <Clock className={`h-3 w-3 ${isUrgent ? "text-destructive" : "text-muted-foreground"}`} />
      </div>
    </div>
  )
}

// ── ReasonBanner ───────────────────────────────────────────────────

function ReasonBanner({
  kind,
  detail,
  t,
}: {
  kind: NonNullable<ApprovalRequest["reason"]>["kind"]
  detail?: string
  t: ReturnType<typeof useTranslation>["t"]
}) {
  const isStrict = kind === "protected_path" || kind === "dangerous_command"
  const palette = isStrict
    ? "border-destructive/40 bg-destructive/10 text-destructive"
    : "border-amber-200/40 bg-amber-50/40 dark:bg-amber-950/10 text-amber-700 dark:text-amber-400"

  return (
    <div
      className={`mb-3 rounded-md border px-2.5 py-1.5 text-[11px] flex items-start gap-2 ${palette}`}
    >
      <ShieldAlert className="h-3.5 w-3.5 mt-0.5 shrink-0" />
      <span>
        <strong className="font-medium">{t(`approval.reasons.${kind}.title`)}</strong>
        {detail && (
          <>
            {" — "}
            <code className="font-mono">{detail}</code>
          </>
        )}
        <span className="block opacity-80">{t(`approval.reasons.${kind}.body`)}</span>
      </span>
    </div>
  )
}
