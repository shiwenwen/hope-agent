import { useCallback, useRef, useMemo, useState } from "react"
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
import { Copy, Paperclip, X } from "lucide-react"
import { useLightbox } from "@/components/common/ImageLightbox"
import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import { FileContextMenu } from "@/components/chat/files/FileActionMenu"
import { useFileResource } from "@/components/chat/files/useFileResource"
import { useFileActionsContext } from "@/components/chat/files/fileActionsContext"
import { StagedFilePreviewPane } from "@/components/chat/files/StagedFilePreviewPane"
import { stagedFilePreviewTarget, type PreviewTarget } from "@/components/chat/files/useFilePreview"
import { formatBytes } from "@/lib/format"
import type { DraftAttachment } from "@/components/chat/files/types"
import { useObjectUrlLease } from "@/components/chat/files/useObjectUrlLease"
import { getPastedTextFileMeta, updatePastedTextAttachment } from "./pastedTextAttachment"
import { MEBIBYTE_BYTES, useFilesystemConfig } from "@/lib/filesystemConfig"

type StagedPreviewTarget = Extract<PreviewTarget, { kind: "clientDraft" }>

interface AttachmentPreviewProps {
  attachedFiles: DraftAttachment[]
  onRemoveFile: (index: number) => void
  onUpdateFile: (index: number, file: File) => void
}

export function AttachmentPreview({
  attachedFiles,
  onRemoveFile,
  onUpdateFile,
}: AttachmentPreviewProps) {
  const { t } = useTranslation()
  const { config: filesystemConfig } = useFilesystemConfig()
  const { openLightbox } = useLightbox()
  const fileActionsContext = useFileActionsContext()
  const ambientPreviewFile = fileActionsContext.onPreviewFile
  const [pastedTextPreview, setPastedTextPreview] = useState<{
    index: number
    fileName: string
    text: string
  } | null>(null)
  const [localFilePreview, setLocalFilePreview] = useState<StagedPreviewTarget | null>(null)

  const openFilePreview = useCallback(
    (target: PreviewTarget) => {
      if (ambientPreviewFile) {
        ambientPreviewFile(target)
      } else if (target.kind === "clientDraft") {
        setLocalFilePreview(target)
      }
    },
    [ambientPreviewFile],
  )

  const openPastedTextPreview = useCallback(
    async (file: File, index: number) => {
      if (file.size > filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES) {
        toast.error(
          t("fileEditor.tooLarge", "File exceeds the {{limit}} MiB edit limit", {
            limit: filesystemConfig.maxTextEditMb,
          }),
        )
        return
      }
      try {
        const text = new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer())
        setPastedTextPreview({ index, fileName: file.name, text })
      } catch {
        toast.error(t("fileEditor.notUtf8", "File is not valid UTF-8 text"))
      }
    },
    [filesystemConfig.maxTextEditMb, t],
  )

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
    const draft = attachedFiles[pastedTextPreview.index]
    if (!draft) return
    const nextFile = getPastedTextFileMeta(draft.file)
      ? updatePastedTextAttachment(draft.file, pastedTextPreview.text)
      : new File([pastedTextPreview.text], draft.file.name, {
          type: draft.file.type || "text/plain",
          lastModified: Date.now(),
        })
    if (nextFile.size > filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES) {
      toast.error(
        t("fileEditor.tooLarge", "File exceeds the {{limit}} MiB edit limit", {
          limit: filesystemConfig.maxTextEditMb,
        }),
      )
      return
    }
    onUpdateFile(pastedTextPreview.index, nextFile)
    setPastedTextPreview(null)
    toast.success(t("common.saved"))
  }, [attachedFiles, filesystemConfig.maxTextEditMb, onUpdateFile, pastedTextPreview, t])

  return (
    <>
      <AnimatedCollapse open={attachedFiles.length > 0}>
        <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
          {attachedFiles.map((draft, index) => (
            <AttachmentPreviewItem
              key={draft.id}
              draft={draft}
              index={index}
              onRemoveFile={onRemoveFile}
              openLightbox={openLightbox}
              onOpenPastedText={openPastedTextPreview}
              onPreviewFile={openFilePreview}
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
              className="h-full min-h-0 resize-none overflow-auto whitespace-pre-wrap font-mono text-xs leading-relaxed"
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
      <Dialog
        open={!!localFilePreview}
        onOpenChange={(open) => {
          if (!open) setLocalFilePreview(null)
        }}
      >
        <DialogContent
          showCloseButton={false}
          className="h-[min(86vh,42rem)] max-h-[86vh] w-[calc(100vw-2rem)] max-w-4xl gap-0 overflow-hidden p-0 sm:w-[calc(100vw-4rem)]"
        >
          <DialogTitle className="sr-only">{localFilePreview?.draft.file.name}</DialogTitle>
          {localFilePreview ? (
            <StagedFilePreviewPane
              key={localFilePreview.previewId}
              target={localFilePreview}
              onReplaceFile={(file) => {
                const index = attachedFiles.findIndex(
                  (draft) => draft.id === localFilePreview.draft.id,
                )
                if (index >= 0) onUpdateFile(index, file)
              }}
              onClose={() => setLocalFilePreview(null)}
              className="h-full min-h-0"
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  )
}

function countLines(text: string): number {
  return text.length === 0 ? 0 : text.split(/\r\n|\r|\n/).length
}

function AttachmentPreviewItem({
  draft,
  index,
  onRemoveFile,
  openLightbox,
  onOpenPastedText,
  onPreviewFile,
}: {
  draft: DraftAttachment
  index: number
  onRemoveFile: (index: number) => void
  openLightbox: (src: string, alt?: string) => void
  onOpenPastedText: (file: File, index: number) => void
  onPreviewFile: (target: PreviewTarget) => void
}) {
  const { t } = useTranslation()
  const file = draft.file
  const blobUrl = useObjectUrlLease(file.type.startsWith("image/") ? file : null) ?? ""
  const pastedTextMeta = getPastedTextFileMeta(file)
  const canPreviewPastedText = !!pastedTextMeta
  const target = useMemo(() => stagedFilePreviewTarget(draft), [draft])
  const actionOverrides = useMemo(
    () => ({
      onPreviewFile: () => {
        if (blobUrl) openLightbox(blobUrl, file.name)
        else if (canPreviewPastedText) void onOpenPastedText(file, index)
        else onPreviewFile(target)
      },
      onEditFile: () => void onOpenPastedText(file, index),
      onRemoveFile: () => onRemoveFile(index),
    }),
    [
      blobUrl,
      canPreviewPastedText,
      file,
      index,
      onOpenPastedText,
      onPreviewFile,
      onRemoveFile,
      openLightbox,
      target,
    ],
  )
  const { primary, run } = useFileResource(target, actionOverrides)
  const sizeLabel = formatBytes(file.size, { fractionDigits: 1 })
  return (
    <FileContextMenu target={target} overrides={actionOverrides}>
      <div
        className="group relative flex items-center gap-1.5 rounded-lg border border-border/50 bg-secondary px-2 py-1 text-xs text-foreground/80 transition-colors animate-in fade-in-0 slide-in-from-bottom-1 duration-150 hover:bg-secondary/80"
        style={{ animationDelay: `${index * 50}ms`, animationFillMode: "both" }}
      >
        <button
          type="button"
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-left"
          aria-label={canPreviewPastedText ? t("chat.pastedTextPreviewOpen") : file.name}
          onClick={() => void run(primary)}
        >
          {blobUrl ? (
            <img src={blobUrl} alt={file.name} className="h-8 w-8 rounded object-cover" />
          ) : (
            <FileTypeIcon name={file.name} mime={file.type} className="h-5 w-5 shrink-0" />
          )}
          <span className="min-w-0 max-w-[180px]">
            <span className="block truncate font-medium text-foreground/90">{file.name}</span>
            <span className="block truncate text-[11px] leading-3 text-muted-foreground">
              {draft.status === "uploading"
                ? t("attachments.uploading", "Uploading…")
                : draft.status === "error"
                  ? draft.error || t("attachments.uploadFailedShort", "Upload failed")
                  : pastedTextMeta
                    ? `${t("chat.pastedTextAttachment")} · ${sizeLabel}`
                    : sizeLabel}
            </span>
          </span>
        </button>
        <button
          type="button"
          aria-label={t("fileActions.remove")}
          className="ml-0.5 rounded p-0.5 text-muted-foreground transition-colors hover:bg-background/60 hover:text-foreground"
          onClick={() => void run("remove")}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    </FileContextMenu>
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
        className="ha-focus-item flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground"
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
