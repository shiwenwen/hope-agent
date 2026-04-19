import { convertFileSrc } from "@tauri-apps/api/core"
import { getTransport } from "@/lib/transport-provider"
import { useLightbox } from "@/components/common/ImageLightbox"
import FileCard from "@/components/chat/message/FileCard"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"

interface Props {
  tool: ToolCall
  className?: string
}

/**
 * Renders a tool's image / file attachments. Shared by `ToolCallBlock` and
 * `ToolCallGroup`'s `GroupItem` so grouped tools don't lose their preview.
 *
 * Two sources:
 *   - `mediaItems` — the post-migration generic attachment channel (any tool).
 *   - `mediaUrls`  — legacy `image_generate` absolute paths from old DB rows.
 */
export default function ToolMediaPreview({ tool, className }: Props) {
  const { openLightbox } = useLightbox()
  const hasMediaItems = !!tool.mediaItems?.length
  const hasLegacyUrls = !hasMediaItems && !!tool.mediaUrls?.length
  if (!hasMediaItems && !hasLegacyUrls) return null

  return (
    <div className={cn("mt-1.5 mb-1 flex flex-wrap gap-2", className)}>
      {hasMediaItems &&
        tool.mediaItems!.map((item, i) => {
          if (item.kind !== "image") return <FileCard key={i} item={item} />
          const src = getTransport().resolveMediaUrl(item)
          if (!src) return <FileCard key={i} item={item} />
          return (
            <button
              key={i}
              type="button"
              onClick={() => openLightbox(src, item.name)}
              className="block rounded-lg overflow-hidden border border-border/50 hover:border-primary/40 transition-colors cursor-zoom-in"
            >
              <img
                src={src}
                alt={item.name}
                className="max-w-72 max-h-72 object-contain bg-secondary/30"
                loading="lazy"
              />
            </button>
          )
        })}
      {hasLegacyUrls &&
        tool.mediaUrls!.map((url, i) => {
          const src =
            url.startsWith("/") && !url.startsWith("/api/") ? convertFileSrc(url) : ""
          if (!src) return null
          return (
            <button
              key={i}
              type="button"
              onClick={() => openLightbox(src, `Generated image ${i + 1}`)}
              className="block rounded-lg overflow-hidden border border-border/50 hover:border-primary/40 transition-colors cursor-zoom-in"
            >
              <img
                src={src}
                alt={`Generated image ${i + 1}`}
                className="max-w-72 max-h-72 object-contain bg-secondary/30"
                loading="lazy"
              />
            </button>
          )
        })}
    </div>
  )
}
