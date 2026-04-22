import { useState, useRef, useCallback, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { useClickOutside } from "@/hooks/useClickOutside"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { Eye, EyeOff, Loader2, Check } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

type AwarenessMode = "off" | "structured" | "llm_digest"

interface SessionAwarenessOverride {
  enabled?: boolean
  mode?: AwarenessMode
}

interface Props {
  sessionId: string | null
  disabled?: boolean
}

/**
 * Compact popover button for per-session behavior awareness override.
 * Placed in the chat input bar alongside TemperatureSlider.
 *
 * When the global feature is disabled, the button is hidden entirely.
 * When visible, it shows an eye icon that opens a small popover with
 * enable/disable toggle + mode selector for this session only.
 */
export default function AwarenessToggle({ sessionId, disabled = false }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [globalEnabled, setGlobalEnabled] = useState(false)
  const [override, setOverride] = useState<SessionAwarenessOverride | null>(
    null,
  )
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useClickOutside(ref, useCallback(() => setOpen(false), []))

  useEffect(() => {
    if (disabled && open) {
      setOpen(false)
    }
  }, [disabled, open])

  // Load global config to check if feature is enabled at all.
  useEffect(() => {
    getTransport()
      .call<{ enabled: boolean }>("get_awareness_config")
      .then((c) => setGlobalEnabled(c.enabled))
      .catch(() => {})
  }, [])

  // Load session-level override when popover opens.
  useEffect(() => {
    if (!open || !sessionId) return
    getTransport()
      .call<{ json: string | null }>("get_session_awareness_override", {
        sessionId,
      })
      .then((res) => {
        if (res.json) {
          try {
            setOverride(JSON.parse(res.json))
          } catch {
            setOverride(null)
          }
        } else {
          setOverride(null)
        }
      })
      .catch(() => setOverride(null))
  }, [open, sessionId])

  // Don't render if global feature is off or no session.
  if (!globalEnabled || !sessionId) return null

  const isOverridden = override !== null
  const isDisabledLocally = override?.enabled === false

  async function saveOverride(next: SessionAwarenessOverride | null) {
    setOverride(next)
    setSaving(true)
    try {
      await getTransport().call("set_session_awareness_override", {
        sessionId,
        json: next ? JSON.stringify(next) : null,
      })
      setSaved(true)
      setTimeout(() => setSaved(false), 1200)
    } catch (e) {
      logger.error(
        "chat",
        "AwarenessToggle::save",
        "Failed to save override",
        e,
      )
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="relative" ref={ref}>
      <IconTip
        label={
          disabled
            ? t("chat.incognitoAwarenessDisabled")
            : t("settings.awareness.title", "Behavior Awareness")
        }
      >
        <button
          type="button"
          disabled={disabled}
          onClick={() => setOpen(!open)}
          className={cn(
            "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 disabled:cursor-not-allowed disabled:opacity-50",
            isDisabledLocally
              ? "text-orange-500"
              : isOverridden
                ? "text-blue-500"
                : "text-muted-foreground hover:text-foreground",
          )}
        >
          {isDisabledLocally ? (
            <EyeOff className="h-3.5 w-3.5" />
          ) : (
            <Eye className="h-3.5 w-3.5" />
          )}
        </button>
      </IconTip>

      {open && !disabled && (
        <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 w-[220px] p-3 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
          {/* Header */}
          <div className="flex items-center justify-between mb-2">
            <span className="text-[11px] text-muted-foreground font-medium">
              {t(
                "settings.awareness.sessionOverride",
                "Session Override",
              )}
            </span>
            {saving && (
              <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
            )}
            {saved && <Check className="h-3 w-3 text-emerald-500" />}
          </div>

          {/* Enable/disable for this session */}
          <div className="flex items-center justify-between py-1.5">
            <span className="text-xs">
              {t("settings.awareness.enabledForSession", "Enabled")}
            </span>
            <Switch
              checked={override?.enabled !== false}
              onCheckedChange={(v) => {
                if (v) {
                  // Remove the "enabled: false" override (inherit global).
                  if (override?.mode) {
                    saveOverride({ mode: override.mode })
                  } else {
                    saveOverride(null)
                  }
                } else {
                  saveOverride({ ...override, enabled: false })
                }
              }}
            />
          </div>

          {/* Mode selector (only when enabled) */}
          {override?.enabled !== false && (
            <div className="mt-1.5">
              <span className="text-[11px] text-muted-foreground font-medium">
                {t("settings.awareness.mode", "Mode")}
              </span>
              <Select
                value={override?.mode ?? "inherit"}
                onValueChange={(v) => {
                  if (v === "inherit") {
                    // Remove mode override.
                    if (override?.enabled === false) {
                      saveOverride({ enabled: false })
                    } else {
                      saveOverride(null)
                    }
                  } else {
                    saveOverride({
                      ...override,
                      mode: v as AwarenessMode,
                    })
                  }
                }}
              >
                <SelectTrigger className="mt-1 h-7 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="inherit">
                    {t(
                      "settings.awareness.modeInherit",
                      "Inherit global",
                    )}
                  </SelectItem>
                  <SelectItem value="structured">
                    {t(
                      "settings.awareness.modeStructured",
                      "Structured",
                    )}
                  </SelectItem>
                  <SelectItem value="llm_digest">
                    {t("settings.awareness.modeLlm", "LLM Digest")}
                  </SelectItem>
                  <SelectItem value="off">
                    {t("settings.awareness.modeOff", "Off")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          )}

          {/* Reset link */}
          {isOverridden && (
            <button
              className="mt-2 text-[10px] text-primary hover:text-primary/80 transition-colors"
              onClick={() => saveOverride(null)}
            >
              {t("settings.awareness.resetOverride", "Reset to global")}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
