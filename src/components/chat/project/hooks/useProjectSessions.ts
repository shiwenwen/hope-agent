/**
 * Independent, paginated session list for a single sidebar project group.
 *
 * Each expanded project fetches its own sessions via `list_project_sessions_cmd`
 * (one request per project), starting at `PROJECT_SESSION_PAGE_SIZE`. "Show more"
 * grows the window by one page; "show less" returns to the first page. Fetching
 * is lazy — collapsed groups issue no request.
 *
 * We use a window-refetch model (always `offset: 0`, `limit: windowSize`) rather
 * than incremental append: pages of 15 are cheap against the local SQLite store,
 * ordering stays correct, and there is no append/dedup race when sessions are
 * created or reordered between pages.
 *
 * Realtime freshness piggybacks on the shared global `sessions` array that the
 * ChatScreen keeps live (full reloads + incremental patches). `changeSignal` is a
 * fingerprint of this project's slice of that array; when it changes — a session
 * was created / renamed / reordered / read / pinned — we refetch the window.
 * `sessionCount` (live from `ProjectMeta`) is a backstop for membership changes
 * that don't surface in the global window (e.g. moving an old session in).
 */

import { useCallback, useEffect, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SessionMeta } from "@/types/chat"
import { PROJECT_SESSION_PAGE_SIZE } from "../../hooks/constants"

export interface UseProjectSessionsParams {
  projectId: string
  /** Whether the project group is expanded — fetching is lazy. */
  expanded: boolean
  /** Fingerprint of the project's slice of the global session array. Used purely
   *  as a realtime change trigger, never for rendering. */
  changeSignal: string
  /** Live total from `ProjectMeta`; backstop trigger for membership changes
   *  outside the global window. */
  sessionCount: number
  /** Target and exact sidebar-order position returned by the unread locator. */
  ensureSessionId?: string | null
  ensureSessionOffset?: number | null
}

export interface UseProjectSessionsReturn {
  sessions: SessionMeta[]
  total: number
  /** True only during the very first load (no data to show yet). */
  loading: boolean
  /** True while a "show more" fetch is in flight. */
  loadingMore: boolean
  /** More sessions exist beyond the current window. */
  hasMore: boolean
  /** Currently showing more than the base page. */
  canCollapse: boolean
  showMore: () => void
  showLess: () => void
}

export function useProjectSessions({
  projectId,
  expanded,
  changeSignal,
  sessionCount,
  ensureSessionId,
  ensureSessionOffset,
}: UseProjectSessionsParams): UseProjectSessionsReturn {
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [total, setTotal] = useState(0)
  const [windowSize, setWindowSize] = useState(PROJECT_SESSION_PAGE_SIZE)
  const [loading, setLoading] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)

  // Tracks whether we've ever loaded so re-expanding shows stale rows instead
  // of a spinner while the background refetch lands.
  const loadedOnceRef = useRef(false)
  // Monotonic request id — only the latest response is applied, so a slow
  // earlier fetch can't clobber a newer window.
  const reqSeqRef = useRef(0)
  // Previous values used to distinguish user-initiated discrete changes (first
  // expand, show more/less) — which fetch immediately — from changeSignal /
  // sessionCount churn during streaming, which is debounced.
  const prevWindowSizeRef = useRef(windowSize)
  const prevExpandedRef = useRef(expanded)

  useEffect(() => {
    if (!expanded) {
      // A collapsed group can't show "loading more"; clear it so a fetch torn
      // down mid-flight by collapse doesn't flash a stale footer spinner on the
      // next expand.
      setLoadingMore(false)
      prevExpandedRef.current = false
      return
    }
    let cancelled = false
    const seq = ++reqSeqRef.current
    if (!loadedOnceRef.current) setLoading(true)

    // First load and window changes (show more/less) are discrete user actions —
    // fetch immediately. Only changeSignal / sessionCount churn (streaming unread
    // / updated_at updates) is debounced, to coalesce storms into one refetch.
    const immediate =
      !loadedOnceRef.current || !prevExpandedRef.current || windowSize !== prevWindowSizeRef.current
    prevWindowSizeRef.current = windowSize
    prevExpandedRef.current = true

    const timer = setTimeout(
      async () => {
        try {
          const requestedLimit = ensureSessionId
            ? Math.max(windowSize, Math.floor(ensureSessionOffset ?? 0) + 1)
            : windowSize
          const [list, totalCount] = await getTransport().call<[SessionMeta[], number]>(
            "list_project_sessions_cmd",
            {
              id: projectId,
              limit: requestedLimit,
              offset: 0,
            },
          )
          if (cancelled || seq !== reqSeqRef.current) return
          if (requestedLimit !== windowSize) setWindowSize(requestedLimit)
          setSessions(list)
          setTotal(totalCount)
          loadedOnceRef.current = true
        } catch (err) {
          if (!cancelled) {
            logger.error("chat", "useProjectSessions", "Failed to load project sessions", err)
          }
        } finally {
          if (!cancelled && seq === reqSeqRef.current) {
            setLoading(false)
            setLoadingMore(false)
          }
        }
      },
      immediate ? 0 : 150,
    )

    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [
    expanded,
    projectId,
    windowSize,
    changeSignal,
    sessionCount,
    ensureSessionId,
    ensureSessionOffset,
  ])

  // Renaming a session that lives only in this per-project window (older than
  // the global session page) never flips `changeSignal` — the renamed row isn't
  // in the global array, and a rename bumps neither `updated_at` nor
  // `sessionCount`, so neither refetch trigger fires. Patch the title optimistically
  // off the rename event (single choke point in ChatScreen::handleRenameSession).
  useEffect(() => {
    const onRenamed = (event: Event) => {
      const detail = (event as CustomEvent<{ id?: string; title?: string }>).detail
      if (!detail?.id) return
      setSessions((prev) => {
        const idx = prev.findIndex((s) => s.id === detail.id)
        if (idx === -1) return prev
        const next = [...prev]
        next[idx] = { ...next[idx], title: detail.title ?? next[idx].title }
        return next
      })
    }
    window.addEventListener("hope:session-renamed", onRenamed)
    return () => window.removeEventListener("hope:session-renamed", onRenamed)
  }, [])

  const showMore = useCallback(() => {
    setLoadingMore(true)
    setWindowSize((w) => w + PROJECT_SESSION_PAGE_SIZE)
  }, [])

  const showLess = useCallback(() => {
    setWindowSize(PROJECT_SESSION_PAGE_SIZE)
    // Truncate immediately so the list shrinks instantly; the effect refetches
    // the smaller window in the background to stay fresh.
    setSessions((prev) => prev.slice(0, PROJECT_SESSION_PAGE_SIZE))
  }, [])

  return {
    sessions,
    total,
    loading,
    loadingMore,
    hasMore: total > sessions.length,
    canCollapse: windowSize > PROJECT_SESSION_PAGE_SIZE,
    showMore,
    showLess,
  }
}
