import { useSyncExternalStore } from "react"

/**
 * Language-neutral compact remaining-time label ("45s" / "3m 20s" / "1h 5m").
 * Shared by the ask_user question block and the sidebar pending countdown.
 */
export function formatRemaining(secs: number): string {
  if (secs <= 0) return "0s"
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  const s = secs % 60
  if (m < 60) return `${m}m ${s}s`
  const h = Math.floor(m / 60)
  return `${h}h ${m % 60}m`
}

// ── Shared 1Hz clock ─────────────────────────────────────────────
// One module-level interval drives every subscribed countdown instead of one
// interval per sidebar row / dialog. The interval only runs while at least one
// component is subscribed, so an idle sidebar costs nothing.
//
// The snapshot is a *cached* `nowMs`, refreshed once per tick, rather than a
// fresh `Date.now()` per read. That satisfies `useSyncExternalStore`'s
// stable-snapshot contract: repeated reads between ticks are referentially
// equal, so no spurious re-render / "getSnapshot should be cached" warning can
// occur at a second boundary.

let nowMs = Date.now()
const listeners = new Set<() => void>()
let tickerId: number | undefined

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  // Refresh on subscribe so a component mounting long after module load paints
  // against a current clock (React re-reads getSnapshot right after subscribe).
  nowMs = Date.now()
  if (tickerId === undefined) {
    tickerId = window.setInterval(() => {
      nowMs = Date.now()
      for (const l of listeners) l()
    }, 1000)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && tickerId !== undefined) {
      window.clearInterval(tickerId)
      tickerId = undefined
    }
  }
}

function getNowSnapshot(): number {
  return nowMs
}

/**
 * Subscribe to the shared 1Hz wall clock. Returns a cached `Date.now()` that
 * advances once per second — safe to read during render.
 */
export function useNowMs(): number {
  return useSyncExternalStore(subscribe, getNowSnapshot)
}

/**
 * Remaining whole seconds until `localDeadlineMs` (already clock-skew
 * corrected by the caller), clamped at 0; `null` when there is no deadline.
 * Derived from the shared clock, so it ticks in lockstep with every other
 * countdown and re-renders at most once per second.
 */
export function useCountdownRemainingSec(localDeadlineMs: number | null): number | null {
  const now = useNowMs()
  if (localDeadlineMs == null) return null
  return Math.max(0, Math.ceil((localDeadlineMs - now) / 1000))
}
