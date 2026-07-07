import React, { useCallback, useEffect, useMemo, useState } from "react"
import { Archive, Download, FolderOpen, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"
import { useLightbox } from "@/components/common/ImageLightbox"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { MediaItem, MessageAttachment } from "@/types/chat"
import type {
  KbAttachment,
  KnowledgeBaseMeta,
  KnowledgeSource,
  KnowledgeSourceKind,
} from "@/types/knowledge"
import { FileMimeIcon } from "./FileCard"

interface UserAttachmentsProps {
  attachments?: MessageAttachment[]
  sessionId?: string | null
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

function sourceKindForAttachment(attachment: MessageAttachment): KnowledgeSourceKind {
  const mime = attachment.mimeType.toLowerCase()
  const name = attachment.name.toLowerCase()
  if (mime.startsWith("audio/") || /\.(mp3|m4a|wav|ogg|opus|flac)$/.test(name)) {
    return "audio_transcript"
  }
  if (mime.startsWith("video/") || /\.(mp4|mov|m4v|webm|mkv)$/.test(name)) {
    return "video_transcript"
  }
  if (
    attachment.kind === "image" ||
    mime.startsWith("image/") ||
    /\.(png|jpe?g|webp|gif|bmp|tiff?|heic)$/.test(name)
  ) {
    return "image_ocr"
  }
  if (mime === "application/pdf" || name.endsWith(".pdf")) return "pdf"
  if (
    mime === "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    name.endsWith(".docx")
  ) {
    return "docx"
  }
  if (mime === "text/markdown" || mime === "text/x-markdown" || /\.(md|markdown)$/.test(name)) {
    return "markdown"
  }
  return "text"
}

function mergeKnowledgeTargets(
  sessionKbs: KbAttachment[],
  allKbs: KnowledgeBaseMeta[],
): KnowledgeBaseMeta[] {
  const merged = new Map<string, KnowledgeBaseMeta>()
  for (const kb of sessionKbs) {
    merged.set(kb.id, {
      ...kb,
      noteCount: 0,
      external: Boolean(kb.rootDir),
    })
  }
  for (const kb of allKbs) {
    if (!merged.has(kb.id)) merged.set(kb.id, kb)
  }
  return [...merged.values()]
}

function UserAttachments({ attachments, sessionId }: UserAttachmentsProps) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const transport = getTransport()
  const canRevealLocal = transport.supportsLocalFileOps()
  const [archiveTarget, setArchiveTarget] = useState<MessageAttachment | null>(null)
  const [archiveKbs, setArchiveKbs] = useState<KnowledgeBaseMeta[]>([])
  const [archiveKbId, setArchiveKbId] = useState("")
  const [archiveLoading, setArchiveLoading] = useState(false)
  const [archiveSubmitting, setArchiveSubmitting] = useState(false)
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

  const openArchiveDialog = useCallback(
    async (attachment: MessageAttachment) => {
      if (!sessionId || !attachment.localPath) return
      setArchiveTarget(attachment)
      setArchiveKbs([])
      setArchiveKbId("")
      setArchiveLoading(true)
      try {
        const [sessionKbs, allKbs] = await Promise.all([
          transport
            .call<KbAttachment[]>("list_session_kbs_cmd", { sessionId })
            .catch(() => [] as KbAttachment[]),
          transport.call<KnowledgeBaseMeta[]>("list_kbs_cmd", { includeArchived: false }),
        ])
        const targets = mergeKnowledgeTargets(sessionKbs, allKbs)
        setArchiveKbs(targets)
        setArchiveKbId(targets[0]?.id ?? "")
        if (targets.length === 0) {
          toast.error(
            t("knowledge.sources.noKnowledgeBase", {
              defaultValue: "No knowledge space available",
            }),
          )
        }
      } catch (e) {
        logger.error(
          "chat",
          "UserAttachments::archiveTargets",
          "Failed to load knowledge targets",
          e,
        )
        toast.error(
          t("knowledge.sources.loadTargetsFailed", {
            defaultValue: "Failed to load knowledge spaces",
          }),
        )
      } finally {
        setArchiveLoading(false)
      }
    },
    [sessionId, t, transport],
  )

  const submitArchive = useCallback(async () => {
    if (!sessionId || !archiveTarget?.localPath || !archiveKbId) return
    setArchiveSubmitting(true)
    try {
      await transport.call<KnowledgeSource>("kb_source_import_session_attachment_cmd", {
        kbId: archiveKbId,
        input: {
          sessionId,
          path: archiveTarget.localPath,
          kind: sourceKindForAttachment(archiveTarget),
          title: archiveTarget.name,
          fileName: archiveTarget.name,
          mimeType: archiveTarget.mimeType,
        },
      })
      toast.success(
        t("knowledge.sources.archivedAttachment", {
          defaultValue: "Archived to knowledge sources",
        }),
      )
      setArchiveTarget(null)
    } catch (e) {
      logger.error("chat", "UserAttachments::archive", "Failed to archive attachment", e)
      toast.error(
        t("knowledge.sources.archiveAttachmentFailed", {
          defaultValue: "Failed to archive attachment",
        }),
      )
    } finally {
      setArchiveSubmitting(false)
    }
  }, [archiveKbId, archiveTarget, sessionId, t, transport])

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
            const canArchive = Boolean(sessionId && attachment.localPath)
            return (
              <span
                key={`${attachment.name}:${attachment.localPath ?? attachment.url ?? index}`}
                className="relative block max-w-full"
              >
                <button
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
                {canArchive && (
                  <IconTip
                    label={t("knowledge.sources.archiveAttachment", {
                      defaultValue: "Archive to knowledge",
                    })}
                  >
                    <button
                      type="button"
                      onClick={() => openArchiveDialog(attachment)}
                      className="absolute right-1 top-1 inline-flex rounded-md bg-background/80 p-1 text-foreground/70 shadow-sm transition-colors hover:bg-background hover:text-foreground"
                    >
                      <Archive className="h-3.5 w-3.5 text-muted-foreground" />
                    </button>
                  </IconTip>
                )}
              </span>
            )
          })}
        </div>
      )}
      {fileItems.length > 0 && (
        <div className="flex flex-wrap justify-end gap-1.5">
          {fileItems.map((attachment, index) => {
            const canOpen = !!(attachment.url || attachment.localPath || attachment.previewUrl)
            const canReveal = canRevealLocal && !!attachment.localPath
            const canArchive = Boolean(sessionId && attachment.localPath)
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
                {canArchive && (
                  <IconTip
                    label={t("knowledge.sources.archiveAttachment", {
                      defaultValue: "Archive to knowledge",
                    })}
                  >
                    <button
                      type="button"
                      onClick={() => openArchiveDialog(attachment)}
                      className="inline-flex bg-background/50 px-1 py-0.5 text-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground"
                    >
                      <Archive className="h-3 w-3 text-muted-foreground" />
                    </button>
                  </IconTip>
                )}
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
      <Dialog open={!!archiveTarget} onOpenChange={(open) => !open && setArchiveTarget(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>
              {t("knowledge.sources.archiveAttachment", { defaultValue: "Archive to knowledge" })}
            </DialogTitle>
          </DialogHeader>
          <div className="space-y-3">
            <div className="flex items-center gap-2 rounded-md border border-border/60 bg-secondary/30 px-2 py-1.5 text-sm">
              <FileMimeIcon
                mime={archiveTarget?.mimeType ?? "application/octet-stream"}
                name={archiveTarget?.name ?? ""}
                className="h-4 w-4 shrink-0 text-muted-foreground"
              />
              <span className="min-w-0 truncate">{archiveTarget?.name}</span>
            </div>
            <Select
              value={archiveKbId}
              onValueChange={setArchiveKbId}
              disabled={archiveLoading || archiveKbs.length === 0}
            >
              <SelectTrigger>
                <SelectValue
                  placeholder={t("knowledge.sources.selectKnowledgeBase", {
                    defaultValue: "Select knowledge space",
                  })}
                />
              </SelectTrigger>
              <SelectContent>
                {archiveKbs.map((kb) => (
                  <SelectItem key={kb.id} value={kb.id}>
                    {kb.emoji ? `${kb.emoji} ${kb.name}` : kb.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setArchiveTarget(null)}>
              {t("common.cancel", { defaultValue: "Cancel" })}
            </Button>
            <Button
              type="button"
              onClick={submitArchive}
              disabled={!archiveKbId || archiveLoading || archiveSubmitting}
            >
              {archiveSubmitting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("common.confirm", { defaultValue: "Confirm" })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
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
