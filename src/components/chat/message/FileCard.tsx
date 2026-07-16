import React, { useMemo } from "react"
import { formatBytes } from "@/lib/format"
import type { MediaItem } from "@/types/chat"
import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import { FileContextMenu, FileActionsMoreButton } from "@/components/chat/files/FileActionMenu"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"

/**
 * Colorful, format-specific file icon resolved from MIME (falling back to the
 * filename). Thin adapter over the shared {@link FileTypeIcon} kept for the
 * existing `(mime, name)` call sites (attachments, workspace panel). Size via
 * `className`; the icon carries its own brand colors.
 */
export function FileMimeIcon({
  mime,
  name,
  className,
}: {
  mime: string
  name: string
  className?: string
}) {
  return <FileTypeIcon name={name} mime={mime} className={className} />
}

/** Downloadable file card rendered for `send_attachment` and any other tool
 *  that emits structured media items via the `__MEDIA_ITEMS__` prefix.
 *  Primary click follows the unified policy (preview / open / download by kind ×
 *  mode); right-click and the ⋯ button expose the full action menu. */
function FileCard({ item }: { item: MediaItem }) {
  const target = useMemo<PreviewTarget>(() => ({ kind: "media", item }), [item])
  const { primary, run } = useFileResource(target)

  return (
    <FileContextMenu target={target}>
      <div className="inline-flex items-center gap-2 max-w-sm rounded-md border border-border/50 bg-secondary/30 hover:bg-secondary/50 transition-colors px-2.5 py-1.5 text-xs">
        <FileMimeIcon mime={item.mimeType} name={item.name} className="h-4 w-4 shrink-0" />
        <button
          type="button"
          onClick={() => run(primary)}
          className="flex flex-col items-start min-w-0 flex-1 text-left hover:text-foreground transition-colors"
        >
          <span className="truncate max-w-[240px] font-medium text-foreground/90">{item.name}</span>
          <span className="text-[10px] text-muted-foreground/70 tabular-nums">
            {formatBytes(item.sizeBytes)}
          </span>
        </button>
        <FileActionsMoreButton target={target} className="shrink-0" />
      </div>
    </FileContextMenu>
  )
}

export default React.memo(FileCard)
