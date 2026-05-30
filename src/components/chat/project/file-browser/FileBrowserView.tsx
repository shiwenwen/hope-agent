/**
 * Reusable project file browser: a workspace tree plus a read-only preview.
 * Mounted in two places — the project settings Files tab (`stacked`) and the
 * right-side chat panel (`split`). Owns selection + draft state and wires the
 * shared {@link useProjectFs} data layer to the tree and preview.
 */

import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronLeft, FilePlus, FolderPlus, FolderTree, RefreshCw, ChevronsDownUp } from "lucide-react"

import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import type { WorkspaceEntry } from "@/lib/transport"
import { useProjectFs } from "../hooks/useProjectFs"
import { useTreeExpansion } from "../hooks/useTreeExpansion"
import { FileBrowserTree, type DraftNode } from "./FileBrowserTree"
import { FilePreviewPane, type QuotePayload } from "./FilePreviewPane"

export interface FileBrowserViewProps {
  scope: "session" | "project"
  scopeId: string | null
  /** The effective working dir; `null` renders the "no working directory" state. */
  rootPath: string | null
  editable?: boolean
  layout?: "split" | "stacked"
  onQuote?: (payload: QuotePayload) => void
  className?: string
}

export function FileBrowserView({
  scope,
  scopeId,
  rootPath,
  editable = false,
  layout = "split",
  onQuote,
  className,
}: FileBrowserViewProps) {
  const { t } = useTranslation()
  const fs = useProjectFs(scope, scopeId)
  const expansion = useTreeExpansion(scope, scopeId ?? "")
  const [selected, setSelected] = useState<WorkspaceEntry | null>(null)
  const [draft, setDraft] = useState<DraftNode | null>(null)

  const onSelectFile = useCallback((entry: WorkspaceEntry) => setSelected(entry), [])
  const onRefresh = useCallback(() => void fs.refreshDir(""), [fs])

  const toolbar = useMemo(
    () => (
      <div className="flex items-center gap-0.5 border-b px-2 py-1">
        <FolderTree className="mr-1 h-3.5 w-3.5 text-muted-foreground" />
        <span className="mr-auto text-xs font-medium text-muted-foreground">
          {t("fileBrowser.panelTitle", "Files")}
        </span>
        {editable ? (
          <>
            <IconTip label={t("fileBrowser.newFile", "New File")}>
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={() => setDraft({ dir: "", isDir: false })}
              >
                <FilePlus className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
            <IconTip label={t("fileBrowser.newFolder", "New Folder")}>
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={() => setDraft({ dir: "", isDir: true })}
              >
                <FolderPlus className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          </>
        ) : null}
        <IconTip label={t("fileBrowser.collapseAll", "Collapse all")}>
          <Button size="icon" variant="ghost" className="h-6 w-6" onClick={expansion.collapseAll}>
            <ChevronsDownUp className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
        <IconTip label={t("fileBrowser.refresh", "Refresh")}>
          <Button size="icon" variant="ghost" className="h-6 w-6" onClick={onRefresh}>
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      </div>
    ),
    [editable, expansion.collapseAll, onRefresh, t],
  )

  if (!scopeId || !rootPath) {
    return (
      <div className={cn("flex h-full items-center justify-center px-6 text-center", className)}>
        <span className="text-sm text-muted-foreground">
          {t("fileBrowser.noWorkingDir", "Set a working directory to browse files")}
        </span>
      </div>
    )
  }

  const tree = (
    <div className="flex min-h-0 flex-1 flex-col">
      {toolbar}
      <div className="min-h-0 flex-1 overflow-auto">
        <FileBrowserTree
          fs={fs}
          expansion={expansion}
          selectedPath={selected?.relPath ?? null}
          onSelectFile={onSelectFile}
          editable={editable}
          draft={draft}
          onDraftChange={setDraft}
        />
      </div>
    </div>
  )

  if (layout === "stacked") {
    // Narrow surface (Files tab): tree full-width; selecting a file swaps to a
    // full-width preview with a back affordance.
    if (selected) {
      return (
        <div className={cn("flex h-full flex-col", className)}>
          <div className="flex items-center gap-1 border-b px-2 py-1">
            <Button size="sm" variant="ghost" className="h-6 gap-1 px-2" onClick={() => setSelected(null)}>
              <ChevronLeft className="h-3.5 w-3.5" />
              {t("common.back", "Back")}
            </Button>
          </div>
          <FilePreviewPane fs={fs} entry={selected} onQuote={onQuote} className="min-h-0 flex-1" />
        </div>
      )
    }
    return <div className={cn("flex h-full flex-col", className)}>{tree}</div>
  }

  // split: tree left, preview right.
  return (
    <div className={cn("flex h-full min-h-0", className)}>
      <div className="flex w-2/5 min-w-[200px] max-w-[420px] flex-col border-r">{tree}</div>
      <FilePreviewPane
        fs={fs}
        entry={selected}
        onQuote={onQuote}
        onClose={selected ? () => setSelected(null) : undefined}
        className="min-h-0 flex-1"
      />
    </div>
  )
}
