// CodeMirror 6 inline preview widgets for the note editor (WS9): render Markdown
// images and KaTeX math (`$…$` / `$$…$$`) live inside the source pane. The source
// text stays the single source of truth — a match is *replaced* by its rendered
// widget only while the cursor/selection is outside it; touching the span reveals
// the raw Markdown for editing. KaTeX is lazy-loaded (npm-bundled, offline, CSP-safe).
//
// Decorations are provided via a StateField (not a ViewPlugin): block math spans
// line breaks, and CM6 forbids *plugin*-supplied decorations that replace newlines
// ("Decorations that replace line breaks may not be specified via plugins"). A
// StateField source is exempt, so `$$\n…\n$$` can be replaced safely.

import { syntaxTree } from "@codemirror/language"
import { type EditorState, RangeSetBuilder, StateField } from "@codemirror/state"
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view"

// Skip whole-document scanning for very large notes — the per-edit / per-cursor
// rebuild scans the full text, so cap it to keep typing snappy.
const MAX_PREVIEW_DOC = 100_000

// ── Lazy KaTeX loader ────────────────────────────────────────────────
type KatexModule = { renderToString: (tex: string, opts: Record<string, unknown>) => string }
let katex: KatexModule | null = null
let loading = false
const waiters: (() => void)[] = []

function ensureKatex(onReady: () => void) {
  if (katex) {
    onReady()
    return
  }
  waiters.push(onReady)
  if (loading) return
  loading = true
  void Promise.all([import("katex"), import("katex/dist/katex.min.css")])
    .then(([mod]) => {
      katex = (mod as { default?: KatexModule }).default ?? (mod as unknown as KatexModule)
      loading = false
      const pending = waiters.splice(0)
      for (const w of pending) w()
    })
    .catch(() => {
      // Leave pending widgets showing their raw-tex fallback; a later widget will
      // retry the import (loading=false, katex=null). Don't drop waiters silently.
      loading = false
    })
}

// ── Widgets ──────────────────────────────────────────────────────────
class ImageWidget extends WidgetType {
  readonly url: string
  readonly alt: string
  constructor(url: string, alt: string) {
    super()
    this.url = url
    this.alt = alt
  }
  eq(other: ImageWidget) {
    return other.url === this.url && other.alt === this.alt
  }
  toDOM() {
    const img = document.createElement("img")
    img.src = this.url
    img.alt = this.alt
    img.className = "cm-preview-img"
    img.loading = "lazy"
    return img
  }
  ignoreEvent() {
    return false
  }
}

class MathWidget extends WidgetType {
  readonly tex: string
  readonly display: boolean
  constructor(tex: string, display: boolean) {
    super()
    this.tex = tex
    this.display = display
  }
  eq(other: MathWidget) {
    return other.tex === this.tex && other.display === this.display
  }
  toDOM() {
    const span = document.createElement("span")
    span.className = this.display ? "cm-math cm-math-block" : "cm-math cm-math-inline"
    const render = () => {
      if (!katex) return
      try {
        span.innerHTML = katex.renderToString(this.tex, {
          displayMode: this.display,
          throwOnError: false,
          output: "html",
        })
      } catch {
        span.textContent = this.tex
      }
    }
    if (katex) render()
    else {
      span.textContent = this.tex
      ensureKatex(render)
    }
    return span
  }
  ignoreEvent() {
    return false
  }
}

interface Match {
  from: number
  to: number
  widget: WidgetType
}

const BLOCK_MATH_RE = /\$\$([\s\S]+?)\$\$/g
// Only preview clearly loadable images — http(s) / data URIs. Relative/local paths
// can't resolve here, so leaving their source visible avoids broken-image noise.
// Alt text excludes newlines so an image never spans a hard line break.
const IMAGE_RE = /!\[([^\]\n]*)\]\((https?:\/\/[^)\s]+|data:[^)\s]+)(?:\s+"[^"]*")?\)/g
// Inline `$…$`, pandoc-style to avoid prose dollar amounts: opener not followed by a
// space, content ends in a non-space, closer not followed by a digit ("$5 and $10").
const INLINE_MATH_RE = /\$(?![ \t])([^$\n]*?[^ \t$])\$(?!\d)/g

function collectMatches(text: string): Match[] {
  const items: Match[] = []
  const blocks: Array<[number, number]> = []

  let m: RegExpExecArray | null
  BLOCK_MATH_RE.lastIndex = 0
  while ((m = BLOCK_MATH_RE.exec(text)) !== null) {
    const from = m.index
    const to = m.index + m[0].length
    blocks.push([from, to])
    const tex = m[1].trim()
    if (tex) items.push({ from, to, widget: new MathWidget(tex, true) })
  }

  IMAGE_RE.lastIndex = 0
  while ((m = IMAGE_RE.exec(text)) !== null) {
    items.push({
      from: m.index,
      to: m.index + m[0].length,
      widget: new ImageWidget(m[2], m[1]),
    })
  }

  INLINE_MATH_RE.lastIndex = 0
  while ((m = INLINE_MATH_RE.exec(text)) !== null) {
    const from = m.index
    const to = m.index + m[0].length
    // Skip `$…$` that lives inside a `$$…$$` block (the inner content matches too).
    if (blocks.some(([bf, bt]) => from < bt && bf < to)) continue
    const tex = m[1].trim()
    if (tex) items.push({ from, to, widget: new MathWidget(tex, false) })
  }

  items.sort((a, b) => a.from - b.from)
  return items
}

/** The `[from, to)` spans this field replaces with image/math widgets. The live-
 *  preview field (livePreviewExtensions.ts) skips these so the two never emit
 *  overlapping replace decorations. */
export function previewMatchRanges(text: string): Array<[number, number]> {
  return collectMatches(text).map((m) => [m.from, m.to])
}

// Markdown code contexts — images / math inside these are literal source, not to be
// rendered (matches what the real preview shows). Container node names from
// @codemirror/lang-markdown's Lezer grammar.
const CODE_NODES = new Set(["FencedCode", "CodeBlock", "InlineCode"])

function codeRanges(state: EditorState): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  syntaxTree(state).iterate({
    enter: (node) => {
      if (CODE_NODES.has(node.name)) ranges.push([node.from, node.to])
    },
  })
  return ranges
}

function buildDecorations(state: EditorState): DecorationSet {
  if (state.doc.length > MAX_PREVIEW_DOC) return Decoration.none
  const builder = new RangeSetBuilder<Decoration>()
  const sel = state.selection.main
  const text = state.doc.toString()
  const code = codeRanges(state)
  const inCode = (from: number, to: number) =>
    code.some(([cf, ct]) => from < ct && cf < to)
  let lastTo = -1
  for (const it of collectMatches(text)) {
    if (it.from < lastTo) continue // defensive: never add overlapping replacements
    // Don't render images/math that live inside Markdown code (fenced / indented /
    // inline) — they're example source, shown literally by the real renderer.
    if (inCode(it.from, it.to)) continue
    // Reveal the raw Markdown whenever the selection touches the span.
    if (sel.from <= it.to && it.from <= sel.to) continue
    builder.add(it.from, it.to, Decoration.replace({ widget: it.widget }))
    lastTo = it.to
  }
  return builder.finish()
}

/** Live inline preview of images + math in the source pane (WS9). Provided as a
 *  StateField so block math (which spans line breaks) can be replaced. */
export function notePreviewWidgets() {
  return StateField.define<DecorationSet>({
    create: (state) => buildDecorations(state),
    update: (deco, tr) =>
      tr.docChanged || tr.selection ? buildDecorations(tr.state) : deco,
    provide: (f) => EditorView.decorations.from(f),
  })
}

/** Theme for the inline preview widgets. */
export const notePreviewTheme = EditorView.baseTheme({
  ".cm-preview-img": {
    display: "inline-block",
    maxWidth: "100%",
    maxHeight: "320px",
    borderRadius: "6px",
    verticalAlign: "middle",
  },
  ".cm-math-block": {
    display: "block",
    textAlign: "center",
    margin: "0.25em 0",
  },
  ".cm-math-inline": {
    display: "inline-block",
  },
})
