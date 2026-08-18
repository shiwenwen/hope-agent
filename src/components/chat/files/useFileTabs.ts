import { useCallback, useMemo, useState } from "react"
import type {
  ProjectFileQuoteReveal,
  ProjectFolderIdentity,
} from "@/components/chat/project/fileQuoteTarget"
import type { FileBrowserDirectoryReveal } from "@/components/chat/project/file-browser/FileBrowserView"
import { useScopedTabState, type ScopeSwitchOptions } from "./useScopedTabState"

/** What a tab's browser currently has selected — drives its label and icon. */
export interface FileTabSelection {
  name: string
  relPath: string
}

/** A file to reveal, already resolved to a root the file browser can address. */
export interface FileTabFileReveal {
  path: string
  name: string
  projectRoot?: ProjectFolderIdentity | null
  /** Read-only git worktree the path was captured in (composer quote chips). */
  worktreeRoot?: string | null
  /** Quoted range to highlight; omitted or zero means "no range". */
  startLine?: number
  endLine?: number
}

export interface FileTabEntry {
  id: string
  /** Pending file reveal for this tab; the nonce re-fires the same path. */
  revealFile: ProjectFileQuoteReveal | null
  /** Pending directory reveal (breadcrumb jump); mutually exclusive with above. */
  revealDirectory: FileBrowserDirectoryReveal | null
  selection: FileTabSelection | null
}

export type FileTabScopeSwitchOptions = ScopeSwitchOptions

export interface UseFileTabs {
  tabs: FileTabEntry[]
  activeId: string | null
  showPanel: boolean
  /** Bumps on every open/reveal, including into an already-visible tab. */
  openNonce: number
  /** Open a new browser tab, optionally revealing a file in it. */
  openTab: (reveal?: FileTabFileReveal | null) => void
  /** Reveal in the active tab, opening one when none exists yet. */
  revealFileInActiveTab: (reveal: FileTabFileReveal) => void
  /** Expand + select a directory in the active tab, opening one when needed. */
  revealDirectoryInActiveTab: (relPath: string, projectRoot: ProjectFolderIdentity | null) => void
  /** Drop every pending reveal so its quoted-range highlight disappears. */
  clearReveals: () => void
  selectTab: (id: string) => void
  closeTab: (id?: string) => void
  closeAllTabs: () => void
  reorderTabs: (source: string, target: string) => void
  setTabSelection: (id: string, selection: FileTabSelection | null) => void
  switchScope: (scopeKey: string, options?: FileTabScopeSwitchOptions) => void
  /** Re-address the open set when a draft chat becomes a real session. */
  renameScope: (from: string, to: string) => void
}

interface FileTabsState {
  tabs: FileTabEntry[]
  activeId: string | null
}

const EMPTY_STATE: FileTabsState = { tabs: [], activeId: null }

let fileTabSeq = 0
let revealSeq = 0

function createTab(revealFile: ProjectFileQuoteReveal | null): FileTabEntry {
  fileTabSeq += 1
  return { id: `files:${fileTabSeq}`, revealFile, revealDirectory: null, selection: null }
}

function toFileReveal(reveal: FileTabFileReveal): ProjectFileQuoteReveal {
  revealSeq += 1
  return {
    path: reveal.path,
    name: reveal.name,
    // Zero lines = "no quoted range"; the preview only highlights positive ones.
    startLine: reveal.startLine ?? 0,
    endLine: reveal.endLine ?? 0,
    projectRoot: reveal.projectRoot ?? undefined,
    worktreeRoot: reveal.worktreeRoot ?? undefined,
    nonce: revealSeq,
  }
}

/** Apply a reveal to the active tab, creating one when the set is empty. */
function withRevealInActiveTab(
  state: FileTabsState,
  patch: Pick<FileTabEntry, "revealFile" | "revealDirectory">,
): FileTabsState {
  const targetId = state.activeId ?? state.tabs[state.tabs.length - 1]?.id
  const index = state.tabs.findIndex((tab) => tab.id === targetId)
  if (index < 0) {
    const tab = { ...createTab(patch.revealFile), revealDirectory: patch.revealDirectory }
    return { tabs: [...state.tabs, tab], activeId: tab.id }
  }
  const tabs = [...state.tabs]
  tabs[index] = { ...tabs[index], ...patch }
  return { tabs, activeId: tabs[index].id }
}

/**
 * Session-local file browser tabs. Each tab is a full browser (tree + preview);
 * new tabs come only from an explicit request, never from picking a file.
 * Tabs and the active id share one state object so closing and scope switching
 * always derive a consistent pair.
 */
export function useFileTabs(): UseFileTabs {
  const { state, setState, switchScope, renameScope } =
    useScopedTabState<FileTabsState>(EMPTY_STATE)
  const [openNonce, setOpenNonce] = useState(0)

  const openTab = useCallback(
    (reveal?: FileTabFileReveal | null) => {
      setState((current) => {
        const tab = createTab(reveal ? toFileReveal(reveal) : null)
        return { tabs: [...current.tabs, tab], activeId: tab.id }
      })
      setOpenNonce((n) => n + 1)
    },
    [setState],
  )

  const revealFileInActiveTab = useCallback(
    (reveal: FileTabFileReveal) => {
      const target = toFileReveal(reveal)
      setState((current) =>
        withRevealInActiveTab(current, { revealFile: target, revealDirectory: null }),
      )
      setOpenNonce((n) => n + 1)
    },
    [setState],
  )

  const revealDirectoryInActiveTab = useCallback(
    (relPath: string, projectRoot: ProjectFolderIdentity | null) => {
      revealSeq += 1
      const target: FileBrowserDirectoryReveal = { relPath, projectRoot, nonce: revealSeq }
      setState((current) =>
        withRevealInActiveTab(current, { revealFile: null, revealDirectory: target }),
      )
      setOpenNonce((n) => n + 1)
    },
    [setState],
  )

  const clearReveals = useCallback(() => {
    setState((current) =>
      current.tabs.some((tab) => tab.revealFile || tab.revealDirectory)
        ? {
            ...current,
            tabs: current.tabs.map((tab) =>
              tab.revealFile || tab.revealDirectory
                ? { ...tab, revealFile: null, revealDirectory: null }
                : tab,
            ),
          }
        : current,
    )
  }, [setState])

  const selectTab = useCallback(
    (id: string) => {
      setState((current) => (current.activeId === id ? current : { ...current, activeId: id }))
    },
    [setState],
  )

  const closeTab = useCallback(
    (id?: string) => {
      setState((current) => {
        const closingId = id ?? current.activeId
        const closingIndex = current.tabs.findIndex((tab) => tab.id === closingId)
        if (closingIndex < 0) return current
        const tabs = current.tabs.filter((tab) => tab.id !== closingId)
        return {
          tabs,
          activeId:
            current.activeId === closingId
              ? (tabs[Math.min(closingIndex, tabs.length - 1)]?.id ?? null)
              : current.activeId,
        }
      })
    },
    [setState],
  )

  const closeAllTabs = useCallback(() => setState(EMPTY_STATE), [setState])

  const reorderTabs = useCallback(
    (source: string, target: string) => {
      setState((current) => {
        const sourceIndex = current.tabs.findIndex((tab) => tab.id === source)
        const targetIndex = current.tabs.findIndex((tab) => tab.id === target)
        if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return current
        const tabs = [...current.tabs]
        const [moved] = tabs.splice(sourceIndex, 1)
        tabs.splice(targetIndex, 0, moved)
        return { ...current, tabs }
      })
    },
    [setState],
  )

  const setTabSelection = useCallback(
    (id: string, selection: FileTabSelection | null) => {
      setState((current) => {
        const index = current.tabs.findIndex((tab) => tab.id === id)
        if (index < 0) return current
        const previous = current.tabs[index].selection
        if (previous?.relPath === selection?.relPath && previous?.name === selection?.name) {
          return current
        }
        const tabs = [...current.tabs]
        tabs[index] = { ...tabs[index], selection }
        return { ...current, tabs }
      })
    },
    [setState],
  )

  // Memoized: this object is a dependency of the FileActionsContext value, and a
  // fresh identity every render would push a new context value through every
  // memo boundary in the transcript (file chips live in message bubbles).
  return useMemo(
    () => ({
      tabs: state.tabs,
      activeId: state.activeId,
      showPanel: state.tabs.length > 0,
      openNonce,
      openTab,
      revealFileInActiveTab,
      revealDirectoryInActiveTab,
      clearReveals,
      selectTab,
      closeTab,
      closeAllTabs,
      reorderTabs,
      setTabSelection,
      switchScope,
      renameScope,
    }),
    [
      clearReveals,
      closeAllTabs,
      closeTab,
      openNonce,
      openTab,
      renameScope,
      reorderTabs,
      revealDirectoryInActiveTab,
      revealFileInActiveTab,
      selectTab,
      setTabSelection,
      state.activeId,
      state.tabs,
      switchScope,
    ],
  )
}
