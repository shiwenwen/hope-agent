import { useCallback, useEffect, useRef, useState } from "react"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { getTransportRevision, useTransport, useTransportRevision } from "@/lib/transport-provider"

export interface AssistantImConnection {
  connectionId: string
  botId: number
  botUsername: string
  generation: number
  state: "unpaired" | "awaiting_code" | "awaiting_confirmation" | "awaiting_activation" | "active"
  userId?: number | null
  chatId?: number | null
  notificationsPaused: boolean
}

export interface AssistantImGrant {
  sessionId: string
  revision: number
  queryStatus: boolean
  delegate: boolean
  stop: boolean
  completionNotice: boolean
  expiresAt: number
}

export interface AssistantImDelivery {
  id: string
  requestId: string
  state: "pending" | "submitting" | "accepted" | "unknown" | "cancelled"
  messageId?: number | null
  createdAt: number
  deferredReason: "timezone_required" | "quiet_hours" | "daily_limit" | null
}

export interface AssistantImSnapshot {
  connection: AssistantImConnection | null
  grants: AssistantImGrant[]
  deliveries: AssistantImDelivery[]
  notificationPolicy: {
    timezone: string | null
    quietStartHour: number
    quietEndHour: number
    dailyLimit: number
  }
}

export interface AssistantImPairingCode {
  connectionId: string
  generation: number
  code: string
  expiresAt: number
}

export function usePersonalAssistantIm(enabled = true) {
  const transport = useTransport()
  const revision = useTransportRevision()
  const requestVersion = useRef(0)
  const failedRead = useRef(false)
  const lifecycleVersion = useRef(0)
  const mutationInFlight = useRef(false)
  const feedbackTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const [snapshot, setSnapshot] = useState<{ data: AssistantImSnapshot; revision: number } | null>(
    null,
  )
  const [loading, setLoading] = useState(true)
  const [pending, setPending] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<"saved" | "failed" | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [checkedAt, setCheckedAt] = useState(Date.now)

  const refresh = useCallback(async () => {
    const version = ++requestVersion.current
    setLoading(true)
    const current = () => revision === getTransportRevision() && version === requestVersion.current
    try {
      const data = await transport.call<AssistantImSnapshot>("personal_assistant_im_status")
      if (!current()) return null
      failedRead.current = false
      setSnapshot({ data, revision })
      setError(null)
      return data
    } catch (cause) {
      if (current()) {
        failedRead.current = true
        setSnapshot(null)
        setError(String(cause))
      }
      return null
    } finally {
      if (current()) {
        setCheckedAt(Date.now())
        setLoading(false)
      }
    }
  }, [transport, revision])

  useEffect(() => {
    lifecycleVersion.current++
    mutationInFlight.current = false
    requestVersion.current++
    setSnapshot(null)
    setPending(null)
    setFeedback(null)
    setError(null)
    failedRead.current = false
    if (!enabled) return
    const verify = () => {
      if (document.visibilityState === "visible") void refresh()
    }
    const changed = () => {
      if (!failedRead.current) verify()
    }
    const offChanged = transport.listen("personal_assistant_im:changed", changed)
    const offResync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, verify)
    document.addEventListener("visibilitychange", verify)
    void refresh()
    const timer = feedbackTimer
    return () => {
      // Invalidate mutations independently of status event refreshes.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      lifecycleVersion.current++
      // This numeric generation invalidates in-flight reads at teardown.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      requestVersion.current++
      clearTimeout(timer.current)
      offChanged()
      offResync()
      document.removeEventListener("visibilitychange", verify)
    }
  }, [enabled, transport, revision, refresh])

  const mutate = useCallback(
    async <T>(action: string, args?: Record<string, unknown>): Promise<T | undefined> => {
      if (!enabled || mutationInFlight.current) return undefined
      mutationInFlight.current = true
      const lifecycle = lifecycleVersion.current
      const current = () =>
        revision === getTransportRevision() && lifecycle === lifecycleVersion.current
      setPending(action)
      setFeedback(null)
      setError(null)
      clearTimeout(feedbackTimer.current)
      let mutationError: string | null = null
      let result: T | undefined
      try {
        result = await transport.call<T>("personal_assistant_im_" + action, args)
      } catch (cause) {
        mutationError = String(cause)
      }
      if (!current()) return undefined
      await refresh()
      if (!current()) return undefined
      mutationInFlight.current = false
      setPending(null)
      setFeedback(mutationError || failedRead.current ? "failed" : "saved")
      if (mutationError) setError(mutationError)
      feedbackTimer.current = setTimeout(() => setFeedback(null), 2000)
      return mutationError ? undefined : result
    },
    [enabled, transport, revision, refresh],
  )

  const data = enabled && snapshot?.revision === revision ? snapshot.data : null
  return { data, loading, pending, feedback, error, revision, checkedAt, refresh, mutate }
}
