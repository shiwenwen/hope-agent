/**
 * HelpFindBar — in-chapter Cmd+F find. Matches are located with a TreeWalker
 * over the rendered chapter's text nodes and highlighted through the CSS
 * Custom Highlight API (no DOM mutation, so React-owned markup is never
 * touched). On engines without `CSS.highlights` the bar still counts and
 * scrolls between matches — only the visual highlight is absent.
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronDown, ChevronUp, X } from "lucide-react"

import { SearchInput } from "@/components/ui/search-input"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

const HIGHLIGHT_ALL = "help-find"
const HIGHLIGHT_ACTIVE = "help-find-active"

interface HelpFindBarProps {
  containerRef: React.RefObject<HTMLElement | null>
  open: boolean
  onClose: () => void
  /** Content identity (language + chapter) — changing it re-runs the search
   *  against the new DOM. A language toggle replaces the whole body, so the
   *  key must include the language, not just the chapter number. */
  contentVersion: string
}

function collectMatches(root: HTMLElement, query: string): Range[] {
  const needle = query.toLowerCase()
  if (!needle) return []
  const ranges: Range[] = []
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? ""
    const lower = text.toLowerCase()
    let idx = lower.indexOf(needle)
    while (idx >= 0) {
      const range = document.createRange()
      range.setStart(node, idx)
      range.setEnd(node, idx + needle.length)
      ranges.push(range)
      idx = lower.indexOf(needle, idx + needle.length)
    }
  }
  return ranges
}

function applyHighlights(ranges: Range[], active: number): void {
  const registry = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights
  if (!registry || typeof Highlight === "undefined") return
  registry.delete(HIGHLIGHT_ALL)
  registry.delete(HIGHLIGHT_ACTIVE)
  if (ranges.length === 0) return
  registry.set(HIGHLIGHT_ALL, new Highlight(...ranges))
  const current = ranges[active]
  if (current) registry.set(HIGHLIGHT_ACTIVE, new Highlight(current))
}

function clearHighlights(): void {
  const registry = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights
  registry?.delete(HIGHLIGHT_ALL)
  registry?.delete(HIGHLIGHT_ACTIVE)
}

export default function HelpFindBar({
  containerRef,
  open,
  onClose,
  contentVersion,
}: HelpFindBarProps) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const [matchCount, setMatchCount] = useState(0)
  const [current, setCurrent] = useState(0)
  const rangesRef = useRef<Range[]>([])
  const inputRef = useRef<HTMLInputElement>(null)

  const runSearch = useCallback(
    (q: string, keepIndex = false) => {
      const root = containerRef.current
      const ranges = root && q.trim() ? collectMatches(root, q.trim()) : []
      rangesRef.current = ranges
      const next = keepIndex ? Math.min(current, Math.max(ranges.length - 1, 0)) : 0
      setMatchCount(ranges.length)
      setCurrent(next)
      applyHighlights(ranges, next)
      const range = ranges[next]
      if (range) {
        const el = range.startContainer.parentElement
        el?.scrollIntoView({ block: "center" })
      }
    },
    [containerRef, current],
  )

  const step = useCallback(
    (dir: 1 | -1) => {
      const ranges = rangesRef.current
      if (ranges.length === 0) return
      const next = (current + dir + ranges.length) % ranges.length
      setCurrent(next)
      applyHighlights(ranges, next)
      ranges[next]?.startContainer.parentElement?.scrollIntoView({ block: "center" })
    },
    [current],
  )

  // Open/close lifecycle: focus + seed the search ONLY when the bar opens;
  // clear everything when it closes. (Focus must not run on chapter switch —
  // it would steal the keyboard from chapter navigation.)
  useEffect(() => {
    if (!open) {
      clearHighlights()
      rangesRef.current = []
      setMatchCount(0)
      setCurrent(0)
      return
    }
    inputRef.current?.focus()
    inputRef.current?.select()
    runSearch(query)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  // Content switched under an open bar (chapter or language): re-anchor the
  // existing query against the new DOM without touching focus.
  useEffect(() => {
    if (open) runSearch(query)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contentVersion])

  useEffect(() => clearHighlights, [])

  if (!open) return null

  return (
    <div className="absolute right-4 top-2 z-20 flex items-center gap-1 rounded-lg border border-border-soft bg-surface-floating/95 p-1 shadow-panel backdrop-blur">
      <SearchInput
        ref={inputRef}
        value={query}
        placeholder={t("help.findPlaceholder")}
        className="h-7 w-48 text-sm"
        onChange={(e) => {
          setQuery(e.target.value)
          runSearch(e.target.value)
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault()
            step(e.shiftKey ? -1 : 1)
          } else if (e.key === "Escape") {
            e.preventDefault()
            onClose()
          }
        }}
      />
      <span
        className={cn(
          "min-w-10 text-center text-xs tabular-nums text-muted-foreground",
          matchCount === 0 && query.trim() && "text-destructive",
        )}
      >
        {matchCount === 0 ? "0/0" : `${current + 1}/${matchCount}`}
      </span>
      <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => step(-1)}>
        <ChevronUp className="h-3.5 w-3.5" />
      </Button>
      <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => step(1)}>
        <ChevronDown className="h-3.5 w-3.5" />
      </Button>
      <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onClose}>
        <X className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}
