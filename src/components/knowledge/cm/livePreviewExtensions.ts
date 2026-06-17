// CodeMirror 6 "live preview" decorations for the note editor (Phase 3, D13).
//
// This is the deliverable for the "visual editor" evaluation: rather than bolting
// a ProseMirror WYSIWYG engine (Milkdown/Tiptap) on top — which would round-trip
// Markdown⇄JSON and quietly rewrite whitespace / frontmatter / wikilinks, breaking
// the `.md`-is-truth contract, D14 offsets, `note_patch` old/new matching and the
// stale-write BLAKE3 guard — we hide the *syntax markers* in place, exactly like
// Obsidian's own (CodeMirror-based) live preview. The document stays plain `.md`;
// only the rendering changes. Touching a line reveals its raw Markdown for editing.
//
// Images + math are handled by `notePreviewWidgets` (previewExtensions.ts); this
// field skips their spans (and fenced/inline code) so the two never produce
// overlapping replace decorations.

import { syntaxTree } from "@codemirror/language"
import { type EditorState, type Range, StateField } from "@codemirror/state"
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view"

import { previewMatchRanges } from "./previewExtensions"

// Same snappiness cap as the inline-preview field: the decoration set is rebuilt
// on every edit / cursor move and scans the whole doc, so skip very large notes.
const MAX_LIVE_DOC = 100_000

/** A `•` standing in for a `-`/`*`/`+` bullet marker. */
class BulletWidget extends WidgetType {
  eq() {
    return true
  }
  toDOM() {
    const span = document.createElement("span")
    span.className = "cm-live-bullet"
    span.textContent = "•"
    return span
  }
  ignoreEvent() {
    return false
  }
}

/** A rendered checkbox standing in for a `[ ]` / `[x]` task marker. The marker's
 *  document position rides along in a data attribute so the click handler can
 *  toggle it (positions are rebuilt on every edit, so they stay current). */
class TaskWidget extends WidgetType {
  private readonly checked: boolean
  private readonly pos: number

  constructor(checked: boolean, pos: number) {
    super()
    this.checked = checked
    this.pos = pos
  }
  eq(other: TaskWidget) {
    return other.checked === this.checked && other.pos === this.pos
  }
  toDOM() {
    const span = document.createElement("span")
    span.className = this.checked ? "cm-live-task cm-live-task-done" : "cm-live-task"
    span.textContent = this.checked ? "☑" : "☐"
    span.dataset.taskPos = String(this.pos)
    span.setAttribute("role", "checkbox")
    span.setAttribute("aria-checked", this.checked ? "true" : "false")
    return span
  }
  ignoreEvent() {
    return false
  }
}

/** A horizontal rule standing in for `---` / `***` / `___`. */
class HrWidget extends WidgetType {
  eq() {
    return true
  }
  toDOM() {
    const span = document.createElement("span")
    span.className = "cm-live-hr"
    span.setAttribute("aria-hidden", "true")
    return span
  }
  ignoreEvent() {
    return false
  }
}

export type CellAlign = "left" | "center" | "right" | null

export interface ParsedTable {
  aligns: CellAlign[]
  header: string[]
  rows: string[][]
}

/** Split one GFM table row into trimmed cells, honoring `\|` escapes and the
 *  optional leading / trailing pipe. */
function splitTableRow(line: string): string[] {
  let s = line.trim()
  if (s.startsWith("|")) s = s.slice(1)
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1)
  const cells: string[] = []
  let cur = ""
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]
    if (ch === "\\" && i + 1 < s.length) {
      cur += s[i + 1]
      i++
      continue
    }
    if (ch === "|") {
      cells.push(cur.trim())
      cur = ""
      continue
    }
    cur += ch
  }
  cells.push(cur.trim())
  return cells
}

/** Parse a GFM table block (header + delimiter + rows) into a render model.
 *  Returns null when the delimiter row is missing/malformed (defensive — the
 *  caller only feeds it spans the markdown parser already tagged as `Table`). */
export function parseGfmTable(src: string): ParsedTable | null {
  const lines = src.split("\n").filter((l) => l.trim().length > 0)
  if (lines.length < 2) return null
  const header = splitTableRow(lines[0])
  const delim = splitTableRow(lines[1])
  if (delim.length === 0 || !delim.every((c) => /^:?-+:?$/.test(c))) return null
  const aligns: CellAlign[] = delim.map((c) => {
    const l = c.startsWith(":")
    const r = c.endsWith(":")
    return l && r ? "center" : r ? "right" : l ? "left" : null
  })
  const rows = lines.slice(2).map(splitTableRow)
  return { aligns, header, rows }
}

/** A rendered GFM table replacing its source lines. Cells are plain text
 *  (`textContent`, so no inline-markdown / HTML injection); editing a cell is
 *  done by clicking into the table, which reveals the raw Markdown. */
class TableWidget extends WidgetType {
  private readonly src: string

  constructor(src: string) {
    super()
    this.src = src
  }
  eq(other: TableWidget) {
    return other.src === this.src
  }
  toDOM() {
    const wrap = document.createElement("div")
    wrap.className = "cm-live-table-wrap"
    const parsed = parseGfmTable(this.src)
    if (!parsed) {
      wrap.textContent = this.src
      return wrap
    }
    const table = document.createElement("table")
    table.className = "cm-live-table"
    const cols = parsed.header.length

    const thead = document.createElement("thead")
    const htr = document.createElement("tr")
    for (let i = 0; i < cols; i++) {
      const th = document.createElement("th")
      th.textContent = parsed.header[i] ?? ""
      if (parsed.aligns[i]) th.style.textAlign = parsed.aligns[i] as string
      htr.appendChild(th)
    }
    thead.appendChild(htr)
    table.appendChild(thead)

    const tbody = document.createElement("tbody")
    for (const row of parsed.rows) {
      const tr = document.createElement("tr")
      for (let i = 0; i < cols; i++) {
        const td = document.createElement("td")
        td.textContent = row[i] ?? ""
        if (parsed.aligns[i]) td.style.textAlign = parsed.aligns[i] as string
        tr.appendChild(td)
      }
      tbody.appendChild(tr)
    }
    table.appendChild(tbody)
    wrap.appendChild(table)
    return wrap
  }
  ignoreEvent() {
    return false
  }
}

const HEADER_LEVEL_RE = /^ATXHeading([1-6])$/
const BULLET_RE = /^[-*+]$/

const hidden = Decoration.replace({})
const bullet = Decoration.replace({ widget: new BulletWidget() })

function buildLive(state: EditorState): DecorationSet {
  if (state.doc.length > MAX_LIVE_DOC) return Decoration.none

  // Lines touched by any selection range show their raw Markdown (Obsidian-style
  // reveal). Editing a heading/bold/etc. then always sees the real characters.
  const activeLines = new Set<number>()
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).number
    const b = state.doc.lineAt(r.to).number
    for (let ln = a; ln <= b; ln++) activeLines.add(ln)
  }

  // Spans owned by the image/math inline-preview field — never decorate inside
  // them (would overlap a replace and throw).
  const protectedRanges = previewMatchRanges(state.doc.toString())
  const isProtected = (from: number, to: number) =>
    protectedRanges.some(([pf, pt]) => from < pt && pf < to)

  const decos: Range<Decoration>[] = []
  const onActiveLine = (pos: number) => activeLines.has(state.doc.lineAt(pos).number)

  syntaxTree(state).iterate({
    enter: (node) => {
      const { name, from, to } = node
      // Images are rendered by the widget field — skip the subtree.
      if (name === "Image") return false
      // Fenced / indented code: don't hide markers (content is literal), but
      // tint the whole block so it reads as a unit. `syntaxHighlighting` colors
      // the body by language. Keep the tint even while editing (all lines).
      if (name === "FencedCode" || name === "CodeBlock") {
        const first = state.doc.lineAt(from).number
        const last = state.doc.lineAt(to).number
        for (let ln = first; ln <= last; ln++) {
          const lineFrom = state.doc.line(ln).from
          const cls =
            ln === first
              ? "cm-live-code-line cm-live-code-top"
              : ln === last
                ? "cm-live-code-line cm-live-code-bottom"
                : "cm-live-code-line"
          decos.push(Decoration.line({ class: cls }).range(lineFrom))
        }
        return false
      }
      // GFM table: replace the source lines with a rendered table, unless the
      // cursor is inside it (then reveal the raw Markdown for editing).
      if (name === "Table") {
        if (isProtected(from, to)) return false
        const first = state.doc.lineAt(from).number
        const last = state.doc.lineAt(to).number
        let active = false
        for (let ln = first; ln <= last; ln++) {
          if (activeLines.has(ln)) {
            active = true
            break
          }
        }
        if (!active) {
          decos.push(
            Decoration.replace({
              block: true,
              widget: new TableWidget(state.doc.sliceString(from, to)),
            }).range(from, to),
          )
        }
        return false
      }
      if (isProtected(from, to) || onActiveLine(from)) return

      switch (name) {
        case "HeaderMark": {
          // Only ATX heading markers (skip the Setext underline form, whose
          // HeaderMark sits on its own line). Hide `#`s + the one trailing space,
          // and size the heading text via a mark on the rest of the line.
          const parent = node.node.parent
          const lvl = parent ? HEADER_LEVEL_RE.exec(parent.name)?.[1] : undefined
          if (!lvl || !parent) return
          const line = state.doc.lineAt(from)
          if (from !== line.from + leadingSpaces(line.text)) return // closing `#`s
          const afterMark =
            to < state.doc.length && state.doc.sliceString(to, to + 1) === " " ? to + 1 : to
          decos.push(hidden.range(from, afterMark))
          if (afterMark < line.to) {
            decos.push(
              Decoration.mark({ class: `cm-live-h${lvl}` }).range(afterMark, line.to),
            )
          }
          break
        }
        case "EmphasisMark":
          decos.push(hidden.range(from, to))
          break
        case "StrikethroughMark":
          decos.push(hidden.range(from, to))
          break
        case "CodeMark":
          decos.push(hidden.range(from, to))
          break
        case "Emphasis":
          decos.push(Decoration.mark({ class: "cm-live-em" }).range(from, to))
          break
        case "StrongEmphasis":
          decos.push(Decoration.mark({ class: "cm-live-strong" }).range(from, to))
          break
        case "Strikethrough":
          decos.push(Decoration.mark({ class: "cm-live-strike" }).range(from, to))
          break
        case "InlineCode":
          decos.push(Decoration.mark({ class: "cm-live-code" }).range(from, to))
          break
        case "Link": {
          // Inline link `[label](url)`: hide the brackets + URL, keep a styled
          // label. Reference / empty-label links (no URL child) are left raw.
          if (!node.node.getChild("URL")) break
          const marks = node.node.getChildren("LinkMark")
          if (marks.length < 2 || marks[0].from !== from) break
          const labelFrom = marks[0].to
          const labelTo = marks[1].from
          if (labelFrom >= labelTo) break
          decos.push(hidden.range(from, labelFrom)) // `[`
          decos.push(hidden.range(labelTo, to)) // `](url)`
          decos.push(Decoration.mark({ class: "cm-live-link" }).range(labelFrom, labelTo))
          break
        }
        case "HorizontalRule":
          decos.push(Decoration.replace({ widget: new HrWidget() }).range(from, to))
          break
        case "TaskMarker": {
          const checked = /[xX]/.test(state.doc.sliceString(from, to))
          decos.push(
            Decoration.replace({ widget: new TaskWidget(checked, from) }).range(from, to),
          )
          if (checked) {
            const line = state.doc.lineAt(from)
            if (to < line.to) {
              decos.push(Decoration.mark({ class: "cm-live-done" }).range(to, line.to))
            }
          }
          break
        }
        case "ListMark": {
          // Task list items render as a checkbox — drop the bullet so the line
          // reads "☐ text", not "• ☐ text".
          const item = node.node.parent
          if (item?.name === "ListItem" && item.getChild("Task")) {
            const afterMark =
              to < state.doc.length && state.doc.sliceString(to, to + 1) === " " ? to + 1 : to
            decos.push(hidden.range(from, afterMark))
            break
          }
          // Render `-`/`*`/`+` as a real bullet; leave ordered-list numbers alone.
          if (BULLET_RE.test(state.doc.sliceString(from, to))) {
            decos.push(bullet.range(from, to))
          }
          break
        }
        case "QuoteMark": {
          // Hide `>` + trailing space; tint the quoted remainder of the line.
          const line = state.doc.lineAt(from)
          const afterMark =
            to < state.doc.length && state.doc.sliceString(to, to + 1) === " " ? to + 1 : to
          decos.push(hidden.range(from, afterMark))
          if (afterMark < line.to) {
            decos.push(Decoration.mark({ class: "cm-live-quote" }).range(afterMark, line.to))
          }
          break
        }
      }
    },
  })

  // RangeSet requires ascending `from`, and for equal `from` the marks must be
  // ordered by startSide; `Decoration.set(arr, true)` sorts for us.
  return Decoration.set(decos, true)
}

function leadingSpaces(s: string): number {
  let n = 0
  while (n < s.length && s[n] === " ") n++
  return n
}

/** Click a rendered task checkbox to toggle `[ ]` ⇄ `[x]` in place. */
const taskToggleHandler = EditorView.domEventHandlers({
  mousedown(event, view) {
    const target = event.target as HTMLElement | null
    if (!target || !target.classList?.contains("cm-live-task")) return false
    const pos = Number(target.dataset.taskPos)
    if (!Number.isFinite(pos)) return false
    const marker = view.state.sliceDoc(pos, pos + 3)
    const m = /^\[([ xX])\]$/.exec(marker)
    if (!m) return false
    event.preventDefault()
    view.dispatch({
      changes: { from: pos + 1, to: pos + 2, insert: m[1] === " " ? "x" : " " },
    })
    return true
  },
})

/** Live-preview decorations (syntax-marker hiding). StateField so replacements may
 *  cover line-spanning constructs without the plugin-decoration restriction; the
 *  paired dom handler makes rendered task checkboxes clickable. */
export function noteLiveDecorations() {
  return [
    StateField.define<DecorationSet>({
      create: (state) => buildLive(state),
      update: (deco, tr) => (tr.docChanged || tr.selection ? buildLive(tr.state) : deco),
      provide: (f) => EditorView.decorations.from(f),
    }),
    taskToggleHandler,
  ]
}

/** Visual styling for the hidden-marker spans. */
export const noteLiveTheme = EditorView.baseTheme({
  ".cm-live-strong": { fontWeight: "700" },
  ".cm-live-em": { fontStyle: "italic" },
  ".cm-live-strike": { textDecoration: "line-through", opacity: "0.7" },
  ".cm-live-code": {
    fontFamily:
      "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace",
    backgroundColor: "rgba(127,127,127,0.16)",
    borderRadius: "3px",
    padding: "0.05em 0.3em",
  },
  ".cm-live-quote": { fontStyle: "italic", opacity: "0.8" },
  ".cm-live-bullet": { opacity: "0.6" },
  ".cm-live-link": {
    color: "#6366f1",
    textDecoration: "underline",
    textUnderlineOffset: "2px",
    cursor: "pointer",
  },
  ".cm-live-task": {
    cursor: "pointer",
    userSelect: "none",
    marginRight: "0.2em",
    color: "var(--color-muted-foreground)",
  },
  ".cm-live-task-done": { color: "#22c55e" },
  ".cm-live-done": { textDecoration: "line-through", opacity: "0.55" },
  ".cm-live-hr": {
    display: "inline-block",
    width: "100%",
    verticalAlign: "middle",
    borderTop: "1px solid var(--color-border)",
  },
  ".cm-live-code-line": {
    backgroundColor: "color-mix(in srgb, var(--color-muted-foreground) 12%, transparent)",
  },
  ".cm-live-code-top": {
    borderTopLeftRadius: "6px",
    borderTopRightRadius: "6px",
  },
  ".cm-live-code-bottom": {
    borderBottomLeftRadius: "6px",
    borderBottomRightRadius: "6px",
  },
  ".cm-live-table-wrap": { margin: "0.3em 0", overflowX: "auto" },
  ".cm-live-table": { borderCollapse: "collapse", lineHeight: "1.4" },
  ".cm-live-table th, .cm-live-table td": {
    border: "1px solid var(--color-border)",
    padding: "0.25em 0.6em",
    textAlign: "left",
  },
  ".cm-live-table th": {
    fontWeight: "700",
    backgroundColor: "color-mix(in srgb, var(--color-muted-foreground) 12%, transparent)",
  },
  ".cm-live-h1": { fontSize: "1.6em", fontWeight: "700", lineHeight: "1.3" },
  ".cm-live-h2": { fontSize: "1.4em", fontWeight: "700", lineHeight: "1.3" },
  ".cm-live-h3": { fontSize: "1.2em", fontWeight: "700", lineHeight: "1.3" },
  ".cm-live-h4": { fontSize: "1.1em", fontWeight: "700" },
  ".cm-live-h5": { fontSize: "1.05em", fontWeight: "700" },
  ".cm-live-h6": { fontSize: "1em", fontWeight: "700", opacity: "0.85" },
})
