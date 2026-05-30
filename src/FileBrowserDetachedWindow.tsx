/**
 * FileBrowserDetachedWindow — root component for the independent file-browser
 * Tauri window. Rendered when `?window=files` is in the URL (see main.tsx).
 * Receives scope / scopeId / rootPath via URL search params.
 *
 * The detached window is read-only for "quote to chat" (there is no chat input
 * here), but full CRUD + git worktree browsing work exactly as in the docked
 * panel via the shared FileBrowserView.
 */

import { useCallback, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { FolderTree, X } from "lucide-react"

import { initLanguageFromConfig } from "@/i18n/i18n"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { FileBrowserView } from "@/components/chat/project/file-browser/FileBrowserView"

export default function FileBrowserDetachedWindow() {
  const { t } = useTranslation()

  const params = new URLSearchParams(window.location.search)
  const scopeParam = params.get("scope")
  const scope: "session" | "project" = scopeParam === "project" ? "project" : "session"
  const scopeId = params.get("scopeId") ?? ""
  const rootPath = params.get("rootPath") || null

  useEffect(() => {
    initLanguageFromConfig()
  }, [])

  const handleClose = useCallback(() => {
    getCurrentWindow().close()
  }, [])

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col bg-background text-foreground">
        {/* Title bar — draggable */}
        <div
          className="flex shrink-0 items-center gap-2 border-b border-border bg-secondary/30 px-3 py-2 pt-8"
          data-tauri-drag-region
        >
          <FolderTree className="h-4 w-4 text-muted-foreground" />
          <span className="flex-1 truncate text-sm font-medium">
            {t("fileBrowser.panelTitle", "Files")}
          </span>
          <IconTip label={t("common.close", "Close")}>
            <button
              type="button"
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
              onClick={handleClose}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>

        <div className="min-h-0 flex-1">
          <FileBrowserView
            scope={scope}
            scopeId={scopeId}
            rootPath={rootPath}
            editable
            layout="split"
            className="h-full"
          />
        </div>
      </div>
    </TooltipProvider>
  )
}
