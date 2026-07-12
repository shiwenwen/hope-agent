import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ShieldAlert, ShieldCheck, FolderOpen, Clock, EyeOff } from "lucide-react"

export interface ApprovalRequest {
  request_id: string
  command: string
  cwd: string
  /** Backend wire field used to keep approvals scoped to their chat session. */
  session_id?: string | null
  /**
   * Optional human-readable reason emitted by the permission engine. Strict
   * reasons render a red warning bar and disable AllowAlways; soft reasons use
   * the normal approval flow.
   */
  reason?: {
    kind:
      | "edit_tool"
      | "edit_command"
      | "dangerous_command"
      | "protected_path"
      | "agent_custom_list"
      | "smart_judge"
      | "browser_evaluate"
      | "browser_raw_cdp"
      | "browser_chrome_access"
      | "browser_download_action"
      | "mac_control_action"
      | "mac_control_dangerous_action"
      | "external_connector_action"
      | "plan_mode_ask"
      | "cron_delete"
    /** Pattern / path / rationale text to display. */
    detail?: string
  }
  /**
   * When true the owning session is incognito: the AllowAlways button is hidden
   * (a persistent grant would outlive the burn-on-close). The backend also
   * forces any AllowAlways to in-memory session scope. Epic E (INCOG-6).
   */
  incognito?: boolean
  /** Authoritative request creation/deadline metadata from the backend. */
  created_at_ms?: number
  server_now_ms?: number
  timeout_at_ms?: number | null
  /** Client-clock deadline translated from timeout_at_ms + server_now_ms. */
  local_timeout_at_ms?: number | null
  timeout_secs?: number
  timeout_action?: "deny" | "proceed"
}

/**
 * Reason kinds that bar AllowAlways and render the destructive (red) palette.
 * Single source of truth shared by the dialog header and the ReasonBanner in
 * this file — mirrors the backend `ApprovalReasonKind::is_strict` strict set, so
 * the two never disagree (e.g. on `plan_mode_ask`). File-private: keeping it
 * unexported satisfies react-refresh/only-export-components (a component file
 * must export only components).
 */
function isStrictReasonKind(
  kind: NonNullable<ApprovalRequest["reason"]>["kind"] | undefined,
): boolean {
  return (
    kind === "protected_path" ||
    kind === "dangerous_command" ||
    kind === "browser_raw_cdp" ||
    kind === "mac_control_dangerous_action" ||
    kind === "external_connector_action" ||
    kind === "plan_mode_ask"
  )
}

/**
 * Reason kinds that bar the AllowAlways button. Strict reasons bar it (per-call
 * confirmation), and `cron_delete` also bars it without being strict: the
 * allowlist matcher for `manage_cron` keys on `action` only (not the job id), so
 * a persisted AllowAlways grant would silently authorize deleting ANY scheduled
 * task forever. Cron delete stays non-strict (normal palette + normal
 * timeout/unattended handling) — it just never offers a standing grant. Mirrors
 * the backend `gate_cron_delete` forcing `allow_always_forbidden=true`.
 */
function barsAllowAlways(
  kind: NonNullable<ApprovalRequest["reason"]>["kind"] | undefined,
): boolean {
  return isStrictReasonKind(kind) || kind === "cron_delete"
}

interface ApprovalDialogProps {
  requests: ApprovalRequest[]
  onRespond: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => void | Promise<void>
}

export default function ApprovalDialog({ requests, onRespond }: ApprovalDialogProps) {
  const { t } = useTranslation()
  const [respondingId, setRespondingId] = useState<string | null>(null)
  const respondingIdsRef = useRef<Set<string>>(new Set())
  const current = requests[0]

  if (!current) return null

  const total = requests.length
  const reason = current.reason
  // `isStrict` drives the destructive (red) header palette; `allowAlwaysBarred`
  // gates the AllowAlways button. They differ for `cron_delete`: non-strict
  // (amber, normal timeout/unattended) but still bars AllowAlways.
  const isStrict = isStrictReasonKind(reason?.kind)
  const allowAlwaysBarred = barsAllowAlways(reason?.kind)
  // E5 (INCOG-6): incognito sessions never persist an AllowAlways grant — hide
  // the button entirely and explain why below the actions.
  const incognito = current.incognito === true
  const isResponding = respondingId === current.request_id

  const respond = async (response: "allow_once" | "allow_always" | "deny") => {
    const requestId = current.request_id
    if (respondingIdsRef.current.has(requestId)) return
    respondingIdsRef.current.add(requestId)
    setRespondingId(requestId)
    try {
      await onRespond(requestId, response)
    } finally {
      respondingIdsRef.current.delete(requestId)
      setRespondingId((active) => (active === requestId ? null : active))
    }
  }

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
            {isStrict ? <ShieldAlert className="h-5 w-5" /> : <ShieldCheck className="h-5 w-5" />}
          </div>
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold text-foreground">{t("approval.title")}</h3>
            {total > 1 && (
              <span className="text-xs text-muted-foreground">
                {t("approval.queueIndicator", { current: 1, total })}
              </span>
            )}
          </div>
          {typeof current.local_timeout_at_ms === "number" && current.local_timeout_at_ms > 0 && (
            <CountdownRing
              key={current.request_id}
              deadlineAtMs={current.local_timeout_at_ms}
              total={
                current.timeout_secs ??
                Math.max(
                  1,
                  Math.ceil(
                    ((current.timeout_at_ms ?? current.local_timeout_at_ms) -
                      (current.created_at_ms ?? current.server_now_ms ?? Date.now())) /
                      1000,
                  ),
                )
              }
              autoAction={current.timeout_action ?? "deny"}
            />
          )}
        </div>

        {/* Reason banner */}
        {reason && <ReasonBanner kind={reason.kind} detail={reason.detail} t={t} />}

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
            onClick={() => void respond("deny")}
            disabled={isResponding}
          >
            {t("approval.deny")}
          </Button>
          <div className="flex-1" />
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void respond("allow_once")}
            disabled={isResponding}
          >
            {t("approval.allowOnce")}
          </Button>
          {!incognito && (
            <Button
              size="sm"
              onClick={() => void respond("allow_always")}
              disabled={allowAlwaysBarred || isResponding}
              title={allowAlwaysBarred ? t("approval.allowAlwaysDisabled") : undefined}
            >
              {t("approval.allowAlways")}
            </Button>
          )}
        </div>

        {/* Incognito notice: AllowAlways is unavailable because nothing persists. */}
        {incognito && (
          <p className="mt-3 flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <EyeOff className="h-3 w-3 shrink-0" />
            {t("approval.incognitoNoAllowAlways")}
          </p>
        )}
      </div>
    </div>
  )
}

// ── CountdownRing ───────────────────────────────────────────────────

function CountdownRing({
  deadlineAtMs,
  total,
  autoAction,
}: {
  deadlineAtMs: number
  total: number
  autoAction: "deny" | "proceed"
}) {
  const { t } = useTranslation()
  const [remaining, setRemaining] = useState(() =>
    Math.max(0, Math.ceil((deadlineAtMs - Date.now()) / 1000)),
  )

  useEffect(() => {
    let id: number | null = null
    const tick = () => {
      const next = Math.max(0, Math.ceil((deadlineAtMs - Date.now()) / 1000))
      setRemaining(next)
      if (next <= 0 && id !== null) {
        window.clearInterval(id)
        id = null
      }
    }
    id = window.setInterval(tick, 250)
    return () => {
      if (id !== null) window.clearInterval(id)
    }
  }, [deadlineAtMs])
  const ratio = total <= 0 ? 0 : Math.max(0, Math.min(1, remaining / total))
  const isUrgent = remaining <= 30
  const stroke = 3
  const size = 28
  const r = (size - stroke) / 2
  const c = 2 * Math.PI * r
  const offset = c * (1 - ratio)
  const color = isUrgent ? "stroke-destructive" : "stroke-primary"
  const timeLabel = formatCountdownTime(remaining)

  return (
    <div
      className="flex shrink-0 items-center gap-1.5"
      title={t("approval.countdownTooltip", {
        seconds: remaining,
        action: t(`approval.countdownAction.${autoAction}`),
      })}
    >
      <span
        className={`min-w-9 text-right text-[11px] font-medium tabular-nums ${
          isUrgent ? "text-destructive" : "text-muted-foreground"
        }`}
      >
        {timeLabel}
      </span>
      <div className="relative h-7 w-7 shrink-0">
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
    </div>
  )
}

function formatCountdownTime(seconds: number): string {
  const safe = Math.max(0, Math.ceil(seconds))
  const s = safe % 60
  const m = Math.floor(safe / 60) % 60
  const h = Math.floor(safe / 3600)

  if (h > 0) {
    return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`
  }
  if (safe >= 60) {
    return `${m}:${s.toString().padStart(2, "0")}`
  }
  return `${safe}s`
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
  const isStrict = isStrictReasonKind(kind)
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
