import React, { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { basename } from "@/lib/path"
import { FileMimeIcon } from "./FileCard"
import type { MessageFileAttachment } from "../chatUtils"
import { FileContextMenu, FileActionsMoreButton } from "@/components/chat/files/FileActionMenu"
import { useFileActions } from "@/components/chat/files/useFileActions"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"

interface FileAttachmentsProps {
  files: MessageFileAttachment[]
  sessionId?: string | null
}

function attachmentKey(file: MessageFileAttachment): string {
  return file.kind === "media"
    ? `media:${file.item.localPath || file.item.url || file.item.name}`
    : `path:${file.path}`
}

function attachmentName(file: MessageFileAttachment): string {
  return file.kind === "media" ? file.item.name : basename(file.path)
}

function attachmentMime(file: MessageFileAttachment): string {
  return file.kind === "media" ? file.item.mimeType : ""
}

function targetFor(file: MessageFileAttachment): PreviewTarget {
  return file.kind === "media"
    ? { kind: "media", item: file.item }
    : { kind: "path", path: file.path, name: basename(file.path) }
}

/** A single modified-file chip: primary click = preview/open/download by kind ×
 *  mode; right-click + ⋯ = the full action menu. */
function AttachmentRow({
  file,
  sessionId,
}: {
  file: MessageFileAttachment
  sessionId?: string | null
}) {
  const target = useMemo(() => targetFor(file), [file])
  const overrides = useMemo(() => ({ sessionId }), [sessionId])
  const { primary, run } = useFileActions(target, overrides)

  return (
    <FileContextMenu target={target} overrides={overrides}>
      <span className="inline-flex items-center gap-0.5 rounded-md bg-muted/50">
        <button
          type="button"
          onClick={() => run(primary)}
          className="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-[13px] text-foreground/70 transition-colors hover:bg-muted hover:text-foreground max-w-[240px]"
        >
          <FileMimeIcon
            mime={attachmentMime(file)}
            name={attachmentName(file)}
            className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
          />
          <span className="truncate">{attachmentName(file)}</span>
        </button>
        <FileActionsMoreButton target={target} overrides={overrides} />
      </span>
    </FileContextMenu>
  )
}

function FileAttachments({ files, sessionId }: FileAttachmentsProps) {
  const { t } = useTranslation()
  if (files.length === 0) return null

  return (
    <div className="mt-2 pt-2 border-t border-border/30">
      <div className="text-[10px] text-muted-foreground/60 mb-1">{t("chat.modifiedFiles")}</div>
      <div className="flex flex-wrap gap-1.5">
        {files.map((file) => (
          <AttachmentRow key={attachmentKey(file)} file={file} sessionId={sessionId} />
        ))}
      </div>
    </div>
  )
}

export default React.memo(FileAttachments)
