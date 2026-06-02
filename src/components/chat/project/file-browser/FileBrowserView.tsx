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
  FolderOpen,
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
import { projectFsPreviewSource } from "@/components/chat/files/previewSource"
import { useDragWidth } from "./useDragWidth"

// The read-only worktree-jump scope encodes its id as a triple
// `base_scope ∣ base_scope_id ∣ target_abs` (U+001F separator) that the backend
// (`WorkspaceScope::for_path`) validates against the base repo's worktree list,
// so the browser can only jump between the current repo's own worktrees — never
// to an arbitrary git repo on the host.
const PATH_SCOPE_SEP = String.fromCharCode(0x1f)
const encodePathScope = (baseScope: string, baseScopeId: string, target: string) =>
  `${baseScope}${PATH_SCOPE_SEP}${baseScopeId}${PATH_SCOPE_SEP}${target}`

export interface FileBrowserViewProps {
  scope: "session" | "project"
  scopeId: string | null
  /** The effective working dir; `null` renders the "no working directory" state. */
  rootPath: string | null
  editable?: boolean
  layout?: "split" | "stacked"
  onQuote?: (payload: QuotePayload) => void
  /** Reveal + select this file and highlight the quoted line range (from a
   *  composer quote-chip click). The nonce re-triggers even for the same path. */
  revealFile?: {
    path: string
    name: string
    startLine: number
    endLine: number
    nonce: number
  } | null
  className?: string
}

export function FileBrowserView({
  scope,
  scopeId,
  rootPath,
  editable = false,
  layout = "split",
  onQuote,
  revealFile,
  className,
}: FileBrowserViewProps) {
  const { t } = useTranslation()

  // Worktree-jump override: the absolute path of the worktree the browser is
  // re-rooted at (read-only `"path"` scope), or null while viewing the host scope.
  const [activeWorktree, setActiveWorktree] = useState<string | null>(null)
  // Absolute path of the host working dir (the `isCurrent` worktree while
  // browsing the host scope), so clicking it returns to the writable main view.
  const [mainRootPath, setMainRootPath] = useState<string | null>(null)
  const [selected, setSelected] = useState<WorkspaceEntry | null>(null)
  // Quoted line range to highlight in the preview after a reveal; cleared when
  // the user picks a different file manually.
  const [revealLines, setRevealLines] = useState<{
    start: number
    end: number
    nonce: number
  } | null>(null)
  const [draft, setDraft] = useState<DraftNode | null>(null)
  const [gitInfo, setGitInfo] = useState<GitInfo | null>(null)

  // Reset overrides when the host scope target changes (setState-during-render).
  const hostKey = `${scope}:${scopeId ?? ""}`
  const [trackedHost, setTrackedHost] = useState(hostKey)
  if (hostKey !== trackedHost) {
    setTrackedHost(hostKey)
    setActiveWorktree(null)
    setMainRootPath(null)
    setSelected(null)
    setRevealLines(null)
    setGitInfo(null)
  }

  const isWorktreeView = activeWorktree !== null
  const activeScope: "session" | "project" | "path" = activeWorktree !== null ? "path" : scope
  const activeScopeId =
    activeWorktree !== null ? encodePathScope(scope, scopeId ?? "", activeWorktree) : scopeId ?? ""
  const effectiveEditable = editable && !isWorktreeView

  const fs = useProjectFs(activeScope, activeScopeId)
  // Memoized so FilePreviewPane's load effect only re-runs when the selected
  // file (or fs) actually changes, not on every unrelated render.
  const previewSource = useMemo(
    () => (selected ? projectFsPreviewSource(fs, selected) : null),
    [fs, selected],
  )
  const expansion = useTreeExpansion(activeScope, activeScopeId)
  const [treeWidth, setTreeWidth] = useFileBrowserSplit(activeScope, activeScopeId)
  const onDragDivider = useDragWidth({
    width: treeWidth,
    min: 180,
    max: 560,
    onChange: setTreeWidth,
  })

  // Reveal a file requested from a composer quote chip: return to the host scope
  // (quotes reference host-scope files) and select it. The tree expands the
  // ancestor chain + scrolls the row into view in response to `selectedPath`
  // (see FileBrowserTree), so this stays pure render-phase state — no expansion
  // side effects and no writes to the wrong (worktree) expansion scope. The null
  // sentinel makes the FIRST mount fire: the panel mounts fresh on the very
  // click that opens it, so seeding from the nonce would no-op the reveal.
  const [trackedRevealNonce, setTrackedRevealNonce] = useState<number | null>(null)
  if (revealFile && revealFile.nonce !== trackedRevealNonce) {
    setTrackedRevealNonce(revealFile.nonce)
    setActiveWorktree(null)
    setSelected({
      name: revealFile.name,
      relPath: revealFile.path,
      isDir: false,
      isSymlink: false,
      size: null,
      modifiedMs: null,
    })
    setRevealLines({
      start: revealFile.startLine,
      end: revealFile.endLine,
      nonce: revealFile.nonce,
    })
  }
  // revealFile cleared (e.g. the quote chip was removed) → drop the highlight.
  if (!revealFile && revealLines) {
    setRevealLines(null)
  }

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
        if (!activeWorktree && info) {
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
  }, [activeScope, activeScopeId, scopeId, activeWorktree])

  const jumpToWorktree = useCallback(
    (path: string) => {
      setSelected(null)
      if (activeWorktree !== null) {
        // Already in a worktree view: clicking the host's current worktree
        // returns to the writable main view; otherwise switch worktrees.
        setActiveWorktree(path === mainRootPath ? null : path)
      } else {
        // Host view: gitInfo currently describes the host repo, so derive the
        // host's current worktree synchronously (the async mainRootPath effect
        // may still be null) and cache it before jumping — otherwise picking the
        // current worktree would wrongly re-root the host dir via the read-only
        // path scope.
        const hostCurrent = gitInfo?.worktrees.find((w) => w.isCurrent)?.path ?? rootPath ?? null
        setMainRootPath(hostCurrent)
        setActiveWorktree(path === hostCurrent ? null : path)
      }
    },
    [activeWorktree, mainRootPath, gitInfo, rootPath],
  )

  const backToRoot = useCallback(() => {
    setSelected(null)
    setActiveWorktree(null)
  }, [])

  const onSelectFile = useCallback((entry: WorkspaceEntry) => {
    setSelected(entry)
    setRevealLines(null) // manual pick: drop any carried-over reveal highlight
  }, [])
  const onRefresh = useCallback(() => void fs.refreshDir(""), [fs])

  const toolbar = useMemo(
    () => (
      <div className="flex items-center gap-0.5 border-b px-2 py-1">
        <FolderOpen className="mr-1 h-3.5 w-3.5 text-muted-foreground" />
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
        <Select
          value={(isWorktreeView ? activeWorktree : mainRootPath) ?? ""}
          onValueChange={jumpToWorktree}
        >
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
          <FilePreviewPane
            source={previewSource}
            onQuote={onQuote}
            highlightLines={revealLines}
            className="min-h-0 flex-1"
          />
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
        source={previewSource}
        onQuote={onQuote}
        highlightLines={revealLines}
        onClose={selected ? () => setSelected(null) : undefined}
        className="min-h-0 min-w-0 flex-1"
      />
    </div>
  )
}
