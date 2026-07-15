import React, { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronDown } from "lucide-react"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"
import { FileMimeIcon } from "./FileCard"
import type { MessageFileAttachment } from "../chatUtils"
import { FileContextMenu, FileActionsMoreButton } from "@/components/chat/files/FileActionMenu"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"

const DEFAULT_VISIBLE_FILE_ATTACHMENTS = 6

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

function targetFor(file: MessageFileAttachment, sessionId?: string | null): PreviewTarget {
  return file.kind === "media"
    ? { kind: "media", item: file.item }
    : {
        kind: "sessionPath",
        sessionId,
        path: file.path,
        name: basename(file.path),
        language: file.language ?? null,
      }
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
  const target = useMemo(() => targetFor(file, sessionId), [file, sessionId])
  const overrides = useMemo(() => ({ sessionId }), [sessionId])
  const { primary, run } = useFileResource(target, overrides)

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
  const [expanded, setExpanded] = useState(false)
  if (files.length === 0) return null

  const hasOverflow = files.length > DEFAULT_VISIBLE_FILE_ATTACHMENTS
  const visibleFiles = hasOverflow ? files.slice(0, DEFAULT_VISIBLE_FILE_ATTACHMENTS) : files
  const hiddenFiles = hasOverflow ? files.slice(DEFAULT_VISIBLE_FILE_ATTACHMENTS) : []

  return (
    <div className="mt-2 pt-2 border-t border-border/30">
      <div className="mb-1 flex items-center justify-between gap-2 text-[10px] text-muted-foreground/60">
        <span>{t("chat.modifiedFiles")}</span>
        {hasOverflow && (
          <span className="shrink-0 tabular-nums">
            {expanded ? files.length : visibleFiles.length}/{files.length}
          </span>
        )}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {visibleFiles.map((file) => (
          <AttachmentRow key={attachmentKey(file)} file={file} sessionId={sessionId} />
        ))}
      </div>
      {hasOverflow && (
        <>
          <AnimatedCollapse open={expanded} durationMs={180}>
            <div className="flex flex-wrap gap-1.5 pt-1.5">
              {hiddenFiles.map((file) => (
                <AttachmentRow key={attachmentKey(file)} file={file} sessionId={sessionId} />
              ))}
            </div>
          </AnimatedCollapse>
          <button
            type="button"
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
            className="mt-1.5 inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
          >
            <ChevronDown
              className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
            />
            <span>
              {expanded
                ? t("chat.showFewerFiles", { defaultValue: "Show fewer files" })
                : t("chat.showMoreFiles", {
                    count: hiddenFiles.length,
                    defaultValue: "Show {{count}} more files",
                  })}
            </span>
          </button>
        </>
      )}
    </div>
  )
}

export default React.memo(FileAttachments)
