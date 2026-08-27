/**
 * Read-only code/text viewer rendered directly with Shiki (the same TextMate
 * highlighter Streamdown uses under the hood) — no Markdown round-trip. Each
 * line carries a `data-line` attribute so a text selection maps to exact 1-based
 * line numbers via the DOM (no fragile string matching), and the gutter line
 * numbers come from a CSS counter (see `.hope-shiki-view` in index.css).
 *
 * Selecting text automatically opens a small menu to copy or quote it. The
 * legacy right-click path remains available (including copy-all with no active
 * selection). We do NOT use Radix ContextMenu here: on macOS WebView the native
 * selection menu pre-empts the bubbling `contextmenu`, so capture-phase handling
 * remains the reliable compatibility path.
 *
 * The rendered `view` is memoized so opening the menu (a state change) doesn't
 * re-create the dangerouslySetInnerHTML node — React bails out on the stable
 * element, leaving the user's text selection (and its highlight) intact.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  createHighlighter,
  createJavaScriptRegexEngine,
  type BundledLanguage,
  type Highlighter,
  type ShikiTransformer,
} from "shiki"
import { Loader2 } from "lucide-react"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"

import { SelectionActionMenu } from "@/components/common/SelectionActionMenu"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { useSelectionActionMenu } from "./useSelectionActionMenu"

export interface CodeSelection {
  startLine: number
  endLine: number
  text: string
}

/** Above this size we skip Shiki's synchronous tokenizer and show plain
 *  monospace text, so a huge file can't block the UI thread. */
const MAX_HIGHLIGHT_BYTES = 400_000
const SHIKI_THEMES = ["github-light", "github-dark"] as const

/**
 * Tauri's production CSP deliberately omits `wasm-unsafe-eval`. Shiki's
 * bundled shortcut uses the Oniguruma WASM engine by default, so it works in
 * jsdom/dev but fails in the packaged WebView and poisons the shortcut's
 * singleton promise. Use Shiki's JavaScript regex engine for this surface so
 * highlighting stays offline and CSP-safe without weakening the app policy.
 */
let previewHighlighterPromise: Promise<Highlighter> | null = null

function getPreviewHighlighter(): Promise<Highlighter> {
  if (!previewHighlighterPromise) {
    const pending = createHighlighter({
      engine: createJavaScriptRegexEngine(),
      langs: [],
      themes: [...SHIKI_THEMES],
    })
    previewHighlighterPromise = pending.catch((error) => {
      // A transient chunk-load failure must not permanently disable every
      // later preview in this renderer process.
      previewHighlighterPromise = null
      throw error
    })
  }
  return previewHighlighterPromise
}

async function renderHighlightedCode(content: string, lang: string): Promise<string> {
  const highlighter = await getPreviewHighlighter()
  if (lang !== "text" && !highlighter.getLoadedLanguages().includes(lang)) {
    await highlighter.loadLanguage(lang as BundledLanguage)
  }
  return highlighter.codeToHtml(content, {
    lang: lang as BundledLanguage,
    themes: { light: SHIKI_THEMES[0], dark: SHIKI_THEMES[1] },
    transformers: [lineData],
  })
}

const lineData: ShikiTransformer = {
  name: "line-data",
  line(node, line) {
    node.properties["data-line"] = String(line)
    return node
  },
}

export function ShikiCodeView({
  content,
  lang,
  onQuote,
  highlightLines,
  className,
}: {
  content: string
  lang: string
  /** When provided, the selection menu offers "quote to chat". */
  onQuote?: (sel: CodeSelection) => void
  /** Highlight + scroll to this 1-based line range (e.g. a quote reveal). */
  highlightLines?: { start: number; end: number; nonce: number } | null
  className?: string
}) {
  const { t } = useTranslation()
  const tooLarge = content.length > MAX_HIGHLIGHT_BYTES
  const [html, setHtml] = useState<string | null>(null)
  // Start in the loading state only when we actually intend to highlight.
  const [loading, setLoading] = useState(!tooLarge)
  const rootRef = useRef<HTMLElement | null>(null)
  const setRootRef = useCallback((el: HTMLElement | null) => {
    rootRef.current = el
  }, [])

  useEffect(() => {
    if (tooLarge) return
    let cancelled = false
    let syntaxError: unknown = null
    void renderHighlightedCode(content, lang)
      .catch((error) => {
        syntaxError = error
        return renderHighlightedCode(content, "text")
      }) // unknown grammar → Shiki plaintext (retains line numbers)
      .then((out) => {
        if (cancelled) return
        if (syntaxError) {
          logger.warn("ui", "ShikiCodeView::render", "syntax grammar failed; used plaintext", {
            lang,
            error: syntaxError instanceof Error ? syntaxError.message : String(syntaxError),
          })
        }
        setHtml(out)
        setLoading(false)
      })
      .catch((error) => {
        if (cancelled) return
        logger.error("ui", "ShikiCodeView::render", "code preview highlighting failed", {
          lang,
          error: error instanceof Error ? error.message : String(error),
        })
        setHtml(null)
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [content, lang, tooLarge])

  // Highlight + scroll to the revealed line range once the HTML is rendered.
  // Re-runs when the html or range changes; the range carries a nonce so a
  // repeat reveal of the same lines re-triggers. Lines are marked with a data
  // attribute styled in CSS (`.hope-shiki-view .line[data-reveal-hl]`).
  useEffect(() => {
    const root = rootRef.current
    if (!root) return
    root.querySelectorAll("[data-reveal-hl]").forEach((el) => el.removeAttribute("data-reveal-hl"))
    if (!highlightLines) return
    let first: Element | null = null
    for (let n = highlightLines.start; n <= highlightLines.end; n++) {
      const line = root.querySelector(`[data-line="${n}"]`)
      if (line) {
        line.setAttribute("data-reveal-hl", "")
        if (!first) first = line
      }
    }
    first?.scrollIntoView({ block: "center" })
  }, [highlightLines, html])

  // Map a DOM node up to its 1-based line number via the `data-line` attribute.
  const lineOf = useCallback((n: Node | null): number | null => {
    let el: Element | null = n instanceof Element ? n : (n?.parentElement ?? null)
    while (el && el !== rootRef.current) {
      const dl = el.getAttribute("data-line")
      if (dl) return Number(dl)
      el = el.parentElement
    }
    return null
  }, [])

  const readSelection = useCallback((): CodeSelection | null => {
    const sel = window.getSelection()
    const text = sel?.toString() ?? ""
    const root = rootRef.current
    if (!sel || sel.isCollapsed || !text.trim() || !root) return null
    if (!root.contains(sel.anchorNode) || !root.contains(sel.focusNode)) return null
    const a = lineOf(sel.anchorNode)
    const b = lineOf(sel.focusNode)
    // Use whichever endpoint resolved to a line. The oversized plain-<pre>
    // fallback has no data-line mapping, so report 0/0 rather than inventing an
    // L1-n range that a later reveal could misinterpret as exact.
    const lo = a ?? b
    const hi = b ?? a
    return lo != null && hi != null
      ? { startLine: Math.min(lo, hi), endLine: Math.max(lo, hi), text }
      : { startLine: 0, endLine: 0, text }
  }, [lineOf])

  const getCopyAllText = useCallback(() => content, [content])
  const { menu, closeMenu, onContextMenuCapture } = useSelectionActionMenu({
    rootRef,
    readSelection,
    getCopyAllText,
  })

  const copyText = useCallback(
    (text: string) => {
      navigator.clipboard.writeText(text).then(
        () => toast.success(t("fileBrowser.copied", "Copied")),
        () => toast.error(t("fileBrowser.copyFailed", "Copy failed")),
      )
    },
    [t],
  )

  // Memoized so a menu open/close (state change) never re-creates this node;
  // React bails out on the stable element and the live text selection survives.
  const view = useMemo(
    () =>
      tooLarge || !html ? (
        <pre
          ref={setRootRef}
          onContextMenuCapture={onContextMenuCapture}
          className={cn("hope-shiki-view px-1 py-2", className)}
        >
          {content}
        </pre>
      ) : (
        <div
          ref={setRootRef}
          onContextMenuCapture={onContextMenuCapture}
          className={cn("hope-shiki-view", className)}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ),
    [tooLarge, html, content, className, onContextMenuCapture, setRootRef],
  )

  if (loading) {
    return (
      <div className={cn("flex items-center justify-center p-6 text-muted-foreground", className)}>
        <Loader2 className="h-4 w-4 animate-spin" />
      </div>
    )
  }

  return (
    <>
      {view}
      <SelectionActionMenu
        open={menu !== null}
        position={menu?.position ?? null}
        text={menu?.text ?? ""}
        copyMode={menu?.copyMode}
        onCopy={copyText}
        quoteDisabled={!menu?.value}
        onQuote={
          onQuote
            ? () => {
                if (menu?.value) onQuote(menu.value)
              }
            : undefined
        }
        onClose={closeMenu}
      />
    </>
  )
}
