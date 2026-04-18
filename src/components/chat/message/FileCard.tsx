import React, { useCallback } from "react"
import { useTranslation } from "react-i18next"
import {
  FileText,
  FileArchive,
  FileType,
  FileSpreadsheet,
  FileCode,
  FileAudio,
  FileVideo,
  FileImage,
  File as FileIcon,
  FolderOpen,
  Download,
} from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import type { MediaItem } from "@/types/chat"

/** Format a byte count into a short human string. */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`
}

type IconKey =
  | "image"
  | "audio"
  | "video"
  | "pdf"
  | "archive"
  | "spreadsheet"
  | "doc"
  | "code"
  | "file"

/** Pick the icon key for a given MIME (falls back to filename extension). */
function resolveIconKey(mime: string, name: string): IconKey {
  const mimeLower = mime.toLowerCase()
  if (mimeLower.startsWith("image/")) return "image"
  if (mimeLower.startsWith("audio/")) return "audio"
  if (mimeLower.startsWith("video/")) return "video"
  if (mimeLower === "application/pdf") return "pdf"
  if (
    mimeLower === "application/zip" ||
    mimeLower === "application/gzip" ||
    mimeLower === "application/x-7z-compressed" ||
    mimeLower === "application/vnd.rar" ||
    mimeLower === "application/x-tar"
  )
    return "archive"
  if (
    mimeLower === "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" ||
    mimeLower === "application/vnd.ms-excel" ||
    mimeLower === "text/csv"
  )
    return "spreadsheet"
  if (
    mimeLower === "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    mimeLower === "application/msword"
  )
    return "doc"
  if (
    mimeLower.startsWith("text/") ||
    mimeLower === "application/json" ||
    mimeLower === "application/xml" ||
    mimeLower === "application/javascript"
  )
    return "code"

  const ext = name.split(".").pop()?.toLowerCase()
  switch (ext) {
    case "pdf":
      return "pdf"
    case "zip":
    case "tar":
    case "gz":
    case "tgz":
    case "7z":
    case "rar":
      return "archive"
    case "xlsx":
    case "xls":
    case "csv":
      return "spreadsheet"
    case "doc":
    case "docx":
      return "doc"
    default:
      return "file"
  }
}

function FileMimeIcon({ mime, name, className }: { mime: string; name: string; className?: string }) {
  const key = resolveIconKey(mime, name)
  switch (key) {
    case "image":
      return <FileImage className={className} />
    case "audio":
      return <FileAudio className={className} />
    case "video":
      return <FileVideo className={className} />
    case "pdf":
      return <FileText className={className} />
    case "archive":
      return <FileArchive className={className} />
    case "spreadsheet":
      return <FileSpreadsheet className={className} />
    case "doc":
      return <FileType className={className} />
    case "code":
      return <FileCode className={className} />
    case "file":
    default:
      return <FileIcon className={className} />
  }
}

/** Downloadable file card rendered for `send_attachment` and any other tool
 *  that emits structured media items via the `__MEDIA_ITEMS__` prefix. */
function FileCard({ item }: { item: MediaItem }) {
  const { t } = useTranslation()
  const transport = getTransport()
  const canRevealLocal = transport.supportsLocalFileOps()

  const handleOpen = useCallback(async () => {
    try {
      await transport.openMedia(item)
    } catch (e) {
      logger.error("chat", "FileCard::open", "Failed to open attachment", e)
    }
  }, [item, transport])

  const handleReveal = useCallback(async () => {
    try {
      await transport.revealMedia(item)
    } catch (e) {
      logger.error("chat", "FileCard::reveal", "Failed to reveal attachment", e)
    }
  }, [item, transport])

  return (
    <div className="inline-flex items-center gap-2 max-w-sm rounded-md border border-border/50 bg-secondary/30 hover:bg-secondary/50 transition-colors px-2.5 py-1.5 text-xs">
      <FileMimeIcon
        mime={item.mimeType}
        name={item.name}
        className="h-4 w-4 shrink-0 text-muted-foreground"
      />
      <button
        type="button"
        onClick={handleOpen}
        className="flex flex-col items-start min-w-0 flex-1 text-left hover:text-foreground transition-colors"
      >
        <span className="truncate max-w-[240px] font-medium text-foreground/90">
          {item.name}
        </span>
        <span className="text-[10px] text-muted-foreground/70 tabular-nums">
          {formatBytes(item.sizeBytes)}
        </span>
      </button>
      <div className="flex items-center gap-0.5 shrink-0">
        <IconTip label={t("chat.openFile")}>
          <button
            type="button"
            onClick={handleOpen}
            className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
          >
            <Download className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        {canRevealLocal && (
          <IconTip label={t("chat.revealInFolder")}>
            <button
              type="button"
              onClick={handleReveal}
              className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
            >
              <FolderOpen className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
      </div>
    </div>
  )
}

export default React.memo(FileCard)
