import { useState, useRef, useEffect } from "react"
import { AlertCircle } from "lucide-react"
import FallbackDetailsPopover from "@/components/chat/FallbackDetailsPopover"
import type { FallbackEvent } from "@/types/chat"

/** Inline banner that mimics the original blockquote style, with a clickable icon for details */
export default function FallbackBanner({ event }: { event: FallbackEvent }) {
  const [showPopover, setShowPopover] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  // Close popover on outside click
  useEffect(() => {
    if (!showPopover) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setShowPopover(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showPopover])

  const from = event.from_model ? ` ← ${event.from_model}` : ""
  const attempt = event.attempt && event.total ? ` [${event.attempt}/${event.total}]` : ""

  return (
    <div
      className="mb-2 border-l-2 border-muted-foreground/30 pl-3 py-0.5 text-sm text-muted-foreground italic"
      ref={ref}
    >
      <span className="relative inline-block">
        <button
          onClick={() => setShowPopover((v) => !v)}
          className="not-italic cursor-pointer hover:scale-110 transition-transform inline-block"
          title="Details"
        >
          <AlertCircle className="inline h-4 w-4 text-amber-500 -mt-0.5" />
        </button>
        <FallbackDetailsPopover event={event} open={showPopover} />
      </span>
      {` Fallback: ${event.model}${from}${attempt}`}
    </div>
  )
}
