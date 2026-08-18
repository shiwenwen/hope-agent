import { useCallback, useRef, useState } from "react"

/** Scope key for a chat that has not been persisted yet. */
export const DRAFT_TAB_SCOPE = "__draft__"

export interface ScopeSwitchOptions {
  /** Restore a previously retained destination set. Defaults to true. */
  restore?: boolean
  /** Retain the set being left. Incognito callers must pass false. */
  cacheCurrent?: boolean
}

export interface ScopedTabState<T> {
  state: T
  setState: React.Dispatch<React.SetStateAction<T>>
  /** Swap the visible set, retaining/restoring per-scope sets in memory. */
  switchScope: (scopeKey: string, options?: ScopeSwitchOptions) => void
  /** Same set, new address — used when a draft becomes a real session. */
  renameScope: (from: string, to: string) => void
}

/**
 * The session-scoped tab bookkeeping shared by the file-browser tabs and the
 * file-preview tabs: one live set plus a per-scope cache, so leaving a session
 * retains its set and returning restores it. `emptyState` must be a stable
 * module constant.
 */
export function useScopedTabState<T>(emptyState: T): ScopedTabState<T> {
  const [state, setState] = useState<T>(emptyState)
  const scopeRef = useRef(DRAFT_TAB_SCOPE)
  const cacheRef = useRef(new Map<string, T>())

  const switchScope = useCallback(
    (scopeKey: string, options: ScopeSwitchOptions = {}) => {
      const { restore = true, cacheCurrent = true } = options
      const previousScope = scopeRef.current
      scopeRef.current = scopeKey
      setState((current) => {
        // Cache writes are idempotent, so a repeated updater call is safe.
        if (previousScope === scopeKey) {
          if (restore && cacheCurrent) return current
          if (!cacheCurrent || !restore) cacheRef.current.delete(scopeKey)
          return restore ? current : emptyState
        }
        if (cacheCurrent) cacheRef.current.set(previousScope, current)
        else cacheRef.current.delete(previousScope)
        if (!restore) {
          cacheRef.current.delete(scopeKey)
          return emptyState
        }
        return cacheRef.current.get(scopeKey) ?? emptyState
      })
    },
    [emptyState],
  )

  const renameScope = useCallback((from: string, to: string) => {
    if (from === to) return
    const cached = cacheRef.current.get(from)
    cacheRef.current.delete(from)
    if (cached !== undefined) cacheRef.current.set(to, cached)
    if (scopeRef.current === from) scopeRef.current = to
  }, [])

  return { state, setState, switchScope, renameScope }
}
