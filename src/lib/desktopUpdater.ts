import { isTauriMode } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { APP_VERSION } from "@/lib/appMeta"

export type DesktopUpdateEvent =
  | { event: "Started"; data: { contentLength: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" }

export interface DesktopUpdate {
  currentVersion: string
  version: string
  body?: string
  date?: string
  /** Download the update without installing (silent pre-download). */
  download?(onEvent?: (event: DesktopUpdateEvent) => void): Promise<void>
  /** Install a previously-downloaded update. */
  install?(): Promise<void>
  downloadAndInstall(onEvent?: (event: DesktopUpdateEvent) => void): Promise<void>
  close?(): Promise<void>
}

// ─── Auto-update config (shared single source of truth) ─────
// Mirrors `AppConfig.auto_update` (crates/ha-core/src/updater/config.rs). Read
// via the `get_auto_update_config` command; cached after first fetch.

export interface AutoUpdateConfig {
  checkEnabled: boolean
  checkIntervalHours: number
  autoDownload: boolean
  notify: boolean
}

const DEFAULT_AUTO_UPDATE_CONFIG: AutoUpdateConfig = {
  checkEnabled: true,
  checkIntervalHours: 0.5,
  autoDownload: true,
  notify: true,
}

const MIN_AUTO_UPDATE_INTERVAL_HOURS = 0.5
const MAX_AUTO_UPDATE_INTERVAL_HOURS = 168
const DEV_FAKE_UPDATE_FLAG = "hope.devFakeUpdate"

function clampAutoUpdateIntervalHours(value: unknown): number {
  const hours = Number(value)
  if (!Number.isFinite(hours)) return MIN_AUTO_UPDATE_INTERVAL_HOURS
  return Math.min(MAX_AUTO_UPDATE_INTERVAL_HOURS, Math.max(MIN_AUTO_UPDATE_INTERVAL_HOURS, hours))
}

let _autoUpdateConfig: AutoUpdateConfig | null = null

/** Fetch (and cache) the auto-update config. Falls back to defaults on error. */
export async function getAutoUpdateConfig(force = false): Promise<AutoUpdateConfig> {
  if (_autoUpdateConfig && !force) return _autoUpdateConfig
  try {
    const cfg = await getTransport().call<AutoUpdateConfig>("get_auto_update_config")
    _autoUpdateConfig = { ...DEFAULT_AUTO_UPDATE_CONFIG, ...cfg }
  } catch {
    _autoUpdateConfig = { ...DEFAULT_AUTO_UPDATE_CONFIG }
  }
  return _autoUpdateConfig
}

/** Invalidate the cached config (call after a settings save). */
export function invalidateAutoUpdateConfig(): void {
  _autoUpdateConfig = null
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
    if (isDevFakeUpdateEnabled()) {
      logger.info("updater", "desktopUpdater::checkForDesktopUpdate", "dev fake update enabled")
      return createDevFakeUpdate()
    }
    // Dev builds always report "up to date" — the running binary isn't a
    // .app/.exe the updater can replace, so a real check is meaningless.
    logger.info("updater", "desktopUpdater::checkForDesktopUpdate", "dev mode — skipping real check, reporting up-to-date")
    return null
  }
  const { check } = await import("@tauri-apps/plugin-updater")
  return (await check()) as DesktopUpdate | null
}

function isDevFakeUpdateEnabled(): boolean {
  try {
    return window.localStorage.getItem(DEV_FAKE_UPDATE_FLAG) === "1"
  } catch {
    return false
  }
}

function createDevFakeUpdate(): DesktopUpdate {
  return {
    currentVersion: APP_VERSION,
    version: "99.99.99",
    date: new Date().toISOString(),
    body:
      "### Dev fake update\n\n" +
      "- Sidebar update button should appear below the logo.\n" +
      "- Clicking it should open this update panel.\n" +
      "- Background pre-download and install controls use the normal UI state.",
    download: emitDevFakeDownload,
    install: async () => {},
    downloadAndInstall: async (onEvent) => {
      await emitDevFakeDownload(onEvent)
    },
    close: async () => {},
  }
}

async function emitDevFakeDownload(onEvent?: (event: DesktopUpdateEvent) => void): Promise<void> {
  const chunkLength = 512 * 1024
  const chunks = 8
  onEvent?.({ event: "Started", data: { contentLength: chunkLength * chunks } })
  for (let i = 0; i < chunks; i += 1) {
    await new Promise((resolve) => window.setTimeout(resolve, 80))
    onEvent?.({ event: "Progress", data: { chunkLength } })
  }
  onEvent?.({ event: "Finished" })
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

/** Download lifecycle for the pending update, surfaced to the toast UI. */
export type DownloadStatus = "idle" | "downloading" | "downloaded"

let _pendingUpdate: DesktopUpdate | null = null
let _checked = false
let _downloadStatus: DownloadStatus = "idle"
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

/** Current silent-download status of the pending update. */
export function getDownloadStatus(): DownloadStatus {
  return _downloadStatus
}

/** Set the pending update (called by AboutPanel after manual check too). */
export async function setPendingUpdate(update: DesktopUpdate | null): Promise<void> {
  if (_pendingUpdate && _pendingUpdate !== update) {
    await disposeDesktopUpdate(_pendingUpdate)
  }
  _pendingUpdate = update
  _downloadStatus = "idle"
  _notify()
}

/**
 * Silently download the pending update without installing, so a later install
 * is instant. Single-flight: concurrent calls share one download. Marks the
 * store `downloaded` on success. Best-effort — a failed silent download just
 * leaves status `idle` so the user can retry from the toast.
 */
let _silentDownloadPromise: Promise<void> | null = null
export function silentDownload(update: DesktopUpdate): Promise<void> {
  if (_downloadStatus === "downloaded") return Promise.resolve()
  if (_silentDownloadPromise) return _silentDownloadPromise
  if (!update.download) return Promise.resolve() // plugin too old; install will download

  _downloadStatus = "downloading"
  _notify()
  _silentDownloadPromise = update
    .download()
    .then(() => {
      _downloadStatus = "downloaded"
      _notify()
    })
    .catch((err) => {
      logger.error("updater", "desktopUpdater::silentDownload", "silent download failed", err)
      _downloadStatus = "idle"
      _notify()
    })
    .finally(() => {
      _silentDownloadPromise = null
    })
  return _silentDownloadPromise
}

/**
 * Download (if not already pre-downloaded) then install the update. Does NOT
 * relaunch — the caller decides when, so an in-flight chat turn isn't cut off.
 * Waits on any in-flight silent download to avoid a double `download()`.
 * `onEvent` receives byte-progress only while actually downloading.
 */
export async function installUpdate(
  update: DesktopUpdate,
  onEvent?: (event: DesktopUpdateEvent) => void,
): Promise<void> {
  if (_silentDownloadPromise) await _silentDownloadPromise
  if (_downloadStatus !== "downloaded" && update.download) {
    _downloadStatus = "downloading"
    _notify()
    await update.download(onEvent)
    _downloadStatus = "downloaded"
    _notify()
  }
  if (update.install) {
    await update.install()
  } else {
    // Plugin predates split download/install — fall back to the combined call.
    await update.downloadAndInstall(onEvent)
  }
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
      logger.error("updater", "desktopUpdater::requestManualCheck", "manual-check listener threw", err)
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
      logger.error("updater", "desktopUpdater::subscribeManualCheckRequests", "manual-check listener threw on flush", err)
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

  _autoCheckPromise = (async () => {
    try {
      const cfg = await getAutoUpdateConfig()
      if (!cfg.checkEnabled) {
        _checked = true
        _notify()
        return null
      }
      const update = await checkForDesktopUpdate()
      _checked = true
      if (update) {
        // `notify` gates surfacing the update to the user (the toast renders
        // off the store's pending update). With notify off we still pre-
        // download if enabled, but stay silent — matching the headless loop.
        if (cfg.notify) await setPendingUpdate(update)
        // Silent pre-download so the eventual install is instant. Fire-and-
        // forget — the toast reflects status via the store.
        if (cfg.autoDownload) void silentDownload(update)
      }
      _notify()
      return update
    } catch {
      _checked = true
      _notify()
      return null
    }
  })()

  return _autoCheckPromise
}

/**
 * Starts a background interval to check for updates. The cadence follows
 * `auto_update.checkIntervalHours` (clamped to the backend-supported range)
 * and the loop is a no-op while `checkEnabled` is false — re-evaluated on
 * each tick so config edits take effect without a restart. Returns a cleanup
 * function.
 */
export function startPeriodicUpdateCheck(): () => void {
  if (!isDesktopUpdaterAvailable()) return () => {}

  let cancelled = false
  let timerId: ReturnType<typeof setTimeout> | undefined

  const scheduleNext = async () => {
    if (cancelled) return
    const cfg = await getAutoUpdateConfig(true)
    const hours = clampAutoUpdateIntervalHours(cfg.checkIntervalHours)
    timerId = setTimeout(async () => {
      if (cancelled) return
      const fresh = await getAutoUpdateConfig(true)
      if (fresh.checkEnabled) {
        await autoCheckForUpdate(true).catch(() => {})
      }
      void scheduleNext()
    }, hours * 60 * 60 * 1000)
  }

  void scheduleNext()

  return () => {
    cancelled = true
    if (timerId) clearTimeout(timerId)
  }
}
