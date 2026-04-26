import { useState, useEffect, useCallback, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { logger } from "@/lib/logger"
import { Keyboard, Check, Loader2, RotateCcw } from "lucide-react"

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

export default function ShortcutSection() {
  const { t } = useTranslation()

  const [shortcuts, setShortcuts] = useState<ShortcutConfig | null>(null)
  const [shortcutSaving, setShortcutSaving] = useState(false)
  const [shortcutSaveStatus, setShortcutSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [recordingId, setRecordingId] = useState<string | null>(null)
  const [chordFirstPart, setChordFirstPart] = useState<string | null>(null)
  const chordTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [shortcutDirty, setShortcutDirty] = useState(false)
  const shortcutSavedRef = useRef("")

  useEffect(() => {
    let cancelled = false
    getTransport().call<ShortcutConfig>("get_shortcut_config")
      .then((sc) => {
        if (cancelled) return
        setShortcuts(sc)
        shortcutSavedRef.current = JSON.stringify(sc)
      })
      .catch((e) => {
        logger.error("settings", "ShortcutSection::load", "Failed to load shortcut config", e)
      })
    return () => { cancelled = true }
  }, [])

  // Pause/resume global shortcuts when recording starts/stops
  useEffect(() => {
    if (recordingId) {
      getTransport().call("set_shortcuts_paused", { paused: true }).catch(() => {})
    } else {
      getTransport().call("set_shortcuts_paused", { paused: false }).catch(() => {})
      setChordFirstPart(null)
      if (chordTimerRef.current) { clearTimeout(chordTimerRef.current); chordTimerRef.current = null }
    }
  }, [recordingId])

  // Ensure shortcuts are resumed if component unmounts during recording
  useEffect(() => {
    return () => { getTransport().call("set_shortcuts_paused", { paused: false }).catch(() => {}) }
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
          finishRecording(`${prevFirst} ${shortcutStr}`)
          return null
        }
        if (chordTimerRef.current) clearTimeout(chordTimerRef.current)
        chordTimerRef.current = setTimeout(() => {
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
      await getTransport().call("save_shortcut_config", { config: shortcuts })
      shortcutSavedRef.current = JSON.stringify(shortcuts)
      setShortcutDirty(false)
      setShortcutSaveStatus("saved")
      setTimeout(() => setShortcutSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ShortcutSection::saveShortcuts", "Failed to save shortcut config", e)
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

  if (!shortcuts) return null

  return (
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
            <Button
              variant="outline"
              size="sm"
              className={cn(
                "h-auto px-3 py-1.5 text-sm font-mono min-w-[120px]",
                recordingId === binding.id
                  ? "border-primary bg-primary/10 text-primary animate-pulse hover:bg-primary/15 hover:text-primary"
                  : "bg-secondary/40 hover:bg-secondary/80",
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
            </Button>
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
  )
}
