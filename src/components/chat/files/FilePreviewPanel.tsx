import { useMemo } from "react"

import { FilePreviewPane } from "@/components/chat/project/file-browser/FilePreviewPane"
import { useTransport } from "@/lib/transport-provider"
import type { PreviewSource } from "./previewSource"
import { fileResourceAdapterFor } from "./fileResourceAdapter"
import { StagedFilePreviewPane } from "./StagedFilePreviewPane"
import { useFileResource } from "./useFileResource"
import type { PreviewTarget } from "./useFilePreview"
import { useFilesystemConfig } from "@/lib/filesystemConfig"

interface FilePreviewPanelProps {
  /** Current preview target (path or media), or `null` for the empty state. */
  target: PreviewTarget | null
  /** Session id — required to authorize path/media reads in HTTP mode. */
  sessionId?: string | null
  /** Replace an edited renderer-local draft in the chat composer. */
  onReplaceDraft?: (draftId: string, file: File) => void
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
  onReplaceDraft,
  onClose,
  maximized,
  onToggleMaximize,
}: FilePreviewPanelProps) {
  if (target?.kind === "clientDraft") {
    return (
      <StagedFilePreviewPane
        key={target.previewId}
        target={target}
        onReplaceFile={(file) => onReplaceDraft?.(target.draft.id, file)}
        onClose={onClose}
        className="h-full min-h-0"
        maximized={maximized}
        onToggleMaximize={onToggleMaximize}
      />
    )
  }

  return (
    <PersistedFilePreviewPanel
      target={target}
      sessionId={sessionId}
      onClose={onClose}
      maximized={maximized}
      onToggleMaximize={onToggleMaximize}
    />
  )
}

type PersistedPreviewTarget = Exclude<PreviewTarget, { kind: "clientDraft" }>

function PersistedFilePreviewPanel({
  target,
  sessionId,
  onClose,
  maximized,
  onToggleMaximize,
}: Omit<FilePreviewPanelProps, "target"> & { target: PersistedPreviewTarget | null }) {
  const transport = useTransport()
  const { config: filesystemConfig } = useFilesystemConfig()
  const { run, isLocal, capabilities } = useFileResource(target, { sessionId })
  const source = useMemo<PreviewSource | null>(() => {
    if (!target) return null
    return fileResourceAdapterFor(target).previewSource(target, {
      transport,
      sessionId,
      filesystemConfig,
    })
  }, [target, sessionId, transport, filesystemConfig])
  const highlightLines =
    target?.kind === "sessionPath" || target?.kind === "workspace"
      ? (target.revealLines ?? null)
      : null

  return (
    <FilePreviewPane
      source={source}
      onClose={onClose}
      onOpen={target && capabilities.open.state === "enabled" ? () => run("open") : undefined}
      onDownload={
        target && !isLocal && capabilities.download.state === "enabled"
          ? () => run("download")
          : undefined
      }
      onEdit={target && capabilities.edit.state === "enabled" ? () => run("edit") : undefined}
      highlightLines={highlightLines}
      className="h-full min-h-0"
      maximized={maximized}
      onToggleMaximize={onToggleMaximize}
    />
  )
}
