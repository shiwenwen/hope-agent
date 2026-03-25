import { useState, useEffect, useRef, useCallback } from "react"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  X,
  RefreshCw,
  Maximize2,
  Minimize2,
  History,
  Download,
  Camera,
} from "lucide-react"
import { IconTip } from "@/components/ui/tooltip"

interface CanvasInfo {
  projectId: string
  title: string
  contentType: string
  projectPath?: string
}

export default function CanvasPanel() {
  const { t } = useTranslation()
  const [canvas, setCanvas] = useState<CanvasInfo | null>(null)
  const [maximized, setMaximized] = useState(false)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const [refreshKey, setRefreshKey] = useState(0)

  // Listen for canvas events from backend
  useEffect(() => {
    const unlisteners: UnlistenFn[] = []

    listen<string>("canvas_show", (event) => {
      try {
        const data = JSON.parse(event.payload)
        setCanvas({
          projectId: data.projectId,
          title: data.title || "Canvas",
          contentType: data.contentType || "html",
          projectPath: data.projectPath,
        })
      } catch {
        /* ignore parse errors */
      }
    }).then((u) => unlisteners.push(u))

    listen<string>("canvas_hide", () => {
      setCanvas(null)
    }).then((u) => unlisteners.push(u))

    listen<string>("canvas_reload", (event) => {
      try {
        const data = JSON.parse(event.payload)
        // If it's the current canvas, refresh
        setCanvas((prev) => {
          if (prev && prev.projectId === data.projectId) {
            setRefreshKey((k) => k + 1)
          }
          return prev
        })
      } catch {
        /* ignore */
      }
    }).then((u) => unlisteners.push(u))

    listen<string>("canvas_deleted", (event) => {
      try {
        const data = JSON.parse(event.payload)
        setCanvas((prev) => {
          if (prev && prev.projectId === data.projectId) {
            return null
          }
          return prev
        })
      } catch {
        /* ignore */
      }
    }).then((u) => unlisteners.push(u))

    // Listen for snapshot requests from backend
    listen<string>("canvas_snapshot_request", (event) => {
      try {
        const data = JSON.parse(event.payload)
        handleSnapshotRequest(data.requestId)
      } catch {
        /* ignore */
      }
    }).then((u) => unlisteners.push(u))

    // Listen for eval requests from backend
    listen<string>("canvas_eval_request", (event) => {
      try {
        const data = JSON.parse(event.payload)
        handleEvalRequest(data.requestId, data.code)
      } catch {
        /* ignore */
      }
    }).then((u) => unlisteners.push(u))

    return () => {
      unlisteners.forEach((u) => u())
    }
  }, [])

  // Handle messages from iframe (eval results, snapshot results)
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (!event.data || typeof event.data !== "object") return

      if (event.data.type === "canvas_eval_result") {
        invoke("canvas_submit_eval_result", {
          requestId: event.data.requestId,
          result: event.data.result ?? null,
          error: event.data.error ?? null,
        }).catch(() => {})
      }

      if (event.data.type === "canvas_snapshot_result") {
        invoke("canvas_submit_snapshot", {
          requestId: event.data.requestId,
          dataUrl: event.data.dataUrl ?? null,
          error: event.data.error ?? null,
        }).catch(() => {})
      }
    }

    window.addEventListener("message", handler)
    return () => window.removeEventListener("message", handler)
  }, [])

  const handleSnapshotRequest = useCallback((requestId: string) => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow) {
      invoke("canvas_submit_snapshot", {
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
        invoke("canvas_submit_eval_result", {
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

  const handleClose = useCallback(() => {
    setCanvas(null)
    setMaximized(false)
  }, [])

  const handleRefresh = useCallback(() => {
    setRefreshKey((k) => k + 1)
  }, [])

  if (!canvas) return null

  // Build the asset URL for the iframe via Tauri asset protocol
  const indexPath = canvas.projectPath
    ? `${canvas.projectPath}/index.html`
    : "" // fallback, shouldn't happen
  const iframeSrc = indexPath ? convertFileSrc(indexPath) : ""

  return (
    <div
      className={
        maximized
          ? "fixed inset-0 z-50 flex flex-col bg-background"
          : "flex flex-col border-l border-border min-w-[320px] max-w-[50vw]"
      }
      style={maximized ? undefined : { width: 480 }}
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
  )
}
