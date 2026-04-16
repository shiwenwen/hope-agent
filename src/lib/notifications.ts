import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification"
import { getTransport } from "@/lib/transport-provider"

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
      const payload = typeof raw === "string" ? JSON.parse(raw) : raw
      if (payload?.category === "notification") {
        loadNotificationConfig().catch(() => {})
      }
    } catch { /* ignore */ }
  })
}

/**
 * Send a native desktop notification.
 * Respects the global toggle and OS permission.
 */
export async function notify(title: string, body: string): Promise<void> {
  if (!cachedConfig?.enabled) return

  let granted = await isPermissionGranted()
  if (!granted) {
    const perm = await requestPermission()
    granted = perm === "granted"
  }
  if (!granted) return

  sendNotification({ title, body })
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
