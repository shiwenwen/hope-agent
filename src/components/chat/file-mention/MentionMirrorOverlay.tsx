import { forwardRef, useMemo, type CSSProperties } from "react"
import { X } from "lucide-react"
import { segmentInput } from "./mentionTokens"

/**
 * Lays a transparent mirror over the textarea so we can paint chip
 * backgrounds and re-color `@path` text without touching the textarea (caret,
 * selection, and IME preview stay native that way).
 *
 * The overlay is on top (`z-10` + `pointer-events-none`); plain segments
 * stay `text-transparent` so the textarea's glyphs show through unchanged,
 * and only the chip segments render visible text — pixel-aligned over the
 * textarea's own characters in blue, replacing the foreground rendering.
 *
 * Layout contract: textarea and overlay must share padding, font,
 * line-height, and width so character grids coincide. `CHAT_INPUT_MIRROR_CLASS`
 * is the single source of truth — both must reference it.
 */

export const CHAT_INPUT_MIRROR_CLASS =
  "text-sm leading-[1.5] px-4 pt-3 pb-1 whitespace-pre-wrap break-words font-sans"

interface MentionMirrorOverlayProps {
  value: string
  /** Mirrors the textarea scrollTop. Updated via parent in onScroll. */
  scrollTop: number
  /** Active when working_dir is set; otherwise renders only plain text. */
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
        className="pointer-events-none absolute inset-0 z-10 overflow-hidden select-none"
      >
        <div className={`${CHAT_INPUT_MIRROR_CLASS} text-transparent`} style={style}>
          {segments.map((seg, i) => {
            if (seg.kind === "text") {
              return seg.text
            }
            return (
              <span
                key={`m-${i}`}
                // px-1.5 -mx-1.5: padding extends the chip background past
                // glyph edges; negative margin cancels the layout cost so the
                // mirror character grid stays aligned with the textarea below.
                className="relative inline-flex items-center rounded-md px-1.5 -mx-1.5 pointer-events-auto group/chip
                           bg-blue-500/15 text-blue-400
                           dark:bg-blue-400/20 dark:text-blue-200"
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
                             h-3.5 w-3.5 rounded-full bg-blue-500 text-white items-center justify-center
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
