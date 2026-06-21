import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { formatBytes } from "@/lib/format"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { cn } from "@/lib/utils"
import {
  Globe,
  Loader2,
  Plug,
  Plus,
  Trash2,
  Power,
  RefreshCw,
  CircleDot,
  CircleOff,
  ExternalLink,
  CheckCircle2,
  AlertTriangle,
  Sparkles,
  Download,
  Copy,
  FolderOpen,
  Save,
  Check,
} from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface BrowserProfileInfo {
  name: string
  path: string
  isBuiltin: boolean
  canDelete: boolean
  headless: boolean
  persistent: boolean
  sizeBytes: number
  lastUsedAt: number | null
  isActive: boolean
}

interface BrowserTabInfo {
  targetId: string
  url: string
  title: string
  isActive: boolean
}

interface BrowserStatus {
  connected: boolean
  mode: "launch" | "connect" | null
  profile: string | null
  connectionUrl: string | null
  profilesDir: string
  tabs: BrowserTabInfo[]
}

interface BrowserExtensionStatus {
  kind: string
  backendAvailable: boolean
  nativeHostName: string
  nativeHostManifestPath?: string | null
  nativeHostManifestExists: boolean
  extensionConnected: boolean
  extensionProtocolVersion?: number | null
  extensionVersion?: string | null
  extensionIds: string[]
  storeUrl?: string | null
  unpackedExtensionPath?: string | null
  nativeHostBinaryHint?: string | null
  message: string
  nextAction?: string | null
}

interface NativeHostInstallResult {
  nativeHostName: string
  hostPath: string
  manifestPath: string
  allowedOrigin: string
  windowsRegistryKey?: string | null
}

interface BrowserExtensionStopResult {
  stoppedTabs: number
  message: string
}

interface LaunchOptions {
  profile?: string | null
  executablePath?: string | null
  headless?: boolean
}

type BrowserMode = "managed" | "user_attach"

// Mirrors `BrowserBackendPreference` (snake_case serde) in
// `crates/ha-core/src/browser/mod.rs`. `None`/unset on the wire = `extension_first`.
type BrowserBackendPreference = "extension_first" | "cdp_only" | "extension_only"

// Browser config is partly UI-managed (`defaultMode` lives here only as a
// remembered tab preference) and partly opaque to this panel — `profiles`,
// `defaultProfile`, `heartbeatIntervalSecs`, `launchCircuit` etc. live in
// `AppConfig.browser` and are configured via `config.json` until full inline
// CRUD lands. We must round-trip those fields unchanged: `browser_set_config`
// replaces `AppConfig.browser` wholesale, so dropping a field here deletes it
// in the backend. The index signature lets us read the server JSON whole and
// echo it back without naming every key.
interface BrowserConfig {
  defaultMode?: BrowserMode
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [key: string]: any
}

interface ProbeUserChromeReport {
  found: boolean
  browserUrl: string
  version?: string
}

interface RuntimeChromiumReport {
  revision: number
  binaryPath: string
}

interface BrowserDoctorReport {
  probe: ProbeUserChromeReport
  chromeAlreadyRunning: boolean
  systemChromePath?: string
  runtimeChromium?: RuntimeChromiumReport
}

interface SpawnUserChromeResult {
  port: number
  debugUrl: string
  userDataDir: string
  chromeWasAlreadyRunning: boolean
}

// ── Helpers ──────────────────────────────────────────────────────

function formatRelative(
  ts: number | null,
  t: (k: string, opts?: Record<string, unknown>) => string,
): string {
  if (!ts) return "—"
  const now = Math.floor(Date.now() / 1000)
  const diff = now - ts
  if (diff < 60) return t("settings.browser.justNow")
  if (diff < 3600) return t("settings.browser.minutesAgo", { count: Math.floor(diff / 60) })
  if (diff < 86400) return t("settings.browser.hoursAgo", { count: Math.floor(diff / 3600) })
  return t("settings.browser.daysAgo", { count: Math.floor(diff / 86400) })
}

// ── Panel ────────────────────────────────────────────────────────

export default function BrowserPanel() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<BrowserStatus | null>(null)
  const [extensionStatus, setExtensionStatus] = useState<BrowserExtensionStatus | null>(null)
  const [extensionIdInput, setExtensionIdInput] = useState<string>("")
  const [nativeHostPathInput, setNativeHostPathInput] = useState<string>("")
  const [profiles, setProfiles] = useState<BrowserProfileInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<
    | null
    | "launch"
    | "connect"
    | "disconnect"
    | "spawn-user-chrome"
    | "install-native-host"
    | "stop-extension-control"
  >(null)
  const [error, setError] = useState<string | null>(null)

  // Launch form
  const [selectedProfile, setSelectedProfile] = useState<string>("")
  const [executablePath, setExecutablePath] = useState<string>("")
  const [headless, setHeadless] = useState<boolean>(false)

  // New profile form
  const [newProfileName, setNewProfileName] = useState<string>("")
  const [creating, setCreating] = useState<boolean>(false)

  // Connect form
  const [connectUrl, setConnectUrl] = useState<string>("http://127.0.0.1:9222")

  // Delete confirm
  const [pendingDelete, setPendingDelete] = useState<BrowserProfileInfo | null>(null)

  // Mode + doctor state
  const [browserCfg, setBrowserCfg] = useState<BrowserConfig>({
    defaultMode: "managed",
  })
  const [savingCfg, setSavingCfg] = useState<boolean>(false)

  // Advanced extension-backend settings — a local draft committed via an
  // explicit three-state Save button (these are HIGH-risk knobs, so no
  // optimistic auto-save like the mode tabs). Defaults mirror the backend:
  // backend preference unset = extension_first; extension.enabled / allowRawCdp
  // unset = true.
  const [advBackendPref, setAdvBackendPref] =
    useState<BrowserBackendPreference>("extension_first")
  const [advExtEnabled, setAdvExtEnabled] = useState<boolean>(true)
  const [advAllowRawCdp, setAdvAllowRawCdp] = useState<boolean>(true)
  const [advSaving, setAdvSaving] = useState<boolean>(false)
  const [advSaveStatus, setAdvSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const [doctor, setDoctor] = useState<BrowserDoctorReport | null>(null)
  // `null` when closed; carries the at-open snapshot of `chromeAlreadyRunning`
  // so the modal copy doesn't flicker if the user takes their time confirming.
  const [confirmSpawn, setConfirmSpawn] = useState<{ chromeAlreadyRunning: boolean } | null>(null)

  // Chromium runtime install — only relevant when the host has no Chrome.
  // `installing` keeps the button in spinner state; `installPercent` lets
  // the live progress bar render before any download bytes have arrived.
  const [installing, setInstalling] = useState<boolean>(false)
  const [installPercent, setInstallPercent] = useState<number | null>(null)
  const [installError, setInstallError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    // Critical path: status / profiles / config must render even if the
    // best-effort doctor probe fails. Use `allSettled` so a 2s probe timeout
    // or a `pgrep` hiccup can't blank the whole panel.
    const [st, pf, cfg, doc, ext] = await Promise.allSettled([
      getTransport().call<BrowserStatus>("browser_get_status"),
      getTransport().call<BrowserProfileInfo[]>("browser_list_profiles"),
      getTransport().call<BrowserConfig>("browser_get_config"),
      getTransport().call<BrowserDoctorReport>("browser_doctor"),
      getTransport().call<BrowserExtensionStatus>("browser_extension_status"),
    ])
    const firstError = [st, pf, cfg, doc].find(
      (r): r is PromiseRejectedResult => r.status === "rejected",
    )
    if (st.status === "fulfilled") setStatus(st.value)
    if (ext.status === "fulfilled") {
      setExtensionStatus(ext.value)
      setExtensionIdInput((prev) => prev || ext.value.extensionIds[0] || "")
      setNativeHostPathInput((prev) => prev || ext.value.nativeHostBinaryHint || "")
    }
    if (pf.status === "fulfilled") setProfiles(pf.value)
    if (cfg.status === "fulfilled") {
      // Keep the full config snapshot so unrelated fields (profiles,
      // heartbeatIntervalSecs, launchCircuit, etc.) survive the next
      // `browser_set_config` round-trip.
      setBrowserCfg({
        ...cfg.value,
        defaultMode: (cfg.value.defaultMode ?? "managed") as BrowserMode,
      })
      // Re-seed the advanced draft from the server snapshot so the controls
      // reflect persisted values after every refresh.
      setAdvBackendPref(
        (cfg.value.backendPreference ?? "extension_first") as BrowserBackendPreference,
      )
      const ext = (cfg.value.extension ?? {}) as Record<string, unknown>
      setAdvExtEnabled(ext.enabled !== false)
      setAdvAllowRawCdp(ext.allowRawCdp !== false)
    }
    if (doc.status === "fulfilled") setDoctor(doc.value)
    if (pf.status === "fulfilled" && !selectedProfile && pf.value.length > 0) {
      const configuredDefault =
        cfg.status === "fulfilled" && typeof cfg.value.defaultProfile === "string"
          ? cfg.value.defaultProfile
          : "managed"
      const initialProfile = pf.value.find((p) => p.name === configuredDefault) ?? pf.value[0]
      setSelectedProfile(initialProfile.name)
      setHeadless(initialProfile.headless)
    }
    if (firstError) {
      logger.error("settings", "BrowserPanel", `Partial refresh failure: ${firstError.reason}`)
      // Only surface as fatal if the core triplet (status / profiles / config) failed.
      if (
        st.status === "rejected" ||
        pf.status === "rejected" ||
        cfg.status === "rejected"
      ) {
        setError(String(firstError.reason))
      }
    }
    setLoading(false)
  }, [selectedProfile])

  useEffect(() => {
    void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Subscribe to Chromium runtime download progress. The backend emits
  // `browser:chromium_download_progress` on every percent boundary and
  // a final `stage: "ready"` payload once the binary is on disk.
  useEffect(() => {
    const unlisten = getTransport().listen(
      "browser:chromium_download_progress",
      (raw) => {
        try {
          const data = JSON.parse(String(raw)) as {
            stage?: string
            percent?: number | null
          }
          if (data.stage === "ready") {
            setInstallPercent(100)
            return
          }
          if (typeof data.percent === "number") {
            setInstallPercent(data.percent)
          }
        } catch {
          /* ignore parse errors — the bus may send legacy shapes */
        }
      },
    )
    return () => {
      try {
        unlisten?.()
      } catch {
        /* ignore */
      }
    }
  }, [])

  const onInstallRuntime = useCallback(async () => {
    setInstalling(true)
    setInstallError(null)
    setInstallPercent(0)
    try {
      await getTransport().call("browser_install_chromium_runtime")
      toast.success(t("settings.browser.installRuntimeReady"))
      await refresh()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.error("settings", "BrowserPanel", `install-chromium-runtime failed: ${msg}`)
      setInstallError(msg)
      toast.error(t("settings.browser.installRuntimeFailed", { error: msg }))
    } finally {
      setInstalling(false)
    }
  }, [refresh, t])

  const runAction = async (
    action: "launch" | "connect" | "disconnect",
    fn: () => Promise<BrowserStatus>,
  ) => {
    setBusy(action)
    setError(null)
    try {
      const st = await fn()
      setStatus(st)
      await refresh()
    } catch (e) {
      logger.error("settings", "BrowserPanel", `${action} failed: ${e}`)
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const onLaunch = () => {
    const opts: LaunchOptions = {
      profile: selectedProfile || null,
      executablePath: executablePath.trim() || null,
      headless,
    }
    void runAction("launch", () =>
      getTransport().call<BrowserStatus>("browser_launch", { options: opts }),
    )
  }

  const onConnect = (url?: string) => {
    const target = (url ?? connectUrl).trim()
    void runAction("connect", () =>
      getTransport().call<BrowserStatus>("browser_connect", { url: target }),
    )
  }

  const onDisconnect = () => {
    void runAction("disconnect", () => getTransport().call<BrowserStatus>("browser_disconnect"))
  }

  const onCreateProfile = async () => {
    const name = newProfileName.trim()
    if (!name) return
    setCreating(true)
    setError(null)
    try {
      const created = await getTransport().call<BrowserProfileInfo>("browser_create_profile", { name })
      setNewProfileName("")
      await refresh()
      setSelectedProfile(name)
      setHeadless(created.headless)
    } catch (e) {
      logger.error("settings", "BrowserPanel", `Create profile failed: ${e}`)
      setError(String(e))
    } finally {
      setCreating(false)
    }
  }

  const onDeleteProfile = async () => {
    if (!pendingDelete) return
    const profileName = pendingDelete.name
    setError(null)
    try {
      await getTransport().call("browser_delete_profile", { name: profileName })
      if (selectedProfile === profileName) setSelectedProfile("")
      setPendingDelete(null)
      await refresh()
      toast.success(t("common.deleted"), {
        description: profileName,
      })
    } catch (e) {
      logger.error("settings", "BrowserPanel", `Delete profile failed: ${e}`)
      setError(String(e))
      setPendingDelete(null)
      toast.error(t("common.deleteFailed"), {
        description: profileName,
      })
    }
  }

  const persistCfg = async (next: BrowserConfig) => {
    // Radio bursts produce same-value clicks; short-circuit so we don't
    // hammer config saves + autosave backups + toast spam.
    if (next.defaultMode === browserCfg.defaultMode) {
      return false
    }
    setBrowserCfg(next)
    setSavingCfg(true)
    try {
      await getTransport().call("browser_set_config", { config: next })
      return true
    } catch (e) {
      logger.error("settings", "BrowserPanel", `set_config failed: ${e}`)
      setError(String(e))
      return false
    } finally {
      setSavingCfg(false)
    }
  }

  const onModeChange = (mode: BrowserMode) => {
    void persistCfg({ ...browserCfg, defaultMode: mode })
  }

  // Has the advanced draft diverged from the persisted snapshot?
  const advDirty =
    advBackendPref !== ((browserCfg.backendPreference ?? "extension_first") as string) ||
    advExtEnabled !== (browserCfg.extension?.enabled !== false) ||
    advAllowRawCdp !== (browserCfg.extension?.allowRawCdp !== false)

  const onSaveAdvanced = async () => {
    setAdvSaving(true)
    setError(null)
    // Merge into the full snapshot so unrelated fields (profiles, launchCircuit,
    // nativeHostName, extensionIds, …) survive the wholesale `browser_set_config`.
    const next: BrowserConfig = {
      ...browserCfg,
      backendPreference: advBackendPref,
      extension: {
        ...(browserCfg.extension ?? {}),
        enabled: advExtEnabled,
        allowRawCdp: advAllowRawCdp,
      },
    }
    try {
      await getTransport().call("browser_set_config", { config: next })
      setBrowserCfg(next)
      setAdvSaveStatus("saved")
      setTimeout(() => setAdvSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "BrowserPanel", `save advanced config failed: ${e}`)
      setError(String(e))
      setAdvSaveStatus("failed")
      setTimeout(() => setAdvSaveStatus("idle"), 2000)
    } finally {
      setAdvSaving(false)
    }
  }

  const openConfirmSpawn = () => {
    // Use the cached doctor snapshot for the modal copy. The doctor refresh
    // runs on panel mount + every user "Refresh" click, so this is rarely
    // stale; even if it is, the spawn itself is idempotent.
    setConfirmSpawn({ chromeAlreadyRunning: doctor?.chromeAlreadyRunning ?? false })
  }

  const onSpawnUserChrome = async () => {
    setBusy("spawn-user-chrome")
    setError(null)
    try {
      const result = await getTransport().call<SpawnUserChromeResult>(
        "browser_spawn_user_chrome",
        { args: {} },
      )
      toast.success(t("settings.browser.spawnUserChrome.spawned", { port: result.port }))
      setConfirmSpawn(null)
      // `spawn_user_chrome` now performs spawn + connect server-side and
      // retains the Chrome process handle inside `BrowserState`. Calling
      // `browser_connect` here would `disconnect()` first and kill the
      // process we just launched. Just refresh the UI to pick up the new
      // connection state.
      void refresh()
    } catch (e) {
      logger.error("settings", "BrowserPanel", `spawn-user-chrome failed: ${e}`)
      setError(String(e))
    } finally {
      setBusy(null)
    }
  }

  const onInstallNativeHost = async () => {
    const extensionId = extensionIdInput.trim()
    const hostPath = nativeHostPathInput.trim()
    if (!extensionId) {
      toast.error(t("settings.browser.extension.toast.idRequired"))
      return
    }
    setBusy("install-native-host")
    setError(null)
    try {
      const result = await getTransport().call<NativeHostInstallResult>(
        "browser_install_native_host_manifest",
        {
          request: {
            extensionId,
            hostPath: hostPath || null,
            nativeHostName: extensionStatus?.nativeHostName || null,
          },
        },
      )
      toast.success(t("settings.browser.extension.toast.hostInstalled"), {
        description: result.manifestPath,
      })
      await refresh()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.error("settings", "BrowserPanel", `install-native-host failed: ${msg}`)
      setError(msg)
      toast.error(t("settings.browser.extension.toast.hostInstallFailed"), { description: msg })
    } finally {
      setBusy(null)
    }
  }

  const onStopExtensionControl = async () => {
    setBusy("stop-extension-control")
    setError(null)
    try {
      const result = await getTransport().call<BrowserExtensionStopResult>(
        "browser_extension_stop_control",
      )
      toast.success(t("settings.browser.extension.toast.controlStopped"), {
        description: result.message,
      })
      await refresh()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      logger.error("settings", "BrowserPanel", `stop-extension-control failed: ${msg}`)
      setError(msg)
      toast.error(t("settings.browser.extension.toast.controlStopFailed"), { description: msg })
    } finally {
      setBusy(null)
    }
  }

  const openChromeExtensions = async () => {
    try {
      await getTransport().call("open_url", { url: "chrome://extensions/" })
    } catch {
      window.open("chrome://extensions/", "_blank", "noopener,noreferrer")
    }
  }

  const openExtensionStore = async () => {
    const url = extensionStatus?.storeUrl
    if (!url) return
    try {
      await getTransport().call("open_url", { url })
    } catch {
      window.open(url, "_blank", "noopener,noreferrer")
    }
  }

  const copyInstallValue = useCallback(
    async (label: string, value?: string | null) => {
      if (!value) return
      try {
        await navigator.clipboard.writeText(value)
        toast.success(t("settings.browser.extension.toast.copied", { item: label }))
      } catch (e) {
        logger.error("settings", "BrowserPanel", `copy ${label} failed: ${e}`)
        toast.error(t("settings.browser.extension.toast.copyFailed", { item: label }))
      }
    },
    [t],
  )

  const revealInstallPath = useCallback(
    async (label: string, path?: string | null) => {
      if (!path) return
      try {
        await getTransport().call("reveal_in_folder", { path })
      } catch (e) {
        logger.error("settings", "BrowserPanel", `reveal ${label} failed: ${e}`)
        toast.error(t("settings.browser.extension.toast.revealFailed", { item: label }))
      }
    },
    [t],
  )

  const connected = status?.connected ?? false

  const statusText = useMemo(() => {
    if (!status) return ""
    if (!status.connected) return t("settings.browser.statusDisconnected")
    const mode =
      status.mode === "launch"
        ? t("settings.browser.modeLaunch")
        : status.mode === "connect"
          ? t("settings.browser.modeConnect")
          : ""
    const prof = status.profile ? ` · ${status.profile}` : ""
    return `${mode}${prof}`
  }, [status, t])

  // Localized display for the backend extension-status enum (`kind`); unknown
  // variants fall back to the raw kind / English backend message.
  const extKindLabel: Record<string, string> = {
    ready: t("settings.browser.extension.kindReady"),
    host_missing: t("settings.browser.extension.kindHostMissing"),
    broker_unavailable: t("settings.browser.extension.kindBrokerUnavailable"),
    version_mismatch: t("settings.browser.extension.kindVersionMismatch"),
    extension_missing: t("settings.browser.extension.kindExtensionMissing"),
  }
  const extKindMessage: Record<string, string> = {
    ready: t("settings.browser.extension.statusReady"),
    host_missing: t("settings.browser.extension.statusHostMissing"),
    broker_unavailable: t("settings.browser.extension.statusBrokerUnavailable"),
    extension_missing: t("settings.browser.extension.statusExtensionMissing"),
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0 overflow-hidden">
      <div className="flex-1 min-w-0 overflow-y-auto p-6">
        <div className="w-full min-w-0 space-y-6">
          {/* Header */}
          <div className="space-y-1">
            <p className="text-xs text-muted-foreground">{t("settings.browser.desc")}</p>
          </div>

          {/* Status card */}
          <div className="rounded-lg border border-border bg-secondary/20 px-4 py-3 flex items-center gap-3">
            {connected ? (
              <CircleDot className="h-4 w-4 text-green-500 shrink-0" />
            ) : (
              <CircleOff className="h-4 w-4 text-muted-foreground shrink-0" />
            )}
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium">
                {connected
                  ? t("settings.browser.statusConnected")
                  : t("settings.browser.statusDisconnected")}
              </div>
              {connected && (
                <div className="text-xs text-muted-foreground truncate">{statusText}</div>
              )}
            </div>
            <IconTip label={t("settings.browser.refresh")}>
              <span className="inline-flex">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void refresh()}
                  disabled={loading || !!busy}
                >
                  <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
                </Button>
              </span>
            </IconTip>
            {connected && (
              <Button size="sm" variant="outline" onClick={onDisconnect} disabled={busy !== null}>
                {busy === "disconnect" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Power className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">{t("settings.browser.disconnect")}</span>
              </Button>
            )}
          </div>

          {extensionStatus && (
            <div
              className={cn(
                "rounded-lg border px-4 py-3 flex items-start gap-3",
                extensionStatus.backendAvailable
                  ? "border-green-500/40 bg-green-500/10"
                  : "border-amber-500/40 bg-amber-500/10",
              )}
            >
              {extensionStatus.backendAvailable ? (
                <CheckCircle2 className="h-4 w-4 text-green-600 shrink-0 mt-0.5" />
              ) : (
                <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" />
              )}
              <div className="flex-1 min-w-0 space-y-1">
                <div className="text-sm font-medium">
                  {t("settings.browser.extension.title")} ·{" "}
                  {extKindLabel[extensionStatus.kind] ?? extensionStatus.kind}
                </div>
                <p className="text-xs text-muted-foreground">
                  {extKindMessage[extensionStatus.kind] ?? extensionStatus.message}
                </p>
                <p className="text-[11px] text-muted-foreground font-mono truncate">
                  {extensionStatus.nativeHostManifestPath || extensionStatus.nativeHostName}
                </p>
                {extensionStatus.extensionConnected && (
                  <p className="text-[11px] text-muted-foreground">
                    {t("settings.browser.extension.versionLine", {
                      version:
                        extensionStatus.extensionVersion ||
                        t("settings.browser.extension.unknown"),
                      protocol:
                        extensionStatus.extensionProtocolVersion ??
                        t("settings.browser.extension.unknown"),
                    })}
                  </p>
                )}
                {extensionStatus.unpackedExtensionPath && (
                  <p className="text-[11px] text-muted-foreground font-mono truncate">
                    {t("settings.browser.extension.loadUnpackedPath", {
                      path: extensionStatus.unpackedExtensionPath,
                    })}
                  </p>
                )}
                <div className="pt-2 flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    className="text-destructive hover:text-destructive"
                    onClick={onStopExtensionControl}
                    disabled={busy !== null}
                  >
                    {busy === "stop-extension-control" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Power className="h-3.5 w-3.5" />
                    )}
                    <span className="ml-1.5">{t("settings.browser.extension.stopControl")}</span>
                  </Button>
                </div>
                {!extensionStatus.backendAvailable && (
                  <div className="pt-3 space-y-3 border-t border-amber-500/20">
                    <div className="space-y-2 text-xs">
                      {extensionStatus.storeUrl ? (
                        <>
                          <div className="flex gap-2">
                            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-amber-500/40 text-[11px] font-medium">
                              1
                            </span>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.stepInstallExtension")}
                              </div>
                              <div className="text-muted-foreground">
                                {t("settings.browser.extension.stepInstallExtensionHint")}
                              </div>
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={openExtensionStore}
                                disabled={busy !== null}
                                className="h-7 px-2"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                                <span className="ml-1.5">{t("settings.browser.extension.openWebStore")}</span>
                              </Button>
                            </div>
                          </div>
                          <div className="flex gap-2">
                            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-amber-500/40 text-[11px] font-medium">
                              2
                            </span>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.stepInstallHost")}
                              </div>
                              <div className="font-mono text-[11px] text-muted-foreground truncate">
                                {extensionStatus.nativeHostBinaryHint ||
                                  t("settings.browser.extension.hostPathUnavailable")}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void revealInstallPath(
                                      t("settings.browser.extension.nativeHostLabel"),
                                      extensionStatus.nativeHostBinaryHint,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.nativeHostBinaryHint}
                                  className="h-7 px-2"
                                >
                                  <FolderOpen className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.showHost")}</span>
                                </Button>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void copyInstallValue(
                                      t("settings.browser.extension.nativeHostPathLabel"),
                                      extensionStatus.nativeHostBinaryHint,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.nativeHostBinaryHint}
                                  className="h-7 px-2"
                                >
                                  <Copy className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.copyHostPath")}</span>
                                </Button>
                              </div>
                            </div>
                          </div>
                          {extensionStatus.unpackedExtensionPath && (
                            <div className="rounded-md border border-border/70 px-3 py-2 space-y-2">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.alphaFallback")}
                              </div>
                              <div className="font-mono text-[11px] text-muted-foreground truncate">
                                {extensionStatus.unpackedExtensionPath}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={openChromeExtensions}
                                  disabled={busy !== null}
                                  className="h-7 px-2"
                                >
                                  <ExternalLink className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.openChromeExtensions")}</span>
                                </Button>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void revealInstallPath(
                                      t("settings.browser.extension.extensionFolderLabel"),
                                      extensionStatus.unpackedExtensionPath,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.unpackedExtensionPath}
                                  className="h-7 px-2"
                                >
                                  <FolderOpen className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.showFolder")}</span>
                                </Button>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void copyInstallValue(
                                      t("settings.browser.extension.extensionPathLabel"),
                                      extensionStatus.unpackedExtensionPath,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.unpackedExtensionPath}
                                  className="h-7 px-2"
                                >
                                  <Copy className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.copyPath")}</span>
                                </Button>
                              </div>
                            </div>
                          )}
                        </>
                      ) : (
                        <>
                          <div className="flex gap-2">
                            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-amber-500/40 text-[11px] font-medium">
                              1
                            </span>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.stepOpenExtensions")}
                              </div>
                              <div className="text-muted-foreground">
                                {t("settings.browser.extension.stepOpenExtensionsHint")}
                              </div>
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={openChromeExtensions}
                                disabled={busy !== null}
                                className="h-7 px-2"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                                <span className="ml-1.5">{t("settings.browser.extension.openChromeExtensions")}</span>
                              </Button>
                            </div>
                          </div>
                          <div className="flex gap-2">
                            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-amber-500/40 text-[11px] font-medium">
                              2
                            </span>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.stepLoadUnpacked")}
                              </div>
                              <div className="font-mono text-[11px] text-muted-foreground truncate">
                                {extensionStatus.unpackedExtensionPath ||
                                  t("settings.browser.extension.extensionPathUnavailable")}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void revealInstallPath(
                                      t("settings.browser.extension.extensionFolderLabel"),
                                      extensionStatus.unpackedExtensionPath,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.unpackedExtensionPath}
                                  className="h-7 px-2"
                                >
                                  <FolderOpen className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.showFolder")}</span>
                                </Button>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void copyInstallValue(
                                      t("settings.browser.extension.extensionPathLabel"),
                                      extensionStatus.unpackedExtensionPath,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.unpackedExtensionPath}
                                  className="h-7 px-2"
                                >
                                  <Copy className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.copyPath")}</span>
                                </Button>
                              </div>
                            </div>
                          </div>
                          <div className="flex gap-2">
                            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-amber-500/40 text-[11px] font-medium">
                              3
                            </span>
                            <div className="min-w-0 flex-1 space-y-1">
                              <div className="font-medium text-foreground">
                                {t("settings.browser.extension.stepInstallHost")}
                              </div>
                              <div className="font-mono text-[11px] text-muted-foreground truncate">
                                {extensionStatus.nativeHostBinaryHint ||
                                  t("settings.browser.extension.hostPathUnavailable")}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void revealInstallPath(
                                      t("settings.browser.extension.nativeHostLabel"),
                                      extensionStatus.nativeHostBinaryHint,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.nativeHostBinaryHint}
                                  className="h-7 px-2"
                                >
                                  <FolderOpen className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.showHost")}</span>
                                </Button>
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() =>
                                    void copyInstallValue(
                                      t("settings.browser.extension.nativeHostPathLabel"),
                                      extensionStatus.nativeHostBinaryHint,
                                    )
                                  }
                                  disabled={busy !== null || !extensionStatus.nativeHostBinaryHint}
                                  className="h-7 px-2"
                                >
                                  <Copy className="h-3.5 w-3.5" />
                                  <span className="ml-1.5">{t("settings.browser.extension.copyHostPath")}</span>
                                </Button>
                              </div>
                            </div>
                          </div>
                        </>
                      )}
                    </div>
                    <div className="grid gap-2 md:grid-cols-2">
                      <Input
                        value={extensionIdInput}
                        placeholder={t("settings.browser.extension.extensionIdPlaceholder")}
                        onChange={(e) => setExtensionIdInput(e.target.value)}
                      />
                      <Input
                        value={nativeHostPathInput}
                        placeholder={t("settings.browser.extension.hostPathPlaceholder")}
                        onChange={(e) => setNativeHostPathInput(e.target.value)}
                      />
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={onInstallNativeHost}
                        disabled={busy !== null || !extensionIdInput.trim()}
                      >
                        {busy === "install-native-host" ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Plug className="h-3.5 w-3.5" />
                      )}
                      <span className="ml-1.5">{t("settings.browser.extension.installHost")}</span>
                    </Button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          {error && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {error}
            </div>
          )}

          {/* Active tabs (when connected) */}
          {connected && status && status.tabs.length > 0 && (
            <div className="space-y-2">
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.sectionTabs")}
              </h3>
              <div className="rounded-md border border-border divide-y divide-border">
                {status.tabs.map((tab) => (
                  <div key={tab.targetId} className="px-3 py-2 flex items-center gap-2 text-sm">
                    {tab.isActive ? (
                      <CircleDot className="h-3 w-3 text-primary shrink-0" />
                    ) : (
                      <ExternalLink className="h-3 w-3 text-muted-foreground shrink-0" />
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="truncate font-medium">
                        {tab.title || t("settings.browser.untitledTab")}
                      </div>
                      <div className="truncate text-xs text-muted-foreground">
                        {tab.url || "about:blank"}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          <Tabs
            value={(browserCfg.defaultMode ?? "managed") as BrowserMode}
            onValueChange={(v) => onModeChange(v as BrowserMode)}
            className="space-y-4"
          >
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="managed">{t("settings.browser.modeStandalone")}</TabsTrigger>
              <TabsTrigger value="user_attach">
                {t("settings.browser.modeUserChrome")}
              </TabsTrigger>
            </TabsList>

            <TabsContent value="managed" className="space-y-6">
              <p className="text-xs text-muted-foreground">
                {t("settings.browser.modeStandaloneHint")}
                {savingCfg && <Loader2 className="inline h-3 w-3 animate-spin ml-2" />}
              </p>

          {/* Launch section */}
          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.sectionLaunch")}
              </h3>
              <p className="text-xs text-muted-foreground mt-1">
                {t("settings.browser.launchHelp")}
              </p>
            </div>

            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-sm font-medium">{t("settings.browser.profileLabel")}</label>
                <Select
                  value={selectedProfile || "__none__"}
                  onValueChange={(v) => {
                    const next = v === "__none__" ? "" : v
                    setSelectedProfile(next)
                    const profile = profiles.find((p) => p.name === next)
                    if (profile) setHeadless(profile.headless)
                  }}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("settings.browser.profilePlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">{t("settings.browser.profileNone")}</SelectItem>
                    {profiles.map((p) => (
                      <SelectItem key={p.name} value={p.name}>
                        {p.name}
                        {p.isActive ? ` · ${t("settings.browser.activeBadge")}` : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {selectedProfile
                    ? t("settings.browser.profileHint")
                    : t("settings.browser.profileNoneHint")}
                </p>
              </div>

              <div className="space-y-1.5">
                <label className="text-sm font-medium">
                  {t("settings.browser.executableLabel")}
                </label>
                <Input
                  value={executablePath}
                  placeholder={t("settings.browser.executablePlaceholder")}
                  onChange={(e) => setExecutablePath(e.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  {t("settings.browser.executableHint")}
                </p>
              </div>

              <div className="flex items-center justify-between">
                <div className="space-y-0.5">
                  <span className="text-sm font-medium">{t("settings.browser.headless")}</span>
                  <p className="text-xs text-muted-foreground">
                    {t("settings.browser.headlessHint")}
                  </p>
                </div>
                <Switch checked={headless} onCheckedChange={setHeadless} />
              </div>

              <Button onClick={onLaunch} disabled={busy !== null}>
                {busy === "launch" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Globe className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">{t("settings.browser.launchButton")}</span>
              </Button>
            </div>
          </div>

          {/* Profiles section */}
          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.sectionProfiles")}
              </h3>
              <p className="text-xs text-muted-foreground mt-1">
                {t("settings.browser.profilesHelp")}
              </p>
            </div>

            {/* Create */}
            <div className="flex gap-2 items-end">
              <div className="flex-1 space-y-1.5">
                <label className="text-sm font-medium">
                  {t("settings.browser.newProfileLabel")}
                </label>
                <Input
                  value={newProfileName}
                  placeholder={t("settings.browser.newProfilePlaceholder")}
                  onChange={(e) => setNewProfileName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && newProfileName.trim()) {
                      e.preventDefault()
                      void onCreateProfile()
                    }
                  }}
                />
              </div>
              <Button
                variant="outline"
                onClick={onCreateProfile}
                disabled={creating || !newProfileName.trim()}
                className="shrink-0"
              >
                {creating ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Plus className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">{t("settings.browser.create")}</span>
              </Button>
            </div>

            {/* Profile list */}
            {profiles.length === 0 ? (
              <div className="rounded-md border border-dashed border-border px-4 py-6 text-center text-xs text-muted-foreground">
                {t("settings.browser.profilesEmpty")}
              </div>
            ) : (
              <div className="rounded-md border border-border divide-y divide-border">
                {profiles.map((p) => (
                  <div key={p.name} className="px-3 py-2.5 flex items-center gap-3 text-sm">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-medium truncate">{p.name}</span>
                        {p.isActive && (
                          <span className="text-[10px] font-medium text-green-600 bg-green-500/10 px-1.5 py-0.5 rounded">
                            {t("settings.browser.activeBadge")}
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground truncate">
                        {formatBytes(p.sizeBytes)} · {formatRelative(p.lastUsedAt, t)}
                      </div>
                    </div>
                    <IconTip
                      label={
                        !p.canDelete || p.isActive
                          ? t("settings.browser.deleteDisabledActive")
                          : t("settings.browser.delete")
                      }
                    >
                      <span className="inline-flex">
                        <Button
                          size="sm"
                          variant="ghost"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setPendingDelete(p)}
                          disabled={p.isActive || !p.canDelete}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </span>
                    </IconTip>
                  </div>
                ))}
              </div>
            )}

            {status && (
              <p className="text-[11px] text-muted-foreground font-mono truncate">
                {status.profilesDir}
              </p>
            )}
          </div>
            </TabsContent>

            <TabsContent value="user_attach" className="space-y-4">
              <p className="text-xs text-muted-foreground">
                {t("settings.browser.modeUserChromeHint")}
                {savingCfg && <Loader2 className="inline h-3 w-3 animate-spin ml-2" />}
              </p>

          {/* Connect / Doctor section */}
          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.sectionConnect")}
              </h3>
              <p className="text-xs text-muted-foreground mt-1">
                {t("settings.browser.connectHelp")}
              </p>
            </div>

            {/* Doctor banner */}
            {doctor?.probe.found ? (
              <div className="rounded-md border border-green-500/40 bg-green-500/10 px-3 py-2.5 flex items-center gap-3 text-sm">
                <CheckCircle2 className="h-4 w-4 text-green-600 shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="font-medium">
                    {t("settings.browser.doctorChromeFound", {
                      url: doctor.probe.browserUrl,
                    })}
                  </div>
                  {doctor.probe.version && (
                    <div className="text-xs text-muted-foreground truncate">
                      {doctor.probe.version}
                    </div>
                  )}
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => onConnect(doctor.probe.browserUrl)}
                  disabled={busy !== null}
                >
                  <Plug className="h-3.5 w-3.5" />
                  <span className="ml-1.5">{t("settings.browser.doctorConnect")}</span>
                </Button>
              </div>
            ) : (
              <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2.5 flex items-center gap-3 text-sm">
                <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="font-medium">{t("settings.browser.doctorChromeMissing")}</div>
                  <div className="text-xs text-muted-foreground">
                    {t("settings.browser.doctorChromeMissingHint")}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void openConfirmSpawn()}
                  disabled={busy !== null}
                >
                  {busy === "spawn-user-chrome" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Sparkles className="h-3.5 w-3.5" />
                  )}
                  <span className="ml-1.5">
                    {t("settings.browser.doctorLaunchUserChrome")}
                  </span>
                </Button>
              </div>
            )}

            {/* Manual connect URL */}
            <div className="flex gap-2 items-end">
              <div className="flex-1 space-y-1.5">
                <label className="text-sm font-medium">
                  {t("settings.browser.connectUrlLabel")}
                </label>
                <Input
                  value={connectUrl}
                  placeholder="http://127.0.0.1:9222"
                  onChange={(e) => setConnectUrl(e.target.value)}
                />
              </div>
              <Button
                variant="outline"
                onClick={() => onConnect()}
                disabled={busy !== null || !connectUrl.trim()}
                className="shrink-0"
              >
                {busy === "connect" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Plug className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">{t("settings.browser.connect")}</span>
              </Button>
            </div>
          </div>
            </TabsContent>
          </Tabs>

          {/* Advanced extension-backend settings — global (not mode-specific),
              committed via an explicit three-state Save button. */}
          <div className="space-y-4 border-t border-border pt-6">
            <div>
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.advanced.section")}
              </h3>
              <p className="text-xs text-muted-foreground mt-1">
                {t("settings.browser.advanced.sectionHint")}
              </p>
            </div>

            {/* Backend preference */}
            <div className="space-y-1.5">
              <label className="text-sm font-medium">
                {t("settings.browser.advanced.backendLabel")}
              </label>
              <Select
                value={advBackendPref}
                onValueChange={(v) => setAdvBackendPref(v as BrowserBackendPreference)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="extension_first">
                    {t("settings.browser.advanced.backendExtensionFirst")}
                  </SelectItem>
                  <SelectItem value="cdp_only">
                    {t("settings.browser.advanced.backendCdpOnly")}
                  </SelectItem>
                  <SelectItem value="extension_only">
                    {t("settings.browser.advanced.backendExtensionOnly")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {t(
                  {
                    extension_first: "settings.browser.advanced.backendExtensionFirstDesc",
                    cdp_only: "settings.browser.advanced.backendCdpOnlyDesc",
                    extension_only: "settings.browser.advanced.backendExtensionOnlyDesc",
                  }[advBackendPref],
                )}
              </p>
            </div>

            {/* Enable extension backend */}
            <div className="flex items-center justify-between">
              <div className="space-y-0.5 pr-4">
                <span className="text-sm font-medium">
                  {t("settings.browser.advanced.enableLabel")}
                </span>
                <p className="text-xs text-muted-foreground">
                  {t("settings.browser.advanced.enableHint")}
                </p>
              </div>
              <Switch checked={advExtEnabled} onCheckedChange={setAdvExtEnabled} />
            </div>

            {/* Allow raw CDP (HIGH risk) */}
            <div className="flex items-center justify-between">
              <div className="space-y-0.5 pr-4">
                <span className="text-sm font-medium">
                  {t("settings.browser.advanced.rawCdpLabel")}
                </span>
                <p className="flex items-start gap-1 text-xs text-amber-600 dark:text-amber-500">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
                  <span>{t("settings.browser.advanced.rawCdpHint")}</span>
                </p>
              </div>
              <Switch
                checked={advAllowRawCdp}
                onCheckedChange={setAdvAllowRawCdp}
                disabled={!advExtEnabled}
              />
            </div>

            {/* Three-state save */}
            <Button
              onClick={() => void onSaveAdvanced()}
              disabled={!advDirty || advSaving}
              className={cn(
                advSaveStatus === "saved" &&
                  "bg-green-500/10 text-green-600 hover:bg-green-500/20",
                advSaveStatus === "failed" &&
                  "bg-destructive/10 text-destructive hover:bg-destructive/20",
              )}
            >
              {advSaving ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  {t("common.saving")}
                </>
              ) : advSaveStatus === "saved" ? (
                <>
                  <Check className="mr-2 h-4 w-4" />
                  {t("common.saved")}
                </>
              ) : advSaveStatus === "failed" ? (
                t("common.saveFailed")
              ) : (
                <>
                  <Save className="mr-2 h-4 w-4" />
                  {t("common.save")}
                </>
              )}
            </Button>
          </div>

          {/* Runtime status — surfaces system Chrome / cached Chromium / "no binary" state.
              Hidden once Chrome is connected: the doctor info is purely about whether
              future launches will work, not what's currently running. */}
          {doctor && !connected && (
            <div className="space-y-2">
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.runtimeStatusLabel")}
              </h3>
              {doctor.systemChromePath ? (
                <div className="rounded-md border border-green-500/40 bg-green-500/10 px-3 py-2.5 flex items-start gap-3 text-sm">
                  <CheckCircle2 className="h-4 w-4 text-green-600 shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0">
                    <div className="font-medium">
                      {t("settings.browser.doctorSystemChrome")}
                    </div>
                    <div className="text-xs text-muted-foreground truncate">
                      {doctor.systemChromePath}
                    </div>
                  </div>
                </div>
              ) : doctor.runtimeChromium ? (
                <div className="rounded-md border border-green-500/40 bg-green-500/10 px-3 py-2.5 flex items-start gap-3 text-sm">
                  <CheckCircle2 className="h-4 w-4 text-green-600 shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0">
                    <div className="font-medium">
                      {t("settings.browser.doctorRuntimeChromium", {
                        rev: doctor.runtimeChromium.revision,
                      })}
                    </div>
                    <div className="text-xs text-muted-foreground truncate">
                      {doctor.runtimeChromium.binaryPath}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2.5 flex items-start gap-3 text-sm">
                  <AlertTriangle className="h-4 w-4 text-amber-600 shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0 space-y-2">
                    <div>
                      <div className="font-medium">{t("settings.browser.doctorNoBinary")}</div>
                      <div className="text-xs text-muted-foreground">
                        {t("settings.browser.doctorNoBinaryHint")}
                      </div>
                    </div>
                    {installing && (
                      <div className="space-y-1">
                        <div className="text-xs text-muted-foreground">
                          {t("settings.browser.installRuntimeRunning", {
                            percent: installPercent ?? 0,
                          })}
                        </div>
                        <div className="h-1.5 w-full overflow-hidden rounded-full bg-secondary/60">
                          <div
                            className="h-full bg-primary transition-all"
                            style={{ width: `${Math.max(0, Math.min(100, installPercent ?? 0))}%` }}
                          />
                        </div>
                      </div>
                    )}
                    {installError && (
                      <div className="text-xs text-destructive">{installError}</div>
                    )}
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => void onInstallRuntime()}
                    disabled={installing}
                  >
                    {installing ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Download className="h-3.5 w-3.5" />
                    )}
                    <span className="ml-1.5">{t("settings.browser.installRuntime")}</span>
                  </Button>
                </div>
              )}
            </div>
          )}

        </div>
      </div>

      <AlertDialog open={!!pendingDelete} onOpenChange={(o) => !o && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.browser.deleteConfirmTitle", { name: pendingDelete?.name ?? "" })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.browser.deleteConfirmDesc")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={onDeleteProfile}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("settings.browser.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={!!confirmSpawn} onOpenChange={(o) => !o && setConfirmSpawn(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.browser.spawnUserChrome.confirmTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirmSpawn?.chromeAlreadyRunning
                ? t("settings.browser.spawnUserChrome.confirmBodyRunning")
                : t("settings.browser.spawnUserChrome.confirmBodyIdle")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void onSpawnUserChrome()}>
              {t("settings.browser.spawnUserChrome.continue")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
