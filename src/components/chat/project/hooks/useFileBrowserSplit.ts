import { useCallback, useState } from "react"

/**
 * localStorage-persisted tree column width (px) for the file browser split
 * layout, keyed by `scope:scopeId`. Same try/catch + quota-safe pattern as
 * {@link useTreeExpansion}.
 */
const KEY_PREFIX = "ha:fs-split:"
const DEFAULT_TREE_WIDTH = 240

function storageKey(scope: string, scopeId: string): string {
  return `${KEY_PREFIX}${scope}:${scopeId}`
}

function readWidth(key: string): number {
  try {
    const raw = window.localStorage.getItem(key)
    const n = raw ? Number(raw) : NaN
    return Number.isFinite(n) && n > 0 ? n : DEFAULT_TREE_WIDTH
  } catch {
    return DEFAULT_TREE_WIDTH
  }
}

export function useFileBrowserSplit(scope: string, scopeId: string) {
  const key = storageKey(scope, scopeId)
  const [width, setWidthState] = useState(() => readWidth(key))

  // Reload the persisted width when the scope target changes, using the
  // setState-during-render pattern (React-recommended over an effect).
  const [trackedKey, setTrackedKey] = useState(key)
  if (key !== trackedKey) {
    setTrackedKey(key)
    setWidthState(readWidth(key))
  }

  const setWidth = useCallback(
    (w: number) => {
      setWidthState(w)
      try {
        window.localStorage.setItem(key, String(Math.round(w)))
      } catch {
        /* ignore quota / private-mode errors */
      }
    },
    [key],
  )

  return [width, setWidth] as const
}
