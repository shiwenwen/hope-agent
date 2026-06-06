import { List } from "lucide-react"
import { useCallback, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { IconTip } from "@/components/ui/tooltip"
import { useClickOutside } from "@/hooks/useClickOutside"

import { parseHeadings } from "./outline"

interface HeadingOutlineProps {
  content: string
  /** Jump the editor to a 1-based source line (via `revealTarget`). */
  onJump: (line: number) => void
}

/** Heading-outline navigator (WS9): a popover listing the note's `#` headings;
 *  clicking one scrolls the source editor to that line. */
export default function HeadingOutline({ content, onJump }: HeadingOutlineProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useClickOutside(
    ref,
    useCallback(() => setOpen(false), []),
  )
  const headings = useMemo(() => parseHeadings(content), [content])

  return (
    <div className="relative" ref={ref}>
      <IconTip label={t("knowledge.outline", "Outline")}>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <List className="h-4 w-4" />
        </button>
      </IconTip>
      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 max-h-80 w-64 overflow-auto rounded-xl border border-border/60 bg-popover/95 p-1.5 shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl duration-150 animate-in fade-in-0 zoom-in-95">
          {headings.length === 0 ? (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              {t("knowledge.outlineEmpty", "No headings")}
            </div>
          ) : (
            headings.map((h, i) => (
              <IconTip key={`${h.line}:${i}`} label={h.text || null} side="left">
                <button
                  type="button"
                  onClick={() => {
                    onJump(h.line)
                    setOpen(false)
                  }}
                  style={{ paddingLeft: `${(h.level - 1) * 12 + 8}px` }}
                  className="block w-full truncate rounded-md py-1 pr-2 text-left text-xs text-foreground/80 hover:bg-muted hover:text-foreground"
                >
                  {h.text || <span className="opacity-40">—</span>}
                </button>
              </IconTip>
            ))
          )}
        </div>
      )}
    </div>
  )
}
