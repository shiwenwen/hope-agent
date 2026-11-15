import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { getTransportRevision, useTransport, useTransportRevision } from "@/lib/transport-provider"
import {
  hasAssistantNotificationPermission,
  isAppWindowFocused,
  subscribeAppWindowFocus,
  submitAssistantNotification,
  type NotificationConfig,
} from "@/lib/notifications"
import { ownerRequestId } from "./useAssistantResearch"

/** One listener per app. Presence suppresses external pushes, without marking
 * anything viewed. The host owns dedupe and quota across every owner client. */
export function useAssistantAttention(enabled: boolean, homeActive: boolean, onOpen: () => void) {
  const transport = useTransport()
  const revision = useTransportRevision()
  const { t } = useTranslation()
  const latest = useRef({ homeActive, onOpen, t })
  latest.current = { homeActive, onOpen, t }
  const presenceRefresh = useRef<(() => void) | null>(null)
  useEffect(() => presenceRefresh.current?.(), [homeActive])

  useEffect(() => {
    if (!enabled) return
    const clientId = ownerRequestId()
    let alive = true
    let busy = false
    const current = () => alive && revision === getTransportRevision()
    const foreground = () => document.visibilityState === "visible" && isAppWindowFocused()
    const presence = () => {
      if (!current()) return
      void transport
        .call("set_personal_assistant_presence", {
          clientId,
          active: latest.current.homeActive && foreground(),
        })
        .catch(() => {})
    }
    presenceRefresh.current = presence
    const cycle = async () => {
      if (!current() || busy || foreground()) return
      busy = true
      const claimKey = ownerRequestId()
      try {
        // Fetch on the captured transport: an old connection's global cached
        // notification preferences cannot disclose this connection's content.
        const config = await transport.call<NotificationConfig>("get_notification_config")
        if (
          !current() ||
          foreground() ||
          !config.enabled ||
          !(await hasAssistantNotificationPermission())
        )
          return
        if (!current() || foreground()) return
        const notice = await transport.call<{ id: string; title: string; body: string } | null>(
          "claim_personal_assistant_desktop_notice",
          { clientId, claimKey },
        )
        if (!notice) return
        const outcome =
          !current() || foreground()
            ? "not_submitted"
            : await submitAssistantNotification(
                notice.title,
                config.showChatContent === false
                  ? latest.current.t("personalAssistant.noticePolicy.privateNotice")
                  : notice.body,
                () => {
                  if (current()) latest.current.onOpen()
                },
              )
        await transport.call("finish_personal_assistant_desktop_notice", {
          id: notice.id,
          clientId,
          claimKey,
          outcome,
        })
      } catch {
        // A claim or submission with an unknown outcome is never retried.
        // The immutable host receipt remains the authority on later cycles.
      } finally {
        busy = false
      }
    }
    const tick = () => {
      presence()
      void cycle()
    }
    const focusOff = subscribeAppWindowFocus(tick)
    document.addEventListener("visibilitychange", tick)
    const eventOff = transport.listen("personal_assistant:attention_changed", () => void cycle())
    const timer = setInterval(tick, 10_000)
    tick()
    return () => {
      alive = false
      presenceRefresh.current = null
      clearInterval(timer)
      focusOff()
      eventOff()
      document.removeEventListener("visibilitychange", tick)
      void transport
        .call("set_personal_assistant_presence", { clientId, active: false })
        .catch(() => {})
    }
  }, [enabled, transport, revision])
}
