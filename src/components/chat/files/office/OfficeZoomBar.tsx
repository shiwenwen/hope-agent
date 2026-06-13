import { Maximize, ZoomIn, ZoomOut } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { MAX_SCALE, MIN_SCALE } from "./useFitZoom"

interface OfficeZoomBarProps {
  scale: number
  fitMode: boolean
  zoomIn: () => void
  zoomOut: () => void
  fitWidth: () => void
}

/** Shared bottom toolbar for office rich previews: zoom out / % / zoom in / fit. */
export function OfficeZoomBar({ scale, fitMode, zoomIn, zoomOut, fitWidth }: OfficeZoomBarProps) {
  const { t } = useTranslation()
  return (
    <div className="flex shrink-0 items-center justify-center gap-1 border-t border-border px-3 py-1.5">
      <IconTip label={t("fileBrowser.zoomOut", "Zoom out")}>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          disabled={scale <= MIN_SCALE}
          onClick={zoomOut}
        >
          <ZoomOut className="h-4 w-4" />
        </Button>
      </IconTip>
      <span className="min-w-[3rem] text-center text-xs tabular-nums text-muted-foreground">
        {Math.round(scale * 100)}%
      </span>
      <IconTip label={t("fileBrowser.zoomIn", "Zoom in")}>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          disabled={scale >= MAX_SCALE}
          onClick={zoomIn}
        >
          <ZoomIn className="h-4 w-4" />
        </Button>
      </IconTip>
      <IconTip label={t("fileBrowser.fitWidth", "Fit width")}>
        <Button
          variant="ghost"
          size="icon"
          className={fitMode ? "h-7 w-7 text-foreground" : "h-7 w-7"}
          onClick={fitWidth}
        >
          <Maximize className="h-4 w-4" />
        </Button>
      </IconTip>
    </div>
  )
}
