import { useState } from "react"
import type { FileChangeMetadata, FileChangesMetadata } from "@/types/chat"

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
  openDiff: (payload: FileChangeMetadata | FileChangesMetadata) => void
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

  const openDiff = (payload: FileChangeMetadata | FileChangesMetadata) => {
    if (payload.kind === "file_change") {
      setActiveChanges([payload])
    } else {
      setActiveChanges(payload.changes.length ? payload.changes : [])
    }
    setActiveIndex(0)
    setShowPanel(true)
  }

  const closeDiff = () => {
    setShowPanel(false)
    setActiveChanges([])
    setActiveIndex(0)
  }

  return {
    showPanel,
    activeChanges,
    activeIndex,
    setActiveIndex,
    openDiff,
    closeDiff,
    panelWidth,
    setPanelWidth,
  }
}
