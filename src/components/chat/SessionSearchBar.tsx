import { useState, useEffect, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { ChevronUp, ChevronDown, Loader2, Search, X } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { renderHighlightedSnippet } from "@/lib/highlight"
import { IconTip } from "@/components/ui/tooltip"
import type { SessionSearchResult } from "@/types/chat"

interface SessionSearchBarProps {
  sessionId: string
  /** Called with the target message id whenever the user navigates to a
   *  new match (debounced search completion, arrow keys, Enter). */
  onJumpTo: (messageId: number) => void
  onClose: () => void
  /** Incremented by the parent when Cmd/Ctrl+F is pressed while the bar is
   *  already open, to re-focus the input. */
  focusSignal?: number
}

const ICON_BUTTON_CLASS =
  "shrink-0 p-0.5 rounded text-muted-foreground hover:text-foreground hover:bg-secondary disabled:opacity-40 disabled:pointer-events-none transition-colors"

/**
 * In-session "find in page" search bar. Non-persistent — mounted only
 * while open. Delegates scroll-and-pulse behaviour to `MessageList` via
 * `onJumpTo` → `pendingScrollTarget`.
 */
export default function SessionSearchBar({
  sessionId,
  onJumpTo,
  onClose,
  focusSignal,
}: SessionSearchBarProps) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  // Sorted by messageId asc so ↑/↓ map to earlier/later in the conversation;
  // FTS5 returns rows by relevance rank which is unintuitive for navigation.
  const [sortedResults, setSortedResults] = useState<SessionSearchResult[]>([])
  const [currentIndex, setCurrentIndex] = useState(0)
  const [searching, setSearching] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  // Latest onJumpTo held in a ref so effects don't re-run on parent rerenders.
  const onJumpToRef = useRef(onJumpTo)
  useEffect(() => {
    onJumpToRef.current = onJumpTo
  }, [onJumpTo])

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [focusSignal])

  useEffect(() => {
    const q = query.trim()
    if (!q) {
      setSortedResults([])
      setSearching(false)
      return
    }
    setSearching(true)
    const timer = setTimeout(async () => {
      try {
        const list = await getTransport().call<SessionSearchResult[]>(
          "search_session_messages_cmd",
          {
            sessionId,
            query: q,
            limit: 200,
          },
        )
        const next = (list ?? []).slice().sort((a, b) => a.messageId - b.messageId)
        setSortedResults(next)
        setCurrentIndex(0)
      } catch (err) {
        logger.error("chat", "SessionSearchBar::search", "search failed", err)
        setSortedResults([])
      } finally {
        setSearching(false)
      }
    }, 250)
    return () => clearTimeout(timer)
  }, [query, sessionId])

  useEffect(() => {
    if (sortedResults.length === 0) return
    const safeIndex = Math.min(currentIndex, sortedResults.length - 1)
    const target = sortedResults[safeIndex]
    if (target) {
      onJumpToRef.current(target.messageId)
    }
  }, [currentIndex, sortedResults])

  const gotoNext = useCallback(() => {
    if (sortedResults.length === 0) return
    setCurrentIndex((i) => (i + 1) % sortedResults.length)
  }, [sortedResults.length])

  const gotoPrev = useCallback(() => {
    if (sortedResults.length === 0) return
    setCurrentIndex((i) => (i - 1 + sortedResults.length) % sortedResults.length)
  }, [sortedResults.length])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape") {
      e.preventDefault()
      onClose()
      return
    }
    if (e.key === "Enter") {
      e.preventDefault()
      if (e.shiftKey) gotoPrev()
      else gotoNext()
      return
    }
    if (e.key === "ArrowDown") {
      e.preventDefault()
      gotoNext()
      return
    }
    if (e.key === "ArrowUp") {
      e.preventDefault()
      gotoPrev()
    }
  }

  const hasQuery = query.trim().length > 0
  const total = sortedResults.length
  const displayCurrent = total === 0 ? 0 : currentIndex + 1
  const currentSnippet = sortedResults[currentIndex]?.contentSnippet ?? ""

  return (
    <div className="px-4 pt-1 pb-2 bg-background border-b border-border/30 animate-in fade-in slide-in-from-top-1 duration-150">
      <div className="flex items-center gap-2 rounded-md bg-muted/50 px-2.5 py-1.5 focus-within:bg-muted/80 transition-colors">
        <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("chat.sessionSearchPlaceholder") || ""}
          className="flex-1 min-w-0 bg-transparent border-none outline-none text-sm text-foreground placeholder:text-muted-foreground"
        />
        {searching && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />}
        {hasQuery && !searching && (
          <span
            className={cn(
              "shrink-0 text-[11px] tabular-nums",
              total === 0 ? "text-muted-foreground/70" : "text-muted-foreground",
            )}
          >
            {t("chat.sessionSearchCount", { current: displayCurrent, total })}
          </span>
        )}
        <IconTip label={t("chat.sessionSearchPrev")}>
          <button
            type="button"
            onClick={gotoPrev}
            disabled={total === 0}
            className={ICON_BUTTON_CLASS}
          >
            <ChevronUp className="h-4 w-4" />
          </button>
        </IconTip>
        <IconTip label={t("chat.sessionSearchNext")}>
          <button
            type="button"
            onClick={gotoNext}
            disabled={total === 0}
            className={ICON_BUTTON_CLASS}
          >
            <ChevronDown className="h-4 w-4" />
          </button>
        </IconTip>
        <IconTip label={t("chat.sessionSearchClose")}>
          <button type="button" onClick={onClose} className={ICON_BUTTON_CLASS}>
            <X className="h-4 w-4" />
          </button>
        </IconTip>
      </div>
      {hasQuery && !searching && total === 0 && (
        <div className="mt-1 px-1 text-[11px] text-muted-foreground/80">
          {t("chat.sessionSearchNoResults")}
        </div>
      )}
      {hasQuery && total > 0 && currentSnippet && (
        <div className="mt-1 px-1 text-[11px] text-muted-foreground/90 line-clamp-1 leading-snug break-words">
          {renderHighlightedSnippet(currentSnippet)}
        </div>
      )}
    </div>
  )
}
