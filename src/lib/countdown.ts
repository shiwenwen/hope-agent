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

// ── Shared 1Hz ticker ────────────────────────────────────────────
// One module-level interval drives every subscribed countdown instead of one
// interval per sidebar row. The interval only runs while at least one
// component is subscribed, so an idle sidebar costs nothing.

const listeners = new Set<() => void>()
let tickerId: number | undefined

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  if (tickerId === undefined) {
    tickerId = window.setInterval(() => {
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

/**
 * Remaining whole seconds until `localDeadlineMs` (already clock-skew
 * corrected by the caller), clamped at 0; `null` when there is no deadline.
 * The snapshot is quantized to whole seconds so repeated reads within one
 * tick are referentially stable for `useSyncExternalStore`.
 */
export function useCountdownRemainingSec(localDeadlineMs: number | null): number | null {
  return useSyncExternalStore(subscribe, () =>
    localDeadlineMs == null
      ? null
      : Math.max(0, Math.ceil((localDeadlineMs - Date.now()) / 1000)),
  )
}
