import { useCallback, useEffect, useRef, useState } from "react"
import { getTransportRevision, useTransport, useTransportRevision } from "@/lib/transport-provider"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"

export interface AssistantPolicySettings {
  desktopToImNotices: boolean
  researchEnabled: boolean
  paused: boolean
  timezone: string | null
  noticeRoute: "none" | "desktop" | "im"
  quietStartHour: number
  quietEndHour: number
}
export interface AssistantPolicyStatus {
  requestedRevision: number
  appliedRevision: number
  settings: AssistantPolicySettings
  pending: boolean
  failed: boolean
}
export interface AssistantAttentionStatus {
  pairing: { connectionId: string; generation: number } | null
  paired: boolean
  deferredReason: string | null
  nextEligibleAt: number | null
  pendingCount: number
  receipts: { id: string; route: string; state: string; submittedAt: number; itemCount: number }[]
}
export interface AssistantSourceInput {
  sessionId: string
  relativePath: string
  focus: string
  durationDays: number
}
export interface AssistantSource {
  id: string
  sessionId: string
  relativePath: string
  focus: string
  revision: number
  version: string | null
  state: "active" | "revoked"
  expiresAt: number
  lastObservedAt: number | null
  gapSince: number | null
  error: boolean
  suspended: boolean
  imSummaryAllowed: boolean
  imDisclosureRevision: number
}
export interface SourceReference {
  sourceId: string
  revision: number
  version: string
  startLine: number
  endLine: number
}
export interface AssistantFinding {
  id: string
  revision: number
  fact: string
  inference: string
  uncertainty: string
  prepared: string
  references: SourceReference[]
  createdAt: string
  viewedAt: string | null
}

export interface ResearchQueryReceipt {
  id: string
  state: string
  reason: string | null
}
export interface ResearchActivity extends ResearchQueryReceipt {
  findingId: string | null
  createdAt: number
}

export function researchActivityKey(activity: ResearchQueryReceipt): string {
  const reasons: Record<string, string> = {
    expired: "expired",
    source_changed: "scopeChanged",
    scope_changed: "scopeChanged",
    budget_exhausted: "budget",
    time_exhausted: "unverified",
    restart_unverified: "unverified",
    execution_unverified: "unverified",
    missing_report: "unverified",
  }
  const states = new Set(["pending", "running", "finished", "failed", "cancelled"])
  const key =
    (activity.reason && reasons[activity.reason]) ||
    (states.has(activity.state) ? activity.state : "failed")
  return `personalAssistant.research.activity.${key}`
}

export function ownerRequestId(): string {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID()
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = (bytes[6] & 15) | 64
  bytes[8] = (bytes[8] & 63) | 128
  const hex = Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("")
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

const empty = {
  sources: [] as AssistantSource[],
  findings: [] as AssistantFinding[],
  activity: [] as ResearchActivity[],
  policy: null as AssistantPolicyStatus | null,
  attention: null as AssistantAttentionStatus | null,
}

export function useAssistantResearch(enabled: boolean) {
  const transport = useTransport()
  const revision = useTransportRevision()
  const [data, setData] = useState({ ...empty, revision })
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const requestVersion = useRef(0)
  const lifetime = useRef(0)
  const alive = useRef(false)
  const allowed = useRef(enabled)
  allowed.current = enabled
  const nonces = useRef(new Map<string, string>())
  const current = useCallback(
    () => alive.current && allowed.current && revision === getTransportRevision(),
    [revision],
  )
  const refresh = useCallback(
    async (invalidate = false) => {
      const ticket = ++requestVersion.current
      if (!current()) return
      if (invalidate) setData((old) => ({ ...old, findings: [] }))
      setLoading(true)
      try {
        const [sources, findings, policy, activity, attention] = await Promise.all([
          transport.call<AssistantSource[]>("list_personal_assistant_sources"),
          transport.call<AssistantFinding[]>("list_personal_assistant_findings"),
          transport.call<AssistantPolicyStatus>("get_personal_assistant_policy"),
          transport.call<ResearchActivity[]>("personal_assistant_research_activity"),
          transport.call<AssistantAttentionStatus>("get_personal_assistant_attention"),
        ])
        if (!current() || ticket !== requestVersion.current) return
        setData({ sources, findings, policy, activity, attention, revision })
        setError(null)
      } catch (cause) {
        if (!current() || ticket !== requestVersion.current) return
        setData({ ...empty, revision })
        setError(String(cause))
      } finally {
        if (current() && ticket === requestVersion.current) setLoading(false)
      }
    },
    [current, transport, revision],
  )

  useEffect(() => {
    alive.current = enabled
    lifetime.current++
    requestVersion.current++
    nonces.current.clear()
    setData({ ...empty, revision })
    setError(null)
    setLoading(false)
    if (!enabled) return
    void refresh()
    let eventTimer: ReturnType<typeof setTimeout> | undefined
    let invalidateNext = false
    const schedule = (invalidate: boolean) => {
      invalidateNext ||= invalidate
      if (invalidate) {
        requestVersion.current++
        setData((old) => ({ ...old, findings: [] }))
      }
      if (eventTimer !== undefined) return
      eventTimer = setTimeout(() => {
        eventTimer = undefined
        const clear = invalidateNext
        invalidateNext = false
        void refresh(clear)
      }, 100)
    }
    const unlisten = transport.listen("personal_assistant:research_changed", (payload) =>
      schedule((payload as { invalidate?: boolean } | null)?.invalidate !== false),
    )
    const resync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, () => schedule(true))
    const onVisible = () => {
      if (document.visibilityState === "visible") void refresh(true)
    }
    document.addEventListener("visibilitychange", onVisible)
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void refresh()
    }, 10_000)
    return () => {
      alive.current = false
      lifetime.current++
      requestVersion.current++
      unlisten()
      resync()
      clearInterval(timer)
      clearTimeout(eventTimer)
      document.removeEventListener("visibilitychange", onVisible)
    }
  }, [enabled, transport, refresh, revision])

  const action = useCallback(
    async <T>(
      command: string,
      args: Record<string, unknown>,
      invalidate = false,
      idempotent = false,
    ) => {
      if (!current()) throw new Error("Assistant connection changed")
      const generation = lifetime.current
      const identity = JSON.stringify([command, args])
      const key = idempotent ? (nonces.current.get(identity) ?? ownerRequestId()) : undefined
      if (key) nonces.current.set(identity, key)
      if (invalidate) {
        requestVersion.current++
        setData((old) => ({ ...old, findings: [] }))
      }
      const input =
        command === "add_personal_assistant_source"
          ? { source: { ...args, requestKey: key } }
          : { ...args, ...(key ? { requestKey: key } : {}) }
      const result = await transport.call<T>(command, input)
      if (!current() || generation !== lifetime.current)
        throw new Error("Assistant connection changed")
      if (key) nonces.current.delete(identity)
      await refresh()
      return result
    },
    [current, transport, refresh],
  )

  const addSource = useCallback(
    (input: AssistantSourceInput) =>
      action<AssistantSource>("add_personal_assistant_source", { ...input }, true, true),
    [action],
  )
  const revokeSource = useCallback(
    (id: string) => action<void>("revoke_personal_assistant_source", { id }, true),
    [action],
  )
  const refreshSource = useCallback(
    (id: string) =>
      action<ResearchQueryReceipt>("check_personal_assistant_source", { id }, false, true),
    [action],
  )
  const viewFinding = useCallback(
    (id: string, findingRevision: number) =>
      action<void>("view_personal_assistant_finding", { id, revision: findingRevision }),
    [action],
  )
  const askFinding = useCallback(
    (id: string, findingRevision: number, question: string) =>
      action<ResearchQueryReceipt>(
        "ask_personal_assistant_finding",
        { id, revision: findingRevision, question },
        false,
        true,
      ),
    [action],
  )
  const savePolicy = useCallback(
    (expectedRevision: number, settings: AssistantPolicySettings) =>
      action<AssistantPolicyStatus>(
        "set_personal_assistant_policy",
        { expectedRevision, settings },
        true,
      ),
    [action],
  )
  const setSourceDisclosure = useCallback(
    (source: AssistantSource, enabled: boolean) => {
      const pairing = data.attention?.pairing
      if (!pairing) return Promise.reject(new Error("Pair your assistant IM first"))
      return action<void>(
        "set_personal_assistant_source_im_disclosure",
        {
          id: source.id,
          revision: source.revision,
          enabled,
          expectedDisclosureRevision: source.imDisclosureRevision,
          expectedConnectionId: pairing.connectionId,
          expectedGeneration: pairing.generation,
        },
        true,
      )
    },
    [data.attention, action],
  )
  const visible = enabled && data.revision === revision ? data : { ...empty, revision }
  return {
    ...visible,
    error: visible === data ? error : null,
    loading: enabled && loading,
    refresh,
    addSource,
    revokeSource,
    refreshSource,
    viewFinding,
    askFinding,
    savePolicy,
    setSourceDisclosure,
  }
}

export type AssistantResearchState = ReturnType<typeof useAssistantResearch>
