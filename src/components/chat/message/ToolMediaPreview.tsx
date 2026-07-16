import { useContext, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { useTransport } from "@/lib/transport-provider"
import { useLightbox } from "@/components/common/ImageLightbox"
import FileCard from "@/components/chat/message/FileCard"
import { FileActionsMoreButton, FileContextMenu } from "@/components/chat/files/FileActionMenu"
import { useFileResource } from "@/components/chat/files/useFileResource"
import { MediaHoistContext } from "@/components/chat/message/mediaHoistContext"
import { extractImageToolMarkers } from "@/components/chat/message/imageToolMarkers"
import { cn } from "@/lib/utils"
import type { MediaItem, ToolCall } from "@/types/chat"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"

interface Props {
  tool: ToolCall
  className?: string
}

function ToolImageMedia({ item }: { item: MediaItem }) {
  const transport = useTransport()
  const { openLightbox } = useLightbox()
  const src = transport.resolveMediaUrl(item)
  const target = useMemo<PreviewTarget>(() => ({ kind: "media", item }), [item])
  const overrides = useMemo(
    () => ({ onPreviewFile: () => src && openLightbox(src, item.name) }),
    [item.name, openLightbox, src],
  )
  const { primary, run } = useFileResource(target, overrides)
  if (!src) return <FileCard item={item} />

  return (
    <FileContextMenu target={target} overrides={overrides}>
      <span className="group relative block">
        <button
          type="button"
          onClick={() => run(primary)}
          className="block cursor-zoom-in overflow-hidden rounded-lg border border-border/50 transition-colors hover:bg-secondary/40"
        >
          <img
            src={src}
            alt={item.name}
            className="max-h-72 max-w-72 bg-secondary/30 object-contain"
            loading="lazy"
          />
        </button>
        <FileActionsMoreButton
          target={target}
          overrides={overrides}
          className="absolute bottom-1 right-1 bg-background/80 opacity-0 shadow-sm group-hover:opacity-100 focus:opacity-100"
        />
      </span>
    </FileContextMenu>
  )
}

/**
 * Renders a tool's image / file attachments. Shared by `ToolCallBlock` and
 * `ToolCallGroup`'s `GroupItem` so grouped tools don't lose their preview.
 *
 * Two sources:
 *   - `mediaItems` — the post-migration generic attachment channel (any tool).
 *   - `mediaUrls`  — legacy `image_generate` absolute paths from old DB rows.
 *   - `image` tool file markers — preview-only inputs, not outbound media.
 */
export default function ToolMediaPreview({ tool, className }: Props) {
  const { t } = useTranslation()
  const transport = useTransport()
  // Suppressed when an ancestor (ProcessedBlockGroup) has hoisted media out to
  // render it once below the collapsed group — avoids double-rendering.
  const hoisted = useContext(MediaHoistContext)
  const hasMediaItems = !!tool.mediaItems?.length
  const hasLegacyUrls = !hasMediaItems && !!tool.mediaUrls?.length
  const imageMarkers =
    !hasMediaItems && !hasLegacyUrls && tool.name === "image"
      ? extractImageToolMarkers(tool.result)
      : []
  const hasImageMarkers = imageMarkers.length > 0
  if (hoisted || (!hasMediaItems && !hasLegacyUrls && !hasImageMarkers)) return null

  return (
    <div className={cn("mt-1.5 mb-1 flex flex-wrap gap-2", className)}>
      {hasMediaItems &&
        tool.mediaItems!.map((item, i) => {
          if (item.kind !== "image") return <FileCard key={i} item={item} />
          return <ToolImageMedia key={i} item={item} />
        })}
      {hasImageMarkers &&
        imageMarkers.map((item, i) => {
          const src = transport.resolveAssetUrl(item.path)
          if (!src) return null
          return (
            <ToolImageMedia
              key={i}
              item={{
                url: src,
                localPath: item.path,
                name: item.name,
                mimeType: "image/*",
                sizeBytes: 0,
                kind: "image",
              }}
            />
          )
        })}
      {hasLegacyUrls &&
        tool.mediaUrls!.map((url, i) => {
          // Legacy `mediaUrls` store absolute filesystem paths — route through
          // the transport so HTTP mode rewrites `~/.hope-agent/image_generate/*`
          // to `/api/generated-images/*`, and Tauri still wraps them via
          // `convertFileSrc`. Unknown / already-rewritten `/api/*` URLs get
          // normalised to `null` and we skip rendering rather than showing a
          // broken `<img>`.
          const src =
            url.startsWith("/") && !url.startsWith("/api/")
              ? (transport.resolveAssetUrl(url) ?? "")
              : ""
          if (!src) return null
          return (
            <ToolImageMedia
              key={i}
              item={{
                url: src,
                localPath: url,
                name: t("chat.generatedImageName", { index: i + 1 }),
                mimeType: "image/*",
                sizeBytes: 0,
                kind: "image",
              }}
            />
          )
        })}
    </div>
  )
}
