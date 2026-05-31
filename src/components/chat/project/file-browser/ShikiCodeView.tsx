/**
 * Read-only code/text viewer rendered directly with Shiki (the same TextMate
 * highlighter Streamdown uses under the hood) — no Markdown round-trip. Each
 * line carries a `data-line` attribute so a text selection maps to exact 1-based
 * line numbers via the DOM (no fragile string matching), and the gutter line
 * numbers come from a CSS counter (see `.hope-shiki-view` in index.css).
 *
 * Right-click opens a small menu to copy the selection (or whole file) and quote
 * the selected lines to chat. We do NOT use Radix ContextMenu here: on macOS
 * WebView the native selection menu (Reload / Inspect / Look Up) pre-empts the
 * bubbling `contextmenu`, so Radix's trigger never fires on a text selection.
 * Instead we preventDefault in the CAPTURE phase (the only point that reliably
 * fires + suppresses the native menu) and render our own positioned menu.
 *
 * The rendered `view` is memoized so opening the menu (a state change) doesn't
 * re-create the dangerouslySetInnerHTML node — React bails out on the stable
 * element, leaving the user's text selection (and its highlight) intact.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react"
import { createPortal } from "react-dom"
import { codeToHtml, type ShikiTransformer } from "shiki"
import { Copy, Loader2, Quote } from "lucide-react"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"

export interface CodeSelection {
  startLine: number
  endLine: number
  text: string
}

/** Above this size we skip Shiki's synchronous tokenizer and show plain
 *  monospace text, so a huge file can't block the UI thread. */
const MAX_HIGHLIGHT_BYTES = 400_000

const MENU_ITEM_CLASS =
  "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground disabled:pointer-events-none disabled:opacity-50"

const lineData: ShikiTransformer = {
  name: "line-data",
  line(node, line) {
    node.properties["data-line"] = String(line)
    return node
  },
}

interface MenuState {
  x: number
  y: number
  sel: CodeSelection | null
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
  /** When provided, the right-click menu offers "quote to chat". */
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
  const [menu, setMenu] = useState<MenuState | null>(null)
  const rootRef = useRef<HTMLElement | null>(null)
  const setRootRef = useCallback((el: HTMLElement | null) => {
    rootRef.current = el
  }, [])

  useEffect(() => {
    if (tooLarge) return
    let cancelled = false
    const render = (l: string) =>
      codeToHtml(content, {
        lang: l,
        themes: { light: "github-light", dark: "github-dark" },
        defaultColor: false,
        transformers: [lineData],
      })
    void render(lang)
      .catch(() => render("text")) // unknown grammar → plaintext
      .then((out) => {
        if (cancelled) return
        setHtml(out)
        setLoading(false)
      })
      .catch(() => {
        if (cancelled) return
        setHtml(null)
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [content, lang, tooLarge])

  // Dismiss the menu on outside pointer-down, Escape, resize, or window blur.
  useEffect(() => {
    if (!menu) return
    const close = () => setMenu(null)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null)
    }
    window.addEventListener("pointerdown", close)
    window.addEventListener("keydown", onKey)
    window.addEventListener("resize", close)
    window.addEventListener("blur", close)
    // scroll doesn't bubble — listen in the capture phase so scrolling ANY
    // container dismisses the fixed menu (it's anchored to the click point, not
    // to the content, so it would otherwise float in place while text scrolls).
    window.addEventListener("scroll", close, true)
    return () => {
      window.removeEventListener("pointerdown", close)
      window.removeEventListener("keydown", onKey)
      window.removeEventListener("resize", close)
      window.removeEventListener("blur", close)
      window.removeEventListener("scroll", close, true)
    }
  }, [menu])

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
    if (!root.contains(sel.anchorNode) && !root.contains(sel.focusNode)) return null
    const a = lineOf(sel.anchorNode)
    const b = lineOf(sel.focusNode)
    // Use whichever endpoint resolved to a line; fall back to a best-effort
    // range only when neither does (e.g. the plain <pre> path has no data-line).
    const lo = a ?? b
    const hi = b ?? a
    return lo != null && hi != null
      ? { startLine: Math.min(lo, hi), endLine: Math.max(lo, hi), text }
      : { startLine: 1, endLine: text.split("\n").length, text }
  }, [lineOf])

  const onContextMenu = useCallback(
    (e: ReactMouseEvent) => {
      // Capture phase: suppress the native WebView menu and open ours. Clamp the
      // anchor to the viewport here (in the event, not during render) so the
      // menu can't open off-screen.
      e.preventDefault()
      const x = Math.min(e.clientX, window.innerWidth - 192)
      const y = Math.min(e.clientY, window.innerHeight - 96)
      setMenu({ x, y, sel: readSelection() })
    },
    [readSelection],
  )

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
          onContextMenuCapture={onContextMenu}
          className={cn("hope-shiki-view px-1 py-2", className)}
        >
          {content}
        </pre>
      ) : (
        <div
          ref={setRootRef}
          onContextMenuCapture={onContextMenu}
          className={cn("hope-shiki-view", className)}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ),
    [tooLarge, html, content, className, onContextMenu, setRootRef],
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
      {menu
        ? createPortal(
            <div
              className="fixed z-50 min-w-[11rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
              style={{ left: menu.x, top: menu.y }}
              // Keep clicks inside from bubbling to the window "close" listener.
              onPointerDown={(e) => e.stopPropagation()}
            >
              <button
                type="button"
                className={MENU_ITEM_CLASS}
                onClick={() => {
                  copyText(menu.sel?.text ?? content)
                  setMenu(null)
                }}
              >
                <Copy className="h-3.5 w-3.5" />
                {menu.sel
                  ? t("fileBrowser.copySelection", "Copy selection")
                  : t("fileBrowser.copyAll", "Copy all")}
              </button>
              {onQuote ? (
                <button
                  type="button"
                  disabled={!menu.sel}
                  className={MENU_ITEM_CLASS}
                  onClick={() => {
                    if (menu.sel) onQuote(menu.sel)
                    setMenu(null)
                  }}
                >
                  <Quote className="h-3.5 w-3.5" />
                  {t("fileBrowser.quoteToChat", "Quote to chat")}
                </button>
              ) : null}
            </div>,
            document.body,
          )
        : null}
    </>
  )
}
