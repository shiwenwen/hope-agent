/**
 * HelpView — the built-in user manual (Help Center) UI.
 *
 * One `get_manual_bundle` round-trip loads all chapters; navigation, the
 * outline, in-chapter find and cross-chapter links are all client-side.
 * Full-text search calls `search_manual` (Rust is the single source of the
 * CJK-aware ranking + STX/ETX snippet contract).
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { BookOpen, List, MessageCircleQuestion, Search, TextSearch } from "lucide-react"

import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import { useClickOutside } from "@/hooks/useClickOutside"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { renderHighlightedSnippet } from "@/lib/highlight"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { emitAskAi } from "@/lib/manual/askAi"
import type { ManualLinkTarget } from "@/lib/manual/helpLinks"
import type { ManualBundle, ManualSearchHit } from "@/lib/manual/manualTypes"
import HelpFindBar from "./HelpFindBar"
import HelpMarkdown from "./HelpMarkdown"

export interface HelpTarget {
  chapter?: number
  anchor?: string
}

interface HelpViewProps {
  initialTarget?: HelpTarget
  /** Bumped when an external navigation request arrives (window re-target). */
  navigateSignal?: { nonce: number; target: HelpTarget } | null
}

export default function HelpView({ initialTarget, navigateSignal }: HelpViewProps) {
  const { t, i18n } = useTranslation()
  const [bundle, setBundle] = useState<ManualBundle | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const [reloadNonce, setReloadNonce] = useState(0)
  /** Explicit user override of the manual content language (zh/en toggle). */
  const [langOverride, setLangOverride] = useState<string | null>(null)
  const [chapter, setChapter] = useState<number>(initialTarget?.chapter ?? 0)
  const [pendingAnchor, setPendingAnchor] = useState<string | null>(initialTarget?.anchor ?? null)
  const [searchQuery, setSearchQuery] = useState("")
  const [searchHits, setSearchHits] = useState<ManualSearchHit[] | null>(null)
  const [findOpen, setFindOpen] = useState(false)
  const [outlineOpen, setOutlineOpen] = useState(false)
  const contentRef = useRef<HTMLDivElement>(null)
  const outlineRef = useRef<HTMLDivElement>(null)
  const searchSeq = useRef(0)
  useClickOutside(
    outlineRef,
    useCallback(() => setOutlineOpen(false), []),
  )

  const effectiveLang: "zh" | "en" = bundle?.effectiveLang === "zh" ? "zh" : "en"

  // Always send an explicit locale: with `language=auto` the backend would
  // resolve its OWN host locale (a Docker server is typically English) while
  // the frontend follows `navigator.language`, so a Chinese browser would get
  // a Chinese UI wrapped around the English manual. `i18n.language` is the
  // locale actually on screen — that is what the content must match.
  const requestLang = langOverride ?? i18n.language

  // ── Bundle load (once per language choice) ────────────────────────────
  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<ManualBundle>("get_manual_bundle", { lang: requestLang })
      .then((b) => {
        if (cancelled) return
        setBundle(b)
        setLoadFailed(false)
      })
      .catch((e) => {
        logger.error("help", "HelpView::loadBundle", "Failed to load manual bundle", { error: e })
        if (!cancelled) setLoadFailed(true)
      })
    return () => {
      cancelled = true
    }
  }, [requestLang, reloadNonce])

  const current = useMemo(
    () => bundle?.chapters.find((c) => c.number === chapter) ?? bundle?.chapters[0] ?? null,
    [bundle, chapter],
  )

  // ── Chapter-switch top scroll ─────────────────────────────────────────
  // Keyed on chapter identity only: consuming a fulfilled anchor
  // (setPendingAnchor(null) below) must NOT re-run this — a shared effect
  // previously snapped every successful anchor jump straight back to the top.
  // Declared before the anchor effect so an anchored navigation scrolls top
  // first, then wins with the anchor position in the same frame.
  const chapterKey = current ? `${effectiveLang}:${current.number}` : ""
  useEffect(() => {
    contentRef.current?.scrollTo({ top: 0 })
  }, [chapterKey])

  // ── Anchor scrolling (after the chapter renders) ──────────────────────
  useEffect(() => {
    if (!current || !pendingAnchor) return
    const root = contentRef.current
    if (!root) return
    let attempts = 0
    const tryScroll = () => {
      const el = root.querySelector<HTMLElement>(`[id="${CSS.escape(pendingAnchor)}"]`)
      if (el) {
        el.scrollIntoView({ block: "start" })
        setPendingAnchor(null)
      } else if (attempts < 10) {
        attempts += 1
        setTimeout(tryScroll, 50)
      } else {
        setPendingAnchor(null)
      }
    }
    requestAnimationFrame(tryScroll)
  }, [current, pendingAnchor])

  // ── External re-target (menu / deep links while the window is open) ───
  // Render-time state adjustment (React's "adjusting state when a prop
  // changes" pattern) — navigateSignal is an event carried by a prop.
  // An empty target (plain "open help" click while already open) is consumed
  // without touching state: it must focus the window, not reset the reading
  // position back to the index.
  const [seenNavNonce, setSeenNavNonce] = useState(0)
  if (navigateSignal && navigateSignal.nonce !== seenNavNonce) {
    setSeenNavNonce(navigateSignal.nonce)
    const { chapter: navChapter, anchor: navAnchor } = navigateSignal.target
    if (navChapter !== undefined || navAnchor) {
      setChapter(navChapter ?? 0)
      setPendingAnchor(navAnchor ?? null)
      setSearchQuery("")
      setSearchHits(null)
    }
  }

  // ── Search (debounced; the empty-query reset happens in onChange) ─────
  useEffect(() => {
    const q = searchQuery.trim()
    if (!q) {
      // Invalidate any in-flight request: a late response must not resurrect
      // the results panel after the user cleared the query.
      searchSeq.current += 1
      return
    }
    const seq = ++searchSeq.current
    const timer = setTimeout(() => {
      getTransport()
        .call<ManualSearchHit[]>("search_manual", { lang: requestLang, query: q })
        .then((hits) => {
          if (searchSeq.current === seq) setSearchHits(hits)
        })
        .catch((e) => logger.error("help", "HelpView::search", "Manual search failed", { error: e }))
    }, 200)
    return () => clearTimeout(timer)
  }, [searchQuery, requestLang])

  const goTo = useCallback((target: HelpTarget) => {
    setChapter(target.chapter ?? 0)
    setPendingAnchor(target.anchor ?? null)
  }, [])

  const handleNavigate = useCallback(
    (target: ManualLinkTarget) => {
      switch (target.kind) {
        case "anchor":
          setPendingAnchor(target.anchor)
          break
        case "chapter":
          goTo({ chapter: target.chapter, anchor: target.anchor })
          break
        case "language-switch":
          setLangOverride(effectiveLang === "zh" ? "en" : "zh")
          break
        case "external":
          openExternalUrl(target.url)
          break
        case "none":
          break
      }
    },
    [effectiveLang, goTo],
  )

  const handleAskAi = useCallback(async () => {
    if (!current) return
    const selection = window.getSelection()?.toString().trim() ?? ""
    const header = t("help.quoteHeader", { chapter: current.title })
    const text = selection ? `${header}\n${selection}` : header
    const delivery = await emitAskAi({ text })
    // Desktop focuses the main window (feedback is implicit); a browser tab
    // cannot switch tabs, so say what happened — silence reads as "broken".
    if (delivery === "delivered") toast.success(t("help.askAiSent"))
    else if (delivery === "no-listener") toast.error(t("help.askAiNoMainTab"))
  }, [current, t])

  // Cmd/Ctrl+F opens the in-chapter find bar (never while typing in inputs
  // that need native find, mirroring the chat screen's guard).
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "f") {
        const target = e.target as HTMLElement | null
        if (target?.isContentEditable) return
        e.preventDefault()
        setFindOpen(true)
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [])

  const outlineHeadings = useMemo(
    () => (current?.headings ?? []).filter((h) => h.level >= 2 && h.level <= 3),
    [current],
  )

  if (loadFailed) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
        <span>{t("help.loadFailed")}</span>
        <Button variant="outline" size="sm" onClick={() => setReloadNonce((n) => n + 1)}>
          {t("help.retry")}
        </Button>
      </div>
    )
  }
  if (!bundle || !current) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        {t("help.loading")}
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0">
      {/* ── Left rail: search + chapters / results ── */}
      <div className="flex w-64 shrink-0 flex-col border-r border-border-soft bg-surface-panel">
        <div className="p-2">
          <SearchInput
            value={searchQuery}
            placeholder={t("help.searchPlaceholder")}
            className="h-8 text-sm"
            onChange={(e) => {
              setSearchQuery(e.target.value)
              if (!e.target.value.trim()) setSearchHits(null)
            }}
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {searchHits ? (
            searchHits.length === 0 ? (
              <div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
                <Search className="h-3.5 w-3.5" />
                {t("help.noResults")}
              </div>
            ) : (
              searchHits.map((hit, i) => (
                <button
                  key={`${hit.chapter}-${hit.line}-${i}`}
                  type="button"
                  className="w-full rounded-md px-2 py-1.5 text-left hover:bg-secondary/40"
                  onClick={() =>
                    goTo({ chapter: hit.chapter, anchor: hit.anchor ?? undefined })
                  }
                >
                  <div className="truncate text-xs font-medium text-foreground">
                    {hit.chapter === 0
                      ? hit.chapterTitle
                      : `${String(hit.chapter).padStart(2, "0")} · ${hit.chapterTitle}`}
                  </div>
                  <div className="line-clamp-2 text-xs text-muted-foreground">
                    {renderHighlightedSnippet(hit.snippet)}
                  </div>
                </button>
              ))
            )
          ) : (
            bundle.chapters.map((c) => (
              <button
                key={c.number}
                type="button"
                className={cn(
                  "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm",
                  c.number === current.number
                    ? "bg-secondary/70 text-foreground"
                    : "text-muted-foreground hover:bg-secondary/40 hover:text-foreground",
                )}
                onClick={() => goTo({ chapter: c.number })}
              >
                {c.number === 0 ? (
                  <BookOpen className="h-3.5 w-3.5 shrink-0" />
                ) : (
                  <span className="w-5 shrink-0 text-xs tabular-nums text-muted-foreground">
                    {String(c.number).padStart(2, "0")}
                  </span>
                )}
                <span className="truncate">{c.title}</span>
              </button>
            ))
          )}
        </div>
        {/* Manual content language toggle (content is zh/en only). */}
        <div className="flex items-center gap-1 border-t border-border-soft p-2">
          {(["zh", "en"] as const).map((lang) => (
            <Button
              key={lang}
              variant="ghost"
              size="sm"
              aria-pressed={effectiveLang === lang}
              className={cn(
                "h-6 flex-1 text-xs",
                effectiveLang === lang
                  ? "bg-secondary/70 text-foreground hover:bg-secondary/70"
                  : "text-muted-foreground",
              )}
              onClick={() => setLangOverride(lang)}
            >
              {lang === "zh" ? "中文" : "English"}
            </Button>
          ))}
        </div>
      </div>

      {/* ── Content ── */}
      <div className="relative flex min-w-0 flex-1 flex-col">
        <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border-soft px-3">
          <span className="min-w-0 flex-1 truncate text-sm font-medium">
            {current.number === 0
              ? current.title
              : `${String(current.number).padStart(2, "0")} · ${current.title}`}
          </span>
          <div className="relative" ref={outlineRef}>
            <IconTip label={t("help.outline")}>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => setOutlineOpen((v) => !v)}
              >
                <List className="h-4 w-4" />
              </Button>
            </IconTip>
            <FloatingMenu
              open={outlineOpen}
              positionClassName="top-full right-0 mt-1.5"
              originClassName="origin-top-right"
              className="max-h-80 w-72 overflow-auto p-1.5"
              onEscapeKeyDown={() => setOutlineOpen(false)}
            >
              {outlineHeadings.map((h) => (
                <button
                  key={h.slug}
                  type="button"
                  className={cn(
                    "block w-full truncate rounded px-2 py-1 text-left text-xs hover:bg-secondary/40",
                    h.level === 3 && "pl-5 text-muted-foreground",
                  )}
                  onClick={() => {
                    setOutlineOpen(false)
                    setPendingAnchor(h.slug)
                  }}
                >
                  {h.text}
                </button>
              ))}
            </FloatingMenu>
          </div>
          <IconTip label={t("help.findPlaceholder")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => setFindOpen(true)}
            >
              <TextSearch className="h-4 w-4" />
            </Button>
          </IconTip>
          <IconTip label={t("help.askAi")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              // Keep the user's text selection alive: the button's mousedown
              // would otherwise collapse it before click reads it.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => void handleAskAi()}
            >
              <MessageCircleQuestion className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
        <HelpFindBar
          containerRef={contentRef}
          open={findOpen}
          onClose={() => setFindOpen(false)}
          contentVersion={chapterKey}
        />
        <div ref={contentRef} className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <div className="mx-auto max-w-4xl">
            <HelpMarkdown
              body={current.body}
              lang={effectiveLang}
              headings={current.headings}
              onNavigate={handleNavigate}
            />
          </div>
        </div>
      </div>
    </div>
  )
}
