import { useState, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import { parseSessionMessages } from "../chatUtils"
import { PAGE_SIZE, SESSION_PAGE_SIZE } from "./constants"
import type { Message, SessionMeta, SessionMessage } from "@/types/chat"

interface UseSessionPaginationParams {
  currentSessionIdRef: React.MutableRefObject<string | null>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  hasMoreRef: React.MutableRefObject<Map<string, boolean>>
  oldestDbIdRef: React.MutableRefObject<Map<string, number>>
  setSessions: React.Dispatch<React.SetStateAction<SessionMeta[]>>
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  sessionsLength: number
}

export interface UseSessionPaginationReturn {
  hasMore: boolean
  setHasMore: React.Dispatch<React.SetStateAction<boolean>>
  loadingMore: boolean
  hasMoreSessions: boolean
  setHasMoreSessions: React.Dispatch<React.SetStateAction<boolean>>
  loadingMoreSessions: boolean
  handleLoadMore: () => Promise<void>
  handleLoadMoreSessions: () => Promise<void>
  reloadSessions: () => Promise<void>
}

export function useSessionPagination({
  currentSessionIdRef,
  sessionCacheRef,
  hasMoreRef,
  oldestDbIdRef,
  setSessions,
  setMessages,
  sessionsLength,
}: UseSessionPaginationParams): UseSessionPaginationReturn {
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [hasMoreSessions, setHasMoreSessions] = useState(false)
  const [loadingMoreSessions, setLoadingMoreSessions] = useState(false)

  const reloadSessions = useCallback(async () => {
    try {
      const [list, total] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: 0,
      })
      setSessions(list)
      setHasMoreSessions(list.length < total)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSessions", "Failed to load sessions", e)
    }
  }, [setSessions])

  const handleLoadMoreSessions = useCallback(async () => {
    if (loadingMoreSessions || !hasMoreSessions) return
    setLoadingMoreSessions(true)
    try {
      const [more, total] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: SESSION_PAGE_SIZE,
        offset: sessionsLength,
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
  }, [loadingMoreSessions, hasMoreSessions, sessionsLength, setSessions])

  const handleLoadMore = useCallback(async () => {
    const curSid = currentSessionIdRef.current
    if (!curSid || loadingMore || !hasMore) return
    const oldestId = oldestDbIdRef.current.get(curSid)
    if (oldestId === undefined) return

    setLoadingMore(true)
    try {
      const olderMsgs = await invoke<SessionMessage[]>("load_session_messages_before_cmd", {
        sessionId: curSid,
        beforeId: oldestId,
        limit: PAGE_SIZE,
      })
      if (olderMsgs.length === 0) {
        hasMoreRef.current.set(curSid, false)
        setHasMore(false)
        return
      }
      const [currentSessions] = await invoke<[SessionMeta[], number]>("list_sessions_cmd", {}).catch(
        () => [[] as SessionMeta[], 0] as [SessionMeta[], number],
      )
      const sessionMeta = currentSessions.find((s) => s.id === curSid)
      const parentSession = sessionMeta?.parentSessionId
        ? currentSessions.find((s) => s.id === sessionMeta.parentSessionId)
        : undefined
      const olderDisplay = parseSessionMessages(olderMsgs, parentSession?.agentId)
      oldestDbIdRef.current.set(curSid, olderMsgs[0].id)
      if (olderMsgs.length < PAGE_SIZE) {
        hasMoreRef.current.set(curSid, false)
        setHasMore(false)
      }

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
  }, [loadingMore, hasMore, currentSessionIdRef, oldestDbIdRef, hasMoreRef, sessionCacheRef, setMessages])

  return {
    hasMore,
    setHasMore,
    loadingMore,
    hasMoreSessions,
    setHasMoreSessions,
    loadingMoreSessions,
    handleLoadMore,
    handleLoadMoreSessions,
    reloadSessions,
  }
}
