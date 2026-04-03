import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage, setLanguage } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  SelectSeparator,
} from "@/components/ui/select"
import { Monitor, Sun, Moon, Check, Loader2, Wifi, Globe, WifiOff, Settings2, Keyboard, RotateCcw } from "lucide-react"

// ── Theme options ──

const THEME_OPTIONS: {
  mode: ThemeMode
  icon: React.ReactNode
  labelKey: string
  descKey: string
}[] = [
  { mode: "auto", icon: <Monitor className="h-5 w-5" />, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: <Sun className="h-5 w-5" />, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: <Moon className="h-5 w-5" />, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

// ── Proxy types ──

interface ProxyConfig {
  mode: "system" | "none" | "custom"
  url: string | null
}

const DEFAULT_PROXY: ProxyConfig = { mode: "system", url: null }

// ── Shortcut types & helpers ──

interface ShortcutBinding {
  id: string
  keys: string
  enabled: boolean
}

interface ShortcutConfig {
  bindings: ShortcutBinding[]
}

const ACTION_LABELS: Record<string, string> = {
  quickChat: "shortcuts.actionQuickChat",
}
const ACTION_DESCS: Record<string, string> = {
  quickChat: "shortcuts.actionQuickChatDesc",
}

const DEFAULT_SHORTCUT_BINDINGS: ShortcutBinding[] = [
  { id: "quickChat", keys: "Alt+Space", enabled: true },
]

const isMac = typeof navigator !== "undefined" && navigator.platform.toUpperCase().includes("MAC")

function formatSingleCombo(combo: string): string {
  if (!combo) return ""
  return combo
    .replace(/CommandOrControl/gi, isMac ? "\u2318" : "Ctrl")
    .replace(/Alt/gi, isMac ? "\u2325" : "Alt")
    .replace(/Shift/gi, isMac ? "\u21E7" : "Shift")
    .replace(/Control/gi, isMac ? "\u2303" : "Ctrl")
    .replace(/Meta/gi, isMac ? "\u2318" : "Win")
    .replace(/Space/gi, "Space")
    .replace(/Comma/gi, ",")
    .replace(/\+/g, " + ")
}

function formatKeyForDisplay(keys: string): string {
  if (!keys) return ""
  // Chord bindings are space-separated (e.g. "CommandOrControl+K CommandOrControl+C")
  const parts = keys.split(/\s+/)
  return parts.map(formatSingleCombo).join("  ")
}

function keyEventToShortcutStr(e: KeyboardEvent): string | null {
  if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) return null
  const parts: string[] = []
  if (e.metaKey || e.ctrlKey) parts.push("CommandOrControl")
  if (e.altKey) parts.push("Alt")
  if (e.shiftKey) parts.push("Shift")
  const modifierCodes = [
    "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight",
    "AltLeft", "AltRight", "MetaLeft", "MetaRight",
  ]
  if (modifierCodes.includes(e.code)) return null
  let keyName: string
  if (e.code.startsWith("Key")) keyName = e.code.slice(3)
  else if (e.code.startsWith("Digit")) keyName = e.code.slice(5)
  else if (e.code === "Space") keyName = "Space"
  else if (e.code === "Comma") keyName = "Comma"
  else if (e.code === "Period") keyName = "Period"
  else if (e.code.startsWith("Arrow")) keyName = e.code.slice(5)
  else if (e.code.startsWith("F") && /^F\d+$/.test(e.code)) keyName = e.code
  else if (["Enter", "Tab", "Escape", "Backspace", "Delete", "Minus", "Equal", "Slash", "Backslash", "BracketLeft", "BracketRight", "Semicolon", "Quote", "Backquote"].includes(e.code)) keyName = e.code
  else keyName = e.key.toUpperCase()
  parts.push(keyName)
  return parts.filter(Boolean).join("+")
}

export default function GeneralPanel() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme } = useTheme()

  // ── Language state ──
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    setLanguage(code)
    setFollowSystem(false)
  }

  // ── System / Effect state ──
  const [autostart, setAutostart] = useState(false)
  const [autostartLoaded, setAutostartLoaded] = useState(false)
  const [uiEffectsEnabled, setUiEffectsEnabled] = useState(true)
  const [uiEffectsLoaded, setUiEffectsLoaded] = useState(false)

  // ── Shortcut state ──
  const [shortcuts, setShortcuts] = useState<ShortcutConfig | null>(null)
  const [shortcutSaving, setShortcutSaving] = useState(false)
  const [shortcutSaveStatus, setShortcutSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [recordingId, setRecordingId] = useState<string | null>(null)
  const [chordFirstPart, setChordFirstPart] = useState<string | null>(null)
  const chordTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [shortcutDirty, setShortcutDirty] = useState(false)
  const shortcutSavedRef = useRef("")

  // ── Proxy state ──
  const [proxy, setProxy] = useState<ProxyConfig>(DEFAULT_PROXY)
  const [proxySaved, setProxySaved] = useState("")
  const [proxySaving, setProxySaving] = useState(false)
  const [proxySaveStatus, setProxySaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [proxyTesting, setProxyTesting] = useState(false)
  const [proxyTestResult, setProxyTestResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const proxyDirty = JSON.stringify(proxy) !== proxySaved

  useEffect(() => {
    let cancelled = false
    Promise.all([
      invoke<boolean>("get_autostart_enabled"),
      invoke<ProxyConfig>("get_proxy_config"),
      invoke<ShortcutConfig>("get_shortcut_config"),
      invoke<boolean>("get_ui_effects_enabled"),
    ])
      .then(([enabled, cfg, sc, effectsEnabled]) => {
        if (cancelled) return
        setAutostart(enabled)
        setAutostartLoaded(true)
        setProxy(cfg)
        setProxySaved(JSON.stringify(cfg))
        setShortcuts(sc)
        shortcutSavedRef.current = JSON.stringify(sc)
        setUiEffectsEnabled(effectsEnabled)
        setUiEffectsLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "GeneralPanel::load", "Failed to load settings", e)
        setAutostartLoaded(true)
      })
    return () => { cancelled = true }
  }, [])

  async function toggleAutostart() {
    const next = !autostart
    setAutostart(next)
    try {
      await invoke("set_autostart_enabled", { enabled: next })
    } catch (e) {
      setAutostart(!next)
      logger.error("settings", "GeneralPanel::toggle", "Failed to set autostart", e)
    }
  }

  async function toggleUiEffects() {
    const next = !uiEffectsEnabled
    setUiEffectsEnabled(next)
    try {
      await invoke("set_ui_effects_enabled", { enabled: next })
      window.dispatchEvent(new Event("ui-effects-changed"))
    } catch (e) {
      setUiEffectsEnabled(!next)
      logger.error("settings", "GeneralPanel::toggle", "Failed to set UI effects", e)
    }
  }

  // ── Proxy handlers ──

  const saveProxy = useCallback(async () => {
    setProxySaving(true)
    try {
      await invoke("save_proxy_config", { config: proxy })
      setProxySaved(JSON.stringify(proxy))
      setProxySaveStatus("saved")
      setTimeout(() => setProxySaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "GeneralPanel::saveProxy", "Failed to save proxy config", e)
      setProxySaveStatus("failed")
      setTimeout(() => setProxySaveStatus("idle"), 2000)
    } finally {
      setProxySaving(false)
    }
  }, [proxy])

  const testProxy = useCallback(async () => {
    setProxyTesting(true)
    setProxyTestResult(null)
    try {
      const msg = await invoke<string>("test_proxy", { config: proxy })
      setProxyTestResult({ ok: true, msg })
    } catch (e) {
      setProxyTestResult({ ok: false, msg: String(e) })
    } finally {
      setProxyTesting(false)
    }
  }, [proxy])

  // ── Shortcut handlers ──

  // Pause/resume global shortcuts when recording starts/stops
  useEffect(() => {
    if (recordingId) {
      invoke("set_shortcuts_paused", { paused: true }).catch(() => {})
    } else {
      invoke("set_shortcuts_paused", { paused: false }).catch(() => {})
      setChordFirstPart(null)
      if (chordTimerRef.current) { clearTimeout(chordTimerRef.current); chordTimerRef.current = null }
    }
  }, [recordingId])

  // Ensure shortcuts are resumed if component unmounts during recording
  useEffect(() => {
    return () => { invoke("set_shortcuts_paused", { paused: false }).catch(() => {}) }
  }, [])

  useEffect(() => {
    if (!recordingId) return
    function finishRecording(keys: string) {
      setShortcuts((prev) => {
        if (!prev) return prev
        const updated = { ...prev, bindings: prev.bindings.map((b) => b.id === recordingId ? { ...b, keys } : b) }
        setShortcutDirty(JSON.stringify(updated) !== shortcutSavedRef.current)
        return updated
      })
      setChordFirstPart(null)
      setRecordingId(null)
      if (chordTimerRef.current) { clearTimeout(chordTimerRef.current); chordTimerRef.current = null }
    }

    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === "Escape") {
        setChordFirstPart(null)
        setRecordingId(null)
        if (chordTimerRef.current) { clearTimeout(chordTimerRef.current); chordTimerRef.current = null }
        return
      }
      const shortcutStr = keyEventToShortcutStr(e)
      if (!shortcutStr) return

      setChordFirstPart((prevFirst) => {
        if (prevFirst) {
          // Second combo captured → complete chord
          finishRecording(`${prevFirst} ${shortcutStr}`)
          return null
        }
        // First combo captured → start chord timer
        if (chordTimerRef.current) clearTimeout(chordTimerRef.current)
        chordTimerRef.current = setTimeout(() => {
          // Timeout → use as single combo
          finishRecording(shortcutStr)
        }, 1500)
        return shortcutStr
      })
    }
    window.addEventListener("keydown", onKeyDown, true)
    return () => window.removeEventListener("keydown", onKeyDown, true)
  }, [recordingId])

  const saveShortcuts = useCallback(async () => {
    if (!shortcuts) return
    setShortcutSaving(true)
    try {
      await invoke("save_shortcut_config", { config: shortcuts })
      shortcutSavedRef.current = JSON.stringify(shortcuts)
      setShortcutDirty(false)
      setShortcutSaveStatus("saved")
      setTimeout(() => setShortcutSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "GeneralPanel::saveShortcuts", "Failed to save shortcut config", e)
      setShortcutSaveStatus("failed")
      setTimeout(() => setShortcutSaveStatus("idle"), 2000)
    } finally {
      setShortcutSaving(false)
    }
  }, [shortcuts])

  const handleShortcutToggle = (id: string, enabled: boolean) => {
    setShortcuts((prev) => {
      if (!prev) return prev
      const updated = { ...prev, bindings: prev.bindings.map((b) => b.id === id ? { ...b, enabled } : b) }
      setShortcutDirty(JSON.stringify(updated) !== shortcutSavedRef.current)
      return updated
    })
  }

  const resetShortcuts = () => {
    const reset = { bindings: DEFAULT_SHORTCUT_BINDINGS.map((b) => ({ ...b })) }
    setShortcuts(reset)
    setShortcutDirty(JSON.stringify(reset) !== shortcutSavedRef.current)
  }

  const proxyModeOptions: { value: ProxyConfig["mode"]; icon: React.ReactNode; label: string; desc: string }[] = [
    { value: "system", icon: <Globe className="h-4 w-4" />, label: t("settings.proxyModeSystem"), desc: t("settings.proxyModeSystemDesc") },
    { value: "none", icon: <WifiOff className="h-4 w-4" />, label: t("settings.proxyModeNone"), desc: t("settings.proxyModeNoneDesc") },
    { value: "custom", icon: <Settings2 className="h-4 w-4" />, label: t("settings.proxyModeCustom"), desc: t("settings.proxyModeCustomDesc") },
  ]

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <Tabs defaultValue="appearance" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="appearance">{t("settings.tabAppearance")}</TabsTrigger>
            <TabsTrigger value="system">{t("settings.tabSystem")}</TabsTrigger>
            <TabsTrigger value="network">{t("settings.tabNetwork")}</TabsTrigger>
          </TabsList>
        </div>

        {/* ── Appearance & Language ── */}
        <TabsContent value="appearance" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="max-w-4xl space-y-8 pt-4">
            {/* Theme */}
            <div>
              <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.appearance")}</h3>
              <p className="text-xs text-muted-foreground mb-3">{t("settings.appearanceDesc")}</p>
              <div className="space-y-1">
                {THEME_OPTIONS.map((opt) => (
                  <button
                    key={opt.mode}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
                      theme === opt.mode
                        ? "bg-primary/10 text-primary font-medium"
                        : "text-foreground hover:bg-secondary/60",
                    )}
                    onClick={() => setTheme(opt.mode)}
                  >
                    <span className={cn("shrink-0", theme === opt.mode ? "text-primary" : "text-muted-foreground")}>
                      {opt.icon}
                    </span>
                    <div className="flex-1 text-left">
                      <div>{t(opt.labelKey)}</div>
                      <div className="text-xs text-muted-foreground font-normal">{t(opt.descKey)}</div>
                    </div>
                    {theme === opt.mode && <Check className="h-4 w-4 text-primary shrink-0" />}
                  </button>
                ))}
              </div>
            </div>

            {/* Language */}
            <div>
              <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.language")}</h3>
              <p className="text-xs text-muted-foreground mb-3">{t("settings.languageDesc")}</p>
              <Select
                value={
                  followSystem
                    ? "system"
                    : (SUPPORTED_LANGUAGES.find(
                        (l) => i18n.language === l.code || i18n.language.startsWith(l.code + "-"),
                      )?.code ?? "system")
                }
                onValueChange={(val) => {
                  if (val === "system") handleFollowSystem()
                  else handleSelectLanguage(val)
                }}
              >
                <SelectTrigger className="w-full max-w-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="system">{t("language.system")}</SelectItem>
                  <SelectSeparator />
                  {SUPPORTED_LANGUAGES.map((lang) => (
                    <SelectItem key={lang.code} value={lang.code}>
                      {lang.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* UI Effects */}
            <div>
              <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.uiEffects", "背景动效")}</h3>
              {uiEffectsLoaded && (
                <div
                  className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
                  onClick={toggleUiEffects}
                >
                  <div className="space-y-0.5">
                    <div className="text-sm font-medium">{t("settings.uiEffectsToggle", "开启动效")}</div>
                    <div className="text-xs text-muted-foreground">{t("settings.uiEffectsDesc", "开启全天候背景及天气特效联动")}</div>
                  </div>
                  <Switch checked={uiEffectsEnabled} onCheckedChange={toggleUiEffects} />
                </div>
              )}
            </div>
          </div>
        </TabsContent>

        {/* ── System & Shortcuts ── */}
        <TabsContent value="system" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="max-w-4xl space-y-8 pt-4">
            {/* Autostart */}
            <div>
              <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.system")}</h3>
              {autostartLoaded && (
                <div
                  className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
                  onClick={toggleAutostart}
                >
                  <div className="space-y-0.5">
                    <div className="text-sm font-medium">{t("settings.systemAutostart")}</div>
                    <div className="text-xs text-muted-foreground">{t("settings.systemAutostartDesc")}</div>
                  </div>
                  <Switch checked={autostart} onCheckedChange={toggleAutostart} />
                </div>
              )}
            </div>

            {/* Shortcuts */}
            {shortcuts && (
              <div>
                <h3 className="text-sm font-semibold text-foreground mb-1">{t("shortcuts.title")}</h3>
                <p className="text-xs text-muted-foreground mb-3">{t("shortcuts.desc")}</p>
                <div className="space-y-2">
                  {shortcuts.bindings.map((binding) => (
                    <div
                      key={binding.id}
                      className="flex items-center gap-3 px-4 py-3 rounded-lg border border-border bg-card"
                    >
                      <Keyboard className="h-4 w-4 text-muted-foreground shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium">
                          {t(ACTION_LABELS[binding.id] ?? binding.id)}
                        </div>
                        {ACTION_DESCS[binding.id] && (
                          <div className="text-xs text-muted-foreground">
                            {t(ACTION_DESCS[binding.id])}
                          </div>
                        )}
                      </div>
                      <button
                        className={cn(
                          "px-3 py-1.5 rounded-md border text-sm font-mono min-w-[120px] text-center transition-colors",
                          recordingId === binding.id
                            ? "border-primary bg-primary/10 text-primary animate-pulse"
                            : "border-border bg-secondary/40 text-foreground hover:bg-secondary/80",
                          !binding.enabled && "opacity-40",
                        )}
                        onClick={() => setRecordingId(recordingId === binding.id ? null : binding.id)}
                        disabled={!binding.enabled}
                      >
                        {recordingId === binding.id
                          ? (chordFirstPart
                            ? `${formatSingleCombo(chordFirstPart)}  ${t("shortcuts.chordNext")}`
                            : t("shortcuts.recording"))
                          : formatKeyForDisplay(binding.keys) || t("shortcuts.unset")}
                      </button>
                      <Switch
                        checked={binding.enabled}
                        onCheckedChange={(v) => handleShortcutToggle(binding.id, v)}
                      />
                    </div>
                  ))}
                </div>
                <p className="text-xs text-muted-foreground mt-2 mb-1">{t("shortcuts.hint")}</p>
                <p className="text-xs text-muted-foreground mb-3">{t("shortcuts.chordHint")}</p>
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    onClick={saveShortcuts}
                    disabled={(!shortcutDirty && shortcutSaveStatus === "idle") || shortcutSaving}
                    className={cn(
                      shortcutSaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
                      shortcutSaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
                    )}
                  >
                    {shortcutSaving ? (
                      <span className="flex items-center gap-1.5">
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        {t("common.saving")}
                      </span>
                    ) : shortcutSaveStatus === "saved" ? (
                      <span className="flex items-center gap-1.5">
                        <Check className="h-3.5 w-3.5" />
                        {t("common.saved")}
                      </span>
                    ) : shortcutSaveStatus === "failed" ? (
                      t("common.saveFailed")
                    ) : (
                      t("common.save")
                    )}
                  </Button>
                  <Button variant="outline" size="sm" onClick={resetShortcuts}>
                    <RotateCcw className="h-3.5 w-3.5 mr-1.5" />
                    {t("shortcuts.reset")}
                  </Button>
                </div>
              </div>
            )}
          </div>
        </TabsContent>

        {/* ── Network / Proxy ── */}
        <TabsContent value="network" className="flex-1 overflow-y-auto px-6 pb-6">
          <div className="max-w-4xl pt-4">
            <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.proxySettings")}</h3>
            <p className="text-xs text-muted-foreground mb-3">{t("settings.proxySettingsDesc")}</p>
            <div className="space-y-3">
              {/* Mode selector */}
              <div className="space-y-1.5">
                {proxyModeOptions.map((opt) => (
                  <div
                    key={opt.value}
                    className={cn(
                      "flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-colors",
                      proxy.mode === opt.value
                        ? "bg-primary/10 border border-primary/30"
                        : "hover:bg-secondary/40 border border-transparent",
                    )}
                    onClick={() => setProxy((p) => ({ ...p, mode: opt.value }))}
                  >
                    <div className={cn("shrink-0", proxy.mode === opt.value ? "text-primary" : "text-muted-foreground")}>
                      {opt.icon}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium">{opt.label}</div>
                      <div className="text-xs text-muted-foreground">{opt.desc}</div>
                    </div>
                    <div className={cn(
                      "h-4 w-4 rounded-full border-2 shrink-0 transition-colors",
                      proxy.mode === opt.value ? "border-primary bg-primary" : "border-muted-foreground/30",
                    )}>
                      {proxy.mode === opt.value && (
                        <div className="h-full w-full flex items-center justify-center">
                          <div className="h-1.5 w-1.5 rounded-full bg-primary-foreground" />
                        </div>
                      )}
                    </div>
                  </div>
                ))}
              </div>

              {/* Custom proxy URL input */}
              {proxy.mode === "custom" && (
                <div className="space-y-1.5">
                  <span className="text-xs text-muted-foreground">{t("settings.proxyUrl")}</span>
                  <Input
                    value={proxy.url ?? ""}
                    placeholder={t("settings.proxyUrlPlaceholder")}
                    onChange={(e) => setProxy((p) => ({ ...p, url: e.target.value || null }))}
                  />
                </div>
              )}

              {/* Save + Test buttons */}
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  onClick={saveProxy}
                  disabled={(!proxyDirty && proxySaveStatus === "idle") || proxySaving}
                  className={cn(
                    proxySaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
                    proxySaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
                  )}
                >
                  {proxySaving ? (
                    <span className="flex items-center gap-1.5">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      {t("common.saving")}
                    </span>
                  ) : proxySaveStatus === "saved" ? (
                    <span className="flex items-center gap-1.5">
                      <Check className="h-3.5 w-3.5" />
                      {t("common.saved")}
                    </span>
                  ) : proxySaveStatus === "failed" ? (
                    t("common.saveFailed")
                  ) : (
                    t("common.save")
                  )}
                </Button>

                <Button
                  variant="secondary"
                  size="sm"
                  disabled={proxyTesting || (proxy.mode === "custom" && !proxy.url?.trim())}
                  onClick={testProxy}
                >
                  {proxyTesting ? (
                    <span className="flex items-center gap-1.5">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      {t("common.testing")}
                    </span>
                  ) : (
                    <span className="flex items-center gap-1.5">
                      <Wifi className="h-3.5 w-3.5" />
                      {t("common.test")}
                    </span>
                  )}
                </Button>
              </div>

              {/* Test result */}
              {proxyTestResult && (
                <div className={cn(
                  "px-3 py-2 rounded-md text-xs",
                  proxyTestResult.ok ? "bg-green-500/10 text-green-600" : "bg-destructive/10 text-destructive",
                )}>
                  <div className="font-medium">
                    {proxyTestResult.ok ? t("settings.proxyTestSuccess") : t("settings.proxyTestFailed")}
                  </div>
                  <pre className="mt-1 whitespace-pre-wrap break-all opacity-80">{proxyTestResult.msg}</pre>
                </div>
              )}
            </div>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
