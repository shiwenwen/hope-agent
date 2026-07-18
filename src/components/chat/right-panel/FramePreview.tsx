import { useTranslation } from "react-i18next"
import { Radio } from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"

export interface FramePreviewReplay {
  thumbJpegBase64: string
  index: number
  total: number
  onExit: () => void
}

interface FramePreviewProps {
  jpegBase64?: string | null
  alt: string
  /** Native frame dimensions — drives the docked aspect-ratio box. */
  widthPx?: number | null
  heightPx?: number | null
  emptyText: string
  errorText?: string | null
  metaText?: string | null
  /** Docked: fixed aspect-ratio block capped at 55% height. Floating: fill. */
  variant: "docked" | "floating"
  replay?: FramePreviewReplay | null
}

/**
 * Live JPEG mirror + replay overlay. In replay mode the live frame keeps
 * streaming into the store; this component only chooses what to display.
 */
export function FramePreview({
  jpegBase64,
  alt,
  widthPx,
  heightPx,
  emptyText,
  errorText,
  metaText,
  variant,
  replay,
}: FramePreviewProps) {
  const { t } = useTranslation()
  const displayed = replay ? replay.thumbJpegBase64 : jpegBase64
  const aspectRatio = widthPx && heightPx && heightPx > 0 ? widthPx / heightPx : 16 / 10

  return (
    <div
      className={cn(
        "relative bg-muted/30",
        variant === "docked"
          ? "w-full shrink-0 max-h-[55%] overflow-hidden"
          : "min-h-0 flex-1",
      )}
      style={variant === "docked" ? { aspectRatio } : undefined}
    >
      {/* Errors win over a cached stale frame — a frozen screenshot with no
          indicator would read as a live mirror while capture is failing. */}
      {errorText ? (
        <div className="flex h-full items-center justify-center px-6 text-center text-sm text-destructive">
          {errorText}
        </div>
      ) : displayed ? (
        <img
          src={`data:image/jpeg;base64,${displayed}`}
          alt={alt}
          className="h-full w-full select-none object-contain"
          draggable={false}
        />
      ) : (
        <div className="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
          {emptyText}
        </div>
      )}

      {replay && (
        <div className="absolute inset-x-0 top-2 flex justify-center">
          <div className="flex items-center gap-2 rounded-full bg-background/90 py-1 pl-3 pr-1 text-xs shadow-sm backdrop-blur">
            <span className="text-muted-foreground">
              {t("chat.controlPanel.history.replaying", {
                n: replay.index,
                total: replay.total,
              })}
            </span>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-6 gap-1 rounded-full px-2 text-[11px]"
              onClick={replay.onExit}
            >
              <Radio className="h-3 w-3" />
              {t("chat.controlPanel.backToLive")}
            </Button>
          </div>
        </div>
      )}

      {metaText && !replay && (
        <div className="pointer-events-none absolute bottom-1 right-2 rounded bg-background/70 px-1.5 py-0.5 text-[10px] text-muted-foreground backdrop-blur">
          {metaText}
        </div>
      )}
    </div>
  )
}
