import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Columns2,
  Copy,
  ExternalLink,
  FoldVertical,
  GitCompare,
  MessageCircle,
  Pilcrow,
  RefreshCw,
  Rows3,
  Search,
  Trash2,
  Undo2,
  X,
} from "lucide-react"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"
import { openExternalUrl } from "@/lib/openExternalUrl"
import type {
  GitFileChange,
  GitMutationTarget,
  SessionGitDiffScope,
  SessionGitDiffSnapshot,
} from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { PANEL_SCROLL_FADE } from "../right-panel/panelFade"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import { IconTip } from "@/components/ui/tooltip"
import type { FileChangeMetadata } from "@/types/chat"
import type { PreviewTarget } from "../files/useFilePreview"
import type { GitDiffContext } from "./useDiffPanel"
import { FileDeltaCounter } from "@/components/chat/message/FileDeltaCounter"
import { UnifiedDiffView } from "./UnifiedDiffView"
import { SplitDiffView } from "./SplitDiffView"
import {
  buildSplitRows,
  buildUnifiedRows,
  buildVisibleRowItems,
  hunkCountForRows,
  isSplitRowChanged,
  isUnifiedRowChanged,
  type DiffViewItem,
  type SplitRow,
  type UnifiedRow,
} from "./diffLayout"

const LAYOUT_STORAGE_KEY = "ha:diff-panel:layout"
const FIRST_DIFF_SCROLL_PADDING_PX = 8
const CONTEXT_LINES = 4
const MAX_RENDERED_DIFF_ITEMS = 1800
const EMPTY_FOLD_IDS = new Set<string>()
type DiffLayout = "unified" | "split"
type DiffLineSide = "old" | "new"
interface DiffLocation {
  line: number
  side: DiffLineSide
}
interface KeyedFoldState {
  key: string
  ids: Set<string>
}
interface KeyedBooleanState {
  key: string
  value: boolean
}
interface KeyedNumberState {
  key: string
  value: number
}

function readStoredLayout(): DiffLayout {
  if (typeof window === "undefined") return "unified"
  const value = window.localStorage.getItem(LAYOUT_STORAGE_KEY)
  return value === "split" ? "split" : "unified"
}

function persistLayout(value: DiffLayout) {
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, value)
  } catch {
    // Storage access failures are non-fatal.
  }
}

interface DiffPanelProps {
  changes: FileChangeMetadata[]
  activeIndex: number
  openNonce: number
  onActiveIndexChange: (index: number) => void
  onClose: () => void
  onPreviewFile?: (target: PreviewTarget) => void
  gitContext?: GitDiffContext | null
  onGitSnapshotChange?: (snapshot: SessionGitDiffSnapshot) => void
  embedded?: boolean
}

/**
 * Right-side diff panel mirroring the PlanPanel embedded mode. Renders one or
 * more file changes coming from a single tool call (write / edit /
 * apply_patch). Selecting a tab switches the rendered file; the layout
 * toggle remembers the user's choice in localStorage.
 */
export function DiffPanel({
  changes,
  activeIndex,
  openNonce,
  onActiveIndexChange,
  onClose,
  onPreviewFile,
  gitContext = null,
  onGitSnapshotChange,
  embedded = false,
}: DiffPanelProps) {
  const { t } = useTranslation()
  const [layout, setLayout] = useState<DiffLayout>(() => readStoredLayout())
  const [collapseContext, setCollapseContext] = useState(true)
  const [ignoreWhitespace, setIgnoreWhitespace] = useState(false)
  const [fileQuery, setFileQuery] = useState("")
  const [gitBusy, setGitBusy] = useState<string | null>(null)
  const [expandedFoldState, setExpandedFoldState] = useState<KeyedFoldState>(() => ({
    key: "",
    ids: new Set(),
  }))
  const [renderAllRowsState, setRenderAllRowsState] = useState<KeyedBooleanState>(() => ({
    key: "",
    value: false,
  }))
  const [activeHunkState, setActiveHunkState] = useState<KeyedNumberState>(() => ({
    key: "",
    value: 0,
  }))
  const [collapsedFileState, setCollapsedFileState] = useState<KeyedFoldState>(() => ({
    key: "",
    ids: new Set(),
  }))
  const [stackedExpandedFoldState, setStackedExpandedFoldState] = useState<KeyedFoldState>(() => ({
    key: "",
    ids: new Set(),
  }))
  const [stackedRenderAllState, setStackedRenderAllState] = useState<KeyedFoldState>(() => ({
    key: "",
    ids: new Set(),
  }))
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const scrollPositionsRef = useRef<Map<string, number>>(new Map())
  const previousOpenNonceRef = useRef(openNonce)

  useEffect(() => {
    persistLayout(layout)
  }, [layout])

  const safeIndex = Math.min(Math.max(0, activeIndex), Math.max(0, changes.length - 1))
  const stackedMode = changes.length > 1
  const changesKey = useMemo(
    () =>
      changes
        .map(
          (c, idx) =>
            `${idx}:${c.path}:${c.action}:${c.before?.length ?? -1}:${c.after?.length ?? -1}`,
        )
        .join("\u001e"),
    [changes],
  )
  const change = changes[safeIndex]
  const changeKey = change
    ? `${safeIndex}:${change.path}:${change.action}:${change.before?.length ?? -1}:${change.after?.length ?? -1}`
    : "none"
  const scrollKey = `${changeKey}:${layout}:${ignoreWhitespace}:${collapseContext}`
  const resetKey = `${openNonce}:${scrollKey}`
  const stackedFileResetKey = `${openNonce}:stacked-files:${changesKey}`
  const stackedDiffResetKey = `${openNonce}:stacked-diff:${changesKey}:${layout}:${ignoreWhitespace}:${collapseContext}`
  const expandedFoldIds =
    expandedFoldState.key === resetKey ? expandedFoldState.ids : EMPTY_FOLD_IDS
  const renderAllRows =
    renderAllRowsState.key === resetKey ? renderAllRowsState.value : false
  const activeHunkIndex = activeHunkState.key === resetKey ? activeHunkState.value : 0
  const collapsedFileIds =
    collapsedFileState.key === stackedFileResetKey ? collapsedFileState.ids : EMPTY_FOLD_IDS
  const stackedExpandedFoldIds =
    stackedExpandedFoldState.key === stackedDiffResetKey
      ? stackedExpandedFoldState.ids
      : EMPTY_FOLD_IDS
  const stackedRenderAllIds =
    stackedRenderAllState.key === stackedDiffResetKey ? stackedRenderAllState.ids : EMPTY_FOLD_IDS
  const setRenderAllRows = useCallback(
    (value: boolean) => setRenderAllRowsState({ key: resetKey, value }),
    [resetKey],
  )
  const setActiveHunkIndex = useCallback(
    (value: number) => setActiveHunkState({ key: resetKey, value }),
    [resetKey],
  )

  const saveScrollPosition = useCallback(() => {
    const container = scrollRef.current
    if (!container || !change) return
    scrollPositionsRef.current.set(scrollKey, container.scrollTop)
  }, [change, scrollKey])

  const handleActiveIndexChange = useCallback(
    (index: number) => {
      saveScrollPosition()
      onActiveIndexChange(index)
    },
    [onActiveIndexChange, saveScrollPosition],
  )

  const filteredChangeIndexes = useMemo(() => {
    const query = fileQuery.trim().toLowerCase()
    if (!query) return changes.map((_, idx) => idx)
    return changes.flatMap((c, idx) => (c.path.toLowerCase().includes(query) ? [idx] : []))
  }, [changes, fileQuery])

  useEffect(() => {
    if (changes.length <= 1 || filteredChangeIndexes.length === 0) return
    if (!filteredChangeIndexes.includes(safeIndex)) {
      handleActiveIndexChange(filteredChangeIndexes[0])
    }
  }, [changes.length, filteredChangeIndexes, handleActiveIndexChange, safeIndex])

  const activeRows = useMemo(
    () =>
      !change
        ? []
        : layout === "unified"
          ? buildUnifiedRows(change.before ?? "", change.after ?? "", { ignoreWhitespace })
          : buildSplitRows(change.before ?? "", change.after ?? "", { ignoreWhitespace }),
    [change, ignoreWhitespace, layout],
  )
  const activeItems = useMemo(
    () =>
      layout === "unified"
        ? buildVisibleRowItems(activeRows as UnifiedRow[], {
            collapseContext,
            expandedFoldIds,
            isChanged: isUnifiedRowChanged,
            contextLines: CONTEXT_LINES,
          })
        : buildVisibleRowItems(activeRows as SplitRow[], {
            collapseContext,
            expandedFoldIds,
            isChanged: isSplitRowChanged,
            contextLines: CONTEXT_LINES,
          }),
    [activeRows, collapseContext, expandedFoldIds, layout],
  )
  const hunkCount = hunkCountForRows(activeRows)
  const clampedActiveHunkIndex = hunkCount > 0 ? Math.min(activeHunkIndex, hunkCount - 1) : 0
  const displayedItems = renderAllRows ? activeItems : activeItems.slice(0, MAX_RENDERED_DIFF_ITEMS)
  const omittedItemCount = Math.max(0, activeItems.length - displayedItems.length)
  const activeHunkLocation = useMemo<DiffLocation | null>(() => {
    const row = activeRows.find((r) => r.hunkIndex === clampedActiveHunkIndex && r.isHunkStart)
    if (!row) return null
    if (layout === "unified") {
      const unifiedRow = row as UnifiedRow
      if (unifiedRow.newLineNumber) return { line: unifiedRow.newLineNumber, side: "new" }
      if (unifiedRow.oldLineNumber) return { line: unifiedRow.oldLineNumber, side: "old" }
      return null
    }
    const splitRow = row as SplitRow
    if (splitRow.right?.lineNumber) return { line: splitRow.right.lineNumber, side: "new" }
    if (splitRow.left?.lineNumber) return { line: splitRow.left.lineNumber, side: "old" }
    return null
  }, [activeRows, clampedActiveHunkIndex, layout])

  const refreshGitScope = useCallback(
    async (scope: SessionGitDiffScope = gitContext?.scope ?? "unstaged") => {
      if (!gitContext || !onGitSnapshotChange) return
      setGitBusy("refresh")
      try {
        const snapshot = await getTransport().call<SessionGitDiffSnapshot>(
          "load_session_git_diff_snapshot_cmd",
          { sessionId: gitContext.sessionId, scope },
        )
        onGitSnapshotChange(snapshot)
      } catch (error) {
        toast.error(error instanceof Error ? error.message : String(error))
      } finally {
        setGitBusy(null)
      }
    },
    [gitContext, onGitSnapshotChange],
  )

  const runGitMutation = useCallback(
    async (action: "stage" | "unstage" | "discard", target: GitMutationTarget) => {
      if (!gitContext || !onGitSnapshotChange || gitBusy) return
      if (
        action === "discard" &&
        !window.confirm(
          t(
            "diffPanel.git.discardConfirm",
            "确定要永久丢弃所选改动吗？未跟踪文件会被删除，此操作无法撤销。",
          ),
        )
      ) {
        return
      }
      setGitBusy(`${action}:${target.kind}:${target.path ?? target.hunkId ?? "all"}`)
      try {
        const snapshot = await getTransport().call<SessionGitDiffSnapshot>(
          "mutate_session_git_index_cmd",
          {
            sessionId: gitContext.sessionId,
            input: {
              expectedRevision: gitContext.revision,
              action,
              target,
              confirmDiscard: action === "discard",
            },
          },
        )
        onGitSnapshotChange(snapshot)
        toast.success(
          action === "stage"
            ? t("diffPanel.git.staged", "已暂存")
            : action === "unstage"
              ? t("diffPanel.git.unstaged", "已取消暂存")
              : t("diffPanel.git.discarded", "已丢弃改动"),
        )
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        if (message.includes("stale_snapshot")) {
          await refreshGitScope(gitContext.scope)
          toast.info(
            t("diffPanel.git.stale", "仓库已发生变化，已刷新，请重新确认。"),
          )
        } else {
          toast.error(message)
        }
      } finally {
        setGitBusy(null)
      }
    },
    [gitBusy, gitContext, onGitSnapshotChange, refreshGitScope, t],
  )

  const renderGitMutationButtons = useCallback(
    (target: GitMutationTarget, compact = false) => {
      if (!gitContext || gitContext.scope === "all") return null
      const sizeClass = compact ? "h-6 px-1.5 text-[10px]" : "h-7 px-2 text-[11px]"
      return (
        <span className="inline-flex shrink-0 items-center gap-1" onClick={(event) => event.stopPropagation()}>
          {gitContext.scope === "unstaged" ? (
            <>
              <button
                type="button"
                className={cn(gitActionButtonClass, sizeClass)}
                disabled={Boolean(gitBusy)}
                onClick={() => void runGitMutation("stage", target)}
              >
                <GitCompare className="h-3 w-3" />
                {t("diffPanel.git.stage", "暂存")}
              </button>
              <button
                type="button"
                className={cn(gitActionButtonClass, sizeClass, "text-destructive hover:text-destructive")}
                disabled={Boolean(gitBusy)}
                onClick={() => void runGitMutation("discard", target)}
              >
                <Trash2 className="h-3 w-3" />
                {t("diffPanel.git.discard", "丢弃")}
              </button>
            </>
          ) : (
            <button
              type="button"
              className={cn(gitActionButtonClass, sizeClass)}
              disabled={Boolean(gitBusy)}
              onClick={() => void runGitMutation("unstage", target)}
            >
              <Undo2 className="h-3 w-3" />
              {t("diffPanel.git.unstage", "取消暂存")}
            </button>
          )}
        </span>
      )
    },
    [gitBusy, gitContext, runGitMutation, t],
  )

  useEffect(() => {
    if (previousOpenNonceRef.current !== openNonce) {
      previousOpenNonceRef.current = openNonce
      scrollPositionsRef.current.clear()
    }
  }, [openNonce])

  const scrollElementIntoDiffView = useCallback((element: HTMLElement) => {
    const container = scrollRef.current
    if (!container) return
    const containerRect = container.getBoundingClientRect()
    const rowRect = element.getBoundingClientRect()
    const targetTop =
      container.scrollTop +
      rowRect.top -
      containerRect.top -
      FIRST_DIFF_SCROLL_PADDING_PX
    container.scrollTo({ top: Math.max(0, targetTop) })
  }, [])

  const scrollToHunk = useCallback(
    (targetIndex: number, options: { expandRenderLimit?: boolean } = {}) => {
      const container = scrollRef.current
      if (!container || hunkCount <= 0) return
      const normalized = ((targetIndex % hunkCount) + hunkCount) % hunkCount
      const selector = `[data-diff-hunk-start='true'][data-diff-hunk-index='${normalized}']`
      const row = container.querySelector<HTMLElement>(selector)
      if (!row && options.expandRenderLimit !== false && omittedItemCount > 0) {
        setRenderAllRows(true)
        window.requestAnimationFrame(() => {
          window.requestAnimationFrame(() => {
            const nextRow = scrollRef.current?.querySelector<HTMLElement>(selector)
            if (!nextRow) return
            scrollElementIntoDiffView(nextRow)
            setActiveHunkIndex(normalized)
          })
        })
        return
      }
      if (!row) return
      scrollElementIntoDiffView(row)
      setActiveHunkIndex(normalized)
    },
    [hunkCount, omittedItemCount, scrollElementIntoDiffView, setActiveHunkIndex, setRenderAllRows],
  )

  useEffect(() => {
    if (stackedMode) return
    const container = scrollRef.current
    if (!container || !change) return
    const saved = scrollPositionsRef.current.get(scrollKey)
    let cancelled = false
    const frame = window.requestAnimationFrame(() => {
      if (cancelled) return
      if (typeof saved === "number") {
        container.scrollTop = saved
        updateActiveHunkFromScroll(container, setActiveHunkIndex)
      } else {
        scrollToHunk(0, { expandRenderLimit: false })
      }
    })
    return () => {
      cancelled = true
      window.cancelAnimationFrame(frame)
    }
  }, [change, scrollKey, scrollToHunk, setActiveHunkIndex, stackedMode])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (isTypingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) return
      if (e.key === "Escape") {
        e.preventDefault()
        onClose()
        return
      }
      if (!stackedMode && (e.key === "n" || e.key === "j")) {
        e.preventDefault()
        scrollToHunk(clampedActiveHunkIndex + 1)
        return
      }
      if (!stackedMode && (e.key === "p" || e.key === "k")) {
        e.preventDefault()
        scrollToHunk(clampedActiveHunkIndex - 1)
        return
      }
      if (e.key === "u") {
        e.preventDefault()
        setLayout("unified")
        return
      }
      if (e.key === "s") {
        e.preventDefault()
        setLayout("split")
        return
      }
      if (e.key === "c") {
        e.preventDefault()
        setCollapseContext((v) => !v)
        return
      }
      if (e.key === "w") {
        e.preventDefault()
        setIgnoreWhitespace((v) => !v)
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [clampedActiveHunkIndex, onClose, scrollToHunk, stackedMode])

  const handleScroll = useCallback(() => {
    const container = scrollRef.current
    if (!container || !change || stackedMode) return
    scrollPositionsRef.current.set(scrollKey, container.scrollTop)
    updateActiveHunkFromScroll(container, setActiveHunkIndex)
  }, [change, scrollKey, setActiveHunkIndex, stackedMode])

  const toggleFold = useCallback((id: string) => {
    setExpandedFoldState((prev) => {
      const next = new Set(prev.key === resetKey ? prev.ids : EMPTY_FOLD_IDS)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return { key: resetKey, ids: next }
    })
  }, [resetKey])

  const toggleStackedFile = useCallback((id: string) => {
    setCollapsedFileState((prev) => {
      const next = new Set(prev.key === stackedFileResetKey ? prev.ids : EMPTY_FOLD_IDS)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return { key: stackedFileResetKey, ids: next }
    })
  }, [stackedFileResetKey])

  const toggleStackedFold = useCallback((fileKey: string, foldId: string) => {
    const id = `${fileKey}\u001f${foldId}`
    setStackedExpandedFoldState((prev) => {
      const next = new Set(prev.key === stackedDiffResetKey ? prev.ids : EMPTY_FOLD_IDS)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return { key: stackedDiffResetKey, ids: next }
    })
  }, [stackedDiffResetKey])

  const setStackedRenderAll = useCallback((fileKey: string) => {
    setStackedRenderAllState((prev) => {
      const next = new Set(prev.key === stackedDiffResetKey ? prev.ids : EMPTY_FOLD_IDS)
      next.add(fileKey)
      return { key: stackedDiffResetKey, ids: next }
    })
  }, [stackedDiffResetKey])

  const copyLocationForChange = useCallback(
    (target: FileChangeMetadata, line: number) => {
      const location = `${target.path}:${line}`
      navigator.clipboard.writeText(location).then(
        () => toast.success(t("diffPanel.locationCopied", "已复制定位")),
        () => toast.error(t("diffPanel.locationCopyFailed", "复制定位失败")),
      )
    },
    [t],
  )

  const openLocationForChange = useCallback(
    (target: FileChangeMetadata, line: number, side: DiffLineSide = "new") => {
      if (side !== "new" || target.action === "delete" || !onPreviewFile) {
        copyLocationForChange(target, line)
        return
      }
      onPreviewFile({
        kind: "path",
        path: target.path,
        name: basename(target.path),
        language: target.language,
        revealLines: { start: line, end: line, nonce: Date.now() },
      })
    },
    [copyLocationForChange, onPreviewFile],
  )

  const copyLocation = useCallback(
    (line: number) => {
      if (!change) return
      copyLocationForChange(change, line)
    },
    [change, copyLocationForChange],
  )

  const openLocation = useCallback(
    (line: number, side: DiffLineSide = "new") => {
      if (!change) return
      openLocationForChange(change, line, side)
    },
    [change, openLocationForChange],
  )

  const stackedSections = useMemo(() => {
    if (!stackedMode) return []
    return filteredChangeIndexes.map((idx) => {
      const c = changes[idx]
      const fileKey = fileKeyForChange(c, idx)
      const rows =
        layout === "unified"
          ? buildUnifiedRows(c.before ?? "", c.after ?? "", { ignoreWhitespace })
          : buildSplitRows(c.before ?? "", c.after ?? "", { ignoreWhitespace })
      const fileExpandedFoldIds = new Set<string>()
      for (const id of stackedExpandedFoldIds) {
        const prefix = `${fileKey}\u001f`
        if (id.startsWith(prefix)) fileExpandedFoldIds.add(id.slice(prefix.length))
      }
      const items =
        layout === "unified"
          ? buildVisibleRowItems(rows as UnifiedRow[], {
              collapseContext,
              expandedFoldIds: fileExpandedFoldIds,
              isChanged: isUnifiedRowChanged,
              contextLines: CONTEXT_LINES,
            })
          : buildVisibleRowItems(rows as SplitRow[], {
              collapseContext,
              expandedFoldIds: fileExpandedFoldIds,
              isChanged: isSplitRowChanged,
              contextLines: CONTEXT_LINES,
            })
      const renderAll = stackedRenderAllIds.has(fileKey)
      const displayed = renderAll ? items : items.slice(0, MAX_RENDERED_DIFF_ITEMS)
      return {
        change: c,
        fileKey,
        items: displayed,
        omittedItemCount: Math.max(0, items.length - displayed.length),
        collapsed: collapsedFileIds.has(fileKey),
      }
    })
  }, [
    changes,
    collapseContext,
    collapsedFileIds,
    filteredChangeIndexes,
    ignoreWhitespace,
    layout,
    stackedExpandedFoldIds,
    stackedMode,
    stackedRenderAllIds,
  ])

  const wrapperClasses = cn(
    "flex h-full min-h-0 w-full flex-col overflow-hidden",
    embedded
      ? ""
      : "max-w-4xl rounded-panel border border-border-soft bg-surface-panel shadow-panel",
  )
  const activeGitChange = gitContext && change ? (change as GitFileChange) : null
  const activeGitHunk = activeGitChange?.hunks[clampedActiveHunkIndex] ?? null
  const activeGitHunkAllowed = Boolean(
    activeGitChange &&
      activeGitHunk &&
      !activeGitChange.binary &&
      !activeGitChange.submodule &&
      !activeGitChange.conflicted &&
      !activeGitChange.untracked &&
      !activeGitChange.oldPath,
  )

  return (
    <div className={wrapperClasses}>
      <div className="flex items-center gap-1.5 px-3 py-2">
        <GitCompare className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate text-sm font-medium">
          {t("diffPanel.title", "文件改动")}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1">
          {!stackedMode && (
            <>
              <IconTip label={t("diffPanel.prevHunk", "上一处变更")}>
                <button
                  type="button"
                  className={iconButtonClass}
                  onClick={() => scrollToHunk(clampedActiveHunkIndex - 1)}
                  disabled={hunkCount === 0}
                >
                  <ChevronUp className="h-3.5 w-3.5" />
                </button>
              </IconTip>
              <IconTip label={t("diffPanel.nextHunk", "下一处变更")}>
                <button
                  type="button"
                  className={iconButtonClass}
                  onClick={() => scrollToHunk(clampedActiveHunkIndex + 1)}
                  disabled={hunkCount === 0}
                >
                  <ChevronDown className="h-3.5 w-3.5" />
                </button>
              </IconTip>
              <span className="hidden min-w-10 text-center text-[11px] tabular-nums text-muted-foreground sm:inline">
                {hunkCount > 0 ? `${clampedActiveHunkIndex + 1}/${hunkCount}` : "0/0"}
              </span>
            </>
          )}
          <span className="inline-flex items-center gap-1 rounded-md border border-border/60 p-0.5">
            <IconTip label={t("diffPanel.layoutUnified", "Unified")}>
              <button
                type="button"
                className={cn(iconButtonClass, layout === "unified" && activeIconButtonClass)}
                onClick={() => setLayout("unified")}
                aria-pressed={layout === "unified"}
              >
                <Rows3 className="h-3.5 w-3.5" />
              </button>
            </IconTip>
            <IconTip label={t("diffPanel.layoutSplit", "Split")}>
              <button
                type="button"
                className={cn(iconButtonClass, layout === "split" && activeIconButtonClass)}
                onClick={() => setLayout("split")}
                aria-pressed={layout === "split"}
              >
                <Columns2 className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          </span>
          <IconTip label={t("diffPanel.toggleContext", "折叠未变更上下文")}>
            <button
              type="button"
              className={cn(iconButtonClass, collapseContext && activeIconButtonClass)}
              onClick={() => setCollapseContext((v) => !v)}
              aria-pressed={collapseContext}
            >
              <FoldVertical className="h-3.5 w-3.5" />
            </button>
          </IconTip>
          <IconTip label={t("diffPanel.ignoreWhitespace", "忽略空白变更")}>
            <button
              type="button"
              className={cn(iconButtonClass, ignoreWhitespace && activeIconButtonClass)}
              onClick={() => setIgnoreWhitespace((v) => !v)}
              aria-pressed={ignoreWhitespace}
            >
              <Pilcrow className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      {gitContext ? (
        <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-y border-border/60 bg-muted/20 px-2 py-1.5">
          <div className="inline-flex rounded-md border border-border/60 bg-background/60 p-0.5">
            {(["unstaged", "staged", "all"] as SessionGitDiffScope[]).map((scope) => (
              <button
                key={scope}
                type="button"
                className={cn(
                  "h-6 rounded px-2 text-[11px] text-muted-foreground transition-colors hover:text-foreground",
                  gitContext.scope === scope && "bg-secondary text-foreground shadow-sm",
                )}
                disabled={Boolean(gitBusy)}
                onClick={() => void refreshGitScope(scope)}
              >
                {scope === "unstaged"
                  ? t("diffPanel.git.unstagedScope", "未暂存")
                  : scope === "staged"
                    ? t("diffPanel.git.stagedScope", "已暂存")
                    : t("diffPanel.git.allScope", "全部")}
              </button>
            ))}
          </div>
          <span className="text-[11px] text-muted-foreground">
            {t("diffPanel.git.fileCount", "{{count}} 个文件", { count: changes.length })}
          </span>
          <span className="ml-auto inline-flex items-center gap-1">
            {renderGitMutationButtons({ kind: "all" })}
            <IconTip label={t("common.refresh", "刷新")}>
              <button
                type="button"
                className={iconButtonClass}
                disabled={Boolean(gitBusy)}
                onClick={() => void refreshGitScope()}
              >
                <RefreshCw className={cn("h-3.5 w-3.5", gitBusy === "refresh" && "animate-spin")} />
              </button>
            </IconTip>
          </span>
        </div>
      ) : null}

      {gitContext?.reviewComments.length ? (
        <details className="shrink-0 border-b border-border/60 bg-blue-500/5">
          <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs font-medium text-foreground/85 hover:bg-secondary/35">
            <MessageCircle className="h-3.5 w-3.5 text-blue-500" />
            <span className="flex-1">
              {t("diffPanel.git.prReviewComments", "PR Review 评论 · {{count}}", {
                count: gitContext.reviewComments.length,
              })}
            </span>
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          </summary>
          <div className="max-h-64 space-y-2 overflow-y-auto border-t border-border/50 p-2">
            {gitContext.reviewComments.slice(0, 20).map((comment) => (
              <div
                key={comment.threadId || comment.commentId}
                className="rounded-lg border border-border/60 bg-background/75 p-2.5"
              >
                <div className="whitespace-pre-wrap break-words text-xs leading-5">{comment.body}</div>
                <div className="mt-1.5 flex items-center gap-2 text-[11px] text-muted-foreground">
                  <span className="min-w-0 flex-1 truncate font-mono">
                    {comment.path}{comment.line ? `:${comment.line}` : ""}
                  </span>
                  <span className="shrink-0">{comment.author}</span>
                  {comment.url ? (
                    <button
                      type="button"
                      className="rounded p-1 hover:bg-secondary hover:text-foreground"
                      onClick={() => openExternalUrl(comment.url!)}
                      aria-label={t("workspace.git.openComment", "打开评论")}
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                    </button>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </details>
      ) : null}

      {stackedMode && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border bg-muted/30 px-2 py-1.5">
          <div className="relative w-40 shrink-0">
            <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
            <SearchInput
              value={fileQuery}
              onChange={(e) => setFileQuery(e.target.value)}
              placeholder={t("diffPanel.searchFiles", "搜索文件")}
              className="h-7 w-full pl-7 pr-2 text-xs"
            />
          </div>
          <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
            {filteredChangeIndexes.length > 0
              ? t("diffPanel.matchingFiles", "{{shown}} / {{total}} 个文件", {
                  shown: filteredChangeIndexes.length,
                  total: changes.length,
                })
              : t("diffPanel.noMatchingFiles", "没有匹配文件")}
          </span>
        </div>
      )}

      {stackedMode ? (
        <div ref={scrollRef} className={cn("flex-1 overflow-auto", PANEL_SCROLL_FADE)}>
          {stackedSections.length > 0 ? (
            <div className="divide-y divide-border/60">
              {stackedSections.map((section) => {
                const c = section.change
                return (
                  <div key={section.fileKey} className="bg-background/40">
                    <button
                      type="button"
                      className="sticky top-0 z-10 flex w-full items-center gap-2 border-b border-border/40 bg-surface-panel/95 px-3 py-2 text-left backdrop-blur transition-colors hover:bg-secondary/45"
                      onClick={() => toggleStackedFile(section.fileKey)}
                      aria-expanded={!section.collapsed}
                    >
                      <ChevronRight
                        className={cn(
                          "h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200",
                          !section.collapsed && "rotate-90",
                        )}
                      />
                      <ActionBadge action={c.action} />
                      <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/90" data-ha-title-tip={c.path}>
                        {c.path}
                      </span>
                      <FileDeltaCounter
                        linesAdded={c.linesAdded}
                        linesRemoved={c.linesRemoved}
                        className="text-[11px]"
                      />
                    </button>
                    {!section.collapsed && (
                      <div>
                        {gitContext && gitContext.scope !== "all" ? (
                          <div className="flex justify-end border-b border-border/40 bg-muted/15 px-2 py-1">
                            {renderGitMutationButtons(
                              { kind: "file", path: c.path },
                              true,
                            )}
                          </div>
                        ) : null}
                        {c.truncated && (
                          <div className="px-3 py-1.5 text-[11px] text-amber-600">
                            {t("diffPanel.fileTooLarge", "文件过大，仅渲染前 256KB")}
                          </div>
                        )}
                        {layout === "unified" ? (
                          <UnifiedDiffView
                            items={section.items as DiffViewItem<UnifiedRow>[]}
                            omittedItemCount={section.omittedItemCount}
                            onToggleFold={(id) => toggleStackedFold(section.fileKey, id)}
                            onRenderAll={() => setStackedRenderAll(section.fileKey)}
                            onCopyLocation={(line) => copyLocationForChange(c, line)}
                            onOpenLocation={(line, side) => openLocationForChange(c, line, side)}
                          />
                        ) : (
                          <SplitDiffView
                            items={section.items as DiffViewItem<SplitRow>[]}
                            omittedItemCount={section.omittedItemCount}
                            onToggleFold={(id) => toggleStackedFold(section.fileKey, id)}
                            onRenderAll={() => setStackedRenderAll(section.fileKey)}
                            onCopyLocation={(line) => copyLocationForChange(c, line)}
                            onOpenLocation={(line, side) => openLocationForChange(c, line, side)}
                          />
                        )}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
              {t("diffPanel.noMatchingFiles", "没有匹配文件")}
            </div>
          )}
        </div>
      ) : change ? (
        <>
          <div className="shrink-0 border-b border-border/60 px-3 py-1.5 text-[11px] text-muted-foreground">
            <div className="flex items-center gap-2 truncate">
              <ActionBadge action={change.action} />
              <span className="min-w-0 flex-1 truncate font-mono">{change.path}</span>
              {activeHunkLocation ? (
                <>
                  <IconTip label={t("diffPanel.copyLocation", "复制当前定位")}>
                    <button
                      type="button"
                      className={iconButtonClass}
                      onClick={() => copyLocation(activeHunkLocation.line)}
                    >
                      <Copy className="h-3.5 w-3.5" />
                    </button>
                  </IconTip>
                  {activeHunkLocation.side === "new" && change.action !== "delete" && onPreviewFile ? (
                    <IconTip label={t("diffPanel.openLocation", "预览到当前行")}>
                      <button
                        type="button"
                        className={iconButtonClass}
                        onClick={() => openLocation(activeHunkLocation.line, activeHunkLocation.side)}
                      >
                        <ExternalLink className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                  ) : null}
                </>
              ) : null}
              <FileDeltaCounter
                linesAdded={change.linesAdded}
                linesRemoved={change.linesRemoved}
                className="text-[11px]"
              />
              {gitContext
                ? renderGitMutationButtons({ kind: "file", path: change.path }, true)
                : null}
            </div>
            {activeGitHunkAllowed && activeGitHunk ? (
              <div className="mt-1 flex items-center gap-2 border-t border-border/45 pt-1">
                <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground/80">
                  {activeGitHunk.header}
                </span>
                {renderGitMutationButtons(
                  { kind: "hunk", path: activeGitChange?.path, hunkId: activeGitHunk.id },
                  true,
                )}
              </div>
            ) : null}
            {change.truncated && (
              <div className="mt-0.5 text-amber-600">
                {t("diffPanel.fileTooLarge", "文件过大，仅渲染前 256KB")}
              </div>
            )}
          </div>
          <div
            ref={scrollRef}
            className={cn("flex-1 overflow-auto", PANEL_SCROLL_FADE)}
            onScroll={handleScroll}
          >
            {layout === "unified" ? (
              <UnifiedDiffView
                items={displayedItems as DiffViewItem<UnifiedRow>[]}
                omittedItemCount={omittedItemCount}
                onToggleFold={toggleFold}
                onRenderAll={() => setRenderAllRows(true)}
                onCopyLocation={copyLocation}
                onOpenLocation={openLocation}
              />
            ) : (
              <SplitDiffView
                items={displayedItems as DiffViewItem<SplitRow>[]}
                omittedItemCount={omittedItemCount}
                onToggleFold={toggleFold}
                onRenderAll={() => setRenderAllRows(true)}
                onCopyLocation={copyLocation}
                onOpenLocation={openLocation}
              />
            )}
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          {t("diffPanel.noDiffData", "无 diff 数据")}
        </div>
      )}
    </div>
  )
}

const iconButtonClass =
  "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
const activeIconButtonClass = "bg-secondary text-foreground"
const gitActionButtonClass =
  "inline-flex items-center justify-center gap-1 rounded border border-border/60 bg-background/70 font-medium text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground disabled:pointer-events-none disabled:opacity-40"

function updateActiveHunkFromScroll(
  container: HTMLElement,
  setActiveHunkIndex: (index: number) => void,
) {
  const starts = Array.from(
    container.querySelectorAll<HTMLElement>("[data-diff-hunk-start='true']"),
  )
  if (starts.length === 0) {
    setActiveHunkIndex(0)
    return
  }
  const anchorTop = container.getBoundingClientRect().top + 24
  let current = Number(starts[0].dataset.diffHunkIndex ?? 0)
  for (const start of starts) {
    if (start.getBoundingClientRect().top <= anchorTop) {
      current = Number(start.dataset.diffHunkIndex ?? current)
    } else {
      break
    }
  }
  setActiveHunkIndex(current)
}

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName.toLowerCase()
  return tag === "input" || tag === "textarea" || target.isContentEditable
}

function ActionBadge({ action }: { action: FileChangeMetadata["action"] }) {
  const { t } = useTranslation()
  const label =
    action === "create"
      ? t("diffPanel.actionCreate", "创建")
      : action === "delete"
        ? t("diffPanel.actionDelete", "删除")
        : t("diffPanel.actionEdit", "修改")
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded border px-1.5 py-0.5 text-[10px] leading-none",
        action === "create" &&
          "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
        action === "delete" &&
          "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300",
        action === "edit" && "border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300",
      )}
    >
      {label}
    </span>
  )
}

function fileKeyForChange(change: FileChangeMetadata, index: number): string {
  return `${index}\u001f${change.path}\u001f${change.action}`
}
