import { useCallback, useEffect, useRef, useState } from "react"

import { logger } from "@/lib/logger"
import { OfficeLoading } from "./OfficeLoading"
import { OfficeZoomBar } from "./OfficeZoomBar"
import type { OfficeViewProps } from "./types"
import { useFitZoom } from "./useFitZoom"

/**
 * Renders a `.docx` into near-original HTML via `docx-preview` (lazy-loaded).
 * `renderAsync` injects its `<style>` into a dedicated hidden `styleRef` (not
 * `document.head`) and prefixes every class with `docx`, keeping the document's
 * CSS scoped to this preview instead of leaking into the app.
 *
 * The page is a fixed print width, so by default it's scaled to fit the panel
 * (see {@link useFitZoom}); the bottom bar offers manual zoom / fit-width.
 */
export function DocxView({ data, onError }: OfficeViewProps) {
  const outerRef = useRef<HTMLDivElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const styleRef = useRef<HTMLDivElement>(null)
  const [rendering, setRendering] = useState(true)

  // Natural width = the fixed page section width + the wrapper's gutters. The
  // `.docx-wrapper` is a block stretched to the container, so its own width is
  // useless for fitting — use the page section plus the wrapper padding.
  const measure = useCallback(() => {
    const body = bodyRef.current
    if (!body) return 0
    const page = body.querySelector<HTMLElement>(".docx")
    const wrapper = body.querySelector<HTMLElement>(".docx-wrapper")
    let cw = page?.offsetWidth ?? 0
    if (page && wrapper) {
      const cs = getComputedStyle(wrapper)
      cw += (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0)
    }
    return cw || body.scrollWidth
  }, [])

  const { scale, fitMode, zoomIn, zoomOut, fitWidth, onContentReady } = useFitZoom(outerRef, measure)

  useEffect(() => {
    let cancelled = false
    const body = bodyRef.current
    const style = styleRef.current
    if (!body || !style) return
    setRendering(true)
    void (async () => {
      try {
        const { renderAsync } = await import("docx-preview")
        if (cancelled) return
        body.replaceChildren()
        style.replaceChildren()
        await renderAsync(data, body, style, {
          className: "docx",
          inWrapper: true,
          breakPages: true,
        })
        if (cancelled) return
        setRendering(false)
        onContentReady()
      } catch (e) {
        if (!cancelled) {
          logger.error(
            "ui",
            "DocxView::render",
            `docx-preview render failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
          )
          onError(e)
        }
      }
    })()
    return () => {
      cancelled = true
      body.replaceChildren()
      style.replaceChildren()
    }
  }, [data, onError, onContentReady])

  return (
    <div className="flex h-full flex-col">
      <div ref={outerRef} className="relative flex-1 overflow-auto bg-muted/30">
        {rendering && (
          <div className="absolute inset-0 z-10 flex items-start justify-center bg-background/60">
            <OfficeLoading />
          </div>
        )}
        <div ref={styleRef} className="hidden" aria-hidden="true" />
        <div ref={bodyRef} style={{ zoom: scale }} />
      </div>
      {!rendering && (
        <OfficeZoomBar
          scale={scale}
          fitMode={fitMode}
          zoomIn={zoomIn}
          zoomOut={zoomOut}
          fitWidth={fitWidth}
        />
      )}
    </div>
  )
}
