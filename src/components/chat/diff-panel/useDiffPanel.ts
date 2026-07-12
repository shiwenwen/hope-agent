import { useCallback, useState } from "react"
import type {
  GitPullRequestReviewComment,
  SessionGitDiffScope,
  SessionGitDiffSnapshot,
} from "@/lib/transport"
import type { FileChangeMetadata, FileChangesMetadata } from "@/types/chat"

export interface GitDiffContext {
  sessionId: string
  scope: SessionGitDiffScope
  revision: string
  reviewComments: GitPullRequestReviewComment[]
}

/**
 * Local state for the right-side diff panel. Mirrors the PlanPanel /
 * CanvasPanel pattern: visibility + width + active payload.
 *
 * `activeChanges` is always normalized to a non-empty array regardless of
 * whether the source was a single `file_change` or a multi-file
 * `file_changes` payload, so the panel renderer has one shape to handle.
 */
export interface UseDiffPanel {
  showPanel: boolean
  activeChanges: FileChangeMetadata[]
  /**
   * Index of the currently selected change within `activeChanges`. Always 0
   * for single-file payloads.
   */
  activeIndex: number
  setActiveIndex: (index: number) => void
  /** Bumps on every `openDiff` — lets ChatScreen claim the active panel even
   *  when the diff panel is already visible. See `UseFilePreview.openNonce`. */
  openNonce: number
  openDiff: (payload: FileChangeMetadata | FileChangesMetadata) => void
  gitContext: GitDiffContext | null
  openGitDiff: (
    snapshot: SessionGitDiffSnapshot,
    sessionId: string,
    reviewComments?: GitPullRequestReviewComment[],
  ) => void
  replaceGitDiff: (snapshot: SessionGitDiffSnapshot) => void
  closeDiff: () => void
  panelWidth: number
  setPanelWidth: (width: number) => void
}

const DEFAULT_DIFF_PANEL_WIDTH = 560

export function useDiffPanel(): UseDiffPanel {
  const [showPanel, setShowPanel] = useState(false)
  const [activeChanges, setActiveChanges] = useState<FileChangeMetadata[]>([])
  const [activeIndex, setActiveIndex] = useState(0)
  const [panelWidth, setPanelWidth] = useState(DEFAULT_DIFF_PANEL_WIDTH)
  const [openNonce, setOpenNonce] = useState(0)
  const [gitContext, setGitContext] = useState<GitDiffContext | null>(null)

  const openDiff = useCallback((payload: FileChangeMetadata | FileChangesMetadata) => {
    setGitContext(null)
    if (payload.kind === "file_change") {
      setActiveChanges([payload])
    } else {
      setActiveChanges(payload.changes.length ? payload.changes : [])
    }
    setActiveIndex(0)
    setShowPanel(true)
    setOpenNonce((n) => n + 1)
  }, [])

  const openGitDiff = useCallback((
    snapshot: SessionGitDiffSnapshot,
    sessionId: string,
    reviewComments: GitPullRequestReviewComment[] = [],
  ) => {
    setActiveChanges(snapshot.changes)
    setActiveIndex(0)
    setGitContext({ sessionId, scope: snapshot.scope, revision: snapshot.revision, reviewComments })
    setShowPanel(true)
    setOpenNonce((n) => n + 1)
  }, [])

  const replaceGitDiff = useCallback((snapshot: SessionGitDiffSnapshot) => {
    setActiveChanges(snapshot.changes)
    setActiveIndex((index) => Math.min(index, Math.max(0, snapshot.changes.length - 1)))
    setGitContext((current) =>
      current
        ? { ...current, scope: snapshot.scope, revision: snapshot.revision }
        : current,
    )
  }, [])

  const closeDiff = useCallback(() => {
    setShowPanel(false)
    setActiveChanges([])
    setActiveIndex(0)
    setGitContext(null)
  }, [])

  return {
    showPanel,
    activeChanges,
    activeIndex,
    setActiveIndex,
    openNonce,
    openDiff,
    gitContext,
    openGitDiff,
    replaceGitDiff,
    closeDiff,
    panelWidth,
    setPanelWidth,
  }
}
