import { useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { X, ExternalLink, Globe } from "lucide-react"
import { cn } from "@/lib/utils"

export interface UrlPreviewData {
  url: string
  finalUrl: string
  title?: string
  description?: string
  image?: string
  favicon?: string
  siteName?: string
  domain: string
}

interface UrlPreviewCardProps {
  data: UrlPreviewData | null // null = loading
  dismissible?: boolean
  onDismiss?: () => void
  className?: string
}

function SkeletonCard() {
  return (
    <div className="flex items-start gap-2.5 rounded-lg border border-border/60 bg-muted/30 p-2.5 animate-pulse">
      <div className="size-4 shrink-0 rounded bg-muted-foreground/20 mt-0.5" />
      <div className="flex-1 min-w-0 space-y-1.5">
        <div className="h-3 w-24 rounded bg-muted-foreground/20" />
        <div className="h-3.5 w-3/4 rounded bg-muted-foreground/20" />
        <div className="h-3 w-full rounded bg-muted-foreground/20" />
      </div>
    </div>
  )
}

export default function UrlPreviewCard({
  data,
  dismissible = false,
  onDismiss,
  className,
}: UrlPreviewCardProps) {
  const [faviconError, setFaviconError] = useState(false)
  const [imageError, setImageError] = useState(false)

  if (!data) return <SkeletonCard />

  // Don't render if there's no meaningful content
  if (!data.title && !data.description) return null

  const handleClick = () => {
    getTransport().call("open_url", { url: data.finalUrl || data.url })
  }

  const handleDismiss = (e: React.MouseEvent) => {
    e.stopPropagation()
    onDismiss?.()
  }

  return (
    <div
      className={cn(
        "group relative flex gap-2.5 rounded-lg border border-border/60 bg-muted/30 p-2.5 cursor-pointer",
        "hover:bg-muted/50 transition-colors duration-150",
        className,
      )}
      onClick={handleClick}
    >
      {/* Favicon */}
      <div className="shrink-0 pt-0.5">
        {data.favicon && !faviconError ? (
          <img
            src={data.favicon}
            alt=""
            className="size-4 rounded-sm object-contain"
            onError={() => setFaviconError(true)}
          />
        ) : (
          <Globe className="size-4 text-muted-foreground" />
        )}
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0 space-y-0.5">
        <div className="text-[11px] text-muted-foreground truncate">
          {data.siteName || data.domain}
        </div>
        {data.title && (
          <div className="text-sm font-medium leading-snug line-clamp-1">
            {data.title}
          </div>
        )}
        {data.description && (
          <div className="text-xs text-muted-foreground leading-relaxed line-clamp-2">
            {data.description}
          </div>
        )}
      </div>

      {/* OG Image thumbnail */}
      {data.image && !imageError && (
        <img
          src={data.image}
          alt=""
          className="size-14 shrink-0 rounded-md object-cover self-center"
          onError={() => setImageError(true)}
        />
      )}

      {/* Dismiss / Open link button */}
      {dismissible ? (
        <button
          onClick={handleDismiss}
          className="shrink-0 self-start p-0.5 rounded-md text-muted-foreground/60 opacity-0 group-hover:opacity-100 transition-opacity hover:text-destructive hover:bg-destructive/10"
        >
          <X className="size-3.5" />
        </button>
      ) : (
        <ExternalLink className="size-3 shrink-0 self-start mt-1 text-muted-foreground/50 opacity-0 group-hover:opacity-100 transition-opacity" />
      )}
    </div>
  )
}
