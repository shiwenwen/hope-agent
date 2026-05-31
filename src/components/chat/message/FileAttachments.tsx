import React, { useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Download, FolderOpen } from "lucide-react"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { basename } from "@/lib/path"
import { FileMimeIcon } from "./FileCard"
import type { MessageFileAttachment } from "../chatUtils"

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

function attachmentPath(file: MessageFileAttachment): string | null {
  return file.kind === "media" ? (file.item.localPath ?? null) : file.path
}

function attachmentMime(file: MessageFileAttachment): string {
  return file.kind === "media" ? file.item.mimeType : ""
}

function FileAttachments({ files, sessionId }: FileAttachmentsProps) {
  const { t } = useTranslation()
  const transport = getTransport()
  const canRevealLocal = transport.supportsLocalFileOps()

  const handleOpen = useCallback(
    async (file: MessageFileAttachment) => {
      try {
        if (file.kind === "media") {
          await transport.openMedia(file.item)
        } else {
          await transport.openFilePath(file.path, { sessionId })
        }
      } catch (e) {
        logger.error("chat", "FileAttachments::open", "Failed to open file", e)
      }
    },
    [sessionId, transport],
  )

  const handleDownload = useCallback(
    async (file: MessageFileAttachment) => {
      try {
        if (file.kind === "media") {
          await transport.downloadMedia(file.item)
        } else {
          await transport.downloadFilePath(file.path, {
            sessionId,
            filename: basename(file.path),
          })
        }
      } catch (e) {
        logger.error("chat", "FileAttachments::download", "Failed to download file", e)
      }
    },
    [sessionId, transport],
  )

  const handleRevealInFolder = useCallback(
    async (file: MessageFileAttachment) => {
      try {
        if (file.kind === "media") {
          await transport.revealMedia(file.item)
        } else {
          await transport.call("reveal_in_folder", { path: file.path })
        }
      } catch (e) {
        logger.error("chat", "FileAttachments::reveal", "Failed to reveal in folder", e)
      }
    },
    [transport],
  )

  if (files.length === 0) return null

  return (
    <div className="mt-2 pt-2 border-t border-border/30">
      <div className="text-[10px] text-muted-foreground/60 mb-1">
        {t("chat.modifiedFiles")}
      </div>
      <div className="flex flex-wrap gap-1.5">
        {files.map((file) => (
          <span key={attachmentKey(file)} className="inline-flex items-center gap-0.5">
            <IconTip label={t("chat.openFile")}>
              <button
                onClick={() => handleOpen(file)}
                className="inline-flex items-center gap-1 pl-2 pr-1.5 py-0.5 rounded-l-md bg-muted/50 hover:bg-muted text-xs text-foreground/70 hover:text-foreground transition-colors max-w-[200px]"
              >
                <FileMimeIcon
                  mime={attachmentMime(file)}
                  name={attachmentName(file)}
                  className="h-3 w-3 shrink-0 text-muted-foreground"
                />
                <span className="truncate">{attachmentName(file)}</span>
              </button>
            </IconTip>
            <IconTip label={t("localModels.actions.download", { defaultValue: "Download" })}>
              <button
                onClick={() => handleDownload(file)}
                className={`inline-flex items-center px-1 py-0.5 bg-muted/50 hover:bg-muted text-foreground/70 hover:text-foreground transition-colors ${
                  canRevealLocal && attachmentPath(file) ? "" : "rounded-r-md"
                }`}
              >
                <Download className="h-3 w-3 text-muted-foreground" />
              </button>
            </IconTip>
            {canRevealLocal && attachmentPath(file) && (
              <IconTip label={t("chat.revealInFolder")}>
                <button
                  onClick={() => handleRevealInFolder(file)}
                  className="inline-flex items-center px-1 py-0.5 rounded-r-md bg-muted/50 hover:bg-muted text-foreground/70 hover:text-foreground transition-colors"
                >
                  <FolderOpen className="h-3 w-3 text-muted-foreground" />
                </button>
              </IconTip>
            )}
          </span>
        ))}
      </div>
    </div>
  )
}

export default React.memo(FileAttachments)
