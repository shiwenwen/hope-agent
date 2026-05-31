import React, { useCallback, useEffect, useMemo } from "react"
import { Download, FolderOpen } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useLightbox } from "@/components/common/ImageLightbox"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { MediaItem, MessageAttachment } from "@/types/chat"
import { FileMimeIcon } from "./FileCard"

interface UserAttachmentsProps {
  attachments?: MessageAttachment[]
}

interface PreviewUrlLease {
  count: number
  revokeTimer: ReturnType<typeof setTimeout> | null
}

const previewUrlLeases = new Map<string, PreviewUrlLease>()

function retainPreviewUrl(url: string): () => void {
  const existing = previewUrlLeases.get(url)
  const lease = existing ?? { count: 0, revokeTimer: null }
  if (lease.revokeTimer) {
    clearTimeout(lease.revokeTimer)
    lease.revokeTimer = null
  }
  lease.count += 1
  previewUrlLeases.set(url, lease)

  return () => {
    const current = previewUrlLeases.get(url)
    if (!current) return
    current.count -= 1
    if (current.count > 0) return

    current.revokeTimer = setTimeout(() => {
      const latest = previewUrlLeases.get(url)
      if (!latest || latest.count > 0) return
      URL.revokeObjectURL(url)
      previewUrlLeases.delete(url)
    }, 0)
  }
}

function mediaItemFromAttachment(attachment: MessageAttachment): MediaItem {
  return {
    url: attachment.url ?? "",
    name: attachment.name,
    mimeType: attachment.mimeType,
    sizeBytes: attachment.sizeBytes,
    kind: attachment.kind === "image" ? "image" : "file",
    ...(attachment.localPath ? { localPath: attachment.localPath } : {}),
  }
}

function resolveAttachmentPreview(attachment: MessageAttachment): string | null {
  if (attachment.previewUrl) return attachment.previewUrl
  return getTransport().resolveMediaUrl(mediaItemFromAttachment(attachment))
}

function UserAttachments({ attachments }: UserAttachmentsProps) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const transport = getTransport()
  const canRevealLocal = transport.supportsLocalFileOps()
  const items = useMemo(() => attachments ?? [], [attachments])
  const previewUrls = useMemo(
    () => items.map((item) => item.previewUrl).filter((url): url is string => Boolean(url)),
    [items],
  )

  useEffect(
    () => {
      const release = previewUrls.map(retainPreviewUrl)
      return () => {
        release.forEach((fn) => fn())
      }
    },
    [previewUrls],
  )

  const handleOpen = useCallback(
    async (attachment: MessageAttachment) => {
      try {
        const item = mediaItemFromAttachment(attachment)
        if (attachment.url || attachment.localPath) {
          await transport.openMedia(item)
        } else if (attachment.previewUrl) {
          openLightbox(attachment.previewUrl, attachment.name)
        }
      } catch (e) {
        logger.error("chat", "UserAttachments::open", "Failed to open attachment", e)
      }
    },
    [openLightbox, transport],
  )

  const handleDownload = useCallback(
    async (attachment: MessageAttachment) => {
      try {
        await transport.downloadMedia(mediaItemFromAttachment(attachment))
      } catch (e) {
        logger.error("chat", "UserAttachments::download", "Failed to download attachment", e)
      }
    },
    [transport],
  )

  const handleReveal = useCallback(
    async (attachment: MessageAttachment) => {
      try {
        await transport.revealMedia(mediaItemFromAttachment(attachment))
      } catch (e) {
        logger.error("chat", "UserAttachments::reveal", "Failed to reveal attachment", e)
      }
    },
    [transport],
  )

  if (items.length === 0) return null

  const imageItems = items.filter((item) => item.kind === "image")
  const imagePreviewItems = imageItems
    .map((attachment, index) => ({
      attachment,
      index,
      src: resolveAttachmentPreview(attachment),
    }))
    .filter((item): item is { attachment: MessageAttachment; index: number; src: string } =>
      Boolean(item.src),
    )
  const imageFallbackItems = imageItems.filter((attachment) => !resolveAttachmentPreview(attachment))
  const quoteItems = items.filter((item) => item.kind === "quote")
  const fileItems = [
    ...items.filter((item) => item.kind !== "image" && item.kind !== "quote"),
    ...imageFallbackItems,
  ]

  return (
    <div className="mb-2 flex flex-col gap-2">
      {imagePreviewItems.length > 0 && (
        <div className="flex flex-wrap justify-end gap-2">
          {imagePreviewItems.map(({ attachment, index, src }) => {
            return (
              <button
                key={`${attachment.name}:${attachment.localPath ?? attachment.url ?? index}`}
                type="button"
                onClick={() => openLightbox(src, attachment.name)}
                className="block max-w-full overflow-hidden rounded-lg border border-border/50 bg-secondary/30 transition-colors hover:border-primary/40 cursor-zoom-in"
              >
                <img
                  src={src}
                  alt={attachment.name}
                  className="max-h-72 max-w-72 object-contain"
                  loading="lazy"
                />
              </button>
            )
          })}
        </div>
      )}
      {fileItems.length > 0 && (
        <div className="flex flex-wrap justify-end gap-1.5">
          {fileItems.map((attachment, index) => {
            const canOpen = !!(attachment.url || attachment.localPath || attachment.previewUrl)
            const canReveal = canRevealLocal && !!attachment.localPath
            return (
              <span
                key={`${attachment.name}:${attachment.localPath ?? attachment.url ?? index}`}
                className="inline-flex max-w-full items-center gap-0.5"
              >
                <IconTip label={t("chat.openFile")}>
                  <button
                    type="button"
                    disabled={!canOpen}
                    onClick={() => handleOpen(attachment)}
                    className="inline-flex max-w-[220px] items-center gap-1 rounded-l-md bg-background/50 py-0.5 pl-2 pr-1.5 text-xs text-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground disabled:cursor-default disabled:opacity-70"
                  >
                    <FileMimeIcon
                      mime={attachment.mimeType}
                      name={attachment.name}
                      className="h-3 w-3 shrink-0 text-muted-foreground"
                    />
                    <span className="truncate">{attachment.name}</span>
                  </button>
                </IconTip>
                <IconTip label={t("localModels.actions.download", { defaultValue: "Download" })}>
                  <button
                    type="button"
                    disabled={!attachment.url && !attachment.localPath}
                    onClick={() => handleDownload(attachment)}
                    className={`inline-flex bg-background/50 px-1 py-0.5 text-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground disabled:cursor-default disabled:opacity-50 ${
                      canReveal ? "" : "rounded-r-md"
                    }`}
                  >
                    <Download className="h-3 w-3 text-muted-foreground" />
                  </button>
                </IconTip>
                {canReveal && (
                  <IconTip label={t("chat.revealInFolder")}>
                    <button
                      type="button"
                      onClick={() => handleReveal(attachment)}
                      className="inline-flex rounded-r-md bg-background/50 px-1 py-0.5 text-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground"
                    >
                      <FolderOpen className="h-3 w-3 text-muted-foreground" />
                    </button>
                  </IconTip>
                )}
              </span>
            )
          })}
        </div>
      )}
      {quoteItems.length > 0 && (
        <div className="flex flex-col items-end gap-1.5">
          {quoteItems.map((q, index) => (
            <div
              key={`${q.name}:${q.quoteLines ?? index}`}
              className="max-w-full overflow-hidden rounded-md border border-border/60 bg-secondary/30 text-left"
            >
              <div className="flex items-center gap-1.5 border-b border-border/40 px-2 py-1 text-xs text-muted-foreground">
                <FileMimeIcon
                  mime="text/plain"
                  name={q.name}
                  className="h-3 w-3 shrink-0"
                />
                <span className="truncate font-medium text-foreground/80">{q.name}</span>
                {q.quoteLines ? <span className="shrink-0">L{q.quoteLines}</span> : null}
              </div>
              {q.quoteContent ? (
                <pre className="max-h-40 max-w-[420px] overflow-auto px-2 py-1.5 text-xs leading-relaxed text-foreground/80">
                  {q.quoteContent}
                </pre>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

export default React.memo(UserAttachments)
