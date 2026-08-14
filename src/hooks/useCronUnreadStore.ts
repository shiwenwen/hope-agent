import { useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

/**
 * Global store for the Scheduled sidebar unread badge. It filters the ordinary
 * Session watermark by Cron provenance (plus legacy `is_cron` compatibility),
 * so Chat and Scheduled never maintain competing read receipts.
 */
type Listener = () => void

let _count = 0
let _initialized = false
const _listeners = new Set<Listener>()
const _unlisten: Array<() => void> = []

function notify() {
  _listeners.forEach((fn) => fn())
}

function setCount(next: number) {
  const n = Number.isFinite(next) && next > 0 ? Math.floor(next) : 0
  if (n === _count) return
  _count = n
  notify()
}

async function reload() {
  try {
    const total = await getTransport().call<number>("cron_unread_total")
    setCount(typeof total === "number" ? total : 0)
  } catch (e) {
    logger.error("cron", "CronUnreadStore::reload", "Failed to load cron unread total", e)
  }
}

/** Initialize the store. Idempotent — extra calls are no-ops (no extra IPC). */
export function initCronUnreadStore() {
  if (_initialized) return
  try {
    _unlisten.push(getTransport().listen("cron:run_completed", () => void reload()))
    _unlisten.push(getTransport().listen("cron:unread_changed", () => void reload()))
    _unlisten.push(
      getTransport().listen("session:unread_changed", (raw) => {
        const payload = raw && typeof raw === "object" ? (raw as { domain?: string | null }) : null
        // Ordinary Cron-origin sessions emit `regular`; legacy rows emit
        // `cron`; mixed bulk/archive mutations are unscoped. All three can
        // change the Scheduled projection, while channel-only reads cannot.
        if (!payload?.domain || payload.domain === "cron" || payload.domain === "regular") {
          void reload()
        }
      }),
    )
    // Only latch initialized once the subscriptions are actually attached, so a
    // throwing listen() leaves the store re-initializable on the next call
    // rather than permanently wedged with no listeners.
    _initialized = true
  } catch (e) {
    logger.error("cron", "CronUnreadStore::subscribe", "Failed to subscribe to cron events", e)
    _unlisten.forEach((fn) => fn())
    _unlisten.length = 0
  }
  void reload()
}

/** Tear down subscriptions and reset state. Used in tests. */
export function disposeCronUnreadStore() {
  _unlisten.forEach((fn) => fn())
  _unlisten.length = 0
  _listeners.clear()
  _count = 0
  _initialized = false
}

/** Refresh the authoritative Scheduled projection from the backend. */
export function refreshCronUnread() {
  void reload()
}

/** Advance one viewed Session's ordinary watermark, then reconcile Scheduled. */
export async function markCronSessionRead(sessionId: string): Promise<void> {
  await getTransport().call("mark_session_read_cmd", { sessionId })
  await reload()
}

/** One-click clear over ordinary Cron-origin and legacy Cron Sessions. */
export async function markAllCronRead(): Promise<void> {
  try {
    await getTransport().call("cron_mark_all_read")
    setCount(0)
  } catch (e) {
    logger.error("cron", "CronUnreadStore::markAllRead", "Failed to mark all cron sessions read", e)
    throw e
  }
}

function subscribe(listener: Listener): () => void {
  _listeners.add(listener)
  return () => _listeners.delete(listener)
}

function getCount(): number {
  return _count
}

export function useCronUnreadStore(): { cronUnreadCount: number } {
  const cronUnreadCount = useSyncExternalStore(subscribe, getCount)
  return { cronUnreadCount }
}
