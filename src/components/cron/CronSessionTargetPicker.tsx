import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, ChevronDown, Loader2, MessageSquare } from "lucide-react"

import { FLAT_CONTROL_SURFACE_CLASS } from "@/components/ui/control-surface"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { SearchInput } from "@/components/ui/search-input"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { SessionMeta, SessionSearchResult } from "@/types/chat"

/** Ordinary chat that a scheduled task can append its occurrences to. */
export interface CronSessionTargetOption {
  id: string
  title?: string | null
}

const RECENT_LIMIT = 30
const SEARCH_LIMIT = 40
const SEARCH_DEBOUNCE_MS = 250

/**
 * Client-side mirror of the backend `is_regular_chat` gate. The preflight
 * remains the authority — this only keeps unusable chats out of the list so the
 * user never picks a target that is guaranteed to be blocked.
 */
function isSchedulableChat(session: SessionMeta): boolean {
  return (
    !session.isCron &&
    !session.parentSessionId &&
    !session.channelInfo &&
    !session.incognito &&
    !session.archivedAt &&
    (session.kind ?? "regular") === "regular"
  )
}

export default function CronSessionTargetPicker({
  value,
  onChange,
  disabled,
}: {
  value: CronSessionTargetOption | null
  onChange: (session: CronSessionTargetOption) => void
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [recent, setRecent] = useState<CronSessionTargetOption[] | null>(null)
  const [results, setResults] = useState<CronSessionTargetOption[] | null>(null)
  const [searching, setSearching] = useState(false)
  // Only the newest request may publish; a slow earlier query must not overwrite
  // the list the user is currently reading.
  const requestSeq = useRef(0)

  useEffect(() => {
    if (!open || recent) return
    let cancelled = false
    getTransport()
      .call<[SessionMeta[], number]>("list_sessions_cmd", {
        limit: RECENT_LIMIT,
        parentSession: false,
      })
      .then(([sessions]) => {
        if (cancelled) return
        setRecent(
          (sessions ?? [])
            .filter(isSchedulableChat)
            .map((session) => ({ id: session.id, title: session.title })),
        )
      })
      .catch((error) => {
        if (cancelled) return
        logger.error("cron", "CronSessionTargetPicker::recent", "listing chats failed", error)
        setRecent([])
      })
    return () => {
      cancelled = true
    }
  }, [open, recent])

  useEffect(() => {
    const trimmed = query.trim()
    if (!trimmed) {
      setResults(null)
      setSearching(false)
      return
    }
    setSearching(true)
    const seq = ++requestSeq.current
    const timer = setTimeout(async () => {
      try {
        const hits = await getTransport().call<SessionSearchResult[]>("search_sessions_cmd", {
          query: trimmed,
          limit: SEARCH_LIMIT,
          types: ["regular"],
        })
        if (seq !== requestSeq.current) return
        const seen = new Set<string>()
        const options: CronSessionTargetOption[] = []
        for (const hit of hits ?? []) {
          if (hit.isCron || hit.parentSessionId || hit.channelType) continue
          if (seen.has(hit.sessionId)) continue
          seen.add(hit.sessionId)
          options.push({ id: hit.sessionId, title: hit.sessionTitle })
        }
        setResults(options)
      } catch (error) {
        if (seq !== requestSeq.current) return
        logger.error("cron", "CronSessionTargetPicker::search", "chat search failed", error)
        setResults([])
      } finally {
        if (seq === requestSeq.current) setSearching(false)
      }
    }, SEARCH_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [query])

  const options = useMemo(() => (query.trim() ? results : recent), [query, recent, results])
  const label = value ? value.title || value.id : t("cron.sessionTargetPlaceholder")

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={disabled}
        className={cn(
          FLAT_CONTROL_SURFACE_CLASS,
          "flex h-9 w-full items-center justify-between gap-1.5 px-2.5 py-1.5 text-sm",
        )}
      >
        <span className={cn("truncate", value ? "text-foreground" : "text-muted-foreground")}>
          {label}
        </span>
        <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground/70" />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[min(22rem,80vw)] p-2">
        <SearchInput
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("cron.sessionTargetSearchPlaceholder")}
          className="h-8 text-xs"
        />
        <div className="mt-2 max-h-64 overflow-y-auto">
          {options === null || (searching && !options.length) ? (
            <div className="flex items-center justify-center gap-2 py-6 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.loading")}
            </div>
          ) : options.length === 0 ? (
            <div className="py-6 text-center text-xs text-muted-foreground">
              {t("cron.sessionTargetEmpty")}
            </div>
          ) : (
            options.map((option) => (
              <button
                key={option.id}
                type="button"
                onClick={() => {
                  onChange(option)
                  setOpen(false)
                }}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-foreground/90 transition-colors hover:bg-secondary/60"
              >
                <MessageSquare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">
                  {option.title || t("chat.untitledSession")}
                </span>
                {value?.id === option.id && <Check className="h-3.5 w-3.5 shrink-0" />}
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}
