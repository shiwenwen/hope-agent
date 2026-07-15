import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SessionMeta } from "@/types/chat"
import { SESSION_PAGE_SIZE } from "../hooks/constants"
import type { SessionFilterType } from "./types"
import { filterSessionsForSidebarTab, sidebarSessionPageArgs } from "./sessionListModel"

const FILTERS: SessionFilterType[] = ["session", "subagent"]

type SessionsByFilter = Record<SessionFilterType, SessionMeta[]>
type NumberByFilter = Record<SessionFilterType, number>
type BooleanByFilter = Record<SessionFilterType, boolean>

const emptySessions = (): SessionsByFilter => ({ session: [], subagent: [] })
const emptyNumbers = (): NumberByFilter => ({ session: 0, subagent: 0 })
const emptyBooleans = (): BooleanByFilter => ({ session: false, subagent: false })

interface UseSidebarSessionPaginationParams {
  selectedAgentId: string | null
  currentSessionId: string | null
  enabled: boolean
  /** The global recent page is only a change signal; it is never rendered here. */
  refreshSignal: SessionMeta[]
  /** Rare reveal path with the backend-computed position in sidebar order. */
  ensureSessionId?: string | null
  ensureSessionOffset?: number | null
}

export function useSidebarSessionPagination({
  selectedAgentId,
  currentSessionId,
  enabled,
  refreshSignal,
  ensureSessionId,
  ensureSessionOffset,
}: UseSidebarSessionPaginationParams) {
  const [sessionsByFilter, setSessionsByFilter] = useState<SessionsByFilter>(emptySessions)
  const [totalsByFilter, setTotalsByFilter] = useState<NumberByFilter>(emptyNumbers)
  const [loading, setLoading] = useState(true)
  const [loadingMoreByFilter, setLoadingMoreByFilter] = useState<BooleanByFilter>(emptyBooleans)
  const loadedRowsRef = useRef<NumberByFilter>(emptyNumbers())
  const loadingMoreRef = useRef<BooleanByFilter>(emptyBooleans())
  const generationRef = useRef(0)
  const queryKeyRef = useRef<string | null>(null)
  const activeSessionIdRef = useRef(currentSessionId)
  activeSessionIdRef.current = currentSessionId

  const reload = useCallback(async () => {
    if (!enabled) return
    const generation = ++generationRef.current
    const queryKey = selectedAgentId ?? "*"
    if (queryKeyRef.current !== queryKey) {
      queryKeyRef.current = queryKey
      loadedRowsRef.current = emptyNumbers()
      setSessionsByFilter(emptySessions())
      setTotalsByFilter(emptyNumbers())
    }
    setLoading(true)
    loadingMoreRef.current = emptyBooleans()
    setLoadingMoreByFilter(emptyBooleans())

    try {
      const pages = await Promise.all(
        FILTERS.map(async (filter) => {
          let [rows, total] = await getTransport().call<[SessionMeta[], number]>(
            "list_sessions_cmd",
            sidebarSessionPageArgs(
              filter,
              selectedAgentId,
              0,
              SESSION_PAGE_SIZE,
              activeSessionIdRef.current,
            ),
          )

          // A second click may target a row outside the first page. The backend
          // returns its exact visual-order offset, so one prefix request replaces
          // a sequential page-by-page scan while preserving a contiguous list.
          if (
            filter === "session" &&
            ensureSessionId &&
            !rows.some((session) => session.id === ensureSessionId) &&
            rows.length < total
          ) {
            const requiredLimit = Math.min(
              total,
              Math.max(SESSION_PAGE_SIZE, Math.floor(ensureSessionOffset ?? 0) + 1),
            )
            if (requiredLimit > rows.length) {
              ;[rows, total] = await getTransport().call<[SessionMeta[], number]>(
                "list_sessions_cmd",
                sidebarSessionPageArgs(
                  filter,
                  selectedAgentId,
                  0,
                  requiredLimit,
                  activeSessionIdRef.current,
                ),
              )
            }
          }
          return {
            filter,
            rows: filterSessionsForSidebarTab(rows, filter, selectedAgentId),
            loadedRows: rows.length,
            total,
          }
        }),
      )
      if (generation !== generationRef.current) return

      const nextSessions = emptySessions()
      const nextTotals = emptyNumbers()
      const nextLoadedRows = emptyNumbers()
      for (const page of pages) {
        nextSessions[page.filter] = page.rows
        nextTotals[page.filter] = page.total
        nextLoadedRows[page.filter] = page.loadedRows
      }
      loadedRowsRef.current = nextLoadedRows
      setSessionsByFilter(nextSessions)
      setTotalsByFilter(nextTotals)
    } catch (error) {
      if (generation === generationRef.current) {
        logger.error(
          "chat",
          "ChatSidebar::loadSessions",
          "Failed to load filtered sidebar sessions",
          error,
        )
      }
    } finally {
      if (generation === generationRef.current) setLoading(false)
    }
  }, [enabled, ensureSessionId, ensureSessionOffset, selectedAgentId])

  useEffect(() => {
    void reload()
    return () => {
      generationRef.current += 1
    }
  }, [reload, refreshSignal])

  const loadMore = useCallback(
    async (filter: SessionFilterType) => {
      if (
        !enabled ||
        loadingMoreRef.current[filter] ||
        loadedRowsRef.current[filter] >= totalsByFilter[filter]
      ) {
        return
      }

      const generation = generationRef.current
      const offset = loadedRowsRef.current[filter]
      loadingMoreRef.current[filter] = true
      setLoadingMoreByFilter((prev) => ({ ...prev, [filter]: true }))
      try {
        const [rows, total] = await getTransport().call<[SessionMeta[], number]>(
          "list_sessions_cmd",
          sidebarSessionPageArgs(
            filter,
            selectedAgentId,
            offset,
            SESSION_PAGE_SIZE,
            activeSessionIdRef.current,
          ),
        )
        if (generation !== generationRef.current) return

        loadedRowsRef.current[filter] = offset + rows.length
        const filteredRows = filterSessionsForSidebarTab(rows, filter, selectedAgentId)
        setSessionsByFilter((prev) => {
          const seen = new Set(prev[filter].map((session) => session.id))
          return {
            ...prev,
            [filter]: [...prev[filter], ...filteredRows.filter((session) => !seen.has(session.id))],
          }
        })
        setTotalsByFilter((prev) => ({ ...prev, [filter]: total }))
      } catch (error) {
        if (generation === generationRef.current) {
          logger.error(
            "chat",
            "ChatSidebar::loadMoreSessions",
            "Failed to load more filtered sidebar sessions",
            error,
          )
        }
      } finally {
        if (generation === generationRef.current) {
          loadingMoreRef.current[filter] = false
          setLoadingMoreByFilter((prev) => ({ ...prev, [filter]: false }))
        }
      }
    },
    [enabled, selectedAgentId, totalsByFilter],
  )

  const hasMoreByFilter = useMemo<BooleanByFilter>(
    () => ({
      session: loadedRowsRef.current.session < totalsByFilter.session,
      subagent: loadedRowsRef.current.subagent < totalsByFilter.subagent,
    }),
    [sessionsByFilter, totalsByFilter],
  )

  return {
    sessionsByFilter,
    loading,
    loadingMoreByFilter,
    hasMoreByFilter,
    loadMore,
    reload,
  }
}
