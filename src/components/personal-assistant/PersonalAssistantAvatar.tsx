import { useState } from "react"
import { Bot, Heart, Sparkles } from "lucide-react"
import { useTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"

export function PersonalAssistantAvatar({
  avatar,
  className,
}: {
  avatar?: string | null
  className?: string
}) {
  const transport = useTransport()
  const [failedSource, setFailedSource] = useState<string | null>(null)
  const preset = avatar?.startsWith("preset:") ? avatar.slice(7) : null
  const Icon = preset === "bot" ? Bot : preset === "heart" ? Heart : Sparkles
  const source = avatar && !preset ? transport.resolveAssetUrl(avatar) : null
  return (
    <span
      className={cn(
        "inline-flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-full bg-primary/10 text-primary",
        className,
      )}
    >
      {source && source !== failedSource ? (
        <img
          src={source}
          alt=""
          className="h-full w-full object-cover"
          onError={() => setFailedSource(source)}
        />
      ) : (
        <Icon className="h-1/2 w-1/2" aria-hidden="true" />
      )}
    </span>
  )
}
