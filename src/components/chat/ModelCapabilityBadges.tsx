import { Image, Video } from "lucide-react"
import { useTranslation } from "react-i18next"

interface ModelCapabilityBadgesProps {
  inputTypes?: string[]
  className?: string
}

export default function ModelCapabilityBadges({
  inputTypes = [],
  className,
}: ModelCapabilityBadgesProps) {
  const { t } = useTranslation()
  const supportsImage = inputTypes.includes("image")
  const supportsVideo = inputTypes.includes("video")
  const imageLabel = t("model.supportsImageMultimodal")
  const videoLabel = t("model.supportsVideoMultimodal")

  if (!supportsImage && !supportsVideo) return null

  return (
    <span className={`inline-flex shrink-0 items-center gap-1 ${className ?? ""}`}>
      {supportsImage && (
        <span
          className="inline-flex h-4 w-4 items-center justify-center text-emerald-600 dark:text-emerald-400"
          aria-label={imageLabel}
          data-ha-tip={imageLabel}
        >
          <Image className="h-3 w-3" aria-hidden="true" />
        </span>
      )}
      {supportsVideo && (
        <span
          className="inline-flex h-4 w-4 items-center justify-center text-emerald-600 dark:text-emerald-400"
          aria-label={videoLabel}
          data-ha-tip={videoLabel}
        >
          <Video className="h-3 w-3" aria-hidden="true" />
        </span>
      )}
    </span>
  )
}
