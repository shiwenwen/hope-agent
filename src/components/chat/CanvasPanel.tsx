import { useState, useEffect, useRef, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { convertFileSrc } from "@tauri-apps/api/core"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { WebviewWindow } from "@tauri-apps/api/webviewWindow"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  X,
  RefreshCw,
  Maximize2,
  Minimize2,
  ExternalLink,
  PanelLeftClose,
} from "lucide-react"
import { IconTip } from "@/components/ui/tooltip"

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
): CanvasInfo {
  return {
    projectId: (src.projectId ?? src.id ?? "") as string,
    title: src.title || "Canvas",
    contentType: src.contentType || "html",
    projectPath: src.projectPath,
  }
}

interface CanvasPanelProps {
  panelWidth?: number
  onPanelWidthChange?: (width: number) => void
  currentSessionId?: string | null
}

export default function CanvasPanel({
  panelWidth = 480,
  onPanelWidthChange,
  currentSessionId = null,
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

  const handleSnapshotRequest = useCallback((requestId: string) => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow) {
      getTransport().call("canvas_submit_snapshot", {
        requestId,
        dataUrl: null,
        error: "Canvas panel is not open or iframe not loaded",
      }).catch(() => {})
      return
    }
    iframe.contentWindow.postMessage(
      { type: "canvas_snapshot", requestId },
      "*",
    )
  }, [])

  const handleEvalRequest = useCallback(
    (requestId: string, code: string) => {
      const iframe = iframeRef.current
      if (!iframe?.contentWindow) {
        getTransport().call("canvas_submit_eval_result", {
          requestId,
          result: null,
          error: "Canvas panel is not open or iframe not loaded",
        }).catch(() => {})
        return
      }
      iframe.contentWindow.postMessage(
        { type: "canvas_eval", requestId, code },
        "*",
      )
    },
    [],
  )

  // Dynamically adjust window min width when canvas is shown/hidden
  useEffect(() => {
    const win = getCurrentWindow()
    if (canvas && !detached) {
      win.setMinSize(new LogicalSize(1280, 480))
    } else {
      win.setMinSize(new LogicalSize(840, 480))
    }
  }, [canvas, detached])

  // Restore canvas when switching sessions: query the session's canvas
  // projects and adopt the most recent one. Backend `canvas_show` events
  // only fire when the model actively creates/shows a canvas — entering
  // a historical session emits no event, so the panel would otherwise
  // stay empty (or stuck on the previous session's canvas).
  useEffect(() => {
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
        setCanvas(toCanvasInfo(projects[0]))
      } catch {
        if (!cancelled) setCanvas(null)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [currentSessionId])

  // Listen for canvas events from backend
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    unlisteners.push(getTransport().listen("canvas_show", (raw) => {
      try {
        const data = parsePayload<CanvasShowPayload>(raw)
        // Drop events from other sessions (e.g. cron / IM / subagent tool
        // calls). Older payloads without sessionId still pass through.
        if (data.sessionId && data.sessionId !== currentSessionIdRef.current) {
          return
        }
        setCanvas(toCanvasInfo(data))
      } catch {
        /* ignore parse errors */
      }
    }))

    unlisteners.push(getTransport().listen("canvas_hide", () => {
      setCanvas(null)
    }))

    unlisteners.push(getTransport().listen("canvas_reload", (raw) => {
      try {
        const data = parsePayload<{ projectId: string }>(raw)
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
    }))

    unlisteners.push(getTransport().listen("canvas_deleted", (raw) => {
      try {
        const data = parsePayload<{ projectId: string }>(raw)
        setCanvas((prev) => {
          if (prev && prev.projectId === data.projectId) {
            return null
          }
          return prev
        })
      } catch {
        /* ignore */
      }
    }))

    // Listen for snapshot requests from backend
    unlisteners.push(getTransport().listen("canvas_snapshot_request", (raw) => {
      try {
        const data = parsePayload<{ requestId: string }>(raw)
        handleSnapshotRequest(data.requestId)
      } catch {
        /* ignore */
      }
    }))

    // Listen for eval requests from backend
    unlisteners.push(getTransport().listen("canvas_eval_request", (raw) => {
      try {
        const data = parsePayload<{ requestId: string; code: string }>(raw)
        handleEvalRequest(data.requestId, data.code)
      } catch {
        /* ignore */
      }
    }))

    return () => {
      unlisteners.forEach((u) => u())
    }
  }, [handleEvalRequest, handleSnapshotRequest])

  // Handle messages from iframe (eval results, snapshot results)
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (!event.data || typeof event.data !== "object") return

      if (event.data.type === "canvas_eval_result") {
        getTransport().call("canvas_submit_eval_result", {
          requestId: event.data.requestId,
          result: event.data.result ?? null,
          error: event.data.error ?? null,
        }).catch(() => {})
      }

      if (event.data.type === "canvas_snapshot_result") {
        getTransport().call("canvas_submit_snapshot", {
          requestId: event.data.requestId,
          dataUrl: event.data.dataUrl ?? null,
          error: event.data.error ?? null,
        }).catch(() => {})
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

    const url = convertFileSrc(`${canvas.projectPath}/index.html`)

    try {
      // Close existing detached window if any
      if (detachedWindowRef.current) {
        await detachedWindowRef.current.close().catch(() => {})
      }

      const webview = new WebviewWindow("canvas-window", {
        url,
        title: `Canvas: ${canvas.title}`,
        width: 800,
        height: 600,
        minWidth: 400,
        minHeight: 300,
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
  }, [canvas])

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

  const handlePanelDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startWidth = panelWidth
    const onMouseMove = (ev: MouseEvent) => {
      const newWidth = Math.min(960, Math.max(320, startWidth - (ev.clientX - startX)))
      onPanelWidthChange?.(newWidth)
    }
    const iframes = document.querySelectorAll("iframe")
    iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = "none"))
    const onMouseUp = () => {
      document.removeEventListener("mousemove", onMouseMove)
      document.removeEventListener("mouseup", onMouseUp)
      document.body.style.cursor = ""
      document.body.style.userSelect = ""
      iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = ""))
    }
    document.addEventListener("mousemove", onMouseMove)
    document.addEventListener("mouseup", onMouseUp)
    document.body.style.cursor = "col-resize"
    document.body.style.userSelect = "none"
  }, [panelWidth, onPanelWidthChange])

  if (!canvas) return null

  // Build the asset URL for the iframe via Tauri asset protocol
  const indexPath = canvas.projectPath
    ? `${canvas.projectPath}/index.html`
    : "" // fallback, shouldn't happen
  const iframeSrc = indexPath ? convertFileSrc(indexPath) : ""

  // When detached, show a compact placeholder panel
  if (detached) {
    return (
      <>
      <div
        className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors"
        onMouseDown={handlePanelDragStart}
      />
      <div className="flex flex-col w-[200px] shrink-0">
        {/* Title Bar */}
        <div
          className="flex items-center gap-2 px-3 py-2 border-b border-border bg-secondary/30 shrink-0"
          data-tauri-drag-region
        >
          <span className="text-sm font-medium truncate flex-1">
            {canvas.title}
          </span>
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
          <p className="text-xs text-muted-foreground text-center">
            {t("canvas.popOutActive")}
          </p>
        </div>
      </div>
      </>
    )
  }

  return (
    <>
    <div
      className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors"
      onMouseDown={handlePanelDragStart}
    />
    <div
      className={
        maximized
          ? "fixed inset-0 z-50 flex flex-col bg-background"
          : "flex flex-col shrink-0 max-w-[50vw]"
      }
      style={maximized ? undefined : { width: panelWidth }}
    >
      {/* Title Bar */}
      <div
        className={cn(
          "flex items-center gap-2 px-3 py-2 border-b border-border bg-secondary/30 shrink-0",
          maximized && "pt-8"
        )}
        data-tauri-drag-region
      >
        <span className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
          {canvas.contentType}
        </span>
        <span className="text-sm font-medium truncate flex-1">
          {canvas.title}
        </span>

        <div className="flex items-center gap-0.5">
          <IconTip label={t("canvas.refresh")}>
            <button
              onClick={handleRefresh}
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          </IconTip>

          <IconTip label={t("canvas.popOut")}>
            <button
              onClick={handleDetach}
              className="p-1 rounded hover:bg-secondary transition-colors text-muted-foreground hover:text-foreground"
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </button>
          </IconTip>

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

      {/* iframe preview */}
      <div className="flex-1 overflow-hidden bg-white dark:bg-zinc-900">
        <iframe
          ref={iframeRef}
          key={`${canvas.projectId}-${refreshKey}`}
          src={iframeSrc}
          sandbox="allow-scripts"
          className="w-full h-full border-0"
          title={canvas.title}
        />
      </div>
    </div>
    </>
  )
}
