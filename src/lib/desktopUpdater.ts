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
  return isTauriMode()
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
