import { useCallback, useRef, useMemo, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { ImagePlus, Paperclip, X } from "lucide-react"
import { useLightbox } from "@/components/common/ImageLightbox"

interface AttachmentPreviewProps {
  attachedFiles: File[]
  onRemoveFile: (index: number) => void
}

export function AttachmentPreview({ attachedFiles, onRemoveFile }: AttachmentPreviewProps) {
  const { openLightbox } = useLightbox()

  // Stable blob URLs with cleanup to prevent memory leaks
  const blobUrls = useMemo(
    () => attachedFiles.map((f) => (f.type.startsWith("image/") ? URL.createObjectURL(f) : "")),
    [attachedFiles],
  )
  useEffect(() => () => { blobUrls.forEach((u) => { if (u) URL.revokeObjectURL(u) }) }, [blobUrls])

  if (attachedFiles.length === 0) return null

  return (
    <div className="flex gap-2 px-3 pt-3 pb-1 flex-wrap">
      {attachedFiles.map((file, index) => (
        <div
          key={`${file.name}-${index}`}
          className="group relative flex items-center gap-1.5 bg-secondary rounded-lg px-2 py-1 text-xs text-foreground/80 border border-border/50 animate-in fade-in-0 slide-in-from-bottom-1 duration-150"
          style={{ animationDelay: `${index * 50}ms`, animationFillMode: "both" }}
        >
          {blobUrls[index] ? (
            <img
              src={blobUrls[index]}
              alt={file.name}
              className="h-8 w-8 rounded object-cover cursor-zoom-in"
              onClick={(e) => {
                e.stopPropagation()
                openLightbox(blobUrls[index], file.name)
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
      ))}
    </div>
  )
}

interface AttachmentButtonsProps {
  onAttachFiles: (files: File[]) => void
}

export default function AttachmentButtons({ onAttachFiles }: AttachmentButtonsProps) {
  const { t } = useTranslation()
  const imageInputRef = useRef<HTMLInputElement>(null)
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
