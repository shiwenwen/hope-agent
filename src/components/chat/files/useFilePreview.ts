import { useCallback, useMemo, useState } from "react"
import { createDraftAttachment, type DraftAttachment, type FileTarget } from "./types"
import { useScopedTabState, type ScopeSwitchOptions } from "./useScopedTabState"

export type PreviewTarget = FileTarget

export interface FilePreviewEntry {
  id: string
  title: string
  target: PreviewTarget
  /** Session whose transcript authorizes HTTP reads for this preview. */
  authorizationSessionId?: string | null
}

let stagedPreviewId = 0

export function stagedFilePreviewTarget(fileOrDraft: File | DraftAttachment): PreviewTarget {
  stagedPreviewId += 1
  const draft =
    fileOrDraft instanceof File ? createDraftAttachment(fileOrDraft, "picker") : fileOrDraft
  return {
    kind: "clientDraft",
    draft,
    previewId: `staged-${stagedPreviewId}`,
  }
}

function previewIdentity(target: PreviewTarget): string {
  switch (target.kind) {
    case "clientDraft":
      return `draft:${target.draft.id}`
    case "workspace":
      return `workspace:${target.scope}:${target.scopeId}:${target.relPath}`
    case "sessionPath":
      return `path:${target.sessionId ?? ""}:${target.path}`
    case "media":
      return `media:${target.item.localPath ?? target.item.url}:${target.item.name}`
    case "knowledgeNote":
      return `knowledge:${target.kbId}:${target.path}`
    case "artifact":
      return `artifact:${target.artifactId}`
  }
}

function previewTitle(target: PreviewTarget): string {
  switch (target.kind) {
    case "clientDraft":
      return target.draft.file.name
    case "workspace":
    case "sessionPath":
    case "artifact":
      return target.name
    case "media":
      return target.item.name
    case "knowledgeNote": {
      const parts = target.path.split(/[\\/]/).filter(Boolean)
      return parts.at(-1) ?? target.path
    }
  }
}

/** MIME hint for a target, so a tab icon can fall back to it when the file name
 *  carries no (or an unknown) extension. */
export function previewTargetMime(target: PreviewTarget): string | null {
  switch (target.kind) {
    case "clientDraft":
      return target.draft.file.type || null
    case "workspace":
    case "sessionPath":
      return target.mime ?? null
    case "media":
      return target.item.mimeType || null
    case "knowledgeNote":
      return "text/markdown"
    case "artifact":
      return "text/html"
  }
}

function toEntry(target: PreviewTarget, authorizationSessionId?: string | null): FilePreviewEntry {
  const authorizationIdentity = authorizationSessionId
    ? `:${encodeURIComponent(authorizationSessionId)}`
    : ""
  return {
    id: `preview:${encodeURIComponent(previewIdentity(target))}${authorizationIdentity}`,
    title: previewTitle(target),
    target,
    authorizationSessionId,
  }
}

export interface UseFilePreview {
  showPanel: boolean
  entries: FilePreviewEntry[]
  activeId: string | null
  target: PreviewTarget | null
  /** Bumps on every open, including focusing an already-open file. */
  openNonce: number
  openPreview: (target: PreviewTarget, authorizationSessionId?: string | null) => void
  selectPreview: (id: string) => void
  closePreview: (id?: string) => void
  closeAllPreviews: () => void
  reorderPreviews: (source: string, target: string) => void
  /** Swap the visible tab set while retaining allowed session sets in memory. */
  switchScope: (scopeKey: string, options?: FilePreviewScopeSwitchOptions) => void
  /** Re-address the open set when a draft chat becomes a real session. */
  renameScope: (from: string, to: string) => void
}

export type FilePreviewScopeSwitchOptions = ScopeSwitchOptions

interface FilePreviewState {
  entries: FilePreviewEntry[]
  activeId: string | null
}

const EMPTY_STATE: FilePreviewState = { entries: [], activeId: null }

/**
 * Session-local file preview tabs. Reopening the same logical file refreshes its
 * target (including reveal lines / object URL nonce) instead of duplicating it.
 * Entries and the active id share one state object so closing and scope
 * switching always derive a consistent pair.
 */
export function useFilePreview(): UseFilePreview {
  const { state, setState, switchScope, renameScope } =
    useScopedTabState<FilePreviewState>(EMPTY_STATE)
  const [openNonce, setOpenNonce] = useState(0)

  const openPreview = useCallback(
    (next: PreviewTarget, authorizationSessionId?: string | null) => {
      const entry = toEntry(next, authorizationSessionId)
      setState((current) => {
        const index = current.entries.findIndex((item) => item.id === entry.id)
        if (index < 0) return { entries: [...current.entries, entry], activeId: entry.id }
        const entries = [...current.entries]
        entries[index] = entry
        return { entries, activeId: entry.id }
      })
      setOpenNonce((n) => n + 1)
    },
    [setState],
  )

  const selectPreview = useCallback(
    (id: string) => {
      setState((current) => (current.activeId === id ? current : { ...current, activeId: id }))
    },
    [setState],
  )

  const closePreview = useCallback(
    (id?: string) => {
      setState((current) => {
        const closingId = id ?? current.activeId
        const closingIndex = current.entries.findIndex((entry) => entry.id === closingId)
        if (closingIndex < 0) return current
        const entries = current.entries.filter((entry) => entry.id !== closingId)
        return {
          entries,
          activeId:
            current.activeId === closingId
              ? (entries[Math.min(closingIndex, entries.length - 1)]?.id ?? null)
              : current.activeId,
        }
      })
    },
    [setState],
  )

  const closeAllPreviews = useCallback(() => setState(EMPTY_STATE), [setState])

  const reorderPreviews = useCallback(
    (source: string, target: string) => {
      setState((current) => {
        const sourceIndex = current.entries.findIndex((entry) => entry.id === source)
        const targetIndex = current.entries.findIndex((entry) => entry.id === target)
        if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return current
        const entries = [...current.entries]
        const [moved] = entries.splice(sourceIndex, 1)
        entries.splice(targetIndex, 0, moved)
        return { ...current, entries }
      })
    },
    [setState],
  )

  const target = useMemo(
    () => state.entries.find((entry) => entry.id === state.activeId)?.target ?? null,
    [state],
  )

  // Memoized for the same reason as `useFileTabs`: it feeds the shared
  // FileActionsContext value, which must not change identity every render.
  return useMemo(
    () => ({
      showPanel: state.entries.length > 0,
      entries: state.entries,
      activeId: state.activeId,
      target,
      openNonce,
      openPreview,
      selectPreview,
      closePreview,
      closeAllPreviews,
      reorderPreviews,
      switchScope,
      renameScope,
    }),
    [
      closeAllPreviews,
      closePreview,
      openNonce,
      openPreview,
      renameScope,
      reorderPreviews,
      selectPreview,
      state.activeId,
      state.entries,
      switchScope,
      target,
    ],
  )
}
