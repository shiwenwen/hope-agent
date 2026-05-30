/**
 * Reusable project file browser: a workspace tree plus a read-only preview.
 * Mounted in two places — the project settings Files tab and the right-side
 * chat panel (both `split`). Owns selection + draft state and wires the shared
 * {@link useProjectFs} data layer to the tree and preview.
 *
 * When the working dir is a git repo, a header bar shows the current branch and
 * a worktree switcher: picking a worktree re-roots the browser at that path via
 * the read-only `"path"` scope (no writes), with a "back" affordance.
 */

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronLeft,
  ChevronsDownUp,
  FilePlus,
  FolderPlus,
  FolderTree,
  GitBranch,
  RefreshCw,
  RotateCcw,
} from "lucide-react"

import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import type { GitInfo, WorkspaceEntry } from "@/lib/transport"
import { useProjectFs } from "../hooks/useProjectFs"
import { useTreeExpansion } from "../hooks/useTreeExpansion"
import { useFileBrowserSplit } from "../hooks/useFileBrowserSplit"
import { FileBrowserTree, type DraftNode } from "./FileBrowserTree"
import { FilePreviewPane, type QuotePayload } from "./FilePreviewPane"
import { useDragWidth } from "./useDragWidth"

type PathRoot = { scope: "path"; scopeId: string }

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

  // Worktree-jump override: when set, the browser re-roots at an absolute path
  // via the read-only `"path"` scope.
  const [activeRoot, setActiveRoot] = useState<PathRoot | null>(null)
  // Absolute path of the host working dir (the `isCurrent` worktree while
  // browsing the host scope), so clicking it returns to the writable main view.
  const [mainRootPath, setMainRootPath] = useState<string | null>(null)
  const [selected, setSelected] = useState<WorkspaceEntry | null>(null)
  const [draft, setDraft] = useState<DraftNode | null>(null)
  const [gitInfo, setGitInfo] = useState<GitInfo | null>(null)

  // Reset overrides when the host scope target changes (setState-during-render).
  const hostKey = `${scope}:${scopeId ?? ""}`
  const [trackedHost, setTrackedHost] = useState(hostKey)
  if (hostKey !== trackedHost) {
    setTrackedHost(hostKey)
    setActiveRoot(null)
    setMainRootPath(null)
    setSelected(null)
    setGitInfo(null)
  }

  const activeScope = activeRoot?.scope ?? scope
  const activeScopeId = activeRoot?.scopeId ?? (scopeId ?? "")
  const isWorktreeView = activeRoot !== null
  const effectiveEditable = editable && !isWorktreeView

  const fs = useProjectFs(activeScope, activeScopeId)
  const expansion = useTreeExpansion(activeScope, activeScopeId)
  const [treeWidth, setTreeWidth] = useFileBrowserSplit(activeScope, activeScopeId)
  const onDragDivider = useDragWidth({
    width: treeWidth,
    min: 180,
    max: 560,
    onChange: setTreeWidth,
  })

  // Read-only git context (branch + worktrees) for the active root. The
  // host-change reset above clears stale git info; here we only ever set it
  // from the async fetch (no synchronous setState in the effect body).
  useEffect(() => {
    if (!scopeId) return
    let cancelled = false
    void getTransport()
      .call<GitInfo | null>("project_git_info", { scope: activeScope, scopeId: activeScopeId })
      .then((info) => {
        if (cancelled) return
        setGitInfo(info)
        if (!activeRoot && info) {
          const cur = info.worktrees.find((w) => w.isCurrent)
          if (cur) setMainRootPath(cur.path)
        }
      })
      .catch(() => {
        if (!cancelled) setGitInfo(null)
      })
    return () => {
      cancelled = true
    }
  }, [activeScope, activeScopeId, scopeId, activeRoot])

  const jumpToWorktree = useCallback(
    (path: string) => {
      setSelected(null)
      if (mainRootPath && path === mainRootPath) setActiveRoot(null)
      else setActiveRoot({ scope: "path", scopeId: path })
    },
    [mainRootPath],
  )

  const backToRoot = useCallback(() => {
    setSelected(null)
    setActiveRoot(null)
  }, [])

  const onSelectFile = useCallback((entry: WorkspaceEntry) => setSelected(entry), [])
  const onRefresh = useCallback(() => void fs.refreshDir(""), [fs])

  const toolbar = useMemo(
    () => (
      <div className="flex items-center gap-0.5 border-b px-2 py-1">
        <FolderTree className="mr-1 h-3.5 w-3.5 text-muted-foreground" />
        <span className="mr-auto text-xs font-medium text-muted-foreground">
          {t("fileBrowser.panelTitle", "Files")}
        </span>
        {effectiveEditable ? (
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
    [effectiveEditable, expansion.collapseAll, onRefresh, t],
  )

  const gitBar = gitInfo ? (
    <div className="flex items-center gap-1.5 border-b bg-muted/20 px-2 py-1 text-xs">
      <GitBranch className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate font-medium text-foreground/80">
        {gitInfo.branch ?? t("fileBrowser.gitDetached", "detached")}
      </span>
      {isWorktreeView ? (
        <IconTip label={t("fileBrowser.backToRoot", "Back to working directory")}>
          <button
            type="button"
            className="inline-flex shrink-0 items-center rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
            onClick={backToRoot}
          >
            <RotateCcw className="h-3 w-3" />
          </button>
        </IconTip>
      ) : null}
      {gitInfo.worktrees.length > 1 ? (
        <Select value={isWorktreeView ? activeScopeId : mainRootPath ?? ""} onValueChange={jumpToWorktree}>
          <SelectTrigger className="ml-auto h-6 w-auto gap-1 border-0 bg-transparent px-1.5 py-0 text-xs">
            <SelectValue placeholder={t("fileBrowser.worktrees", "Worktrees")} />
          </SelectTrigger>
          <SelectContent>
            {gitInfo.worktrees.map((wt) => (
              <SelectItem key={wt.path} value={wt.path} className="text-xs">
                <span className="font-medium">{basename(wt.path)}</span>
                <span className="ml-1.5 text-muted-foreground">
                  {wt.branch ?? t("fileBrowser.gitDetached", "detached")}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
    </div>
  ) : null

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
      {gitBar}
      {toolbar}
      <div className="min-h-0 flex-1 overflow-auto">
        <FileBrowserTree
          fs={fs}
          expansion={expansion}
          selectedPath={selected?.relPath ?? null}
          onSelectFile={onSelectFile}
          editable={effectiveEditable}
          draft={draft}
          onDraftChange={setDraft}
        />
      </div>
    </div>
  )

  if (layout === "stacked") {
    // Narrow surface: tree full-width; selecting a file swaps to a full-width
    // preview with a back affordance.
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

  // split: tree left, preview right, with a draggable divider between them.
  return (
    <div className={cn("flex h-full min-h-0", className)}>
      <div className="flex min-w-0 shrink-0 flex-col" style={{ width: treeWidth }}>
        {tree}
      </div>
      <div
        className="group relative w-px shrink-0 cursor-col-resize bg-border"
        onMouseDown={onDragDivider}
        role="separator"
        aria-orientation="vertical"
        aria-label={t("fileBrowser.resizeTree", "Resize file tree")}
      >
        {/* Wider invisible hit area around the 1px divider. */}
        <div className="absolute inset-y-0 -left-1 -right-1" />
        <div className="absolute inset-y-0 -left-px -right-px transition-colors group-hover:bg-primary/40" />
      </div>
      <FilePreviewPane
        fs={fs}
        entry={selected}
        onQuote={onQuote}
        onClose={selected ? () => setSelected(null) : undefined}
        className="min-h-0 min-w-0 flex-1"
      />
    </div>
  )
}
