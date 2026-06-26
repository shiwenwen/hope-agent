import { useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

/**
 * Global store for the cron sidebar unread badge. The aggregate count comes from
 * the backend (`cron_unread_total`); it's refreshed whenever a cron run completes
 * (`cron:run_completed`) or the unread set changes (`cron:unread_changed`, e.g.
 * after a one-click "mark all read"). Mirrors `useDraftSkillsStore`'s shape.
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
  _initialized = true
  void reload()
  try {
    _unlisten.push(getTransport().listen("cron:run_completed", () => void reload()))
    _unlisten.push(getTransport().listen("cron:unread_changed", () => void reload()))
  } catch (e) {
    logger.error("cron", "CronUnreadStore::subscribe", "Failed to subscribe to cron events", e)
  }
}

/** Tear down subscriptions. Used in tests. */
export function disposeCronUnreadStore() {
  _unlisten.forEach((fn) => fn())
  _unlisten.length = 0
  _initialized = false
}

/** Refresh the badge from the backend (e.g. after viewing a cron conversation). */
export function refreshCronUnread() {
  void reload()
}

/** One-click clear: mark every cron session read, then zero the badge. */
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
