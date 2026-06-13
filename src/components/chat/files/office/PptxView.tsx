import { useCallback, useEffect, useRef, useState } from "react"
import { ChevronLeft, ChevronRight, GalleryHorizontal, GalleryVertical } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { OfficeLoading } from "./OfficeLoading"
import { OfficeZoomControls } from "./OfficeZoomBar"
import type { OfficeViewProps } from "./types"
import { useFitZoom } from "./useFitZoom"

type PptxMode = "continuous" | "flip"

/** Lazy-render lookahead: slides within this margin of the viewport render. */
const PREFETCH_MARGIN = "400px 0px"
/** Placeholder aspect until the first slide reveals the deck's real ratio. */
const DEFAULT_ASPECT = 16 / 9

/**
 * Renders a `.pptx` via `pptxviewjs` (lazy-loaded) in two switchable layouts:
 *
 * - **continuous** (default): every slide stacked vertically, each on its own
 *   `<canvas>`, lazily rendered as it scrolls near the viewport
 *   (`IntersectionObserver`) — scroll-to-read like a web page.
 * - **flip**: one slide on a single canvas with prev/next navigation.
 *
 * One `PPTXViewer` instance serves both via `renderSlide(i, canvas)`; render
 * calls are serialized through a promise chain because the viewer carries
 * internal per-slide state. Zoom is pure CSS (`zoom` on the stack) — pptxviewjs
 * rasterizes to the canvas's on-screen box, so the bitmap stays 1:1 with the
 * displayed size at any zoom (no double-scaling). Canvas output means text
 * isn't selectable and animations aren't reproduced (the inherent pptx limit);
 * an initial-render failure bubbles through `onError` to the text fallback.
 */
export function PptxView({ data, onError }: OfficeViewProps) {
  const { t } = useTranslation()
  const outerRef = useRef<HTMLDivElement>(null)
  const viewerRef = useRef<import("pptxviewjs").PPTXViewer | null>(null)
  const slideCanvasRefs = useRef<(HTMLCanvasElement | null)[]>([])
  const flipCanvasRef = useRef<HTMLCanvasElement | null>(null)
  // Slides already rasterized in the *current* layout (cleared on mode switch
  // since the canvases remount). Guards against duplicate IntersectionObserver
  // renders and lets re-entry into continuous skip already-painted slides.
  const renderedRef = useRef<Set<number>>(new Set())
  // Serialize renders — concurrent renderSlide() calls race the viewer's state.
  const renderChainRef = useRef<Promise<void>>(Promise.resolve())
  // Flip mode fits to width once per deck / mode-entry. Re-measuring on every
  // slide nav would re-sample the canvas width under the current CSS zoom and
  // corrupt the fit baseline; all slides share one size so the first fit holds.
  const flipFitDoneRef = useRef(false)

  const [mode, setMode] = useState<PptxMode>("continuous")
  const [count, setCount] = useState(0)
  const [current, setCurrent] = useState(0)
  const [slideAspect, setSlideAspect] = useState(DEFAULT_ASPECT)
  const [loading, setLoading] = useState(true)

  const measure = useCallback(
    () => (mode === "flip" ? flipCanvasRef.current : slideCanvasRefs.current[0])?.offsetWidth ?? 0,
    [mode],
  )
  const { scale, fitMode, zoomIn, zoomOut, fitWidth, onContentReady } = useFitZoom(outerRef, measure)

  const enqueue = useCallback((fn: () => Promise<void>) => {
    const next = renderChainRef.current.then(fn).catch((e) => {
      logger.error(
        "ui",
        "PptxView::render",
        `slide render failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
      )
    })
    renderChainRef.current = next
    return next
  }, [])

  // Decks use one slide size throughout — learn it from the first rasterized
  // canvas so the lazy placeholders reserve the right height.
  const captureAspect = useCallback((canvas: HTMLCanvasElement | null) => {
    if (!canvas || canvas.width <= 0 || canvas.height <= 0) return
    const aspect = canvas.width / canvas.height
    setSlideAspect((prev) => (Math.abs(aspect - prev) > 0.001 ? aspect : prev))
  }, [])

  // Load the deck once per file.
  useEffect(() => {
    let cancelled = false
    setLoading(true)
    renderedRef.current = new Set()
    renderChainRef.current = Promise.resolve()
    flipFitDoneRef.current = false
    void (async () => {
      try {
        const { PPTXViewer } = await import("pptxviewjs")
        if (cancelled) return
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
        if (cancelled) return
        const viewer = new PPTXViewer({ slideSizeMode: "fit" })
        await viewer.loadFile(data)
        if (cancelled) {
          viewer.destroy()
          return
        }
        viewerRef.current = viewer
        setCount(viewer.getSlideCount())
        setCurrent(viewer.getCurrentSlideIndex())
        setLoading(false)
      } catch (e) {
        logger.error(
          "ui",
          "PptxView::render",
          `pptxviewjs load failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
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
  }, [data, onError])

  const renderContinuousSlide = useCallback(
    (index: number, primary = false) =>
      enqueue(async () => {
        const viewer = viewerRef.current
        const canvas = slideCanvasRefs.current[index]
        if (!viewer || !canvas || renderedRef.current.has(index)) return
        try {
          await viewer.renderSlide(index, canvas)
        } catch (e) {
          logger.error(
            "ui",
            "PptxView::render",
            `slide ${index} render failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
          )
          // First-slide failure means the deck can't be rasterized at all —
          // fall back to text extraction (mirrors the original initial render).
          // A later slide failing just leaves that one blank.
          if (primary) onError(e)
          return
        }
        renderedRef.current.add(index)
        captureAspect(canvas)
        if (primary) requestAnimationFrame(() => onContentReady())
      }),
    [enqueue, captureAspect, onContentReady, onError],
  )

  // Continuous: eagerly render the first slide so the deck's aspect ratio and
  // fit-zoom are known immediately; the rest stream in via the observer below.
  useEffect(() => {
    if (loading || mode !== "continuous" || count === 0) return
    void renderContinuousSlide(0, true)
  }, [loading, mode, count, renderContinuousSlide])

  // Continuous: lazy-render slides as their placeholders approach the viewport.
  useEffect(() => {
    if (loading || mode !== "continuous" || count === 0) return
    const root = outerRef.current
    if (!root) return
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          const index = Number((entry.target as HTMLElement).dataset.slide)
          if (!Number.isNaN(index)) void renderContinuousSlide(index)
        }
      },
      { root, rootMargin: PREFETCH_MARGIN },
    )
    root.querySelectorAll<HTMLElement>("[data-slide]").forEach((el) => io.observe(el))
    return () => io.disconnect()
  }, [loading, mode, count, renderContinuousSlide])

  // Flip: (re)render the current slide whenever it changes (incl. on entering
  // flip mode, since `mode` is a dependency and the canvas remounts).
  useEffect(() => {
    if (loading || mode !== "flip") return
    void enqueue(async () => {
      const viewer = viewerRef.current
      const canvas = flipCanvasRef.current
      if (!viewer || !canvas) return
      await viewer.renderSlide(current, canvas)
      captureAspect(canvas)
      // Fit only on the first flip render after a deck load / mode switch —
      // never on slide nav (re-measuring under zoom would skew the baseline).
      // Resize re-fits via useFitZoom's ResizeObserver.
      if (!flipFitDoneRef.current) {
        flipFitDoneRef.current = true
        requestAnimationFrame(() => onContentReady())
      }
    })
  }, [loading, mode, current, enqueue, captureAspect, onContentReady])

  const toggleMode = useCallback(() => {
    // Canvases remount on switch, so forget what was painted in the old layout.
    renderedRef.current = new Set()
    renderChainRef.current = Promise.resolve()
    flipFitDoneRef.current = false
    setMode((m) => (m === "continuous" ? "flip" : "continuous"))
  }, [])

  const go = useCallback(
    (dir: -1 | 1) => setCurrent((c) => Math.min(count - 1, Math.max(0, c + dir))),
    [count],
  )

  return (
    <div className="flex h-full flex-col">
      <div ref={outerRef} className="relative flex-1 overflow-auto bg-muted/30 p-3">
        {loading && (
          <div className="absolute inset-0 z-10 flex items-start justify-center bg-background/60">
            <OfficeLoading />
          </div>
        )}
        {!loading && mode === "continuous" && (
          <div className="mx-auto flex w-full flex-col gap-3" style={{ zoom: scale }}>
            {Array.from({ length: count }, (_, i) => (
              <div
                key={i}
                data-slide={i}
                className="w-full overflow-hidden rounded-sm bg-white shadow-sm ring-1 ring-border/40"
                style={{ aspectRatio: slideAspect }}
              >
                <canvas
                  ref={(el) => {
                    slideCanvasRefs.current[i] = el
                  }}
                  className="block h-full w-full"
                />
              </div>
            ))}
          </div>
        )}
        {!loading && mode === "flip" && (
          <div className="w-full" style={{ zoom: scale }}>
            <canvas ref={flipCanvasRef} className="block w-full" />
          </div>
        )}
      </div>
      {!loading && (
        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-border px-3 py-1.5">
          <div className="flex items-center gap-1">
            <IconTip
              label={
                mode === "continuous"
                  ? t("fileBrowser.pptSwitchToPaged", "翻页模式")
                  : t("fileBrowser.pptSwitchToContinuous", "连续阅读")
              }
            >
              <Button variant="ghost" size="icon" className="h-7 w-7" onClick={toggleMode}>
                {mode === "continuous" ? (
                  <GalleryHorizontal className="h-4 w-4" />
                ) : (
                  <GalleryVertical className="h-4 w-4" />
                )}
              </Button>
            </IconTip>
            {mode === "flip" && count > 1 && (
              <>
                <IconTip label={t("fileBrowser.prevSlide", "Previous slide")}>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    disabled={current <= 0}
                    onClick={() => go(-1)}
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
                    disabled={current >= count - 1}
                    onClick={() => go(1)}
                  >
                    <ChevronRight className="h-4 w-4" />
                  </Button>
                </IconTip>
              </>
            )}
          </div>
          <div className="flex items-center gap-1">
            <OfficeZoomControls
              scale={scale}
              fitMode={fitMode}
              zoomIn={zoomIn}
              zoomOut={zoomOut}
              fitWidth={fitWidth}
            />
          </div>
        </div>
      )}
    </div>
  )
}
