import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { desktopUnreadCount } from "@/lib/unread"
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

export { PAGE_SIZE, SESSION_PAGE_SIZE } from "./constants"

export interface UseChatSessionReturn {
  // State
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  sessions: SessionMeta[]
  agents: AgentSummaryForSidebar[]
  currentAgentId: string
  setCurrentAgentId: React.Dispatch<React.SetStateAction<string>>
  agentName: string
  setAgentName: React.Dispatch<React.SetStateAction<string>>
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionIds: Set<string>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  hasMore: boolean
  loadingMore: boolean
  hasMoreAfter: boolean
  loadingMoreAfter: boolean
  hasMoreSessions: boolean
  loadingMoreSessions: boolean
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
  handleDeleteSession: (sessionId: string) => Promise<void>
  handleLoadMore: () => Promise<void>
  handleLoadMoreAfter: () => Promise<void>
  /** Drop the partial around-window and reload the latest page. */
  resetToLatest: () => Promise<void>
  handleLoadMoreSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  updateSessionMeta: (sessionId: string, updater: (prev: SessionMeta) => SessionMeta) => void
}

interface UseChatSessionOptions {
  availableModels: AvailableModel[]
  setActiveModel: React.Dispatch<React.SetStateAction<ActiveModel | null>>
  globalActiveModelRef: React.MutableRefObject<ActiveModel | null>
  handleModelChange: (key: string) => void
  applyModelForDisplay: (key: string) => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  onSidebarAggregatesChanged?: () => void
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
  handleModelChange,
  applyModelForDisplay,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  onSidebarAggregatesChanged,
}: UseChatSessionOptions): UseChatSessionReturn {
  const { t } = useTranslation()
  const [messages, setMessages] = useState<Message[]>([])
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(null)
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [currentAgentId, setCurrentAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [agentName, setAgentName] = useState("")
  const [loading, setLoading] = useState(false)
  const [loadingSessionIds, setLoadingSessionIds] = useState<Set<string>>(new Set())
  const [pendingScrollIntent, setPendingScrollIntent] = useState<
    { messageId: number; highlightTerms: string[] | null } | null
  >(null)
  const clearPendingScrollIntent = useCallback(() => setPendingScrollIntent(null), [])

  const currentSessionIdRef = useRef<string | null>(null)
  const switchVersionRef = useRef(0)
  const sessionCacheRef = useRef<Map<string, Message[]>>(new Map())
  const loadingSessionsRef = useRef<Set<string>>(new Set())
  const hasMoreRef = useRef<Map<string, boolean>>(new Map())
  const hasMoreAfterRef = useRef<Map<string, boolean>>(new Map())
  const oldestDbIdRef = useRef<Map<string, number>>(new Map())
  const newestDbIdRef = useRef<Map<string, number>>(new Map())
  const userPaginatedDepthRef = useRef<Map<string, number>>(new Map())
  // De-dupes background reload-and-merge calls so rapid A→B→A switches
  // don't issue redundant DB reads for the same sid.
  const inFlightReloadsRef = useRef<Set<string>>(new Set())
  // Mirror of `messages` so `jumpToMessage` can synchronously check whether
  // a target message is already loaded without stale-closure hazards.
  const messagesRef = useRef<Message[]>([])
  // Mirror of `sessions` so callbacks reading session metadata don't have to
  // list `sessions` in their deps (which would invalidate them on every
  // streaming meta tick and cascade re-renders into the sidebar tree).
  const sessionsRef = useRef<SessionMeta[]>([])
  // Tracks the previous `currentSessionId` so the effect below can fire
  // `purge_session_if_incognito` exactly once per swap.
  const previousSessionIdRef = useRef<string | null>(null)

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

  // --- Session pagination sub-hook ---
  const {
    hasMore,
    setHasMore,
    loadingMore,
    hasMoreAfter,
    setHasMoreAfter,
    loadingMoreAfter,
    hasMoreSessions,
    // setHasMoreSessions not needed at this level
    loadingMoreSessions,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    handleLoadMoreSessions,
    reloadSessions,
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
    sessionsLength: sessions.length,
  })

  // --- Channel streaming sub-hook ---
  useChannelStreaming({
    currentSessionIdRef,
    sessionCacheRef,
    loadingSessionsRef,
    setMessages,
    setLoading,
    setLoadingSessionIds,
    reloadSessions,
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
          const isLiveStreaming =
            loadingSessionsRef.current.has(k) && cache.has(k)
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
          prev.map((session) =>
            session.id === sessionId ? { ...session, pinnedAt } : session,
          ),
        ),
      )
      try {
        await getTransport().call("set_session_pinned_cmd", { sessionId, pinned })
        await reloadSessions()
      } catch (e) {
        logger.error("ui", "ChatScreen::pinSession", "Failed to update session pin", e)
        notify(t("common.saveFailed"), String(e))
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
        ...agentIds.map((id) => byId.get(id)).filter((agent): agent is AgentSummaryForSidebar => !!agent),
        ...current.filter((agent) => !agentIds.includes(agent.id)),
      ]
      setAgents(next)
      try {
        await getTransport().call("reorder_agents", { agentIds: next.map((agent) => agent.id) })
      } catch (e) {
        logger.error("ui", "ChatScreen::reorderAgents", "Failed to reorder agents", e)
        notify(t("common.saveFailed"), String(e))
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
    return () => {
      if (timer) clearTimeout(timer)
      offApproval()
      offAskUser()
      offChanged()
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
  // `parent_agent_stream` `done`/`error` listener in `useNotificationListeners`
  // (which calls `reloadSessions` unconditionally, post-persist). No extra
  // listener is needed here — the new unread surfaces once that fires.

  // Compute total unread count — channel and sub-agent sessions don't surface
  // global unread indicators in the primary chat entry, and the active session
  // reads as 0. Shares the rule with the sidebar / tab / project badges via
  // `desktopUnreadCount` so the surfaces can't drift.
  const totalUnreadCount = useMemo(
    () => sessions.reduce((sum, s) => sum + desktopUnreadCount(s, currentSessionId), 0),
    [sessions, currentSessionId],
  )

  useEffect(() => {
    onUnreadCountChange?.(totalUnreadCount)
  }, [totalUnreadCount, onUnreadCountChange])

  // Switch to an existing session
  const handleSwitchSession = useCallback(
    async (
      sessionId: string,
      opts?: { targetMessageId?: number; highlightTerms?: string[] },
    ) => {
      const targetMessageId = opts?.targetMessageId
      const highlightTerms = opts?.highlightTerms
      // Always reload when jumping to a specific message; otherwise skip if
      // already viewing the same session.
      if (!sessionId) return
      if (targetMessageId === undefined && sessionId === currentSessionIdRef.current) {
        return
      }

      const version = ++switchVersionRef.current

      // If target session is in cache and we don't need to jump to a specific
      // message, restore immediately + kick a background reload-and-merge
      // so any external-channel updates (IM / CLI / cron) made while we
      // were away converge into the cached view within ~1 RTT.
      const cached = sessionCacheRef.current.get(sessionId)
      if (targetMessageId === undefined && cached) {
        setMessages(cached)
        setHasMore(hasMoreRef.current.get(sessionId) ?? false)
        setHasMoreAfter(hasMoreAfterRef.current.get(sessionId) ?? false)
        setLoading(loadingSessionsRef.current.has(sessionId))
        setCurrentSessionId(sessionId)
        touchSessionCacheLru(sessionId)
        // Skip background reload while streaming — the helper's unconditional
        // cache write would drop the in-flight assistant placeholder (no DB
        // row yet), making the bubble vanish mid-stream.
        if (
          !inFlightReloadsRef.current.has(sessionId) &&
          !loadingSessionsRef.current.has(sessionId)
        ) {
          inFlightReloadsRef.current.add(sessionId)
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
          }).finally(() => {
            inFlightReloadsRef.current.delete(sessionId)
          })
        }
      } else {
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
          setCurrentSessionId(sessionId)
          touchSessionCacheLru(sessionId)
        } catch (e) {
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

      // Use fresh sessions list for session lookup
      const [currentSessions] = await getTransport()
        .call<[SessionMeta[], number]>("list_sessions_cmd", {})
        .catch(() => [[] as SessionMeta[], 0] as [SessionMeta[], number])
      const currentAgents = await getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[])
      const session = currentSessions.find((s) => s.id === sessionId)
      if (session) {
        setCurrentAgentId(session.agentId)
        const agent = currentAgents.find((a) => a.id === session.agentId)
        if (agent) setAgentName(agent.name)

        // Restore the model used in this session (if still available)
        if (session.providerId && session.modelId) {
          const modelExists = availableModels.some(
            (m) => m.providerId === session.providerId && m.modelId === session.modelId,
          )
          if (modelExists) {
            handleModelChange(`${session.providerId}::${session.modelId}`)
          }
        } else {
          // Session has no model info, fallback to agent's configured model or global default
          try {
            const agentConfig = await getTransport().call<AgentConfig>("get_agent_config", {
              id: session.agentId,
            })
            if (agentConfig.model.primary) {
              const modelExists = availableModels.some(
                (m) => `${m.providerId}::${m.modelId}` === agentConfig.model.primary,
              )
              if (modelExists) {
                applyModelForDisplay(agentConfig.model.primary)
                // Mark session as read and refresh (await + log; see #6 below).
                try {
                  await getTransport().call("mark_session_read_cmd", { sessionId })
                } catch (e) {
                  logger.warn(
                    "session",
                    "ChatScreen::switchSession",
                    "Failed to mark session as read",
                    e,
                  )
                }
                reloadSessions()
                onSidebarAggregatesChanged?.()
                return
              }
            }
          } catch {
            // ignore
          }
          // No agent model or unavailable — restore global default
          if (globalActiveModelRef.current) {
            setActiveModel(globalActiveModelRef.current)
          }
        }
      }

      // Mark session as read and refresh unread counts. Await so a failed write
      // can't leave the session reading as unread after reload, and surface the
      // error instead of swallowing it (#6 — was fire-and-forget + silent).
      try {
        await getTransport().call("mark_session_read_cmd", { sessionId })
      } catch (e) {
        logger.warn(
          "session",
          "ChatScreen::switchSession",
          "Failed to mark session as read",
          e,
        )
      }
      reloadSessions()
      onSidebarAggregatesChanged?.()
    },
    [
      availableModels,
      handleModelChange,
      applyModelForDisplay,
      globalActiveModelRef,
      setActiveModel,
      reloadSessions,
      onSidebarAggregatesChanged,
      setHasMore,
      setHasMoreAfter,
      touchSessionCacheLru,
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

      const currentAgents = await getTransport()
        .call<AgentSummaryForSidebar[]>("list_agents")
        .catch(() => [] as AgentSummaryForSidebar[])
      const agent = currentAgents.find((a) => a.id === agentId)
      setMessages([])
      setCurrentSessionId(null)
      setLoading(false)
      setHasMore(false)
      setHasMoreAfter(false)
      setCurrentAgentId(agentId)
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
    ],
  )

  // Note: entering a Project no longer pre-materializes a session. The chat now
  // stays in draft (currentSessionId=null) with the project remembered as
  // `draftProjectId` in ChatScreen, and the session is created — bound to the
  // project — on first send via the `chat` command's `projectId`. See the
  // `handleNewChatInProject` wrapper in ChatScreen.tsx.

  // Delete a session
  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      const sessionTitle =
        sessions.find((s) => s.id === sessionId)?.title || t("chat.untitledSession")
      try {
        await getTransport().call("delete_session_cmd", { sessionId })
        evictSessionLocal(sessionId)
        if (currentSessionIdRef.current === sessionId) {
          setMessages([])
          setCurrentSessionId(null)
          setLoading(false)
          setHasMore(false)
          setHasMoreAfter(false)
        }
        reloadSessions()
        onSidebarAggregatesChanged?.()
        toast.success(t("common.deleted"), {
          description: sessionTitle,
        })
      } catch (err) {
        logger.error("session", "ChatScreen::deleteSession", "Failed to delete session", err)
        toast.error(t("common.deleteFailed"), {
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
    ],
  )

  return {
    messages,
    setMessages,
    currentSessionId,
    setCurrentSessionId,
    currentSessionIdRef,
    sessions,
    agents,
    currentAgentId,
    setCurrentAgentId,
    agentName,
    setAgentName,
    loading,
    setLoading,
    loadingSessionIds,
    setLoadingSessionIds,
    hasMore,
    loadingMore,
    hasMoreAfter,
    loadingMoreAfter,
    hasMoreSessions,
    loadingMoreSessions,
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
    handleDeleteSession,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    handleLoadMoreSessions,
    updateSessionMessages,
    updateSessionMeta,
  }
}
