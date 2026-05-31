/**
 * Per-scope expanded-directory state for the file browser tree, persisted to
 * localStorage so the tree restores its open folders across reloads. Mirrors
 * the folding-persistence pattern used by the sidebar `ProjectSection`.
 */

import { useCallback, useEffect, useState } from "react"

const KEY_PREFIX = "ha:fs-expanded:"

function storageKey(scope: string, scopeId: string): string {
  return `${KEY_PREFIX}${scope}:${scopeId}`
}

function readSet(key: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) return new Set()
    const arr = JSON.parse(raw) as unknown
    return new Set(Array.isArray(arr) ? (arr as string[]) : [])
  } catch {
    return new Set()
  }
}

function writeSet(key: string, set: Set<string>) {
  try {
    window.localStorage.setItem(key, JSON.stringify([...set]))
  } catch {
    /* ignore quota / private-mode errors */
  }
}

export interface UseTreeExpansion {
  expanded: Set<string>
  isExpanded: (path: string) => boolean
  toggle: (path: string) => void
  setOpen: (path: string, open: boolean) => void
  collapseAll: () => void
}

export function useTreeExpansion(scope: string, scopeId: string): UseTreeExpansion {
  const key = storageKey(scope, scopeId)
  const [expanded, setExpanded] = useState<Set<string>>(() => readSet(key))

  // Re-read when the scope changes (e.g. switching project / session).
  useEffect(() => {
    setExpanded(readSet(key))
  }, [key])

  const toggle = useCallback(
    (path: string) => {
      setExpanded((prev) => {
        const next = new Set(prev)
        if (next.has(path)) next.delete(path)
        else next.add(path)
        writeSet(key, next)
        return next
      })
    },
    [key],
  )

  const setOpen = useCallback(
    (path: string, open: boolean) => {
      setExpanded((prev) => {
        if (open === prev.has(path)) return prev
        const next = new Set(prev)
        if (open) next.add(path)
        else next.delete(path)
        writeSet(key, next)
        return next
      })
    },
    [key],
  )

  const collapseAll = useCallback(() => {
    setExpanded(() => {
      const empty = new Set<string>()
      writeSet(key, empty)
      return empty
    })
  }, [key])

  const isExpanded = useCallback((path: string) => expanded.has(path), [expanded])

  return { expanded, isExpanded, toggle, setOpen, collapseAll }
}
