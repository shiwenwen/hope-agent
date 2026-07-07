import { useCallback, useRef, useMemo, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
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
import { getPastedTextFileMeta } from "./pastedTextAttachment"

interface AttachmentPreviewProps {
  attachedFiles: File[]
  onRemoveFile: (index: number) => void
}

export function AttachmentPreview({ attachedFiles, onRemoveFile }: AttachmentPreviewProps) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const [pastedTextPreview, setPastedTextPreview] = useState<{
    fileName: string
    text: string
    chars: number
    lines: number
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

  const openPastedTextPreview = useCallback(async (file: File) => {
    const text = await file.text()
    const meta = getPastedTextFileMeta(file)
    setPastedTextPreview({
      fileName: file.name,
      text,
      chars: meta?.charCount ?? text.length,
      lines: meta?.lineCount ?? countLines(text),
    })
  }, [])

  const copyPreviewText = useCallback(async () => {
    if (!pastedTextPreview) return
    await navigator.clipboard.writeText(pastedTextPreview.text)
    toast.success(t("chat.copied"))
  }, [pastedTextPreview, t])

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
        <DialogContent className="max-h-[86vh] max-w-3xl grid-rows-[auto_minmax(0,1fr)_auto]">
          <DialogHeader>
            <DialogTitle className="truncate">
              {pastedTextPreview?.fileName || t("chat.pastedTextPreviewTitle")}
            </DialogTitle>
            <DialogDescription>
              {pastedTextPreview
                ? t("chat.pastedTextPreviewMeta", {
                    chars: pastedTextPreview.chars.toLocaleString(),
                    lines: pastedTextPreview.lines.toLocaleString(),
                  })
                : null}
            </DialogDescription>
          </DialogHeader>
          <pre className="min-h-0 overflow-auto rounded-md border border-border/70 bg-secondary/35 p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground">
            {pastedTextPreview?.text}
          </pre>
          <DialogFooter className="gap-2 sm:gap-2">
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
              <Button type="button">{t("common.close")}</Button>
            </DialogClose>
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
  onOpenPastedText: (file: File) => void
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
            void onOpenPastedText(file)
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
