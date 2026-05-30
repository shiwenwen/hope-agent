/**
 * Read-only code/text viewer rendered directly with Shiki (the same TextMate
 * highlighter Streamdown uses under the hood) — no Markdown round-trip. Each
 * line carries a `data-line` attribute so a text selection maps to exact 1-based
 * line numbers via the DOM (no fragile string matching), and the gutter line
 * numbers come from a CSS counter (see `.hope-shiki-view` in index.css).
 */

import { useEffect, useRef, useState } from "react"
import { codeToHtml, type ShikiTransformer } from "shiki"
import { Loader2 } from "lucide-react"

import { cn } from "@/lib/utils"

export interface CodeSelection {
  startLine: number
  endLine: number
  text: string
}

/** Above this size we skip Shiki's synchronous tokenizer and show plain
 *  monospace text, so a huge file can't block the UI thread. */
const MAX_HIGHLIGHT_BYTES = 400_000

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
  onSelectionChange,
  className,
}: {
  content: string
  lang: string
  onSelectionChange?: (sel: CodeSelection | null) => void
  className?: string
}) {
  const tooLarge = content.length > MAX_HIGHLIGHT_BYTES
  const [html, setHtml] = useState<string | null>(null)
  // Start in the loading state only when we actually intend to highlight.
  const [loading, setLoading] = useState(!tooLarge)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // Huge files render as a plain <pre> (see below); no async highlight.
    // `loading` starts true for highlightable files and this instance is keyed
    // by file path, so we never need to synchronously reset it in the effect.
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

  const onMouseUp = () => {
    if (!onSelectionChange) return
    const sel = window.getSelection()
    const text = sel?.toString() ?? ""
    if (!sel || sel.isCollapsed || !text.trim() || !rootRef.current) {
      onSelectionChange(null)
      return
    }
    const lineOf = (n: Node | null): number | null => {
      let el: Element | null = n instanceof Element ? n : (n?.parentElement ?? null)
      while (el && el !== rootRef.current) {
        const dl = el.getAttribute("data-line")
        if (dl) return Number(dl)
        el = el.parentElement
      }
      return null
    }
    const a = lineOf(sel.anchorNode)
    const b = lineOf(sel.focusNode)
    if (a == null || b == null) {
      onSelectionChange({ startLine: 1, endLine: text.split("\n").length, text })
      return
    }
    onSelectionChange({ startLine: Math.min(a, b), endLine: Math.max(a, b), text })
  }

  if (tooLarge || (!loading && !html)) {
    return (
      <pre className={cn("hope-shiki-view px-1 py-2 font-mono", className)}>{content}</pre>
    )
  }

  if (loading) {
    return (
      <div className={cn("flex items-center justify-center p-6 text-muted-foreground", className)}>
        <Loader2 className="h-4 w-4 animate-spin" />
      </div>
    )
  }

  return (
    <div
      ref={rootRef}
      onMouseUp={onMouseUp}
      className={cn("hope-shiki-view", className)}
      dangerouslySetInnerHTML={{ __html: html ?? "" }}
    />
  )
}
