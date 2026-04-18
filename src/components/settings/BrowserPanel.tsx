import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
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
  Chrome,
  Loader2,
  Plug,
  Plus,
  Trash2,
  Power,
  RefreshCw,
  CircleDot,
  CircleOff,
  ExternalLink,
} from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface BrowserProfileInfo {
  name: string
  path: string
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

interface LaunchOptions {
  profile?: string | null
  executablePath?: string | null
  headless?: boolean
}

// ── Helpers ──────────────────────────────────────────────────────

function formatBytes(n: number): string {
  if (!n) return "0 B"
  const units = ["B", "KB", "MB", "GB"]
  let i = 0
  let v = n
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`
}

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
  const [profiles, setProfiles] = useState<BrowserProfileInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<null | "launch" | "connect" | "disconnect">(null)
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

  const refresh = useCallback(async () => {
    try {
      const [st, pf] = await Promise.all([
        getTransport().call<BrowserStatus>("browser_get_status"),
        getTransport().call<BrowserProfileInfo[]>("browser_list_profiles"),
      ])
      setStatus(st)
      setProfiles(pf)
      if (!selectedProfile && pf.length > 0) {
        setSelectedProfile(pf[0].name)
      }
    } catch (e) {
      logger.error("settings", "BrowserPanel", `Failed to load status: ${e}`)
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [selectedProfile])

  useEffect(() => {
    void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

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

  const onConnect = () => {
    void runAction("connect", () =>
      getTransport().call<BrowserStatus>("browser_connect", { url: connectUrl.trim() }),
    )
  }

  const onDisconnect = () => {
    void runAction("disconnect", () =>
      getTransport().call<BrowserStatus>("browser_disconnect"),
    )
  }

  const onCreateProfile = async () => {
    const name = newProfileName.trim()
    if (!name) return
    setCreating(true)
    setError(null)
    try {
      await getTransport().call("browser_create_profile", { name })
      setNewProfileName("")
      await refresh()
      setSelectedProfile(name)
    } catch (e) {
      logger.error("settings", "BrowserPanel", `Create profile failed: ${e}`)
      setError(String(e))
    } finally {
      setCreating(false)
    }
  }

  const onDeleteProfile = async () => {
    if (!pendingDelete) return
    setError(null)
    try {
      await getTransport().call("browser_delete_profile", { name: pendingDelete.name })
      if (selectedProfile === pendingDelete.name) setSelectedProfile("")
      setPendingDelete(null)
      await refresh()
    } catch (e) {
      logger.error("settings", "BrowserPanel", `Delete profile failed: ${e}`)
      setError(String(e))
      setPendingDelete(null)
    }
  }

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

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="space-y-6 max-w-3xl">
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
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void refresh()}
              disabled={loading || !!busy}
              title={t("settings.browser.refresh")}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
            </Button>
            {connected && (
              <Button
                size="sm"
                variant="outline"
                onClick={onDisconnect}
                disabled={busy !== null}
              >
                {busy === "disconnect" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Power className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">{t("settings.browser.disconnect")}</span>
              </Button>
            )}
          </div>

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
                  <div
                    key={tab.targetId}
                    className="px-3 py-2 flex items-center gap-2 text-sm"
                  >
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
                <label className="text-sm font-medium">
                  {t("settings.browser.profileLabel")}
                </label>
                <Select
                  value={selectedProfile || "__none__"}
                  onValueChange={(v) => setSelectedProfile(v === "__none__" ? "" : v)}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("settings.browser.profilePlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">
                      {t("settings.browser.profileNone")}
                    </SelectItem>
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
                  <Chrome className="h-3.5 w-3.5" />
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
                  <div
                    key={p.name}
                    className="px-3 py-2.5 flex items-center gap-3 text-sm"
                  >
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
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-destructive hover:text-destructive"
                      onClick={() => setPendingDelete(p)}
                      disabled={p.isActive}
                      title={
                        p.isActive
                          ? t("settings.browser.deleteDisabledActive")
                          : t("settings.browser.delete")
                      }
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
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

          {/* Advanced: connect to existing */}
          <div className="space-y-4">
            <div>
              <h3 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.browser.sectionConnect")}
              </h3>
              <p className="text-xs text-muted-foreground mt-1">
                {t("settings.browser.connectHelp")}
              </p>
            </div>
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
                onClick={onConnect}
                disabled={busy !== null || !connectUrl.trim()}
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
    </div>
  )
}
