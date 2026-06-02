import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ExternalLink, RefreshCw, X, Hand, Globe } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { RightPanelShell } from "./right-panel/RightPanelShell"

// ── Types (mirror ha_core::browser::frame::BrowserFramePayload) ─────────

interface BrowserFramePayload {
  targetId?: string | null
  url?: string | null
  title?: string | null
  jpegBase64: string
  capturedAt: number
  backend: string
}

interface BrowserPanelProps {
  /** Right-panel width in px. Driven by the same drag handler ChatScreen uses
   *  for the sibling Plan / Diff / Canvas panels. */
  panelWidth?: number
  onPanelWidthChange?: (width: number) => void
  collapsed?: boolean
  onClose: () => void
}

// ── Constants ────────────────────────────────────────────────────────────

/** Backend-emitted event name (see `crates/ha-core/src/browser/frame.rs`). */
const BROWSER_FRAME_EVENT = "browser:frame"
/** Fallback poll interval for catching user-initiated browser changes. */
const POLL_INTERVAL_MS = 1000

// ── Component ────────────────────────────────────────────────────────────

export default function BrowserPanel({
  panelWidth = 480,
  onPanelWidthChange,
  collapsed = false,
  onClose,
}: BrowserPanelProps) {
  const { t } = useTranslation()
  const [frame, setFrame] = useState<BrowserFramePayload | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [paused, setPaused] = useState(false)
  const mountedRef = useRef(true)

  const refresh = useCallback(async () => {
    try {
      const next = await getTransport().call<BrowserFramePayload | null>(
        "browser_capture_frame",
      )
      if (!mountedRef.current) return
      // `null` is the backend's empty signal — no active backend or browser
      // disconnected. Surface it as idle by clearing the frame; otherwise the
      // 1Hz poll would silently freeze on the last stale screenshot.
      setFrame(next)
      setError(null)
    } catch (e) {
      logger.warn("ui", "BrowserPanel::capture", "browser_capture_frame failed", e)
      if (mountedRef.current) {
        setError(t("chat.browserPanel.captureFailed"))
      }
    }
  }, [t])

  // Mount-only lifecycle flag. Keep it independent from collapsed/paused so
  // in-flight capture responses from stale effects cannot update after unmount.
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  // Initial frame when the visible panel opens. Skip while collapsed; the
  // EventBus listener still tracks pushed frames, but active captures pause.
  useEffect(() => {
    if (collapsed) return
    const initialTimer = setTimeout(() => {
      if (mountedRef.current) void refresh()
    }, 0)
    return () => clearTimeout(initialTimer)
  }, [collapsed, refresh])

  // Mount-only EventBus listener. Independent of paused/collapsed so pushed
  // frames keep the local mirror fresh without active polling.
  useEffect(() => {
    const unlisten = getTransport().listen(BROWSER_FRAME_EVENT, (raw) => {
      const payload = parsePayload<BrowserFramePayload>(raw)
      if (payload && mountedRef.current) {
        setFrame(payload)
        setError(null)
      }
    })

    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [])

  // 1Hz fallback poll. Re-binds only when `paused` flips, not on every render.
  useEffect(() => {
    if (paused || collapsed) return
    const interval = setInterval(() => {
      void refresh()
    }, POLL_INTERVAL_MS)
    return () => clearInterval(interval)
  }, [collapsed, paused, refresh])

  // ── Render ─────────────────────────────────────────────────────────────

  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("chat.browserPanel.resizePanel", "Resize browser panel")}
      collapsed={collapsed}
      contentKey="browser"
    >
      {/* Header */}
      <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
        <Globe className="h-4 w-4 text-muted-foreground" />
        <div className="flex-1 truncate text-sm font-medium">
          {frame?.title || t("chat.browserPanel.idleTitle")}
        </div>
        {frame?.backend && (
          <span
            className={cn(
              "rounded px-1.5 py-0.5 text-[10px] font-medium uppercase",
              frame.backend === "mcp"
                ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                : "bg-blue-500/15 text-blue-700 dark:text-blue-300",
            )}
            title={t("chat.browserPanel.backendBadgeTooltip", {
              backend: frame.backend,
            })}
          >
            {frame.backend}
          </span>
        )}
        <IconTip label={t("chat.browserPanel.refresh")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            onClick={() => void refresh()}
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
        <IconTip label={t("chat.browserPanel.close")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            onClick={onClose}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      </div>

      {/* URL bar */}
      {frame?.url && (
        <div className="truncate border-b border-border/60 bg-muted/40 px-3 py-1 text-xs text-muted-foreground">
          {frame.url}
        </div>
      )}

      {/* Frame */}
      <div className="relative flex-1 overflow-auto bg-muted/30">
        {error ? (
          <div className="flex h-full items-center justify-center px-6 text-center text-sm text-destructive">
            {error}
          </div>
        ) : frame?.jpegBase64 ? (
          <img
            src={`data:image/jpeg;base64,${frame.jpegBase64}`}
            alt={frame.title || "Browser frame"}
            className="block h-auto w-full select-none"
            draggable={false}
          />
        ) : (
          <div className="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
            {t("chat.browserPanel.idleHint")}
          </div>
        )}
      </div>

      {/* Footer actions */}
      <div className="flex items-center gap-2 border-t border-border/60 px-3 py-2">
        <IconTip label={t("chat.browserPanel.takeOverHint")}>
          <Button
            type="button"
            variant={paused ? "default" : "outline"}
            size="sm"
            className="h-7 gap-1.5 text-xs"
            onClick={() => setPaused((p) => !p)}
          >
            <Hand className="h-3.5 w-3.5" />
            {paused
              ? t("chat.browserPanel.resumeMirror")
              : t("chat.browserPanel.takeOver")}
          </Button>
        </IconTip>
        {frame?.url && (
          <IconTip label={t("chat.browserPanel.openExternal")}>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 text-xs"
              onClick={() => {
                if (frame.url) window.open(frame.url, "_blank")
              }}
            >
              <ExternalLink className="h-3.5 w-3.5" />
              {t("chat.browserPanel.openExternalShort")}
            </Button>
          </IconTip>
        )}
        <div className="ml-auto text-[10px] text-muted-foreground">
          {frame?.capturedAt
            ? new Date(frame.capturedAt).toLocaleTimeString()
            : ""}
        </div>
      </div>
    </RightPanelShell>
  )
}
