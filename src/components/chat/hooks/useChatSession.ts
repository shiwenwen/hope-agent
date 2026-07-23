import { useState, useRef, useEffect, useCallback } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { notify } from "@/lib/notifications"
import {
  capMessagesAndSyncCursors,
  materializeMessages,
  reloadAndMergeSessionMessages,
} from "../chatUtils"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { useSessionPagination } from "./useSessionPagination"
import { useChannelStreaming } from "./useChannelStreaming"
import { PAGE_SIZE, SESSION_CACHE_LRU_LIMIT } from "./constants"
import { resolveAvailableDisplayModel } from "../modelSelection"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  SessionMessage,
  AgentSummaryForSidebar,
  SubagentEvent,
} from "@/types/chat"
import type { AgentConfig } from "@/components/settings/types"
import { confirmDiscardDirtyFileEditors } from "../files/fileDirtyRegistry"

export { PAGE_SIZE, SESSION_PAGE_SIZE } from "./constants"

export interface UseChatSessionReturn {
  // State
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  /** False while the selected transcript is loading or its load has failed. */
  currentSessionContentReady: boolean
  sessions: SessionMeta[]
  totalUnreadCount: number
  agents: AgentSummaryForSidebar[]
  currentAgentId: string
  setCurrentAgentId: React.Dispatch<React.SetStateAction<string>>
  agentName: string
  setAgentName: React.Dispatch<React.SetStateAction<string>>
  historyLoading: boolean
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionIds: Set<string>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  hasMore: boolean
  loadingMore: boolean
  hasMoreAfter: boolean
  loadingMoreAfter: boolean
  sessionsLoading: boolean
  /**
   * Search-jump intent for MessageList: which message to scroll to + which
   * literal substrings to inline-highlight inside it. `null` between jumps.
   * Consumer calls `clearPendingScrollIntent` once the scroll has been
   * applied. Single object so the (target, terms) invariant can't drift —
   * terms are always tied to a specific scroll target.
   */
  pendingScrollIntent: { messageId: number; highlightTerms: string[] | null } | null
  clearPendingScrollIntent: () => void
  /**
   * Scroll the current session to a specific message and briefly highlight
   * it. If the target is not in the currently loaded window, reloads a
   * window of messages centred on the target first. Used by the in-chat
   * "find in page" search bar. Optional `highlightTerms` are painted inline
   * inside the target bubble via the CSS Custom Highlight API.
   */
  jumpToMessage: (messageId: number, highlightTerms?: string[]) => Promise<void>

  // Refs
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>
  /** Bound a session's `messages` array to the dynamic cap; used by
   *  `useChatStream` after appending a user msg / assistant placeholder. */
  capMessagesForSession: (sessionId: string, msgs: Message[]) => Message[]
  /** Bump session in LRU; used at session-cache write sites that don't
   *  otherwise route through `handleSwitchSession`. */
  touchSessionCacheLru: (sessionId: string) => void

  // Handlers
  reloadSessions: () => Promise<void>
  reloadAgents: () => Promise<void>
  handleToggleSessionPinned: (sessionId: string, pinned: boolean) => Promise<void>
  handleReorderAgents: (agentIds: string[]) => Promise<void>
  handleSwitchSession: (
    sessionId: string,
    opts?: { targetMessageId?: number; highlightTerms?: string[] },
  ) => Promise<void>
  handleNewChat: (agentId: string) => Promise<void>
  handleArchiveSession: (sessionId: string) => Promise<void>
  handleLoadMore: () => Promise<void>
  handleLoadMoreAfter: () => Promise<void>
  /** Drop the partial around-window and reload the latest page. */
  resetToLatest: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  updateSessionMeta: (sessionId: string, updater: (prev: SessionMeta) => SessionMeta) => void
}

interface UseChatSessionOptions {
  availableModels: AvailableModel[]
  setActiveModel: React.Dispatch<React.SetStateAction<ActiveModel | null>>
  globalActiveModelRef: React.MutableRefObject<ActiveModel | null>
  applyModelForDisplay: (key: string) => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  onSidebarAggregatesChanged?: () => void
  /** Current chat is selected, focused, visible, and following its latest tail. */
  activeSessionReadable: boolean
  /** Ref form for transport callbacks that must avoid stale render closures. */
  activeSessionReadableRef: React.MutableRefObject<boolean>
}

function sortSessionsForSidebar(sessions: SessionMeta[]): SessionMeta[] {
  return sessions.slice().sort((a, b) => {
    const aPinned = a.pinnedAt ? Date.parse(a.pinnedAt) || 0 : 0
    const bPinned = b.pinnedAt ? Date.parse(b.pinnedAt) || 0 : 0
    if (aPinned !== bPinned) return bPinned - aPinned
    return (Date.parse(b.updatedAt) || 0) - (Date.parse(a.updatedAt) || 0)
  })
}

export function useChatSession({
  availableModels,
  setActiveModel,
  globalActiveModelRef,
  applyModelForDisplay,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  onSidebarAggregatesChanged,
  activeSessionReadable,
  activeSessionReadableRef,
}: UseChatSessionOptions): UseChatSessionReturn {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [totalUnreadCount, setTotalUnreadCount] = useState(0)
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [currentAgentId, setCurrentAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [agentName, setAgentName] = useState("")
  const [historyLoading, setHistoryLoading] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [pendingScrollIntent, setPendingScrollIntent] = useState<{
    messageId: number
    highlightTerms: string[] | null
  } | null>(null)
  const clearPendingScrollIntent = useCallback(() => setPendingScrollIntent(null), [])

  const currentSessionIdRef = useRef<string | null>(null)
  const historyLoadingRef = useRef(false)
  const failedSessionLoadsRef = useRef(new Set<string>())
  const switchVersionRef = useRef(0)
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const hasMoreRef = useRef<Map<string, boolean>>(new Map())
  const hasMoreAfterRef = useRef<Map<string, boolean>>(new Map())
  const oldestDbIdRef = useRef<Map<string, number>>(new Map())
  const newestDbIdRef = useRef<Map<string, number>>(new Map())
  const userPaginatedDepthRef = useRef<Map<string, number>>(new Map())
  // Mirror of `messages` so `jumpToMessage` can synchronously check whether
  // a target message is already loaded without stale-closure hazards.
  const messagesRef = useRef<Message[]>([])
  // Mirror of `sessions` so callbacks reading session metadata don't have to
  // list `sessions` in their deps (which would invalidate them on every
  // streaming meta tick and cascade re-renders into the sidebar tree).
  const sessionsRef = useRef<SessionMeta[]>([])
  const agentsRef = useRef<AgentSummaryForSidebar[]>([])
  // Tracks the previous `currentSessionId` so the effect below can fire
  // `purge_session_if_incognito` exactly once per swap.
  const previousSessionIdRef = useRef<string | null>(null)

  const updateHistoryLoading = useCallback((value: boolean) => {
    historyLoadingRef.current = value
    setHistoryLoading(value)
  }, [])

  // Keep ref in sync with state
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  useEffect(() => {
    messagesRef.current = messages
  }, [messages])

  useEffect(() => {
    sessionsRef.current = sessions
  }, [sessions])

  useEffect(() => {
    agentsRef.current = agents
  }, [agents])

  // --- Session pagination sub-hook ---
  const {
    hasMore,
    setHasMore,
    loadingMore,
    hasMoreAfter,
    setHasMoreAfter,
    loadingMoreAfter,
    sessionsLoading,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    reloadSessions: reloadSessionPage,
  } = useSessionPagination({
    currentSessionIdRef,
    sessionCacheRef,
    hasMoreRef,
    hasMoreAfterRef,
    oldestDbIdRef,
    newestDbIdRef,
    userPaginatedDepthRef,
    sessionsRef,
    setSessions,
    setMessages,
  })

  const reloadUnreadTotal = useCallback(async () => {
    try {
      const currentSessionId = currentSessionIdRef.current
      const currentSessionCanRead =
        activeSessionReadableRef.current &&
        !historyLoadingRef.current &&
        !!currentSessionId &&
        !failedSessionLoadsRef.current.has(currentSessionId)
      const count = await getTransport().call<number>("regular_unread_total_cmd", {
        activeSessionId: currentSessionCanRead ? currentSessionId : undefined,
      })
      setTotalUnreadCount(Number.isFinite(count) && count > 0 ? Math.floor(count) : 0)
    } catch (e) {
      logger.warn("session", "ChatScreen::loadUnreadTotal", "Failed to load unread total", e)
    }
  }, [activeSessionReadableRef])

  // Session rows remain paginated for rendering, while the badge aggregate is
  // queried independently across the whole database. This prevents pinned or
  // older conversations outside the first page from disappearing from Dock /
  // global unread counts.
  const reloadSessions = useCallback(async () => {
    await Promise.all([reloadSessionPage(), reloadUnreadTotal()])
  }, [reloadSessionPage, reloadUnreadTotal])

  // --- Channel streaming sub-hook ---
  useChannelStreaming({
    currentSessionIdRef,
    sessionCacheRef,
    loadingSessionsRef,
    setMessages,
    setLoading,
    setLoadingSessionIds,
    reloadSessions,
    activeSessionReadableRef,
  })

  /** Update messages for a specific session. If it's the current session, also update state. */
  const updateSessionMessages = useCallback(
    (sessionId: string, updater: (prev: Message[]) => Message[]) => {
      const hasCached = sessionCacheRef.current.has(sessionId)
      const prev = sessionCacheRef.current.get(sessionId) || []
      const next = updater(prev)
      if (
        !hasCached &&
        currentSessionIdRef.current !== sessionId &&
        next === prev &&
        next.length === 0
      ) {
        return
      }
      sessionCacheRef.current.set(sessionId, next)
      // Track the tail dbId so handleLoadMoreAfter has a fresh anchor even
      // after streaming appends. Otherwise the after-pagination cursor stays
      // pinned to whatever was loaded at switch time and the second
      // handleLoadMoreAfter call would re-fetch already-displayed rows.
      const tail = next.length > 0 ? next[next.length - 1] : null
      if (tail && typeof tail.dbId === "number") {
        const prevNewest = newestDbIdRef.current.get(sessionId)
        if (prevNewest === undefined || tail.dbId > prevNewest) {
          newestDbIdRef.current.set(sessionId, tail.dbId)
        }
      }
      if (currentSessionIdRef.current === sessionId) {
        setMessages(next)
      }
    },
    [],
  )

  const upsertSessionMeta = useCallback((meta: SessionMeta) => {
    setSessions((prev) => {
      const idx = prev.findIndex((session) => session.id === meta.id)
      if (idx === -1) return sortSessionsForSidebar([meta, ...prev])
      if (prev[idx] === meta) return prev
      const next = [...prev]
      next[idx] = meta
      return sortSessionsForSidebar(next)
    })
  }, [])

  const updateSessionMeta = useCallback(
    (sessionId: string, updater: (prev: SessionMeta) => SessionMeta) => {
      setSessions((prev) => {
        let changed = false
        const next = prev.map((session) => {
          if (session.id !== sessionId) return session
          const updated = updater(session)
          if (updated !== session) changed = true
          return updated
        })
        return changed ? next : prev
      })
    },
    [],
  )

  const activateSessionShell = useCallback(
    (sessionId: string, opts: { clearMessages: boolean; hasMoreAfter?: boolean }) => {
      currentSessionIdRef.current = sessionId
      if (opts.clearMessages) {
        setMessages([])
        setPendingScrollIntent(null)
      }
      setHasMore(false)
      setHasMoreAfter(opts.hasMoreAfter ?? false)
      setLoading(loadingSessionsRef.current.has(sessionId))
      setCurrentSessionId(sessionId)

      const meta = sessionsRef.current.find((s) => s.id === sessionId)
      if (meta) {
        setCurrentAgentId(meta.agentId)
        const agent = agentsRef.current.find((a) => a.id === meta.agentId)
        if (agent) setAgentName(agent.name)
      }
    },
    [setHasMore, setHasMoreAfter],
  )

  // Per-session ref cleanup shared by explicit-delete / incognito purge /
  // LRU evict. Touches refs only — sidebar state and loading flags are
  // owned by callers that need them (only `evictSessionLocal` does).
  const clearPerSessionRefs = useCallback((sessionId: string) => {
    sessionCacheRef.current.delete(sessionId)
    hasMoreRef.current.delete(sessionId)
    hasMoreAfterRef.current.delete(sessionId)
    oldestDbIdRef.current.delete(sessionId)
    newestDbIdRef.current.delete(sessionId)
    userPaginatedDepthRef.current.delete(sessionId)
  }, [])

  const evictSessionLocal = useCallback(
    (sessionId: string) => {
      clearPerSessionRefs(sessionId)
      loadingSessionsRef.current.delete(sessionId)
      setLoadingSessionIds((prev) => {
        if (!prev.has(sessionId)) return prev
        const next = new Set(prev)
        next.delete(sessionId)
        return next
      })
      setSessions((prev) => {
        const next = prev.filter((s) => s.id !== sessionId)
        return next.length === prev.length ? prev : next
      })
    },
    [clearPerSessionRefs],
  )

  // Bump `sessionId` to the tail of the LRU (Map preserves insertion order
  // — `delete + set` re-orders) and evict the oldest non-protected entries
  // until we're back under cap. Protected: the active session, and any
  // session that's both streaming AND still has cache (the `&& has(sid)`
  // half avoids the "ghost streaming after evict" case where
  // loadingSessions still references a sid we already dropped).
  // If every remaining entry is protected, accept temporary overflow.
  const touchSessionCacheLru = useCallback(
    (sessionId: string) => {
      const cache = sessionCacheRef.current
      if (cache.has(sessionId)) {
        const v = cache.get(sessionId)!
        cache.delete(sessionId)
        cache.set(sessionId, v)
      }
      while (cache.size > SESSION_CACHE_LRU_LIMIT) {
        let evicted = false
        for (const k of cache.keys()) {
          const isCurrent = k === currentSessionIdRef.current
          const isLiveStreaming = loadingSessionsRef.current.has(k) && cache.has(k)
          if (isCurrent || isLiveStreaming) continue
          clearPerSessionRefs(k)
          evicted = true
          break
        }
        if (!evicted) break
      }
    },
    [clearPerSessionRefs],
  )

  // Post-append hook handed to `useChatStream` so the streaming hook can
  // bound its messages array without learning the topology of the cap's
  // per-session refs. Returns `msgs` unchanged when under cap.
  const capMessagesForSession = useCallback(
    (sessionId: string, msgs: Message[]): Message[] => {
      const result = capMessagesAndSyncCursors(
        sessionId,
        msgs,
        userPaginatedDepthRef.current.get(sessionId) ?? 0,
        oldestDbIdRef,
        hasMoreRef,
      )
      // cap fired (result shorter)? Mirror hasMoreRef into the React
      // state of the active session — MessageList's "Load More"
      // affordance reads state, not the ref. Non-current sessions pick
      // it up on the next handleSwitchSession.
      if (result !== msgs && currentSessionIdRef.current === sessionId) {
        setHasMore(true)
      }
      return result
    },
    [setHasMore],
  )

  const purgeIncognitoSession = useCallback(
    (sessionIdToLeave: string | null) => {
      if (!sessionIdToLeave) return
      const previousMeta = sessionsRef.current.find((s) => s.id === sessionIdToLeave)
      if (!previousMeta?.incognito) return
      evictSessionLocal(sessionIdToLeave)
      // E6 (INCOG-1 / DELETE-5): best-effort cancel any in-flight turn BEFORE the
      // burn so it stops streaming against a session that's about to vanish. The
      // backend cleanup watcher also live-cancels on `session:purged` (double
      // insurance) — this is just the faster client-side nudge. `turnId: null`
      // tells the backend to cancel whatever turn is active for the session.
      // Fire-and-forget; a no-op when nothing is running.
      void getTransport()
        .call("stop_chat", { sessionId: sessionIdToLeave, turnId: null })
        .catch(() => {})
      void getTransport()
        .call("purge_session_if_incognito", { sessionId: sessionIdToLeave })
        .catch((err) => {
          logger.warn(
            "chat",
            "useChatSession::purgeIncognito",
            `purge failed for ${sessionIdToLeave}`,
            err,
          )
        })
    },
    [evictSessionLocal],
  )

  // Centralized close-on-leave: any path that mutates `currentSessionId`
  // (sidebar click, new chat, project new chat, deep-link nav, jumpToMessage,
  // delete-session-while-active) reaches this effect and the previous session
  // is purged exactly once. Beats open-coding the call at every navigation
  // entry point.
  useEffect(() => {
    const previous = previousSessionIdRef.current
    previousSessionIdRef.current = currentSessionId
    if (previous && previous !== currentSessionId) {
      purgeIncognitoSession(previous)
    }
  }, [currentSessionId, purgeIncognitoSession])

  // Load agent list. Also pulls the global `default_agent_id` so the
  // implicit "current agent" state matches what the user configured in
  // settings — without this, `currentAgentId` is stuck at the hardcoded
  // "ha-main" until the user switches manually, defeating the setting.
  const reloadAgents = useCallback(async () => {
    try {
      const [list, defaultId] = await Promise.all([
        getTransport().call<AgentSummaryForSidebar[]>("list_agents"),
        getTransport()
          .call<string | null>("get_default_agent_id")
          .catch(() => null),
      ])
      setAgents(list)
      // Only reseed the implicit selection while no concrete session is
      // active. Once the user is inside a session we already track its
      // agent_id and don't want to clobber it.
      if (!currentSessionIdRef.current) {
        const id =
          typeof defaultId === "string" && defaultId.trim().length > 0
            ? defaultId
            : DEFAULT_AGENT_ID
        setCurrentAgentId(id)
        const match = list.find((a) => a.id === id)
        if (match) setAgentName(match.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::loadAgents", "Failed to load agents", e)
    }
  }, [])

  const handleToggleSessionPinned = useCallback(
    async (sessionId: string, pinned: boolean) => {
      const pinnedAt = pinned ? new Date().toISOString() : null
      setSessions((prev) =>
        sortSessionsForSidebar(
          prev.map((session) => (session.id === sessionId ? { ...session, pinnedAt } : session)),
        ),
      )
      try {
        await getTransport().call("set_session_pinned_cmd", { sessionId, pinned })
        await reloadSessions()
      } catch (e) {
        logger.error("ui", "ChatScreen::pinSession", "Failed to update session pin", e)
        toast.error(t("common.saveFailed"), { description: String(e) })
        await reloadSessions()
      }
    },
    [reloadSessions, t],
  )

  const handleReorderAgents = useCallback(
    async (agentIds: string[]) => {
      const current = agents
      const byId = new Map(current.map((agent) => [agent.id, agent]))
      const next = [
        ...agentIds
          .map((id) => byId.get(id))
          .filter((agent): agent is AgentSummaryForSidebar => !!agent),
        ...current.filter((agent) => !agentIds.includes(agent.id)),
      ]
      setAgents(next)
      try {
        await getTransport().call("reorder_agents", { agentIds: next.map((agent) => agent.id) })
      } catch (e) {
        logger.error("ui", "ChatScreen::reorderAgents", "Failed to reorder agents", e)
        toast.error(t("common.saveFailed"), { description: String(e) })
        setAgents(current)
      }
    },
    [agents, t],
  )

  useEffect(() => {
    reloadSessions()
    reloadAgents()
  }, [reloadSessions, reloadAgents])

  // Refresh agent list when agents are created/saved/deleted in settings panel
  useEffect(() => {
    const handler = () => {
      reloadAgents()
    }
    window.addEventListener("agents-changed", handler)
    return () => window.removeEventListener("agents-changed", handler)
  }, [reloadAgents])

  // Pick up changes to the global default agent from the settings panel.
  useEffect(() => {
    return getTransport().listen("config:changed", () => {
      void reloadAgents()
    })
  }, [reloadAgents])

  // Durable assistant appends and every read-state mutation emit this event.
  // Debounce bursts from multi-round/background work and re-query instead of
  // trusting an event payload that may already be stale.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    let needsRegularAggregate = false
    const schedule = (raw: unknown) => {
      const payload =
        raw && typeof raw === "object"
          ? (raw as { domain?: "regular" | "channel" | "cron" | null })
          : null
      if (payload?.domain === "cron") return
      if (payload?.domain !== "channel") needsRegularAggregate = true
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        const refreshRegular = needsRegularAggregate
        needsRegularAggregate = false
        void (refreshRegular ? reloadSessions() : reloadSessionPage())
        if (refreshRegular) onSidebarAggregatesChanged?.()
      }, 100)
    }
    const off = getTransport().listen("session:unread_changed", schedule)
    return () => {
      if (timer) clearTimeout(timer)
      off()
    }
  }, [reloadSessionPage, reloadSessions, onSidebarAggregatesChanged])

  // Listen for cron job completions to refresh unread counts + send notification
  useEffect(() => {
    return getTransport().listen("cron:run_completed", (raw) => {
      reloadSessions()
      const payload = raw as {
        job_id: string
        job_name: string
        status: string
        notify: boolean
        auto_disabled?: boolean
        consecutive_failures?: number
        failure_reason?: string
      }
      if (!payload.job_name) return
      if (payload.auto_disabled) {
        // Auto-disable always notifies (overrides notify_on_complete) — the user
        // must know a scheduled task stopped running (§5).
        const reason = payload.failure_reason
          ? t(`notification.cronReason.${payload.failure_reason}`, payload.failure_reason)
          : ""
        notify(
          t("notification.cronDisabled"),
          t("notification.cronDisabledBody", {
            name: payload.job_name,
            count: payload.consecutive_failures ?? 0,
            reason,
          }),
        )
      } else if (payload.notify) {
        if (payload.status === "success") {
          notify(t("notification.cronSuccess"), payload.job_name)
        } else if (payload.status === "empty") {
          // Review fix #5: a zero-output run is neither success nor error —
          // surface a neutral "completed, no output" notice, not a success toast.
          notify(t("notification.cronEmpty"), payload.job_name)
        } else if (payload.status === "cancelled") {
          // Review fix #6: a cancelled run isn't a failure — don't show the error
          // toast (the user, or another endpoint, cancelled it deliberately).
          notify(t("notification.cronCancelled"), payload.job_name)
        } else {
          // §10 (D4): surface *why* it failed (timeout / config / transient), not
          // just the job name, when the backend classified a reason.
          const body = payload.failure_reason
            ? `${payload.job_name} — ${t(`notification.cronReason.${payload.failure_reason}`, payload.failure_reason)}`
            : payload.job_name
          notify(t("notification.cronError"), body)
        }
      }
    })
  }, [reloadSessions, t])

  // Listen for pending-interaction lifecycle events so the sidebar refreshes
  // its `pendingInteractionCount` for non-active sessions in near-real-time.
  // Coalesce bursts via a 300ms trailing debounce — the underlying query is
  // cheap but we don't need to thrash the list.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    const schedule = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        reloadSessions()
      }, 300)
    }
    const offApproval = getTransport().listen("approval_required", schedule)
    const offAskUser = getTransport().listen("ask_user_request", schedule)
    const offChanged = getTransport().listen("session_pending_interactions_changed", schedule)
    const offResync = getTransport().listen(TRANSPORT_EVENT_RESYNC_REQUIRED, schedule)
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") schedule()
    }
    window.addEventListener("focus", schedule)
    document.addEventListener("visibilitychange", onVisibilityChange)
    return () => {
      if (timer) clearTimeout(timer)
      offApproval()
      offAskUser()
      offChanged()
      offResync()
      window.removeEventListener("focus", schedule)
      document.removeEventListener("visibilitychange", onVisibilityChange)
    }
  }, [reloadSessions])

  // Listen for sub-agent events — manage loading state + refresh sidebar
  useEffect(() => {
    return getTransport().listen("subagent_event", (raw) => {
      const payload = raw as SubagentEvent
      const childSid = payload.childSessionId
      if (childSid) {
        if (["spawning", "running"].includes(payload.status)) {
          loadingSessionsRef.current.add(childSid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        } else {
          loadingSessionsRef.current.delete(childSid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
        }
      }
      if (["completed", "error", "timeout", "killed", "spawning"].includes(payload.status)) {
        reloadSessions()
      }
    })
  }, [reloadSessions])

  // Note: background completions injected into a session (tool job / sub-agent
  // / group result → `<task-notification>`) already refresh the sidebar via the
  // parent-injection stream end listeners (`chat:stream_end` on the main screen,
  // `parent_agent_stream` done/error on lightweight screens). No extra listener
  // is needed here — the new unread surfaces once those fire.

  useEffect(() => {
    onUnreadCountChange?.(totalUnreadCount)
  }, [totalUnreadCount, onUnreadCountChange])

  const renderedThroughMessageId = messages.reduce<number | null>(
    (latest, message) =>
      typeof message.dbId === "number" && (latest === null || message.dbId > latest)
        ? message.dbId
        : latest,
    null,
  )

  useEffect(() => {
    const sessionId = currentSessionId
    if (
      !activeSessionReadable ||
      !sessionId ||
      renderedThroughMessageId === null ||
      historyLoading ||
      failedSessionLoadsRef.current.has(sessionId)
    ) {
      void reloadUnreadTotal()
      return
    }

    let cancelled = false
    let firstFrame = 0
    let secondFrame = 0

    // Wait for the transcript paint and its scroll effects to settle. This is
    // especially important for search jumps: the shell can be selected before
    // MessageList has scrolled to a historical hit, which must not clear newer
    // unread output at the tail.
    firstFrame = window.requestAnimationFrame(() => {
      secondFrame = window.requestAnimationFrame(() => {
        if (
          cancelled ||
          currentSessionIdRef.current !== sessionId ||
          !activeSessionReadableRef.current ||
          historyLoadingRef.current ||
          failedSessionLoadsRef.current.has(sessionId)
        ) {
          void reloadUnreadTotal()
          return
        }

        void getTransport()
          .call("mark_session_read_cmd", {
            sessionId,
            throughMessageId: renderedThroughMessageId,
          })
          .then(async () => {
            if (cancelled || currentSessionIdRef.current !== sessionId) return
            // Re-read the row instead of optimistically forcing it to zero: a
            // newer assistant row may have landed after the rendered watermark
            // was captured and must remain unread.
            await reloadSessions()
            if (!cancelled) onSidebarAggregatesChanged?.()
          })
          .catch((error) => {
            logger.warn(
              "session",
              "ChatScreen::readVisibleSession",
              "Failed to mark the visible session as read",
              error,
            )
          })
      })
    })

    return () => {
      cancelled = true
      window.cancelAnimationFrame(firstFrame)
      window.cancelAnimationFrame(secondFrame)
    }
  }, [
    activeSessionReadable,
    activeSessionReadableRef,
    currentSessionId,
    historyLoading,
    onSidebarAggregatesChanged,
    renderedThroughMessageId,
    reloadSessions,
    reloadUnreadTotal,
  ])

  // Switch to an existing session
  const handleSwitchSession = useCallback(
    async (sessionId: string, opts?: { targetMessageId?: number; highlightTerms?: string[] }) => {
      const targetMessageId = opts?.targetMessageId
      const highlightTerms = opts?.highlightTerms
      // Always reload when jumping to a specific message; otherwise skip if
      // already viewing the same session.
      if (!sessionId) return
      if (targetMessageId === undefined && sessionId === currentSessionIdRef.current) {
        return
      }
      if (
        sessionId !== currentSessionIdRef.current &&
        !confirmDiscardDirtyFileEditors(
          t("fileEditor.unsavedBody", "Discard the current edits before leaving this file?"),
        )
      ) {
        return
      }

      const version = ++switchVersionRef.current

      // If target session is in cache and we don't need to jump to a specific
      // message, restore immediately + kick a background reload-and-merge
      // so any external-channel updates (IM / CLI / cron) made while we
      // were away converge into the cached view within ~1 RTT.
      const cached = sessionCacheRef.current.get(sessionId)
      if (targetMessageId === undefined && cached) {
        failedSessionLoadsRef.current.delete(sessionId)
        const shouldRefreshCache = !loadingSessionsRef.current.has(sessionId)
        // Cached rows may lag durable output. Keep the transcript readable on
        // screen, but do not let unread state advance or active-session
        // aggregates exclude it until the authoritative tail has converged.
        updateHistoryLoading(shouldRefreshCache)
        currentSessionIdRef.current = sessionId
        setMessages(cached)
        setHasMore(hasMoreRef.current.get(sessionId) ?? false)
        setHasMoreAfter(hasMoreAfterRef.current.get(sessionId) ?? false)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
        touchSessionCacheLru(sessionId)
        // Skip background reload while streaming — the helper's unconditional
        // cache write would drop the in-flight assistant placeholder (no DB
        // row yet), making the bubble vanish mid-stream.
        if (shouldRefreshCache) {
          void reloadAndMergeSessionMessages({
            sessionId,
            pageSize: PAGE_SIZE,
            sessionCacheRef,
            setMessages: (msgs) => {
              if (
                currentSessionIdRef.current === sessionId &&
                !loadingSessionsRef.current.has(sessionId)
              ) {
                setMessages(msgs)
              }
            },
          }).then((refreshed) => {
            if (refreshed) {
              failedSessionLoadsRef.current.delete(sessionId)
            } else {
              failedSessionLoadsRef.current.add(sessionId)
            }
            if (
              switchVersionRef.current === version &&
              currentSessionIdRef.current === sessionId
            ) {
              updateHistoryLoading(false)
            }
          })
        }
      } else {
        const alreadyCurrent = sessionId === currentSessionIdRef.current
        failedSessionLoadsRef.current.delete(sessionId)
        updateHistoryLoading(true)
        if (!alreadyCurrent) {
          // Make the navigation visible immediately. The message window is filled
          // below when the DB/transport round-trip finishes.
          activateSessionShell(sessionId, { clearMessages: true })
        }
        try {
          let msgs: SessionMessage[]
          let hasMoreBefore: boolean
          let hasMoreAfterFlag = false
          if (targetMessageId !== undefined) {
            // Symmetric 40/40 around-window so a hit shows enough context
            // both ways for handleLoadMoreAfter to take over naturally.
            const [m, , hasMoreB, hasMoreA] = await getTransport().call<
              [SessionMessage[], number, boolean, boolean]
            >("load_session_messages_around_cmd", {
              sessionId,
              targetMessageId,
              before: 40,
              after: 40,
            })
            msgs = m
            hasMoreBefore = hasMoreB
            hasMoreAfterFlag = hasMoreA
          } else {
            // hasMore is authoritative from DB; don't infer from msgs.length
            // since user-boundary alignment may extend beyond the requested limit.
            const [m, , hasMore] = await getTransport().call<[SessionMessage[], number, boolean]>(
              "load_session_messages_latest_cmd",
              { sessionId, limit: PAGE_SIZE },
            )
            msgs = m
            hasMoreBefore = hasMore
          }
          const displayMessages = await materializeMessages(sessionId, msgs, sessionsRef)
          if (switchVersionRef.current !== version) return // stale switch
          failedSessionLoadsRef.current.delete(sessionId)
          updateHistoryLoading(false)
          sessionCacheRef.current.set(sessionId, displayMessages)
          hasMoreRef.current.set(sessionId, hasMoreBefore)
          hasMoreAfterRef.current.set(sessionId, hasMoreAfterFlag)
          if (msgs.length > 0) {
            oldestDbIdRef.current.set(sessionId, msgs[0].id)
            newestDbIdRef.current.set(sessionId, msgs[msgs.length - 1].id)
          }
          // Cache miss = fresh build, paginate high-watermark restarts at 0.
          userPaginatedDepthRef.current.set(sessionId, 0)
          setMessages(displayMessages)
          setHasMore(hasMoreBefore)
          setHasMoreAfter(hasMoreAfterFlag)
          setLoading(loadingSessionsRef.current.has(sessionId))
          currentSessionIdRef.current = sessionId
          setCurrentSessionId(sessionId)
          touchSessionCacheLru(sessionId)
        } catch (e) {
          failedSessionLoadsRef.current.add(sessionId)
          if (switchVersionRef.current === version && currentSessionIdRef.current === sessionId) {
            updateHistoryLoading(false)
          }
          logger.error("session", "ChatScreen::switchSession", "Failed to load session", {
            sessionId,
            error: e,
          })
          return
        }
      }

      if (targetMessageId !== undefined) {
        setPendingScrollIntent({
          messageId: targetMessageId,
          highlightTerms: highlightTerms ?? null,
        })
      }

      if (switchVersionRef.current !== version) return // stale switch

      let session = sessionsRef.current.find((s) => s.id === sessionId)
      if (!session) {
        const fetchedSession = await getTransport()
          .call<SessionMeta | null>("get_session_cmd", { sessionId })
          .catch(() => null)
        if (switchVersionRef.current !== version) return // stale switch
        session = fetchedSession ?? undefined
      }
      if (session) {
        upsertSessionMeta(session)
        setCurrentAgentId(session.agentId)
        const agent = agentsRef.current.find((a) => a.id === session.agentId)
        if (agent) setAgentName(agent.name)

        const sessionPreferred =
          session.providerId && session.modelId
            ? { providerId: session.providerId, modelId: session.modelId }
            : null
        let agentPrimary: string | null = null
        if (!resolveAvailableDisplayModel(availableModels, sessionPreferred, null, null)) {
          try {
            const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
              id: session.agentId,
            })
            if (switchVersionRef.current !== version) return // stale switch
            agentPrimary = agentConfig.model.primary ?? null
          } catch {
            // A missing agent config still falls through to the global candidate.
          }
        }

        const displayModel = resolveAvailableDisplayModel(
          availableModels,
          sessionPreferred,
          agentPrimary,
          globalActiveModelRef.current,
        )
        if (displayModel) {
          applyModelForDisplay(`${displayModel.providerId}::${displayModel.modelId}`)
        } else {
          setActiveModel(null)
        }
      }

      // Read-state is advanced by the visibility effect above only after this
      // session is truly readable. Keep hidden/programmatic navigation unread.
      if (!activeSessionReadableRef.current) {
        void reloadSessions()
      }
    },
    [
      availableModels,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      reloadSessions,
      activeSessionReadableRef,
      activateSessionShell,
      setHasMore,
      setHasMoreAfter,
      touchSessionCacheLru,
      updateHistoryLoading,
      upsertSessionMeta,
      t,
    ],
  )

  // Jump to a specific message within the *current* session. If the target
  // is already in the loaded window, just sets `pendingScrollIntent` to let
  // MessageList scroll & pulse. Otherwise reloads a window of messages
  // centred on the target (delegating to handleSwitchSession).
  const jumpToMessage = useCallback(
    async (messageId: number, highlightTerms?: string[]) => {
      const sid = currentSessionIdRef.current
      if (!sid) return
      const exists = messagesRef.current.some((m) => m.dbId === messageId)
      if (exists) {
        setPendingScrollIntent({ messageId, highlightTerms: highlightTerms ?? null })
        return
      }
      await handleSwitchSession(sid, { targetMessageId: messageId, highlightTerms })
    },
    [handleSwitchSession],
  )

  // Navigate to a specific session when initialSessionId changes
  useEffect(() => {
    if (!initialSessionId) return
    ;(async () => {
      await reloadSessions()
      await handleSwitchSession(initialSessionId)
      onSessionNavigated?.()
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialSessionId])

  // Create a new chat with a specific agent
  const handleNewChat = useCallback(
    async (agentId: string) => {
      // Save current session to cache
      // (cache is already maintained by updateSessionMessages)
      const cachedAgent = agents.find((a) => a.id === agentId)
      updateHistoryLoading(false)
      setMessages([])
      setCurrentSessionId(null)
      setLoading(false)
      setHasMore(false)
      setHasMoreAfter(false)
      setCurrentAgentId(agentId)
      setAgentName(cachedAgent?.name ?? "")

      const currentAgents = await getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[])
      const agent = currentAgents.find((a) => a.id === agentId)
      if (agent) {
        setAgentName(agent.name)
      }

      // Apply agent's configured model, or restore global default
      try {
        const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
          id: agentId,
        })
        if (agentConfig.model.primary) {
          const modelExists = availableModels.some(
            (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
          )
          if (modelExists) {
            applyModelForDisplay(agentConfig.model.primary)
            return
          }
        }
      } catch {
        // ignore
      }
      // No agent model configured or unavailable — restore global default
      if (globalActiveModelRef.current) {
        setActiveModel(globalActiveModelRef.current)
      }
    },
    [
      availableModels,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      setHasMore,
      setHasMoreAfter,
      agents,
      updateHistoryLoading,
    ],
  )

  // Note: entering a Project no longer pre-materializes a session. The chat now
  // stays in draft (currentSessionId=null) with the project remembered as
  // `draftProjectId` in ChatScreen, and the session is created — bound to the
  // project — on first send via the `chat` command's `projectId`. See the
  // `handleNewChatInProject` wrapper in ChatScreen.tsx.

  // Archive a session without deleting its transcript.
  const handleArchiveSession = useCallback(
    async (sessionId: string) => {
      const sessionTitle =
        sessions.find((s) => s.id === sessionId)?.title || t("chat.untitledSession")
      try {
        await getTransport().call("set_session_archived_cmd", { sessionId, archived: true })
        evictSessionLocal(sessionId)
        if (currentSessionIdRef.current === sessionId) {
          setMessages([])
          setCurrentSessionId(null)
          updateHistoryLoading(false)
          setLoading(false)
          setHasMore(false)
          setHasMoreAfter(false)
        }
        reloadSessions()
        onSidebarAggregatesChanged?.()
        toast.success(t("chat.sessionArchived"), {
          description: sessionTitle,
        })
      } catch (err) {
        logger.error("session", "ChatScreen::archiveSession", "Failed to archive session", err)
        toast.error(t("chat.archiveSessionFailed"), {
          description: sessionTitle,
        })
      }
    },
    [
      reloadSessions,
      setHasMore,
      setHasMoreAfter,
      evictSessionLocal,
      sessions,
      t,
      onSidebarAggregatesChanged,
      updateHistoryLoading,
    ],
  )

  return {
    messages,
    setMessages,
    currentSessionId,
    setCurrentSessionId,
    currentSessionIdRef,
    currentSessionContentReady:
      !historyLoading &&
      (!currentSessionId || !failedSessionLoadsRef.current.has(currentSessionId)),
    sessions,
    totalUnreadCount,
    agents,
    currentAgentId,
    setCurrentAgentId,
    agentName,
    setAgentName,
    historyLoading,
    loading,
    setLoading,
    loadingSessionIds,
    setLoadingSessionIds,
    hasMore,
    loadingMore,
    hasMoreAfter,
    loadingMoreAfter,
    sessionsLoading,
    pendingScrollIntent,
    clearPendingScrollIntent,
    jumpToMessage,
    sessionCacheRef,
    loadingSessionsRef,
    hasMoreRef,
    oldestDbIdRef,
    capMessagesForSession,
    touchSessionCacheLru,
    reloadSessions,
    reloadAgents,
    handleToggleSessionPinned,
    handleReorderAgents,
    handleSwitchSession,
    handleNewChat,
    handleArchiveSession,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    updateSessionMessages,
    updateSessionMeta,
  }
}
