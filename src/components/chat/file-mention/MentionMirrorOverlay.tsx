import { forwardRef, useMemo, type CSSProperties } from "react"
import { X } from "lucide-react"
import { segmentInput } from "./mentionTokens"

/**
 * Transparent mirror of the textarea content with chip backgrounds behind
 * each `@path` mention. Stacked under the textarea — text glyphs still come
 * from the textarea, only chip backdrop + X-button bleed through.
 *
 * Layout contract: this overlay and the textarea must share padding, font,
 * line-height, and width so character grids coincide. `CHAT_INPUT_MIRROR_CLASS`
 * is the single source of truth; both must reference it.
 *
 * Pointer-events: wrapper is `pointer-events-none` so click-to-position
 * caret reaches the textarea; only the X-button opts back into events.
 */

export const CHAT_INPUT_MIRROR_CLASS =
  "text-sm leading-[1.5] px-4 pt-3 pb-1 whitespace-pre-wrap break-words font-sans"

interface MentionMirrorOverlayProps {
  value: string
  /** Mirrors the textarea scrollTop. Updated via parent in onScroll. */
  scrollTop: number
  /** Active when working_dir is set; otherwise we render only plain text. */
  enabled: boolean
  /** X-button click handler. Receives the raw mention text including `@`. */
  onRemoveMention: (raw: string) => void
}

const MentionMirrorOverlay = forwardRef<HTMLDivElement, MentionMirrorOverlayProps>(
  function MentionMirrorOverlay(
    { value, scrollTop, enabled, onRemoveMention },
    ref,
  ) {
    const segments = useMemo(
      () =>
        enabled
          ? segmentInput(value)
          : [{ kind: "text" as const, text: value }],
      [value, enabled],
    )

    const style: CSSProperties = {
      transform: `translateY(${-scrollTop}px)`,
    }

    return (
      <div
        ref={ref}
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 overflow-hidden select-none"
      >
        <div className={`${CHAT_INPUT_MIRROR_CLASS} text-transparent`} style={style}>
          {segments.map((seg, i) => {
            if (seg.kind === "text") {
              return seg.text
            }
            return (
              <span
                key={`m-${i}`}
                // px-[1px] (and not px-0.5) intentionally: the chip background
                // needs to bleed slightly past glyph edges without shifting
                // textarea char positions away from this mirror's grid.
                className="relative inline-flex items-center bg-primary/12 text-primary/90 rounded-md px-[1px] group/chip pointer-events-auto"
              >
                {seg.raw}
                <button
                  type="button"
                  // onMouseDown (not onClick): textarea's blur-on-click would
                  // dismiss the popper before our handler fires.
                  onMouseDown={(e) => {
                    e.preventDefault()
                    e.stopPropagation()
                    onRemoveMention(seg.raw)
                  }}
                  className="absolute -right-1.5 -top-1.5 hidden group-hover/chip:flex
                             h-3.5 w-3.5 rounded-full bg-primary text-primary-foreground items-center justify-center
                             shadow ring-1 ring-background"
                  aria-label="Remove mention"
                  tabIndex={-1}
                >
                  <X className="h-2.5 w-2.5" />
                </button>
              </span>
            )
          })}
        </div>
      </div>
    )
  },
)

export default MentionMirrorOverlay
