import { useCallback, useMemo, useState } from "react"
import type { PanelActionEntry } from "@/hooks/usePanelActionHistory"
import type { FramePreviewReplay } from "@/components/chat/right-panel/FramePreview"

/**
 * Timeline step-replay selection, shared by the browser / mac control panel
 * contents. Only entries with a thumbnail are selectable; clicking the active
 * entry exits replay. Everything is derived from `entries` each render, so an
 * entry evicted from the capped list silently drops the replay instead of
 * rendering a stale "step 0 / N" overlay.
 */
export function useReplaySelection(entries: PanelActionEntry[]): {
  replay: FramePreviewReplay | null
  replayActionId: string | null
  onSelect: (entry: PanelActionEntry) => void
} {
  const [selected, setSelected] = useState<PanelActionEntry | null>(null)

  const onSelect = useCallback((entry: PanelActionEntry) => {
    setSelected((prev) =>
      prev?.actionId === entry.actionId ? null : entry.thumbJpegBase64 ? entry : prev,
    )
  }, [])

  const exit = useCallback(() => setSelected(null), [])

  const { replay, replayActionId } = useMemo(() => {
    if (!selected?.thumbJpegBase64) return { replay: null, replayActionId: null }
    const index = entries.findIndex((e) => e.actionId === selected.actionId)
    if (index < 0) return { replay: null, replayActionId: null }
    return {
      replay: {
        thumbJpegBase64: selected.thumbJpegBase64,
        index: index + 1,
        total: entries.length,
        onExit: exit,
      },
      replayActionId: selected.actionId,
    }
  }, [entries, exit, selected])

  return { replay, replayActionId, onSelect }
}
