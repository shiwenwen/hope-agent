import { getTransport } from "@/lib/transport-provider"

/**
 * Mirrors `ha_core::config::StartupNotificationConfig` — see
 * `crates/ha-core/src/channel/worker/startup_watcher.rs` for the
 * subsystem these knobs drive.
 */
export interface StartupNotificationConfig {
  enabled: boolean
  windowSecs: number
  globalMax: number
  cooldownSecs: number
  crashLoopThreshold: number
}

export async function loadStartupNotificationConfig(): Promise<StartupNotificationConfig> {
  return await getTransport().call<StartupNotificationConfig>(
    "get_startup_notification_config",
  )
}

export async function saveStartupNotificationConfig(
  config: StartupNotificationConfig,
): Promise<void> {
  await getTransport().call("save_startup_notification_config", { config })
}
