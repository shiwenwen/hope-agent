import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import { Camera } from "lucide-react"

interface AvatarSectionProps {
  avatar: string | null | undefined
  cropSrc: string | null
  onAvatarPick: () => void
  onCropConfirm: (blob: Blob) => void
  onCropCancel: () => void
}

export default function AvatarSection({
  avatar,
  cropSrc,
  onAvatarPick,
  onCropConfirm,
  onCropCancel,
}: AvatarSectionProps) {
  const { t } = useTranslation()

  return (
    <>
      {/* ── Avatar ── */}
      <div
        className="flex flex-col items-center gap-2 py-4 cursor-pointer"
        onClick={onAvatarPick}
      >
        <div className="w-16 h-16 rounded-full bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors">
          {avatar ? (
            <img
              src={getTransport().resolveAssetUrl(avatar) ?? avatar}
              className="w-full h-full object-cover"
              alt=""
            />
          ) : (
            <Camera className="h-5 w-5 text-muted-foreground/40" />
          )}
        </div>
        <span className="text-xs text-muted-foreground">
          {t("settings.profileAvatarChange")}
        </span>
      </div>

      {/* Avatar crop dialog */}
      {cropSrc && (
        <AvatarCropDialog
          open={!!cropSrc}
          imageSrc={cropSrc}
          onConfirm={onCropConfirm}
          onCancel={onCropCancel}
        />
      )}
    </>
  )
}
