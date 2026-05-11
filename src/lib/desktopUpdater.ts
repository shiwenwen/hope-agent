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

/**
 * UI-level gate: any Tauri desktop shell (including `pnpm tauri dev`) shows
 * the manual-check button so the flow can be exercised in dev. The actual
 * `check()` call short-circuits in dev to avoid the plugin's noisy
 * "endpoint unreachable" log when running off a non-bundled binary.
 */
export function isDesktopUpdaterAvailable(): boolean {
  return isTauriMode()
}

export async function checkForDesktopUpdate(): Promise<DesktopUpdate | null> {
  if (!isTauriMode()) return null
  if (!import.meta.env.PROD) {
    // Dev builds always report "up to date" — the running binary isn't a
    // .app/.exe the updater can replace, so a real check is meaningless.
    console.info("[updater] dev mode — skipping real check, reporting up-to-date")
    return null
  }
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

// ─── Manual-check request bus ───────────────────────────────
// Bridges "Check for Updates" entry points (macOS app menu, future
// keyboard shortcut, etc.) to AboutPanel without relying on Tauri event
// timing. The native menu emits `open-settings { section: "about" }` +
// `desktop-update-check` back-to-back; if the panel isn't mounted yet,
// a direct event listener inside AboutPanel misses the event entirely
// because Tauri events don't queue for future subscribers.
//
// Contract:
//   - App.tsx always mounts and forwards the event into requestManualCheck().
//   - If no subscriber is mounted, the request is queued (single-slot).
//   - When AboutPanel mounts and subscribes, any queued request is
//     delivered immediately so the check still runs.

let _checkPending = false
const _checkListeners = new Set<() => void>()

export function requestManualCheck(): void {
  if (_checkListeners.size === 0) {
    _checkPending = true
    return
  }
  _checkListeners.forEach((fn) => {
    try {
      fn()
    } catch (err) {
      console.error("[updater] manual-check listener threw", err)
    }
  })
}

export function subscribeManualCheckRequests(listener: () => void): () => void {
  _checkListeners.add(listener)
  if (_checkPending) {
    _checkPending = false
    try {
      listener()
    } catch (err) {
      console.error("[updater] manual-check listener threw on flush", err)
    }
  }
  return () => {
    _checkListeners.delete(listener)
  }
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
