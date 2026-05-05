import { useState, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { materializeMessages } from "../chatUtils"
import { PAGE_SIZE, SESSION_PAGE_SIZE } from "./constants"
import type { Message, SessionMeta, SessionMessage } from "@/types/chat"

interface UseSessionPaginationParams {
  currentSessionIdRef: React.MutableRefObject<string | null>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  hasMoreAfterRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>
  newestDbIdRef: React.MutableRefObject<Map<string, number>>
  /** Used by `materializeMessages` to find the parent session's agentId
   *  without round-tripping the full session list on every page. */
  sessionsRef: React.MutableRefObject<SessionMeta[]>
  setSessions: React.Dispatch<React.SetStateAction<SessionMeta[]>>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  sessionsLength: number
}

export interface UseSessionPaginationReturn {
  hasMore: boolean
  setHasMore: React.Dispatch<React.SetStateAction<boolean>>
  loadingMore: boolean
  hasMoreAfter: boolean
  setHasMoreAfter: React.Dispatch<React.SetStateAction<boolean>>
  loadingMoreAfter: boolean
  hasMoreSessions: boolean
  setHasMoreSessions: React.Dispatch<React.SetStateAction<boolean>>
  loadingMoreSessions: boolean
  handleLoadMore: () => Promise<void>
  handleLoadMoreAfter: () => Promise<void>
  /**
   * Reload the latest page of the current session, dropping any partial
   * around-window state. Used by the jump-to-latest button when the user is
   * sitting on a search-jump window (`hasMoreAfter === true`) and wants to
   * return to the live tail.
   */
  resetToLatest: () => Promise<void>
  handleLoadMoreSessions: () => Promise<void>
  reloadSessions: () => Promise<void>
}

export function useSessionPagination({
  currentSessionIdRef,
  sessionCacheRef,
  hasMoreRef,
  hasMoreAfterRef,
  oldestDbIdRef,
  newestDbIdRef,
  sessionsRef,
  setSessions,
  setMessages,
  sessionsLength,
}: UseSessionPaginationParams): UseSessionPaginationReturn {
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [hasMoreAfter, setHasMoreAfter] = useState(false)
  const [loadingMoreAfter, setLoadingMoreAfter] = useState(false)
  const [hasMoreSessions, setHasMoreSessions] = useState(false)
  const [loadingMoreSessions, setLoadingMoreSessions] = useState(false)

  const reloadSessions = useCallback(async () => {
    try {
      const [list, total] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: 0,
        activeSessionId: currentSessionIdRef.current ?? undefined,
      })
      setSessions(list)
      setHasMoreSessions(list.length < total)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSessions", "Failed to load sessions", e)
    }
  }, [setSessions, currentSessionIdRef])

  const handleLoadMoreSessions = useCallback(async () => {
    if (loadingMoreSessions || !hasMoreSessions) return
    setLoadingMoreSessions(true)
    try {
      const [more, total] = await getTransport().call<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: sessionsLength,
        activeSessionId: currentSessionIdRef.current ?? undefined,
      })
      if (more.length === 0) {
        setHasMoreSessions(false)
        return
      }
      setSessions((prev) => {
        const existingIds = new Set(prev.map((s) => s.id))
        const newItems = more.filter((s) => !existingIds.has(s.id))
        const merged = [...prev, ...newItems]
        setHasMoreSessions(merged.length < total)
        return merged
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::loadMoreSessions", "Failed to load more sessions", e)
    } finally {
      setLoadingMoreSessions(false)
    }
  }, [loadingMoreSessions, hasMoreSessions, sessionsLength, setSessions, currentSessionIdRef])

  const handleLoadMore = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid || loadingMore || !hasMore) return
    const oldestId = oldestDbIdRef.current.get(curSid)
    if (oldestId === undefined) return

    setLoadingMore(true)
    try {
      // Backend returns `[messages, hasMore]`. Rows may exceed PAGE_SIZE when
      // the oldest requested row falls mid-way through an assistant turn and
      // the server aligns the window back to the previous user boundary.
      const [olderMsgs, hasMoreBefore] = await getTransport().call<
        [SessionMessage[], boolean]
      >("load_session_messages_before_cmd", {
        sessionId: curSid,
        beforeId: oldestId,
        limit: PAGE_SIZE,
      })
      if (olderMsgs.length === 0) {
        hasMoreRef.current.set(curSid, false)
        setHasMore(false)
        return
      }
      const olderDisplay = await materializeMessages(curSid, olderMsgs, sessionsRef)
      oldestDbIdRef.current.set(curSid, olderMsgs[0].id)
      hasMoreRef.current.set(curSid, hasMoreBefore)
      setHasMore(hasMoreBefore)

      setMessages((prev) => {
        const merged = [...olderDisplay, ...prev]
        sessionCacheRef.current.set(curSid, merged)
        return merged
      })
    } catch (e) {
      logger.error("session", "ChatScreen::loadMore", "Failed to load older messages", { error: e })
    } finally {
      setLoadingMore(false)
    }
  }, [
    loadingMore,
    hasMore,
    currentSessionIdRef,
    oldestDbIdRef,
    hasMoreRef,
    sessionCacheRef,
    sessionsRef,
    setMessages,
  ])

  // Forward-pagination twin of `handleLoadMore` — fires when the user scrolls
  // past the tail of a partial around-window. `newestDbIdRef` is the cursor
  // (kept fresh by `updateSessionMessages` on streaming append) so re-entering
  // the function never re-fetches already-rendered rows.
  const handleLoadMoreAfter = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid || loadingMoreAfter || !hasMoreAfter) return
    const newestId = newestDbIdRef.current.get(curSid)
    if (newestId === undefined) return

    setLoadingMoreAfter(true)
    try {
      const [newerMsgs, hasMoreA] = await getTransport().call<
        [SessionMessage[], boolean]
      >("load_session_messages_after_cmd", {
        sessionId: curSid,
        afterId: newestId,
        limit: PAGE_SIZE,
      })
      if (newerMsgs.length === 0) {
        hasMoreAfterRef.current.set(curSid, false)
        if (currentSessionIdRef.current === curSid) setHasMoreAfter(false)
        return
      }
      const newerDisplay = await materializeMessages(curSid, newerMsgs, sessionsRef)
      // Always advance per-session cursors + cache (the data is correct for
      // `curSid` regardless of which session is now on screen). UI state is
      // only safe to touch when curSid is still current — otherwise we'd
      // overwrite the active session's view with rows from another session
      // and pollute its cache via the functional-setState `prev` snapshot.
      const prevCached = sessionCacheRef.current.get(curSid) ?? []
      const seen = new Set<number>()
      for (const m of prevCached) {
        if (m.dbId !== undefined) seen.add(m.dbId)
      }
      const fresh = newerDisplay.filter(
        (m) => m.dbId === undefined || !seen.has(m.dbId),
      )
      const merged = [...prevCached, ...fresh]
      sessionCacheRef.current.set(curSid, merged)
      newestDbIdRef.current.set(curSid, newerMsgs[newerMsgs.length - 1].id)
      hasMoreAfterRef.current.set(curSid, hasMoreA)
      if (currentSessionIdRef.current === curSid) {
        setMessages(merged)
        setHasMoreAfter(hasMoreA)
      }
    } catch (e) {
      logger.error("session", "ChatScreen::loadMoreAfter", "Failed to load newer messages", {
        error: e,
      })
    } finally {
      setLoadingMoreAfter(false)
    }
  }, [
    loadingMoreAfter,
    hasMoreAfter,
    currentSessionIdRef,
    newestDbIdRef,
    hasMoreAfterRef,
    sessionCacheRef,
    sessionsRef,
    setMessages,
  ])

  // Drop the partial around-window and reload the live tail. Wired to
  // jump-to-latest in MessageList when `hasMoreAfter` is true — without it
  // the button could only `scrollTo(scrollHeight)` of the truncated array.
  const resetToLatest = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid) return
    try {
      const [latest, , hasMoreBefore] = await getTransport().call<
        [SessionMessage[], number, boolean]
      >("load_session_messages_latest_cmd", { sessionId: curSid, limit: PAGE_SIZE })
      const display = await materializeMessages(curSid, latest, sessionsRef)
      // Refresh per-session cache + cursors regardless of current session;
      // they describe the latest tail of `curSid` and are correct to keep
      // around for the next time the user views it. UI state, however,
      // belongs to whichever session is currently mounted — touching it
      // when curSid has been left behind would overwrite the active view.
      sessionCacheRef.current.set(curSid, display)
      hasMoreRef.current.set(curSid, hasMoreBefore)
      hasMoreAfterRef.current.set(curSid, false)
      if (latest.length > 0) {
        oldestDbIdRef.current.set(curSid, latest[0].id)
        newestDbIdRef.current.set(curSid, latest[latest.length - 1].id)
      }
      if (currentSessionIdRef.current === curSid) {
        setMessages(display)
        setHasMore(hasMoreBefore)
        setHasMoreAfter(false)
      }
    } catch (e) {
      logger.error("session", "ChatScreen::resetToLatest", "Failed to reload latest messages", {
        error: e,
      })
    }
  }, [
    currentSessionIdRef,
    hasMoreAfterRef,
    hasMoreRef,
    newestDbIdRef,
    oldestDbIdRef,
    sessionCacheRef,
    sessionsRef,
    setMessages,
  ])

  return {
    hasMore,
    setHasMore,
    loadingMore,
    hasMoreAfter,
    setHasMoreAfter,
    loadingMoreAfter,
    hasMoreSessions,
    setHasMoreSessions,
    loadingMoreSessions,
    handleLoadMore,
    handleLoadMoreAfter,
    resetToLatest,
    handleLoadMoreSessions,
    reloadSessions,
  }
}
