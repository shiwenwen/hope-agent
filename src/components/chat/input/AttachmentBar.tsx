import { useEffect, useRef, useState, type ChangeEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { ImagePlus, Paperclip, X } from "lucide-react"
import { useLightbox } from "@/components/common/ImageLightbox"

interface AttachmentPreviewProps {
  attachedFiles: File[]
  onRemoveFile: (index: number) => void
}

interface BlobUrlEntry {
  file: File
  url: string
}

export function AttachmentPreview({ attachedFiles, onRemoveFile }: AttachmentPreviewProps) {
  const { openLightbox } = useLightbox()
  const blobUrlMapRef = useRef<Map<File, string>>(new Map())
  const [blobUrlEntries, setBlobUrlEntries] = useState<BlobUrlEntry[]>([])

  useEffect(() => {
    const currentFiles = new Set(attachedFiles)
    const blobUrlMap = blobUrlMapRef.current

    for (const [file, url] of blobUrlMap) {
      if (!currentFiles.has(file)) {
        URL.revokeObjectURL(url)
        blobUrlMap.delete(file)
      }
    }

    for (const file of attachedFiles) {
      if (file.type.startsWith("image/") && !blobUrlMap.has(file)) {
        blobUrlMap.set(file, URL.createObjectURL(file))
      }
    }

    setBlobUrlEntries(
      attachedFiles
        .map((file) => {
          const url = blobUrlMap.get(file)
          return url ? { file, url } : null
        })
        .filter((entry): entry is BlobUrlEntry => Boolean(entry)),
    )
  }, [attachedFiles])

  useEffect(
    () => () => {
      for (const url of blobUrlMapRef.current.values()) {
        URL.revokeObjectURL(url)
      }
      blobUrlMapRef.current.clear()
    },
    [],
  )

  if (attachedFiles.length === 0) return null

  return (
    <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
      {attachedFiles.map((file, index) => {
        const blobUrl = blobUrlEntries.find((entry) => entry.file === file)?.url ?? ""

        return (
          <div
            key={`${file.name}-${index}`}
            className="group relative flex items-center gap-1.5 bg-secondary rounded-lg px-2 py-1 text-xs text-foreground/80 border border-border/50 animate-in fade-in-0 slide-in-from-bottom-1 duration-150"
            style={{ animationDelay: `${index * 50}ms`, animationFillMode: "both" }}
          >
            {blobUrl ? (
              <img
                src={blobUrl}
                alt={file.name}
                className="h-8 w-8 rounded object-cover cursor-zoom-in"
                onClick={(e) => {
                  e.stopPropagation()
                  openLightbox(blobUrl, file.name)
                }}
              />
            ) : (
              <Paperclip className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            )}
            <span className="max-w-[120px] truncate">{file.name}</span>
            <button
              className="ml-0.5 text-muted-foreground hover:text-foreground transition-colors"
              onClick={() => onRemoveFile(index)}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )
      })}
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

  const handleFileSelect = (e: ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (files) {
      onAttachFiles(Array.from(files))
    }
    e.target.value = ""
    onPicked?.()
  }

  return (
    <>
      <button
        type="button"
        className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground"
        onClick={() => fileInputRef.current?.click()}
      >
        <Paperclip className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
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

export function AttachImageButton({ onAttachFiles }: AttachmentButtonsProps) {
  const { t } = useTranslation()
  const imageInputRef = useRef<HTMLInputElement>(null)

  const handleFileSelect = (e: ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (files) {
      onAttachFiles(Array.from(files))
    }
    e.target.value = ""
  }

  return (
    <>
      <IconTip label={t("chat.attachImage")}>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
          onClick={() => imageInputRef.current?.click()}
        >
          <ImagePlus className="h-4 w-4" />
        </Button>
      </IconTip>
      <input
        ref={imageInputRef}
        type="file"
        accept="image/*"
        multiple
        className="hidden"
        onChange={handleFileSelect}
      />
    </>
  )
}

export function AttachFileButton({ onAttachFiles }: AttachmentButtonsProps) {
  const { t } = useTranslation()
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (files) {
      onAttachFiles(Array.from(files))
    }
    e.target.value = ""
  }

  return (
    <>
      <IconTip label={t("chat.attachFile")}>
        <Button
          variant="ghost"
          size="icon"
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
  return (
    <>
      <AttachImageButton onAttachFiles={onAttachFiles} />
      <AttachFileButton onAttachFiles={onAttachFiles} />
    </>
  )
}
