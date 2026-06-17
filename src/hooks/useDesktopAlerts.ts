import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { MCP_EVENTS } from "@/lib/mcp"
import { getCachedConfig, notifyIfBackground } from "@/lib/notifications"
import { logger } from "@/lib/logger"

// Background-job terminal statuses worth a "跑完叫我" notification — skip
// user-cancelled / restart-interrupted (not noteworthy outcomes).
const NOTIFIABLE_JOB_STATUSES = new Set(["completed", "failed", "timed_out"])

// Truncate user-visible strings (commands, questions) so notifications
// don't blow past Notification Center's character limit.
function truncate(s: string, max = 80): string {
  if (!s) return ""
  return s.length > max ? `${s.slice(0, max - 1)}…` : s
}

function textPreview(value: unknown): string {
  if (typeof value === "string") return value
  if (value && typeof value === "object") {
    const record = value as { fallback?: unknown; key?: unknown }
    if (typeof record.fallback === "string") return record.fallback
    if (typeof record.key === "string") return record.key
  }
  return ""
}

// Leading-edge cooldown for channel auth failures. When several IM
// accounts fail at the same instant (e.g. system clock jump invalidates
// every bot token at once, or post-resume DNS hasn't recovered for any
// of them), the watchdog emits one event per account back-to-back. We
// surface only the first inside this window — the user just needs to
// know "go check your IM channels," not get N back-to-back popups.
const CHANNEL_AUTH_COOLDOWN_MS = 1500

export function useDesktopAlerts() {
  const { t } = useTranslation()
  // Keep `t` in a ref so the listener effect can re-read the current
  // translation function without re-subscribing on every i18n change.
  const tRef = useRef(t)
  useEffect(() => {
    tRef.current = t
  }, [t])

  useEffect(() => {
    const transport = getTransport()
    let lastChannelAuthAt = 0

    function bindAlert<T>(
      event: string,
      source: string,
      build: (p: T) => { title: string; body: string } | null,
    ): () => void {
      return transport.listen(event, (raw) => {
        try {
          const parsed = parsePayload<T>(raw)
          if (!parsed) return
          const result = build(parsed)
          if (result) notifyIfBackground(result.title, result.body)
        } catch (e) {
          logger.error("ui", source, `Bad ${event} payload`, e)
        }
      })
    }

    const offApproval = bindAlert<{ command?: string }>(
      "approval_required",
      "useDesktopAlerts::approval",
      (req) => {
        const tx = tRef.current
        const body = truncate(req?.command ?? "")
        return {
          title: tx("notification.approvalRequired"),
          body: body || tx("notification.approvalRequiredFallback"),
        }
      },
    )

    const offApprovalTimedOut = bindAlert<{
      timeout_secs?: number
      timeout_action?: "deny" | "proceed"
    }>(
      "approval_timed_out",
      "useDesktopAlerts::approval_timeout",
      (event) => {
        const tx = tRef.current
        const bodyKey =
          event?.timeout_action === "proceed"
            ? "notification.approvalTimedOut_proceeded"
            : "notification.approvalTimedOut_denied"
        return {
          title: tx("notification.approvalTimedOut"),
          body: tx(bodyKey, {
            seconds: event?.timeout_secs ?? 0,
          }),
        }
      },
    )

    const offAskUser = bindAlert<{ questions?: Array<{ text?: unknown }> }>(
      "ask_user_request",
      "useDesktopAlerts::ask_user",
      (group) => {
        const tx = tRef.current
        const firstQ = truncate(textPreview(group?.questions?.[0]?.text))
        return {
          title: tx("notification.askUserRequired"),
          body: firstQ || tx("notification.askUserRequiredFallback"),
        }
      },
    )

    const offAskUserTimedOut = bindAlert<{
      timeoutSecs?: number
      usedDefaultValues?: boolean
      questionPreview?: string
    }>(
      "ask_user_timed_out",
      "useDesktopAlerts::ask_user_timeout",
      (event) => {
        const tx = tRef.current
        const preview = truncate(event?.questionPreview ?? "")
        const body = tx(
          event?.usedDefaultValues
            ? "notification.askUserTimedOutDefaults"
            : "notification.askUserTimedOutNoDefaults",
          { seconds: event?.timeoutSecs ?? 0 },
        )
        return {
          title: tx("notification.askUserTimedOut"),
          body: preview ? `${preview} — ${body}` : body,
        }
      },
    )

    const offMcpAuth = bindAlert<{ name?: string }>(
      MCP_EVENTS.AUTH_REQUIRED,
      "useDesktopAlerts::mcp_auth",
      (ev) => {
        const tx = tRef.current
        const name = ev?.name ?? ""
        return {
          title: tx("notification.mcpAuthRequired"),
          body: name
            ? tx("notification.mcpAuthRequiredBody", { name })
            : tx("notification.mcpAuthRequiredFallback"),
        }
      },
    )

    const offChannelAuth = bindAlert<{ label?: string; channelId?: string }>(
      "channel:auth_failed",
      "useDesktopAlerts::channel_auth",
      (ev) => {
        const now = Date.now()
        if (now - lastChannelAuthAt < CHANNEL_AUTH_COOLDOWN_MS) return null
        lastChannelAuthAt = now
        const tx = tRef.current
        const label = ev?.label || ev?.channelId || ""
        return {
          title: tx("notification.channelAuthFailed"),
          body: label
            ? tx("notification.channelAuthFailedBody", { label })
            : tx("notification.channelAuthFailedFallback"),
        }
      },
    )

    // R4: "跑完叫我" — a background job (tool / group, R3 `job:*`) finishing in
    // the background fires a desktop notification, gated by the dedicated
    // `notifyOnBackgroundJobComplete` toggle. Subagent jobs ride `subagent:*` and
    // are out of scope here. Background-only via `notifyIfBackground` (the user
    // already sees the panel/badge when the window is up front).
    const offJobCompleted = bindAlert<{
      tool?: string
      status?: string
      kind?: string
    }>("job:completed", "useDesktopAlerts::job_completed", (ev) => {
      if (getCachedConfig()?.notifyOnBackgroundJobComplete === false) return null
      const status = ev?.status ?? ""
      if (!NOTIFIABLE_JOB_STATUSES.has(status)) return null
      const tx = tRef.current
      const failed = status !== "completed"
      // A Group's `tool` is the internal id "subagent:batch" — show a friendly
      // localized label instead; tool jobs surface their real tool name.
      const body =
        ev?.kind === "group"
          ? tx("backgroundJobs.kindGroup", "任务组")
          : truncate(ev?.tool ?? "") ||
            tx("notification.backgroundJobFallback", "一个后台任务已结束")
      return {
        title: failed
          ? tx("notification.backgroundJobFailed", "后台任务失败")
          : tx("notification.backgroundJobComplete", "后台任务完成"),
        body,
      }
    })

    return () => {
      offApproval()
      offApprovalTimedOut()
      offAskUser()
      offAskUserTimedOut()
      offMcpAuth()
      offChannelAuth()
      offJobCompleted()
    }
  }, [])
}
