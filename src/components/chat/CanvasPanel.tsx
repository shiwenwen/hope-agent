import { useState, useEffect, useRef, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload, isTauriMode } from "@/lib/transport"
import { MAIN_WINDOW_MIN_HEIGHT, MAIN_WINDOW_MIN_WIDTH } from "@/lib/mainWindowSize"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { WebviewWindow } from "@tauri-apps/api/webviewWindow"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { X, RefreshCw, Maximize2, Minimize2, ExternalLink, PanelLeftClose } from "lucide-react"
import { IconTip } from "@/components/ui/tooltip"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import ArtifactViewer from "@/components/artifacts/ArtifactViewer"

interface CanvasInfo {
  projectId: string
  title: string
  contentType: string
  projectPath?: string
}

interface CanvasShowPayload {
  projectId: string
  title?: string
  contentType?: string
  projectPath?: string
  sessionId?: string | null
}

interface CanvasProjectView {
  id: string
  title: string
  contentType: string
  projectPath?: string
  sessionId?: string
}

function toCanvasInfo(
  src: { id?: string; projectId?: string } & Omit<CanvasShowPayload, "projectId">,
  fallbackTitle: string,
): CanvasInfo {
  return {
    projectId: (src.projectId ?? src.id ?? "") as string,
    title: src.title || fallbackTitle,
    contentType: src.contentType || "html",
    projectPath: src.projectPath,
  }
}

interface CanvasPanelProps {
  panelWidth?: number
  onPanelWidthChange?: (width: number) => void
  reservedMainWidth?: number
  currentSessionId?: string | null
  onOpenChange?: (open: boolean) => void
  visible?: boolean
  collapsed?: boolean
  overlay?: boolean
  /** Kept for the shared panel call site; iframe panels intentionally ignore
   * zero-width mount animation so WebView hit testing is valid immediately. */
  animateOnMount?: boolean
}

export const CLOSE_CANVAS_PANEL_EVENT = "hope-agent:close-canvas"

export default function CanvasPanel({
  panelWidth = 480,
  onPanelWidthChange,
  reservedMainWidth,
  currentSessionId = null,
  onOpenChange,
  visible = true,
  collapsed = false,
  overlay = false,
}: CanvasPanelProps) {
  const { t } = useTranslation()
  const [canvas, setCanvas] = useState<CanvasInfo | null>(null)
  const [maximized, setMaximized] = useState(false)
  const [detached, setDetached] = useState(false)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const [refreshKey, setRefreshKey] = useState(0)
  const detachedWindowRef = useRef<WebviewWindow | null>(null)
  // Read by canvas_show listener to ignore events fired from other sessions
  // (e.g. cron/channel/subagent tool calls that emit canvas_show globally).
  // ref so the listener is not re-subscribed on every session switch.
  const currentSessionIdRef = useRef<string | null>(currentSessionId)
  useEffect(() => {
    currentSessionIdRef.current = currentSessionId
  }, [currentSessionId])

  // Reset transient UI state on session change via render-phase prev-prop
  // tracking (https://react.dev/reference/react/useState) — doing this in an
  // effect would trip `react-hooks/set-state-in-effect`.
  const [prevSessionId, setPrevSessionId] = useState<string | null | undefined>(currentSessionId)
  if (prevSessionId !== currentSessionId) {
    setPrevSessionId(currentSessionId)
    setMaximized(false)
    setDetached(false)
  }

  const handleSnapshotRequest = useCallback((requestId: string) => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow) {
      getTransport()
        .call("canvas_submit_snapshot", {
          requestId,
          dataUrl: null,
          error: t("canvas.notReadyError"),
        })
        .catch(() => {})
      return
    }
    iframe.contentWindow.postMessage({ type: "canvas_snapshot", requestId }, "*")
  }, [t])

  const handleEvalRequest = useCallback((requestId: string, code: string) => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow) {
      getTransport()
        .call("canvas_submit_eval_result", {
          requestId,
          result: null,
          error: t("canvas.notReadyError"),
        })
        .catch(() => {})
      return
    }
    iframe.contentWindow.postMessage({ type: "canvas_eval", requestId, code }, "*")
  }, [t])

  // Dynamically adjust window min width when canvas is shown/hidden
  useEffect(() => {
    if (!isTauriMode()) return
    const win = getCurrentWindow()
    if (canvas && !detached) {
      win.setMinSize(new LogicalSize(1280, MAIN_WINDOW_MIN_HEIGHT))
    } else {
      win.setMinSize(new LogicalSize(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT))
    }
  }, [canvas, detached])

  useEffect(() => {
    onOpenChange?.(!!canvas)
  }, [canvas, onOpenChange])

  // Restore canvas when switching sessions: query the session's canvas
  // projects and adopt the most recent one. Backend `canvas_show` events
  // only fire when the model actively creates/shows a canvas — entering
  // a historical session emits no event, so the panel would otherwise
  // stay empty (or stuck on the previous session's canvas).
  useEffect(() => {
    if (detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
    }
    if (!currentSessionId) {
      queueMicrotask(() => setCanvas(null))
      return
    }
    let cancelled = false
    ;(async () => {
      try {
        const projects = await getTransport().call<CanvasProjectView[]>(
          "list_canvas_projects_by_session",
          { sessionId: currentSessionId },
        )
        if (cancelled) return
        if (!projects || projects.length === 0) {
          setCanvas(null)
          return
        }
        setCanvas(toCanvasInfo(projects[0], t("canvas.panelTitle")))
      } catch {
        if (!cancelled) setCanvas(null)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [currentSessionId, t])

  // Listen for canvas events from backend
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    unlisteners.push(
      getTransport().listen("canvas_show", (raw) => {
        try {
          const data = parsePayload<CanvasShowPayload>(raw)
          if (!data) return
          // Drop events from other sessions (e.g. cron / IM / subagent tool
          // calls). Older payloads without sessionId still pass through.
          if (data.sessionId && data.sessionId !== currentSessionIdRef.current) {
            return
          }
          setCanvas(toCanvasInfo(data, t("canvas.panelTitle")))
        } catch {
          /* ignore parse errors */
        }
      }),
    )

    unlisteners.push(
      getTransport().listen("canvas_hide", () => {
        setCanvas(null)
      }),
    )

    // Window-level shortcut used by sibling panels (e.g. BrowserPanel
    // auto-open on `browser:frame`) to enforce the right-side panel mutex.
    // We can't reach into this component from `ChatScreen` directly, so the
    // close request flows over a CustomEvent.
    const onForceClose = () => setCanvas(null)
    window.addEventListener(CLOSE_CANVAS_PANEL_EVENT, onForceClose)
    unlisteners.push(() => window.removeEventListener(CLOSE_CANVAS_PANEL_EVENT, onForceClose))

    unlisteners.push(
      getTransport().listen("canvas_reload", (raw) => {
        try {
          const data = parsePayload<{ projectId: string }>(raw)
          if (!data) return
          // If it's the current canvas, refresh
          setCanvas((prev) => {
            if (prev && prev.projectId === data.projectId) {
              setRefreshKey((k) => k + 1)
              // If in detached window, close and re-open to refresh
              if (detachedWindowRef.current) {
                detachedWindowRef.current.close().catch(() => {})
                detachedWindowRef.current = null
                setDetached(false)
                // Re-trigger detach after a short delay
                setTimeout(() => {
                  // handleDetach will be called by the effect or manually
                }, 100)
              }
            }
            return prev
          })
        } catch {
          /* ignore */
        }
      }),
    )

    unlisteners.push(
      getTransport().listen("canvas_deleted", (raw) => {
        try {
          const data = parsePayload<{ projectId: string }>(raw)
          if (!data) return
          setCanvas((prev) => {
            if (prev && prev.projectId === data.projectId) {
              return null
            }
            return prev
          })
        } catch {
          /* ignore */
        }
      }),
    )

    // Listen for snapshot requests from backend
    unlisteners.push(
      getTransport().listen("canvas_snapshot_request", (raw) => {
        try {
          const data = parsePayload<{ requestId: string }>(raw)
          if (!data) return
          handleSnapshotRequest(data.requestId)
        } catch {
          /* ignore */
        }
      }),
    )

    // Listen for eval requests from backend
    unlisteners.push(
      getTransport().listen("canvas_eval_request", (raw) => {
        try {
          const data = parsePayload<{ requestId: string; code: string }>(raw)
          if (!data) return
          handleEvalRequest(data.requestId, data.code)
        } catch {
          /* ignore */
        }
      }),
    )

    return () => {
      unlisteners.forEach((u) => u())
    }
  }, [handleEvalRequest, handleSnapshotRequest, t])

  // Handle messages from iframe (eval results, snapshot results)
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (!event.data || typeof event.data !== "object") return

      if (event.data.type === "canvas_eval_result") {
        getTransport()
          .call("canvas_submit_eval_result", {
            requestId: event.data.requestId,
            result: event.data.result ?? null,
            error: event.data.error ?? null,
          })
          .catch(() => {})
      }

      if (event.data.type === "canvas_snapshot_result") {
        getTransport()
          .call("canvas_submit_snapshot", {
            requestId: event.data.requestId,
            dataUrl: event.data.dataUrl ?? null,
            error: event.data.error ?? null,
          })
          .catch(() => {})
      }
    }

    window.addEventListener("message", handler)
    return () => window.removeEventListener("message", handler)
  }, [])

  // Clean up detached window when canvas is closed
  useEffect(() => {
    if (!canvas && detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
      queueMicrotask(() => setDetached(false))
    }
  }, [canvas])

  const handleClose = useCallback(() => {
    // Close detached window if exists
    if (detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
    }
    setCanvas(null)
    setMaximized(false)
    setDetached(false)
  }, [])

  const handleDetach = useCallback(async () => {
    if (!canvas?.projectPath) return

    // Detached window is Tauri-only (uses `WebviewWindow`). In HTTP mode
    // the Detach button is hidden, but guard defensively anyway.
    if (!isTauriMode()) return

    const url =
      getTransport().artifactPreviewUrl(canvas.projectId, canvas.projectPath) ?? ""
    if (!url) return

    try {
      // Close existing detached window if any
      if (detachedWindowRef.current) {
        await detachedWindowRef.current.close().catch(() => {})
      }

      const webview = new WebviewWindow("canvas-window", {
        url,
        title: t("canvas.detachedWindowTitle", { title: canvas.title }),
        width: 800,
        height: 600,
        minWidth: 400,
        minHeight: 300,
        acceptFirstMouse: true,
        center: true,
      })

      webview.once("tauri://created", () => {
        detachedWindowRef.current = webview
        setDetached(true)
        setMaximized(false)
      })

      webview.once("tauri://error", () => {
        detachedWindowRef.current = null
        setDetached(false)
      })

      // Listen for window close
      webview.once("tauri://destroyed", () => {
        detachedWindowRef.current = null
        setDetached(false)
      })
    } catch {
      /* ignore creation errors */
    }
  }, [canvas, t])

  const handleRefresh = useCallback(() => {
    setRefreshKey((k) => k + 1)
    // If detached, close and re-detach to refresh
    if (detachedWindowRef.current && canvas?.projectPath) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
      setDetached(false)
      // Re-detach after a tick
      setTimeout(() => handleDetach(), 100)
    }
  }, [canvas, handleDetach])

  const handleReattach = useCallback(() => {
    if (detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
    }
    setDetached(false)
  }, [])

  if (!canvas || !visible) return null

  // When detached, show a compact placeholder panel
  if (detached) {
    return (
      <RightPanelShell
        width={panelWidth}
        onWidthChange={onPanelWidthChange}
        resizeLabel={t("canvas.resizePanel", "Resize canvas panel")}
        reservedMainWidth={reservedMainWidth}
        collapsed={collapsed}
        overlay={overlay}
        contentKey="canvas-detached"
      >
        {/* Title Bar */}
        <div
          className="flex h-11 items-center gap-2 bg-surface-panel/95 px-4 shrink-0"
          data-tauri-drag-region
        >
          <span className="text-sm font-medium truncate flex-1">{canvas.title}</span>
          <div className="flex items-center gap-0.5">
            <IconTip label={t("canvas.reattach")}>
              <button
                onClick={handleReattach}
                className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
              >
                <PanelLeftClose className="h-3.5 w-3.5" />
              </button>
            </IconTip>
            <IconTip label={t("canvas.close")}>
              <button
                onClick={handleClose}
                className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          </div>
        </div>
        <div className="flex-1 flex items-center justify-center p-4">
          <p className="text-xs text-muted-foreground text-center">{t("canvas.popOutActive")}</p>
        </div>
      </RightPanelShell>
    )
  }

  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("canvas.resizePanel", "Resize canvas panel")}
      maximized={maximized}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      overlay={overlay}
      contentKey="canvas"
    >
      {/* Title Bar */}
      <div
        className={cn(
          "flex h-11 items-center gap-2 bg-surface-panel px-4 shrink-0",
          maximized && "h-[72px] items-end pb-2 pt-7",
        )}
        data-tauri-drag-region
      >
        <span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
          {canvas.contentType}
        </span>
        <span className="text-sm font-medium truncate flex-1">{canvas.title}</span>

        <div className="flex items-center gap-0.5">
          <IconTip label={t("canvas.refresh")}>
            <button
              onClick={handleRefresh}
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          </IconTip>

          {/* Detach is desktop-only — spawning a WebviewWindow requires Tauri. */}
          {isTauriMode() && (
            <IconTip label={t("canvas.popOut")}>
              <button
                onClick={handleDetach}
                className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          )}

          <IconTip label={maximized ? t("canvas.minimize") : t("canvas.maximize")}>
            <button
              onClick={() => setMaximized((v) => !v)}
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
            >
              {maximized ? (
                <Minimize2 className="h-3.5 w-3.5" />
              ) : (
                <Maximize2 className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>

          <IconTip label={t("canvas.close")}>
            <button
              onClick={handleClose}
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
      </div>

      {/* iframe preview — no scroll-fade mask: the iframe scrolls internally,
          so a mask on this non-scrolling wrapper would permanently dim the live
          canvas's top/bottom edge. */}
      <div className="min-h-0 min-w-0 flex-1 overflow-hidden bg-white dark:bg-surface-app">
        <ArtifactViewer
          ref={iframeRef}
          artifactId={canvas.projectId}
          projectPath={canvas.projectPath}
          refreshKey={`${canvas.projectId}-${refreshKey}`}
          title={canvas.title}
        />
      </div>
    </RightPanelShell>
  )
}
