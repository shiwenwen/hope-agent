import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { getTransport } from "@/lib/transport-provider"
import { isTauriMode, parsePayload } from "@/lib/transport"

export interface NotificationConfig {
  enabled: boolean
}

let cachedConfig: NotificationConfig | null = null

/** Load notification config from backend and cache it. */
export async function loadNotificationConfig(): Promise<NotificationConfig> {
  cachedConfig = await getTransport().call<NotificationConfig>("get_notification_config")
  return cachedConfig
}

/** Get cached notification config (may be null if not loaded yet). */
export function getCachedConfig(): NotificationConfig | null {
  return cachedConfig
}

/** Save notification config to backend and update cache. */
export async function saveNotificationConfig(config: NotificationConfig): Promise<void> {
  await getTransport().call("save_notification_config", { config })
  cachedConfig = config
}

/**
 * Listen for backend config:changed events and hot-reload notification config.
 * Returns an unlisten function. Should be called once at app startup.
 */
export function listenNotificationConfigChange(): () => void {
  return getTransport().listen("config:changed", (raw) => {
    try {
      const payload = parsePayload<{ category?: string }>(raw)
      if (payload?.category === "notification") {
        loadNotificationConfig().catch(() => {})
      }
    } catch { /* ignore */ }
  })
}

/**
 * Send a native desktop notification.
 * Respects the global toggle and OS permission.
 *
 * In `hope-agent server` Web GUI (no `__TAURI_INTERNALS__`), routes
 * through the browser's `Notification` API instead of Tauri's plugin —
 * the Tauri plugin's `isPermissionGranted` / `sendNotification` go
 * through `invoke()` and silently reject in a plain browser.
 */
export async function notify(title: string, body: string): Promise<void> {
  if (!cachedConfig?.enabled) return

  if (isTauriMode()) {
    let granted = await isPermissionGranted()
    if (!granted) {
      const perm = await requestPermission()
      granted = perm === "granted"
    }
    if (!granted) return
    sendNotification({ title, body })
    return
  }

  if (typeof window === "undefined" || !("Notification" in window)) return
  if (Notification.permission === "default") {
    try {
      const perm = await Notification.requestPermission()
      if (perm !== "granted") return
    } catch {
      return
    }
  } else if (Notification.permission !== "granted") {
    return
  }
  try {
    new Notification(title, { body })
  } catch {
    // Browsers may throw on unsupported options or insecure (non-https)
    // contexts. Swallow — the alert is best-effort.
  }
}

/**
 * Determine if notifications are enabled for a given agent.
 * @param agentNotify - Per-agent override: true=on, false=off, null/undefined=use global
 */
export function isAgentNotifyEnabled(agentNotify: boolean | null | undefined): boolean {
  if (agentNotify === true) return true
  if (agentNotify === false) return false
  return cachedConfig?.enabled ?? true
}

// Cached focus state — updated by `initFocusTracking` so background-aware
// checks don't pay an IPC roundtrip on every alert.
let isWindowFocused = true
let focusTrackingStarted = false

/**
 * Start tracking the main window focus state. App-level singleton: safe
 * to call from multiple mount points (StrictMode double-invoke, hot
 * reload), and listeners stay registered for the process lifetime — no
 * cleanup is exposed.
 *
 * The fire-and-forget shape sidesteps a StrictMode race that the
 * cleanup-returning shape used to hit: if mount A's promise hadn't
 * resolved yet, cleanup A would tear down the listener while mount B's
 * call hit the `started` guard and returned a no-op, leaving the app
 * with no focus listener at all.
 */
export async function initFocusTracking(): Promise<void> {
  if (focusTrackingStarted) return
  focusTrackingStarted = true
  isWindowFocused = typeof document !== "undefined" ? document.hasFocus() : true

  if (isTauriMode()) {
    const win = getCurrentWindow()
    isWindowFocused = await win.isFocused()
    await win.onFocusChanged(({ payload }) => {
      isWindowFocused = payload
    })
    return
  }

  if (typeof window !== "undefined") {
    window.addEventListener("focus", () => {
      isWindowFocused = true
    })
    window.addEventListener("blur", () => {
      isWindowFocused = false
    })
  }
}

/**
 * Send a notification only when the app is in the background (unfocused
 * or hidden). Used for "needs user action" alerts — approval, ask_user,
 * MCP OAuth, channel auth failure — which the user already sees in-UI
 * when the window is up front.
 */
export async function notifyIfBackground(title: string, body: string): Promise<void> {
  if (isWindowFocused) return
  await notify(title, body)
}
