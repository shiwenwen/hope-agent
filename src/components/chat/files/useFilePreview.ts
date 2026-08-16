import { useCallback, useMemo, useRef, useState } from "react"
import { createDraftAttachment, type DraftAttachment, type FileTarget } from "./types"

export type PreviewTarget = FileTarget

export interface FilePreviewEntry {
  id: string
  title: string
  target: PreviewTarget
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

function toEntry(target: PreviewTarget): FilePreviewEntry {
  return {
    id: `preview:${encodeURIComponent(previewIdentity(target))}`,
    title: previewTitle(target),
    target,
  }
}

export interface UseFilePreview {
  showPanel: boolean
  entries: FilePreviewEntry[]
  activeId: string | null
  target: PreviewTarget | null
  /** Bumps on every open, including focusing an already-open file. */
  openNonce: number
  openPreview: (target: PreviewTarget) => void
  selectPreview: (id: string) => void
  closePreview: (id?: string) => void
  closeAllPreviews: () => void
  reorderPreviews: (source: string, target: string) => void
  /** Swap the visible tab set while retaining allowed session sets in memory. */
  switchScope: (scopeKey: string, options?: FilePreviewScopeSwitchOptions) => void
}

export interface FilePreviewScopeSwitchOptions {
  /** Restore a previously retained destination set. Defaults to true. */
  restore?: boolean
  /** Retain the set being left. Incognito callers must pass false. */
  cacheCurrent?: boolean
}

/**
 * Session-local file preview tabs. Reopening the same logical file refreshes its
 * target (including reveal lines / object URL nonce) instead of duplicating it.
 */
export function useFilePreview(): UseFilePreview {
  const [entries, setEntries] = useState<FilePreviewEntry[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [openNonce, setOpenNonce] = useState(0)
  const entriesRef = useRef(entries)
  const activeIdRef = useRef(activeId)
  const scopeRef = useRef("__draft__")
  const scopeCacheRef = useRef(
    new Map<string, { entries: FilePreviewEntry[]; activeId: string | null }>(),
  )
  entriesRef.current = entries
  activeIdRef.current = activeId

  const openPreview = useCallback((next: PreviewTarget) => {
    const entry = toEntry(next)
    setEntries((current) => {
      const index = current.findIndex((item) => item.id === entry.id)
      if (index < 0) return [...current, entry]
      const updated = [...current]
      updated[index] = entry
      return updated
    })
    setActiveId(entry.id)
    setOpenNonce((n) => n + 1)
  }, [])

  const selectPreview = useCallback((id: string) => {
    setActiveId((current) => (current === id ? current : id))
  }, [])

  const closePreview = useCallback(
    (id?: string) => {
      const closingId = id ?? activeId
      const closingIndex = entries.findIndex((entry) => entry.id === closingId)
      if (closingIndex < 0) return
      const next = entries.filter((entry) => entry.id !== closingId)
      setEntries(next)
      setActiveId((selected) =>
        selected === closingId
          ? (next[Math.min(closingIndex, next.length - 1)]?.id ?? null)
          : selected,
      )
    },
    [activeId, entries],
  )

  const closeAllPreviews = useCallback(() => {
    setEntries([])
    setActiveId(null)
  }, [])

  const reorderPreviews = useCallback((source: string, target: string) => {
    setEntries((current) => {
      const sourceIndex = current.findIndex((entry) => entry.id === source)
      const targetIndex = current.findIndex((entry) => entry.id === target)
      if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return current
      const next = [...current]
      const [moved] = next.splice(sourceIndex, 1)
      next.splice(targetIndex, 0, moved)
      return next
    })
  }, [])

  const switchScope = useCallback(
    (scopeKey: string, options: FilePreviewScopeSwitchOptions = {}) => {
      const { restore = true, cacheCurrent = true } = options
      const currentScope = scopeRef.current

      if (currentScope === scopeKey) {
        if (restore && cacheCurrent) return
        if (!cacheCurrent || !restore) scopeCacheRef.current.delete(scopeKey)
        if (!restore) {
          entriesRef.current = []
          activeIdRef.current = null
          setEntries([])
          setActiveId(null)
        }
        return
      }

      if (cacheCurrent) {
        scopeCacheRef.current.set(currentScope, {
          entries: entriesRef.current,
          activeId: activeIdRef.current,
        })
      } else {
        scopeCacheRef.current.delete(currentScope)
      }

      const saved = restore ? scopeCacheRef.current.get(scopeKey) : undefined
      if (!restore) scopeCacheRef.current.delete(scopeKey)
      const nextEntries = saved?.entries ?? []
      const nextActiveId = saved?.activeId ?? null
      scopeRef.current = scopeKey
      entriesRef.current = nextEntries
      activeIdRef.current = nextActiveId
      setEntries(nextEntries)
      setActiveId(nextActiveId)
    },
    [],
  )

  const target = useMemo(
    () => entries.find((entry) => entry.id === activeId)?.target ?? null,
    [activeId, entries],
  )

  return {
    showPanel: entries.length > 0,
    entries,
    activeId,
    target,
    openNonce,
    openPreview,
    selectPreview,
    closePreview,
    closeAllPreviews,
    reorderPreviews,
    switchScope,
  }
}
