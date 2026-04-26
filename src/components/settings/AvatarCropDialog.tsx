import { useState, useCallback } from "react"
import { logger } from "@/lib/logger"
import Cropper from "react-easy-crop"
import type { Area } from "react-easy-crop"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/components/ui/alert-dialog"
import { Slider } from "@/components/ui/slider"
import { useTranslation } from "react-i18next"

interface AvatarCropDialogProps {
  /** Image source URL (convertFileSrc or data URL) */
  imageSrc: string
  /** Called with the cropped image Blob on confirm */
  onConfirm: (blob: Blob) => void
  /** Called on cancel */
  onCancel: () => void
  /** Whether the dialog is open */
  open: boolean
}

/**
 * Create a cropped image from a canvas.
 * Crops the image to a square region defined by `croppedAreaPixels`.
 */
async function getCroppedBlob(imageSrc: string, croppedAreaPixels: Area): Promise<Blob> {
  const image = await createImage(imageSrc)
  const canvas = document.createElement("canvas")
  const ctx = canvas.getContext("2d")!

  // Output size (match crop area pixels)
  const size = Math.min(croppedAreaPixels.width, croppedAreaPixels.height)
  // Cap output to 512px max for avatars
  const outputSize = Math.min(size, 512)
  canvas.width = outputSize
  canvas.height = outputSize

  ctx.drawImage(
    image,
    croppedAreaPixels.x,
    croppedAreaPixels.y,
    croppedAreaPixels.width,
    croppedAreaPixels.height,
    0,
    0,
    outputSize,
    outputSize,
  )

  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob)
      else reject(new Error("Canvas toBlob failed"))
    }, "image/png")
  })
}

function createImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.addEventListener("load", () => resolve(img))
    img.addEventListener("error", (e) => reject(e))
    img.crossOrigin = "anonymous"
    img.src = url
  })
}

export function AvatarCropDialog({ imageSrc, onConfirm, onCancel, open }: AvatarCropDialogProps) {
  const { t } = useTranslation()
  const [crop, setCrop] = useState({ x: 0, y: 0 })
  const [zoom, setZoom] = useState(1)
  const [croppedAreaPixels, setCroppedAreaPixels] = useState<Area | null>(null)
  const [processing, setProcessing] = useState(false)

  const onCropComplete = useCallback((_: Area, croppedPixels: Area) => {
    setCroppedAreaPixels(croppedPixels)
  }, [])

  const handleConfirm = async () => {
    if (!croppedAreaPixels) return
    setProcessing(true)
    try {
      const blob = await getCroppedBlob(imageSrc, croppedAreaPixels)
      onConfirm(blob)
    } catch (e) {
      logger.error("ui", "AvatarCropDialog::crop", "Crop failed", e)
    } finally {
      setProcessing(false)
    }
  }

  return (
    <AlertDialog open={open}>
      <AlertDialogContent className="max-w-md p-0 overflow-hidden">
        <AlertDialogHeader className="px-5 pt-5 pb-0">
          <AlertDialogTitle className="text-base">{t("settings.avatarCropTitle")}</AlertDialogTitle>
        </AlertDialogHeader>

        {/* Crop area */}
        <div className="relative w-full" style={{ height: 320 }}>
          <Cropper
            image={imageSrc}
            crop={crop}
            zoom={zoom}
            aspect={1}
            cropShape="round"
            showGrid={false}
            onCropChange={setCrop}
            onZoomChange={setZoom}
            onCropComplete={onCropComplete}
          />
        </div>

        {/* Zoom slider */}
        <div className="px-5 pb-2 flex items-center gap-3">
          <span className="text-xs text-muted-foreground shrink-0">{t("settings.avatarZoom")}</span>
          <Slider
            min={1}
            max={3}
            step={0.05}
            value={[zoom]}
            onValueChange={([v]) => setZoom(v)}
            className="flex-1"
          />
        </div>

        <AlertDialogFooter className="px-5 pb-4">
          <AlertDialogCancel onClick={onCancel} className="text-muted-foreground">
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction onClick={handleConfirm} disabled={processing}>
            {processing ? t("common.saving") : t("common.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
