import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { MCP_EVENTS } from "@/lib/mcp"
import { notifyIfBackground } from "@/lib/notifications"
import { logger } from "@/lib/logger"

// Truncate user-visible strings (commands, questions) so notifications
// don't blow past Notification Center's character limit.
function truncate(s: string, max = 80): string {
  if (!s) return ""
  return s.length > max ? `${s.slice(0, max - 1)}…` : s
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
          const result = build(parsePayload<T>(raw))
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

    const offAskUser = bindAlert<{ questions?: Array<{ text?: string }> }>(
      "ask_user_request",
      "useDesktopAlerts::ask_user",
      (group) => {
        const tx = tRef.current
        const firstQ = truncate(group?.questions?.[0]?.text ?? "")
        return {
          title: tx("notification.askUserRequired"),
          body: firstQ || tx("notification.askUserRequiredFallback"),
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

    return () => {
      offApproval()
      offAskUser()
      offMcpAuth()
      offChannelAuth()
    }
  }, [])
}
