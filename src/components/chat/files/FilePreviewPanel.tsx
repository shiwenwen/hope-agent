import { useMemo } from "react"

import { FilePreviewPane } from "@/components/chat/project/file-browser/FilePreviewPane"
import {
  mediaPreviewSource,
  pathPreviewSource,
  type PreviewSource,
} from "./previewSource"
import type { PreviewTarget } from "./useFilePreview"

interface FilePreviewPanelProps {
  /** Current preview target (path or media), or `null` for the empty state. */
  target: PreviewTarget | null
  /** Session id — required to authorize path/media reads in HTTP mode. */
  sessionId?: string | null
  onClose: () => void
  /** Fullscreen toggle — mirrors the files / canvas panels' maximize affordance. */
  maximized?: boolean
  onToggleMaximize?: () => void
}

/**
 * Right-side exclusive panel that previews a single file from anywhere in chat
 * (Markdown links, message attachments, the workspace panel). Turns the active
 * {@link PreviewTarget} into a {@link PreviewSource} and hands it to the shared
 * {@link FilePreviewPane} (reused from the project file browser).
 */
export default function FilePreviewPanel({
  target,
  sessionId,
  onClose,
  maximized,
  onToggleMaximize,
}: FilePreviewPanelProps) {
  const source = useMemo<PreviewSource | null>(() => {
    if (!target) return null
    return target.kind === "media"
      ? mediaPreviewSource(target.item, sessionId)
      : pathPreviewSource(target.path, target.name, sessionId, target.mime)
  }, [target, sessionId])

  return (
    <FilePreviewPane
      source={source}
      onClose={onClose}
      className="h-full min-h-0"
      maximized={maximized}
      onToggleMaximize={onToggleMaximize}
    />
  )
}
