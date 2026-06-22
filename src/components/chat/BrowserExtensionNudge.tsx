import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { RefreshCw, X } from "lucide-react"
import IconChrome from "~icons/logos/chrome"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SettingsSection } from "@/components/settings/types"

// Persisted "don't show again" flag. UI-only state, so localStorage is the right
// home (no need to round-trip a config write for a dismissal preference).
const DISMISS_KEY = "ha:browserExtensionNudgeDismissed"

// Grace delay before surfacing the banner. After an app restart the extension
// is briefly "not connected" while it reconnects (host respawn + hello). We wait
// this out and re-check, so a transient reconnect window never flashes a banner —
// only a genuinely-unavailable extension does.
const SHOW_GRACE_MS = 6000

interface BrowserConfigShape {
  backendPreference?: string
  extension?: { enabled?: boolean }
}
interface ExtensionStatusShape {
  backendAvailable: boolean
  kind: string
}
interface FallbackPayload {
  statusKind?: string
  sessionId?: string
}

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === "1"
  } catch {
    return false
  }
}

/**
 * Soft, dismissible onboarding banner shown at the top of the chat when the
 * extension backend is the configured default but unavailable. Two triggers:
 *   1. Proactive — on mount, if extension is default and not ready (first-run /
 *      "you could drive your real Chrome" discovery).
 *   2. At-use — the `browser:extension_fallback` event, emitted by the backend
 *      when an extension-preferred action silently fell back to CDP.
 * Message switches between "install/set up" and "reload to update" by status
 * kind. The hard-blocker `browser:extension_required` toast lives separately in
 * ChatScreen; this banner never blocks.
 */
export function BrowserExtensionNudge({
  sessionId,
  onOpenSettings,
}: {
  sessionId?: string | null
  onOpenSettings?: (section?: SettingsSection) => void
}) {
  const { t } = useTranslation()
  const [kind, setKind] = useState<string | null>(null)
  const dismissedRef = useRef<boolean>(readDismissed())
  const confirmTimerRef = useRef<number | null>(null)

  // Confirm-then-show: never surface immediately. Wait out the post-restart
  // reconnect window, then re-check, and only show if the extension is STILL
  // unavailable (and is the configured default). Triggered both proactively on
  // mount and by the at-use fallback event; the timer guard coalesces bursts.
  const confirmAndShow = useCallback(() => {
    if (dismissedRef.current || confirmTimerRef.current != null) return
    confirmTimerRef.current = window.setTimeout(async () => {
      confirmTimerRef.current = null
      if (dismissedRef.current) return
      try {
        const [cfg, status] = await Promise.all([
          getTransport().call<BrowserConfigShape>("browser_get_config"),
          getTransport().call<ExtensionStatusShape>("browser_extension_status"),
        ])
        const extensionIsDefault =
          cfg.backendPreference !== "cdp_only" && cfg.extension?.enabled !== false
        if (extensionIsDefault && !status.backendAvailable) {
          setKind(status.kind)
        }
      } catch (e) {
        logger.error("chat", "BrowserExtensionNudge", `confirm probe failed: ${e}`)
      }
    }, SHOW_GRACE_MS)
  }, [])

  // Proactive (first-run discovery): confirm after the grace delay on mount.
  useEffect(() => {
    confirmAndShow()
    return () => {
      if (confirmTimerRef.current != null) {
        clearTimeout(confirmTimerRef.current)
        confirmTimerRef.current = null
      }
    }
  }, [confirmAndShow])

  // At-use trigger: backend emits this when an extension-preferred action fell
  // back to CDP because the extension was unavailable.
  useEffect(() => {
    const unlisten = getTransport().listen("browser:extension_fallback", (raw) => {
      try {
        const payload = JSON.parse(String(raw)) as FallbackPayload
        if (payload.sessionId && sessionId && payload.sessionId !== sessionId) return
        confirmAndShow()
      } catch {
        /* ignore malformed payloads */
      }
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        /* ignore */
      }
    }
  }, [sessionId, confirmAndShow])

  if (!kind) return null

  const isReload = kind === "version_mismatch"
  const title = isReload
    ? t("chat.browserExtensionNudge.reloadTitle", { defaultValue: "Reload the Chrome extension" })
    : t("chat.browserExtensionNudge.installTitle", {
        defaultValue: "Let the AI drive your real Chrome",
      })
  const body = isReload
    ? t("chat.browserExtensionNudge.reloadBody", {
        defaultValue:
          "The installed extension doesn’t match the host. Reload it to restore control of your real Chrome.",
      })
    : t("chat.browserExtensionNudge.installBody", {
        defaultValue:
          "Install the Hope Agent extension to operate your signed-in Chrome. For now it runs in an isolated browser.",
      })

  const dismissForNow = () => setKind(null)
  const dismissForever = () => {
    try {
      localStorage.setItem(DISMISS_KEY, "1")
    } catch {
      /* ignore */
    }
    dismissedRef.current = true
    setKind(null)
  }

  return (
    <div className="shrink-0 border-b border-amber-500/30 bg-amber-500/10 px-4 py-2.5">
      <div className="flex items-start gap-3">
        {isReload ? (
          <RefreshCw className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        ) : (
          <IconChrome className="mt-0.5 h-4 w-4 shrink-0" />
        )}
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-foreground">{title}</div>
          <p className="text-xs text-muted-foreground">{body}</p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            className="h-7"
            onClick={() => {
              onOpenSettings?.("browser")
              dismissForNow()
            }}
          >
            {t("chat.browserExtensionNudge.openSettings", { defaultValue: "Open settings" })}
          </Button>
          <Button size="sm" variant="ghost" className="h-7" onClick={dismissForever}>
            {t("chat.browserExtensionNudge.dismiss", { defaultValue: "Don’t show again" })}
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-7 w-7"
            onClick={dismissForNow}
            aria-label={t("common.close", { defaultValue: "Close" })}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </div>
  )
}
