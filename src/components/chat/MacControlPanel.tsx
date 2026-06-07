import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Monitor, RefreshCw, X } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { RightPanelShell } from "./right-panel/RightPanelShell"

interface MacControlAppSummary {
  pid: number
  bundleId?: string | null
  name?: string | null
}

interface MacControlBounds {
  x: number
  y: number
  width: number
  height: number
}

interface MacControlFramePayload {
  snapshotId: string
  mediaId?: string | null
  path?: string | null
  jpegBase64: string
  widthPx: number
  heightPx: number
  target?: "display" | "window"
  displayId?: number | null
  windowId?: string | null
  windowTitle?: string | null
  boundsPoints?: MacControlBounds | null
  scale?: number | null
  capturedAt: number
  frontmostApp?: MacControlAppSummary | null
}

interface MacControlFrameResponse {
  frame?: MacControlFramePayload | null
  error?: string | null
}

interface MacControlPanelProps {
  panelWidth?: number
  onPanelWidthChange?: (width: number) => void
  reservedMainWidth?: number
  collapsed?: boolean
  onClose: () => void
}

const MAC_CONTROL_FRAME_EVENT = "mac_control:frame"
const POLL_INTERVAL_MS = 1000

export default function MacControlPanel({
  panelWidth = 480,
  onPanelWidthChange,
  reservedMainWidth,
  collapsed = false,
  onClose,
}: MacControlPanelProps) {
  const { t } = useTranslation()
  const [frame, setFrame] = useState<MacControlFramePayload | null>(null)
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)

  const refresh = useCallback(async () => {
    try {
      const response = await getTransport().call<MacControlFrameResponse>(
        "mac_control_capture_frame",
      )
      if (!mountedRef.current) return
      setFrame(response.frame ?? null)
      setError(response.error ?? null)
    } catch (e) {
      logger.warn(
        "ui",
        "MacControlPanel::capture",
        "mac_control_capture_frame failed",
        e,
      )
      if (mountedRef.current) {
        setError(t("chat.browserPanel.captureFailed"))
      }
    }
  }, [t])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  useEffect(() => {
    if (collapsed) return
    const initialTimer = setTimeout(() => {
      if (mountedRef.current) void refresh()
    }, 0)
    return () => clearTimeout(initialTimer)
  }, [collapsed, refresh])

  useEffect(() => {
    const unlisten = getTransport().listen(MAC_CONTROL_FRAME_EVENT, (raw) => {
      const payload = parsePayload<MacControlFramePayload>(raw)
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

  useEffect(() => {
    if (collapsed) return
    const interval = setInterval(() => {
      void refresh()
    }, POLL_INTERVAL_MS)
    return () => clearInterval(interval)
  }, [collapsed, refresh])

  const title = frame?.frontmostApp?.name || t("settings.macControl.title")

  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("chat.browserPanel.resizePanel", "Resize panel")}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      contentKey="mac-control"
    >
      <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
        <Monitor className="h-4 w-4 text-muted-foreground" />
        <div className="flex-1 truncate text-sm font-medium">{title}</div>
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

      <div className="relative flex-1 overflow-auto bg-muted/30">
        {error ? (
          <div className="flex h-full items-center justify-center px-6 text-center text-sm text-destructive">
            {error}
          </div>
        ) : frame?.jpegBase64 ? (
          <img
            src={`data:image/jpeg;base64,${frame.jpegBase64}`}
            alt={title}
            className="block h-auto w-full select-none"
            draggable={false}
          />
        ) : (
          <div className="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
            {t("settings.macControl.messages.blocked")}
          </div>
        )}
      </div>

      <div className="flex items-center gap-2 border-t border-border/60 px-3 py-2">
        <div className="truncate text-[10px] text-muted-foreground">
          {frame?.path ?? ""}
        </div>
        <div className="ml-auto shrink-0 text-[10px] text-muted-foreground">
          {frame
            ? `${frame.widthPx}x${frame.heightPx} · ${new Date(
                frame.capturedAt,
              ).toLocaleTimeString()}`
            : ""}
        </div>
      </div>
    </RightPanelShell>
  )
}
