import { useCallback, useRef, useMemo, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import { Copy, FileText, Paperclip, X } from "lucide-react"
import { useLightbox } from "@/components/common/ImageLightbox"
import { getPastedTextFileMeta, updatePastedTextAttachment } from "./pastedTextAttachment"

interface AttachmentPreviewProps {
  attachedFiles: File[]
  onRemoveFile: (index: number) => void
  onUpdateFile: (index: number, file: File) => void
}

export function AttachmentPreview({
  attachedFiles,
  onRemoveFile,
  onUpdateFile,
}: AttachmentPreviewProps) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const [pastedTextPreview, setPastedTextPreview] = useState<{
    index: number
    fileName: string
    text: string
  } | null>(null)

  // Stable blob URLs with cleanup to prevent memory leaks
  const blobUrls = useMemo(
    () => attachedFiles.map((f) => (f.type.startsWith("image/") ? URL.createObjectURL(f) : "")),
    [attachedFiles],
  )
  useEffect(
    () => () => {
      blobUrls.forEach((u) => {
        if (u) URL.revokeObjectURL(u)
      })
    },
    [blobUrls],
  )

  const openPastedTextPreview = useCallback(async (file: File, index: number) => {
    const text = await file.text()
    setPastedTextPreview({
      index,
      fileName: file.name,
      text,
    })
  }, [])

  const previewStats = pastedTextPreview
    ? {
        chars: pastedTextPreview.text.length,
        lines: countLines(pastedTextPreview.text),
      }
    : null

  const copyPreviewText = useCallback(async () => {
    if (!pastedTextPreview) return
    await navigator.clipboard.writeText(pastedTextPreview.text)
    toast.success(t("chat.copied"))
  }, [pastedTextPreview, t])

  const savePreviewText = useCallback(() => {
    if (!pastedTextPreview) return
    const file = attachedFiles[pastedTextPreview.index]
    if (!file) return
    onUpdateFile(pastedTextPreview.index, updatePastedTextAttachment(file, pastedTextPreview.text))
    setPastedTextPreview(null)
    toast.success(t("common.saved"))
  }, [attachedFiles, onUpdateFile, pastedTextPreview, t])

  return (
    <>
      <AnimatedCollapse open={attachedFiles.length > 0}>
        <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
          {attachedFiles.map((file, index) => (
            <AttachmentPreviewItem
              key={`${file.name}-${index}`}
              file={file}
              index={index}
              blobUrl={blobUrls[index] || ""}
              onRemoveFile={onRemoveFile}
              openLightbox={openLightbox}
              onOpenPastedText={openPastedTextPreview}
            />
          ))}
        </div>
      </AnimatedCollapse>
      <Dialog
        open={!!pastedTextPreview}
        onOpenChange={(open) => {
          if (!open) setPastedTextPreview(null)
        }}
      >
        <DialogContent className="flex h-[min(86vh,42rem)] max-h-[86vh] w-[calc(100vw-2rem)] max-w-4xl flex-col gap-0 overflow-hidden p-0 sm:w-[calc(100vw-4rem)]">
          <DialogHeader className="shrink-0 px-6 pt-6 pr-12">
            <DialogTitle className="truncate text-left">
              {pastedTextPreview?.fileName || t("chat.pastedTextPreviewTitle")}
            </DialogTitle>
            <DialogDescription className="text-left">
              {previewStats
                ? t("chat.pastedTextPreviewMeta", {
                    chars: previewStats.chars.toLocaleString(),
                    lines: previewStats.lines.toLocaleString(),
                  })
                : null}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 px-6 py-4">
            <Textarea
              aria-label={t("chat.pastedTextPreviewTitle")}
              spellCheck={false}
              value={pastedTextPreview?.text ?? ""}
              onChange={(event) =>
                setPastedTextPreview((preview) =>
                  preview ? { ...preview, text: event.target.value } : preview,
                )
              }
              className="h-full min-h-0 resize-none overflow-auto whitespace-pre-wrap border-border/70 bg-secondary/35 font-mono text-xs leading-relaxed shadow-none"
            />
          </div>
          <DialogFooter className="shrink-0 gap-2 border-t border-border/70 px-6 py-4 sm:gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => void copyPreviewText()}
              disabled={!pastedTextPreview}
            >
              <Copy className="h-4 w-4" />
              {t("chat.copy")}
            </Button>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                {t("common.cancel")}
              </Button>
            </DialogClose>
            <Button type="button" onClick={savePreviewText} disabled={!pastedTextPreview}>
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

function countLines(text: string): number {
  return text.length === 0 ? 0 : text.split(/\r\n|\r|\n/).length
}

function AttachmentPreviewItem({
  file,
  index,
  blobUrl,
  onRemoveFile,
  openLightbox,
  onOpenPastedText,
}: {
  file: File
  index: number
  blobUrl: string
  onRemoveFile: (index: number) => void
  openLightbox: (src: string, alt?: string) => void
  onOpenPastedText: (file: File, index: number) => void
}) {
  const { t } = useTranslation()
  const pastedTextMeta = getPastedTextFileMeta(file)
  const canPreviewPastedText = !!pastedTextMeta
  return (
    <div
      className="group relative flex items-center gap-1.5 bg-secondary rounded-lg px-2 py-1 text-xs text-foreground/80 border border-border/50 animate-in fade-in-0 slide-in-from-bottom-1 duration-150"
      style={{ animationDelay: `${index * 50}ms`, animationFillMode: "both" }}
    >
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left disabled:cursor-default"
        disabled={!blobUrl && !canPreviewPastedText}
        aria-label={canPreviewPastedText ? t("chat.pastedTextPreviewOpen") : file.name}
        onClick={() => {
          if (blobUrl) {
            openLightbox(blobUrl, file.name)
          } else if (canPreviewPastedText) {
            void onOpenPastedText(file, index)
          }
        }}
      >
        {blobUrl ? (
          <img src={blobUrl} alt={file.name} className="h-8 w-8 rounded object-cover" />
        ) : pastedTextMeta ? (
          <FileText className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
        ) : (
          <Paperclip className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
        )}
        <span className="min-w-0 max-w-[160px]">
          <span className="block truncate">{file.name}</span>
          {pastedTextMeta ? (
            <span className="block truncate text-[11px] leading-3 text-muted-foreground">
              {t("chat.pastedTextAttachment")}
            </span>
          ) : null}
        </span>
      </button>
      <button
        className="ml-0.5 text-muted-foreground hover:text-foreground transition-colors"
        onClick={() => onRemoveFile(index)}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}

interface AttachmentButtonsProps {
  onAttachFiles: (files: File[]) => void
}

interface AttachFilesMenuItemProps extends AttachmentButtonsProps {
  onPicked?: () => void
}

export function AttachFilesMenuItem({ onAttachFiles, onPicked }: AttachFilesMenuItemProps) {
  const { t } = useTranslation()
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files
      if (files) {
        onAttachFiles(Array.from(files))
      }
      e.target.value = ""
      onPicked?.()
    },
    [onAttachFiles, onPicked],
  )

  return (
    <>
      <button
        type="button"
        className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground"
        onClick={() => fileInputRef.current?.click()}
      >
        <Paperclip className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate">{t("chat.attachPhotosAndFiles")}</span>
      </button>
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={handleFileSelect}
      />
    </>
  )
}

export function AttachFilesButton({ onAttachFiles }: AttachmentButtonsProps) {
  const { t } = useTranslation()
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files
      if (files) {
        onAttachFiles(Array.from(files))
      }
      e.target.value = ""
    },
    [onAttachFiles],
  )

  return (
    <>
      <IconTip label={t("chat.attachPhotosAndFiles")}>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("chat.attachPhotosAndFiles")}
          className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
          onClick={() => fileInputRef.current?.click()}
        >
          <Paperclip className="h-4 w-4" />
        </Button>
      </IconTip>
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={handleFileSelect}
      />
    </>
  )
}

export default function AttachmentButtons({ onAttachFiles }: AttachmentButtonsProps) {
  return <AttachFilesButton onAttachFiles={onAttachFiles} />
}
