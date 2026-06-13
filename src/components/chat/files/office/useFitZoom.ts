import { useCallback, useEffect, useRef, useState } from "react"

export const MIN_SCALE = 0.25
export const MAX_SCALE = 3
const ZOOM_STEP = 0.1

export interface FitZoom {
  /** Current scale (apply as CSS `zoom` on the content element). */
  scale: number
  /** True while auto-fitting to width (re-fits on resize). */
  fitMode: boolean
  zoomIn: () => void
  zoomOut: () => void
  fitWidth: () => void
  /**
   * Call once after the content has rendered AT zoom=1 (i.e. on first paint,
   * before any user zoom) — captures the content's natural width and fits it.
   */
  onContentReady: () => void
}

/**
 * Fit-to-width + manual zoom for a fixed-width rendered document (docx page,
 * xlsx table grid). Apply the returned `scale` as CSS `zoom` on the content
 * element so the layout box scales too (no empty gutters). `measure` must report
 * the content's natural width measured at zoom=1; it's only sampled via
 * `onContentReady`, then reused on resize so a later zoom doesn't skew the fit.
 */
export function useFitZoom<T extends HTMLElement>(
  outerRef: React.RefObject<T | null>,
  measure: () => number,
  fitPadding = 16,
): FitZoom {
  const contentWidthRef = useRef(0)
  // Latest `measure` without re-subscribing the ResizeObserver, so the recovery
  // path in computeFit can re-measure after a too-early first sample.
  const measureRef = useRef(measure)
  useEffect(() => {
    measureRef.current = measure
  }, [measure])
  const [scale, setScale] = useState(1)
  const [fitMode, setFitMode] = useState(true)

  const computeFit = useCallback(() => {
    const outer = outerRef.current
    if (!outer) return
    let cw = contentWidthRef.current
    if (cw <= 0) {
      // Recover from a first sample taken before layout settled (measure()==0):
      // re-measure now (the ResizeObserver also fires on attach, so this retries
      // until layout is ready). Safe because cw<=0 only before any successful
      // capture — i.e. before the user could have zoomed — so we never bake in a
      // zoomed width. Once captured (>0) we never re-measure, so a later zoom
      // can't skew the fit baseline.
      cw = measureRef.current()
      if (cw > 0) contentWidthRef.current = cw
    }
    if (cw <= 0) return
    const avail = outer.clientWidth - fitPadding
    setScale(avail > 0 ? Math.min(1, avail / cw) : 1)
  }, [outerRef, fitPadding])

  const onContentReady = useCallback(() => {
    contentWidthRef.current = measure()
    computeFit()
  }, [measure, computeFit])

  // Re-fit on container resize while in fit mode (observe also fires on attach).
  useEffect(() => {
    if (!fitMode) return
    const outer = outerRef.current
    if (!outer) return
    const ro = new ResizeObserver(() => computeFit())
    ro.observe(outer)
    return () => ro.disconnect()
  }, [fitMode, computeFit, outerRef])

  const round = (n: number) => Math.round(n * 100) / 100
  const zoomOut = useCallback(() => {
    setFitMode(false)
    setScale((s) => Math.max(MIN_SCALE, round(s - ZOOM_STEP)))
  }, [])
  const zoomIn = useCallback(() => {
    setFitMode(false)
    setScale((s) => Math.min(MAX_SCALE, round(s + ZOOM_STEP)))
  }, [])
  const fitWidth = useCallback(() => setFitMode(true), [])

  return { scale, fitMode, zoomIn, zoomOut, fitWidth, onContentReady }
}
