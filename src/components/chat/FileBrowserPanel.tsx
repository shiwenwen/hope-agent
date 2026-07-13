/**
 * Right-side file browser panel. Mounted permanently (like CanvasPanel) and
 * toggled with the `visible` prop rather than conditionally rendered, so the
 * detached-window state survives panel switches / collapses. Provides:
 *   - a "pop out to a separate window" affordance (desktop only),
 *   - maximize / restore,
 *   - and a compact placeholder while detached.
 *
 * Switching sessions resets transient state (detached / maximized) and closes
 * any stale detached window, so a popped-out window never keeps showing — or
 * mutating — the previous session's files.
 *
 * The detached window is a Tauri WebviewWindow loading
 * `index.html?window=files&scope=&scopeId=&rootPath=` — see
 * `src/FileBrowserDetachedWindow.tsx` and the router in `src/main.tsx`.
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { WebviewWindow } from "@tauri-apps/api/webviewWindow"
import {
  ChevronRight,
  ExternalLink,
  FolderOpen,
  Maximize2,
  Minimize2,
  PanelLeftClose,
} from "lucide-react"

import { IconTip } from "@/components/ui/tooltip"
import { isTauriMode } from "@/lib/transport"
import { cn } from "@/lib/utils"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { FileBrowserView } from "./project/file-browser/FileBrowserView"
import type { QuotePayload } from "./project/file-browser/FilePreviewPane"

interface FileBrowserPanelProps {
  scope: "session" | "project"
  scopeId: string | null
  rootPath: string | null
  /** Used to disambiguate / title the detached window. */
  sessionId?: string | null
  /** Whether this panel is the active right-side panel. Hidden (but kept
   *  mounted) when false, so detached state survives panel switches. */
  visible: boolean
  collapsed?: boolean
  overlay?: boolean
  animateOnMount?: boolean
  panelWidth: number
  onPanelWidthChange: (w: number) => void
  reservedMainWidth?: number
  onQuote?: (payload: QuotePayload) => void
  /** A click on a quote chip in the composer: reveal + select this file and
   *  highlight the quoted line range. */
  revealFile?: {
    path: string
    name: string
    startLine: number
    endLine: number
    nonce: number
  } | null
  onClose: () => void
}

export function FileBrowserPanel({
  scope,
  scopeId,
  rootPath,
  sessionId,
  visible,
  collapsed = false,
  overlay = false,
  animateOnMount = false,
  panelWidth,
  onPanelWidthChange,
  reservedMainWidth,
  onQuote,
  revealFile,
  onClose,
}: FileBrowserPanelProps) {
  const { t } = useTranslation()
  const desktopMode = isTauriMode()
  const [detached, setDetached] = useState(false)
  const [maximized, setMaximized] = useState(false)
  const detachedWindowRef = useRef<WebviewWindow | null>(null)

  // Reset transient UI state on session change via render-phase prev-prop
  // tracking (doing it in an effect would trip react-hooks/set-state-in-effect).
  const [prevScopeId, setPrevScopeId] = useState<string | null>(scopeId)
  if (prevScopeId !== scopeId) {
    setPrevScopeId(scopeId)
    setDetached(false)
    setMaximized(false)
  }

  // Close a stale detached window when the session changes (it was frozen on
  // the previous session's scope) and on unmount.
  useEffect(() => {
    if (detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
    }
  }, [scopeId])

  useEffect(() => {
    return () => {
      if (detachedWindowRef.current) {
        detachedWindowRef.current.close().catch(() => {})
        detachedWindowRef.current = null
      }
    }
  }, [])

  const handleDetach = useCallback(async () => {
    if (!desktopMode || !scopeId) return
    try {
      if (detachedWindowRef.current) {
        await detachedWindowRef.current.close().catch(() => {})
        detachedWindowRef.current = null
      }
      const params = new URLSearchParams({ window: "files", scope, scopeId })
      if (rootPath) params.set("rootPath", rootPath)
      if (sessionId) params.set("sessionId", sessionId)
      const webview = new WebviewWindow("files-window", {
        url: `index.html?${params.toString()}`,
        title: t("fileBrowser.panelTitle", "Files"),
        width: 900,
        height: 680,
        minWidth: 480,
        minHeight: 360,
        acceptFirstMouse: true,
        center: true,
      })
      // Compare against `webview` so a previous window's late destroyed/error
      // callback can't clobber the new window's state.
      webview.once("tauri://created", () => {
        detachedWindowRef.current = webview
        setDetached(true)
        setMaximized(false)
      })
      webview.once("tauri://error", () => {
        if (detachedWindowRef.current === webview) detachedWindowRef.current = null
        setDetached(false)
      })
      webview.once("tauri://destroyed", () => {
        if (detachedWindowRef.current === webview) {
          detachedWindowRef.current = null
          setDetached(false)
        }
      })
    } catch {
      /* ignore window creation errors */
    }
  }, [desktopMode, scope, scopeId, rootPath, sessionId, t])

  const handleReattach = useCallback(() => {
    if (detachedWindowRef.current) {
      detachedWindowRef.current.close().catch(() => {})
      detachedWindowRef.current = null
    }
    setDetached(false)
  }, [])

  // Kept mounted but not rendered when another panel is active, so `detached`
  // and the window handle survive panel switches (mirrors CanvasPanel).
  if (!visible) return null

  const titleBar = (
    // When maximized the panel covers the whole window (fixed inset-0), so pad
    // the top to clear the macOS overlay traffic lights — mirrors CanvasPanel.
    <div className={cn("flex items-center gap-1 border-b px-2 py-1", maximized && "pt-7")}>
      <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="text-xs font-medium text-muted-foreground">
        {t("fileBrowser.panelTitle", "Files")}
      </span>
      <div className="ml-auto flex items-center gap-0.5">
        {detached ? (
          <IconTip label={t("fileBrowser.reattach", "Reattach")}>
            <button
              type="button"
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              onClick={handleReattach}
            >
              <PanelLeftClose className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        ) : desktopMode ? (
          <>
            <IconTip label={t("fileBrowser.openInWindow", "Open in a separate window")}>
              <button
                type="button"
                className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                onClick={handleDetach}
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </button>
            </IconTip>
            <IconTip label={maximized ? t("fileBrowser.minimize", "Restore") : t("fileBrowser.maximize", "Maximize")}>
              <button
                type="button"
                className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                onClick={() => setMaximized((v) => !v)}
              >
                {maximized ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}
              </button>
            </IconTip>
          </>
        ) : null}
        <IconTip label={t("common.close", "Close")}>
          <button
            type="button"
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={() => {
              if (detached) handleReattach()
              setMaximized(false)
              onClose()
            }}
          >
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        </IconTip>
      </div>
    </div>
  )

  const body = detached ? (
    <div className="flex h-full flex-col">
      {titleBar}
      <div className="flex flex-1 items-center justify-center p-4">
        <p className="text-center text-xs text-muted-foreground">
          {t("fileBrowser.popOutActive", "Opened in a separate window")}
        </p>
      </div>
    </div>
  ) : (
    <div className="flex h-full flex-col">
      {titleBar}
      <FileBrowserView
        scope={scope}
        scopeId={scopeId}
        rootPath={rootPath}
        editable
        layout="split"
        onQuote={onQuote}
        revealFile={revealFile}
        className="min-h-0 flex-1"
      />
    </div>
  )

  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("fileBrowser.resizePanel", "Resize files panel")}
      maxWidth={1000}
      maximized={maximized}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      overlay={overlay}
      animateOnMount={animateOnMount}
      contentKey={detached ? "files-detached" : "files"}
    >
      {body}
    </RightPanelShell>
  )
}
