import { useCallback, useEffect, useRef, useState } from "react"
import { ChevronLeft, ChevronRight, Maximize, ZoomIn, ZoomOut } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { OfficeLoading } from "./OfficeLoading"
import type { OfficeViewProps } from "./types"
import { MAX_SCALE, MIN_SCALE, useFitZoom } from "./useFitZoom"

/**
 * Renders a `.pptx` slide-by-slide onto a `<canvas>` via `pptxviewjs`
 * (lazy-loaded). pptxviewjs sizes its render from the canvas's
 * `getBoundingClientRect`, so the canvas must already have a non-zero display
 * width — we give it `w-full` and wait one frame for layout before rendering.
 * Canvas output means text isn't selectable and animations aren't reproduced
 * (the inherent limit of client-side pptx). Failures are logged (to diagnose
 * unsupported decks) and bubble through `onError` to the text fallback.
 */
export function PptxView({ data, onError }: OfficeViewProps) {
  const { t } = useTranslation()
  const outerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const viewerRef = useRef<import("pptxviewjs").PPTXViewer | null>(null)
  const [loading, setLoading] = useState(true)
  const [count, setCount] = useState(0)
  const [current, setCurrent] = useState(0)
  const [busy, setBusy] = useState(false)

  const measure = useCallback(() => canvasRef.current?.offsetWidth ?? 0, [])
  const { scale, fitMode, zoomIn, zoomOut, fitWidth, onContentReady } = useFitZoom(outerRef, measure)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    void (async () => {
      try {
        const { PPTXViewer } = await import("pptxviewjs")
        if (cancelled || !canvasRef.current) return
        // Let the canvas get a real layout width first — pptxviewjs reads it via
        // getBoundingClientRect and renders blank/0 if measured at 0×0.
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
        if (cancelled || !canvasRef.current) return
        const viewer = new PPTXViewer({ canvas: canvasRef.current, slideSizeMode: "fit" })
        await viewer.loadFile(data)
        if (cancelled) {
          viewer.destroy()
          return
        }
        await viewer.render()
        if (cancelled) {
          viewer.destroy()
          return
        }
        viewerRef.current = viewer
        setCount(viewer.getSlideCount())
        setCurrent(viewer.getCurrentSlideIndex())
        setLoading(false)
        onContentReady()
      } catch (e) {
        logger.error(
          "ui",
          "PptxView::render",
          `pptxviewjs render failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
          e instanceof Error ? { stack: e.stack } : { value: String(e) },
        )
        if (!cancelled) onError(e)
      }
    })()
    return () => {
      cancelled = true
      try {
        viewerRef.current?.destroy()
      } catch {
        /* ignore teardown errors */
      }
      viewerRef.current = null
    }
  }, [data, onError, onContentReady])

  const go = async (dir: -1 | 1) => {
    const v = viewerRef.current
    if (!v || busy) return
    setBusy(true)
    try {
      await (dir === 1 ? v.nextSlide() : v.previousSlide())
      setCurrent(v.getCurrentSlideIndex())
    } catch (e) {
      // A transient per-slide navigation hiccup must NOT tear the whole deck
      // down to the text fallback (that's only for an initial-render failure) —
      // log it and stay on the current slide.
      logger.error(
        "ui",
        "PptxView::navigate",
        `slide navigation failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div ref={outerRef} className="relative flex-1 overflow-auto bg-muted/30 p-3">
        {loading && (
          <div className="absolute inset-0 z-10 flex items-start justify-center bg-background/60">
            <OfficeLoading />
          </div>
        )}
        <div className="w-full" style={{ zoom: scale }}>
          <canvas ref={canvasRef} className="block w-full" />
        </div>
      </div>
      {!loading && (
        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-border px-3 py-1.5">
          <div className="flex items-center gap-1">
            {count > 1 && (
              <>
                <IconTip label={t("fileBrowser.prevSlide", "Previous slide")}>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    disabled={busy || current <= 0}
                    onClick={() => void go(-1)}
                  >
                    <ChevronLeft className="h-4 w-4" />
                  </Button>
                </IconTip>
                <span className="text-xs tabular-nums text-muted-foreground">
                  {current + 1} / {count}
                </span>
                <IconTip label={t("fileBrowser.nextSlide", "Next slide")}>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    disabled={busy || current >= count - 1}
                    onClick={() => void go(1)}
                  >
                    <ChevronRight className="h-4 w-4" />
                  </Button>
                </IconTip>
              </>
            )}
          </div>
          <div className="flex items-center gap-1">
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
        </div>
      )}
    </div>
  )
}
