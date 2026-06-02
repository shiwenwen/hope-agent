import { useCallback, useState } from "react"
import type { MediaItem } from "@/types/chat"

/**
 * What the right-side preview panel is currently showing. A `path` target is an
 * absolute filesystem path (Markdown links, workspace files, attachment paths);
 * a `media` target is a chat attachment `MediaItem`. The panel turns either into
 * a `PreviewSource` (see `previewSource.ts`).
 */
export type PreviewTarget =
  | { kind: "path"; path: string; name: string; mime?: string }
  | { kind: "media"; item: MediaItem }

export interface UseFilePreview {
  showPanel: boolean
  target: PreviewTarget | null
  /** Open (or replace) the preview panel with a target. */
  openPreview: (target: PreviewTarget) => void
  closePreview: () => void
}

/**
 * Local state for the right-side file-preview panel. Mirrors `useDiffPanel`:
 * visibility + active target. ChatScreen feeds `showPanel` into the exclusive
 * right-panel visibility map and passes `openPreview` down as `onPreviewFile`.
 */
export function useFilePreview(): UseFilePreview {
  const [showPanel, setShowPanel] = useState(false)
  const [target, setTarget] = useState<PreviewTarget | null>(null)

  const openPreview = useCallback((next: PreviewTarget) => {
    setTarget(next)
    setShowPanel(true)
  }, [])

  const closePreview = useCallback(() => {
    setShowPanel(false)
    setTarget(null)
  }, [])

  return { showPanel, target, openPreview, closePreview }
}
