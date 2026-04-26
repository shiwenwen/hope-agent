import { isTauriMode } from "@/lib/transport"

export type DesktopUpdateEvent =
  | { event: "Started"; data: { contentLength: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" }

export interface DesktopUpdate {
  currentVersion: string
  version: string
  body?: string
  date?: string
  downloadAndInstall(onEvent?: (event: DesktopUpdateEvent) => void): Promise<void>
  close?(): Promise<void>
}

export function isDesktopUpdaterAvailable(): boolean {
  // In `pnpm tauri dev`, the GitHub release updater endpoint may legitimately
  // be unavailable and the plugin logs that as an error. Only check updates
  // from packaged desktop builds.
  return isTauriMode() && import.meta.env.PROD
}

export async function checkForDesktopUpdate(): Promise<DesktopUpdate | null> {
  if (!isDesktopUpdaterAvailable()) return null
  const { check } = await import("@tauri-apps/plugin-updater")
  return (await check()) as DesktopUpdate | null
}

export async function disposeDesktopUpdate(
  update: DesktopUpdate | null | undefined,
): Promise<void> {
  if (!update?.close) return
  await update.close()
}

export async function relaunchDesktopApp(): Promise<void> {
  if (!isDesktopUpdaterAvailable()) return
  const { relaunch } = await import("@tauri-apps/plugin-process")
  await relaunch()
}

// ─── Global update store ────────────────────────────────────
// Module-level singleton so every component sees the same state.

type Listener = () => void

let _pendingUpdate: DesktopUpdate | null = null
let _checked = false
const _listeners = new Set<Listener>()

function _notify() {
  _listeners.forEach((fn) => fn())
}

/** Subscribe to update-store changes. Returns unsubscribe function. */
export function subscribeUpdateStore(listener: Listener): () => void {
  _listeners.add(listener)
  return () => _listeners.delete(listener)
}

/** Read current pending update (may be null). */
export function getPendingUpdate(): DesktopUpdate | null {
  return _pendingUpdate
}

/** Whether the initial auto-check has completed. */
export function hasChecked(): boolean {
  return _checked
}

/** Set the pending update (called by AboutPanel after manual check too). */
export async function setPendingUpdate(update: DesktopUpdate | null): Promise<void> {
  if (_pendingUpdate && _pendingUpdate !== update) {
    await disposeDesktopUpdate(_pendingUpdate)
  }
  _pendingUpdate = update
  _notify()
}

/**
 * Auto-check for updates silently in the background.
 * Returns the update if found, null otherwise.
 * Safe to call multiple times — subsequent calls are no-ops.
 */
let _autoCheckPromise: Promise<DesktopUpdate | null> | null = null

export function autoCheckForUpdate(force = false): Promise<DesktopUpdate | null> {
  if (!isDesktopUpdaterAvailable()) return Promise.resolve(null)
  if (_autoCheckPromise && !force) return _autoCheckPromise

  _autoCheckPromise = checkForDesktopUpdate()
    .then(async (update) => {
      _checked = true
      if (update) {
        await setPendingUpdate(update)
      }
      _notify()
      return update
    })
    .catch(() => {
      _checked = true
      _notify()
      return null
    })

  return _autoCheckPromise
}

/** 
 * Starts a background interval to check for updates every 12 hours. 
 * Returns a cleanup function.
 */
export function startPeriodicUpdateCheck(): () => void {
  if (!isDesktopUpdaterAvailable()) return () => {}
  
  // 12 hours in milliseconds
  const CHECK_INTERVAL = 12 * 60 * 60 * 1000
  
  const timerId = setInterval(() => {
    autoCheckForUpdate(true).catch(() => {})
  }, CHECK_INTERVAL)
  
  return () => clearInterval(timerId)
}
