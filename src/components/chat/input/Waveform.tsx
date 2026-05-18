import { memo, useEffect, useRef, useState } from "react"

import { cn } from "@/lib/utils"

interface WaveformProps {
  /** Rolling RMS history (0-1). Oldest first, newest at the end. */
  levels: number[]
  /** Optional override for max bar height in px. Defaults to 28. */
  maxHeight?: number
  className?: string
}

/** Width + horizontal margin of a single bar, in px. Tuned to roughly
 * match the screenshot reference — thin 2px bars with 2px gaps. */
const BAR_WIDTH = 2
const BAR_GAP = 2

/**
 * Inline waveform — one vertical bar per "slot" across the full
 * container width. The number of slots is computed from the rendered
 * width via ResizeObserver so the wave always fills the available
 * space regardless of how wide the chat input is. Latest sample sits
 * on the right edge so the wave visually scrolls left.
 *
 * The component is memoized because parent re-renders on every level
 * tick (20 Hz). Without memo, the surrounding ChatInput tree would
 * reconcile far more than needed.
 */
function WaveformInner({ levels, maxHeight = 28, className }: WaveformProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [binCount, setBinCount] = useState(0)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const recalc = () => {
      const width = el.clientWidth
      // (BAR_WIDTH + BAR_GAP) per bin; trailing bin has no gap, so add
      // BAR_GAP back to width.
      const n = Math.max(0, Math.floor((width + BAR_GAP) / (BAR_WIDTH + BAR_GAP)))
      setBinCount((prev) => (prev === n ? prev : n))
    }
    recalc()
    const ro = new ResizeObserver(recalc)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Right-align the levels: newest level lands on the rightmost bar.
  // If the history is shorter than the available bins, left-pad with
  // zeros so the wave appears to scroll in from the right.
  const slots: number[] = new Array(binCount).fill(0)
  if (binCount > 0 && levels.length > 0) {
    const take = Math.min(binCount, levels.length)
    const srcStart = levels.length - take
    const dstStart = binCount - take
    for (let i = 0; i < take; i++) slots[dstStart + i] = levels[srcStart + i]
  }

  return (
    <div
      ref={containerRef}
      className={cn("flex h-7 flex-1 items-center overflow-hidden", className)}
      style={{ gap: `${BAR_GAP}px` }}
      aria-hidden
    >
      {slots.map((level, i) => {
        // sqrt curve emphasises mid-level signal: low-volume speech
        // still produces visible bars without saturating peaks. The RMS
        // values from the analyser rarely exceed ~0.4, so linear
        // mapping leaves the wave looking flat.
        const scaled = Math.sqrt(Math.min(1, level))
        const h = Math.max(2, Math.round(scaled * maxHeight))
        return (
          <span
            key={i}
            className="shrink-0 rounded-sm bg-foreground/70 transition-[height] duration-75"
            style={{ width: `${BAR_WIDTH}px`, height: `${h}px` }}
          />
        )
      })}
    </div>
  )
}

export const Waveform = memo(WaveformInner)
