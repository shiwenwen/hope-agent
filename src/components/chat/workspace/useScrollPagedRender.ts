import { useEffect, useState } from "react"

export interface ScrollPagedRender<T> {
  visible: T[]
  hasMore: boolean
  /** Callback ref for the bottom sentinel marker (rendered while `hasMore`). */
  setSentinel: (el: HTMLElement | null) => void
}

/**
 * Render a long list incrementally as the user scrolls — no "load more"
 * button. Renders the first `step` items; when a bottom sentinel scrolls into
 * view, reveals `step` more. Used by the workspace panel's output / sources
 * sections (each a fixed-height, internally-scrolling card).
 *
 * The observer uses the viewport root (`root: null`); a sentinel inside an
 * `overflow:auto` ancestor is naturally clipped by it, so it only intersects
 * once scrolled into the section's visible area — no explicit root ref needed.
 *
 * `resetKey` (the session id) collapses the window back to `step` only on
 * session switch — NOT on every list update — so a streaming turn appending
 * new artifacts doesn't yank the user's scroll position back to the top.
 */
export function useScrollPagedRender<T>(
  items: T[],
  opts: { step?: number; resetKey?: unknown } = {},
): ScrollPagedRender<T> {
  const step = opts.step ?? 20
  const resetKey = opts.resetKey
  const [limit, setLimit] = useState(step)
  const [sentinel, setSentinel] = useState<HTMLElement | null>(null)

  // Reset the window on session switch — the React-blessed "adjust state during
  // render" pattern (prev value in state), which avoids a set-state-in-effect.
  const [prevKey, setPrevKey] = useState(resetKey)
  if (prevKey !== resetKey) {
    setPrevKey(resetKey)
    setLimit(step)
  }

  const hasMore = items.length > limit
  const visible = hasMore ? items.slice(0, limit) : items

  // Re-create the observer whenever `limit` changes so that, after a bump, an
  // already-visible sentinel re-evaluates and keeps filling until it's pushed
  // out of view (a single IntersectionObserver only fires on transitions).
  useEffect(() => {
    if (!sentinel || !hasMore) return
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setLimit((n) => n + step)
      },
      { rootMargin: "160px" },
    )
    obs.observe(sentinel)
    return () => obs.disconnect()
  }, [sentinel, hasMore, limit, step])

  return { visible, hasMore, setSentinel }
}
