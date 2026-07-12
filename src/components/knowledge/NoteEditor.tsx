import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete"
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands"
import { markdown, markdownLanguage } from "@codemirror/lang-markdown"
import { languages as codeLanguages } from "@codemirror/language-data"
import { indentOnInput, syntaxHighlighting } from "@codemirror/language"
import { lintKeymap } from "@codemirror/lint"
import { searchKeymap } from "@codemirror/search"
import { Compartment, EditorSelection, EditorState } from "@codemirror/state"
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
} from "@codemirror/view"
import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import { MessageSquareQuote, Sparkles } from "lucide-react"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { FloatingMenu } from "@/components/ui/floating-menu"
import type { NoteEditorMode } from "@/types/knowledge"

import { fetchNoteRef } from "./noteRefFetch"
import NoteTransclusionView from "./NoteTransclusionView"
import OutlineView from "./OutlineView"
import { cleanEmbedRef, noteExcerpt } from "./transclusionParse"

import { noteHighlightStyle } from "./cm/highlightStyle"
import { noteLiveDecorations, noteLiveTheme } from "./cm/livePreviewExtensions"
import { notePreviewTheme, notePreviewWidgets } from "./cm/previewExtensions"
import {
  brokenLinkLinter,
  wikilinkCompletion,
  wikilinkDecorations,
  wikilinkHover,
  wikilinkTheme,
  type WikilinkData,
} from "./cm/wikilinkExtensions"

interface NoteEditorProps {
  value: string
  onChange: (value: string) => void
  readOnly?: boolean
  mode: NoteEditorMode
  /** Live wikilink data (note titles/paths, tags, resolvable targets). */
  data: WikilinkData
  /**
   * Scroll + place the cursor at this position when it changes (1-based line,
   * 0-based code-point col). Used for backlink / search precision navigation
   * (G3). New object identity re-fires even for the same line. Source/Split
   * only — no-op in preview-only mode (no editor view).
   */
  revealTarget?: { line: number; col?: number } | null
  /** KB id — enables `![[ ]]` transclusion in the preview pane (WS2). */
  kbId?: string | null
  /** Current note rel-path — seeds the transclusion cycle guard (self-embed). */
  notePath?: string | null
  /** Open a note when an embed header is clicked. */
  onOpenNote?: (relPath: string) => void
  /** Bumped on knowledge:changed to invalidate the embed cache. */
  embedCacheKey?: number
  /** Outline-mode heading jump: the caller switches to an editable mode and
   *  reveals the line so the jump is visible (CM6 isn't mounted in outline mode). */
  onOutlineJump?: (line: number) => void
  /** Floating selection toolbar (Cursor-style): add the current selection to the
   *  AI chat as a quote. Shown automatically when text is selected. */
  onReferenceSelection?: () => void
  /** Floating selection toolbar: quick-rewrite the current selection. Hidden in
   *  read-only mode. */
  onRewriteSelection?: () => void
}

/** Imperative handle for AI-rewrite (WS9): read the current selection and splice
 *  rewritten text back in. Offsets are CM6 document positions. */
export interface NoteEditorHandle {
  getSelection: () => { from: number; to: number; text: string } | null
  replaceRange: (from: number, to: number, text: string) => void
  docLength: () => number
  /** Live editor text (CM6 doc) — used to re-anchor a stale quick-rewrite. */
  getText: () => string
}

const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "13px",
    backgroundColor: "transparent",
    // Follow the app's light / dark theme — plain (untagged) markdown text and
    // code-block bodies inherit this, so the editor is readable on both themes.
    color: "var(--color-foreground)",
  },
  ".cm-content": {
    fontFamily:
      "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace",
    padding: "12px 16px",
    color: "var(--color-foreground)",
    caretColor: "var(--color-foreground)",
  },
  ".cm-scroller": { overflow: "auto" },
  "&.cm-focused": { outline: "none" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none" },
  // drawSelection() paints its own layer; give it a visible accent tint on both
  // themes instead of CM6's default gray.
  ".cm-selectionBackground": {
    background: "color-mix(in srgb, #6366f1 20%, transparent)",
  },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    background: "color-mix(in srgb, #6366f1 28%, transparent)",
  },
  // highlightActiveLine() otherwise falls back to CM6's base-theme tint, which
  // reads as a muddy band. Use a per-theme token (defined in index.css) so the
  // current-line highlight stays very faint on light and a touch stronger on
  // dark; tune the two `--cm-active-line` values there, not here.
  ".cm-activeLine": {
    background: "var(--cm-active-line)",
  },
})

/** Persisted source-pane fraction for split mode (clamped to [0.2, 0.8]). */
const SPLIT_RATIO_KEY = "hope.knowledge.splitRatio"
const SPLIT_RATIO_MIN = 0.2
const SPLIT_RATIO_MAX = 0.8

function readSplitRatio(): number {
  if (typeof window === "undefined") return 0.5
  try {
    const n = Number(window.localStorage.getItem(SPLIT_RATIO_KEY))
    return Number.isFinite(n) && n >= SPLIT_RATIO_MIN && n <= SPLIT_RATIO_MAX ? n : 0.5
  } catch {
    return 0.5
  }
}

/**
 * CodeMirror 6 markdown note editor (design D13). Three modes: Source (CM6),
 * Preview (streamdown), Split. The document is always plain `.md` text — wikilink
 * chip decorations / autocomplete / broken-link lint operate on top of it.
 */
const NoteEditor = forwardRef<NoteEditorHandle, NoteEditorProps>(function NoteEditor(
  {
    value,
    onChange,
    readOnly = false,
    mode,
    data,
    revealTarget,
    kbId,
    notePath,
    onOpenNote,
    embedCacheKey = 0,
    onOutlineJump,
    onReferenceSelection,
    onRewriteSelection,
  },
  ref,
) {
  const hostRef = useRef<HTMLDivElement | null>(null)
  const previewRef = useRef<HTMLDivElement | null>(null)
  const viewRef = useRef<EditorView | null>(null)
  // Floating selection toolbar (Cursor-style) — viewport coords or null.
  const [selBar, setSelBar] = useState<{ top: number; left: number } | null>(null)
  const lastSelBarRef = useRef<{ top: number; left: number } | null>(null)
  if (selBar) lastSelBarRef.current = selBar
  const selBarRef = useRef<HTMLDivElement | null>(null)
  const onChangeRef = useRef(onChange)
  const dataRef = useRef<WikilinkData>(data)
  // Hover card (WS9) reads these so the once-built extension always sees the
  // current KB / cache epoch without rebuilding the editor.
  const kbIdRef = useRef<string | null | undefined>(kbId)
  const bustRef = useRef(embedCacheKey)
  const readOnlyComp = useRef(new Compartment())
  // Toggles the live-preview decorations (syntax-marker hiding) without rebuilding
  // the editor when switching between source / live / split.
  const liveComp = useRef(new Compartment())
  // True only while we push an external `value` into the editor programmatically
  // (opening / switching a note). The updateListener checks it so a programmatic
  // doc swap is NOT reported as a user edit — otherwise just opening a note marks
  // it dirty.
  const applyingExternalRef = useRef(false)
  // Split-mode (source + preview) draggable divider. Ratio = source-pane
  // fraction of the editor area; persisted so it survives reloads.
  const splitContainerRef = useRef<HTMLDivElement | null>(null)
  const [splitRatio, setSplitRatio] = useState(readSplitRatio)
  const [isSplitResizing, setIsSplitResizing] = useState(false)
  const { t } = useTranslation()

  onChangeRef.current = onChange
  dataRef.current = data
  kbIdRef.current = kbId
  bustRef.current = embedCacheKey

  useEffect(() => {
    if (typeof window === "undefined") return
    try {
      window.localStorage.setItem(SPLIT_RATIO_KEY, String(splitRatio))
    } catch {
      /* private mode / quota — non-fatal */
    }
  }, [splitRatio])

  const onSplitDragStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const container = splitContainerRef.current
      if (!container) return
      const totalWidth = container.getBoundingClientRect().width
      if (totalWidth <= 0) return
      setIsSplitResizing(true)
      const startX = e.clientX
      const startRatio = splitRatio
      const onMove = (ev: MouseEvent) => {
        const next = Math.min(
          SPLIT_RATIO_MAX,
          Math.max(SPLIT_RATIO_MIN, startRatio + (ev.clientX - startX) / totalWidth),
        )
        setSplitRatio(next)
      }
      // Preview may host iframes (Mermaid) — suspend them so the drag isn't eaten.
      const iframes = document.querySelectorAll("iframe")
      iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = "none"))
      const onUp = () => {
        document.removeEventListener("mousemove", onMove)
        document.removeEventListener("mouseup", onUp)
        window.removeEventListener("blur", onUp)
        document.body.style.cursor = ""
        document.body.style.userSelect = ""
        iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = ""))
        setIsSplitResizing(false)
      }
      document.addEventListener("mousemove", onMove)
      document.addEventListener("mouseup", onUp)
      window.addEventListener("blur", onUp)
      document.body.style.cursor = "col-resize"
      document.body.style.userSelect = "none"
    },
    [splitRatio],
  )

  const showSource = mode === "source" || mode === "split" || mode === "live"
  const showPreview = mode === "preview" || mode === "split"
  const showOutline = mode === "outline"

  // Seed the transclusion cycle guard with the note itself so `![[self]]` is
  // caught at depth 0. New Set identity per note path.
  const embedSeen = useMemo(
    () => (notePath ? new Set([notePath]) : new Set<string>()),
    [notePath],
  )

  // Create the editor once when the source pane mounts.
  useEffect(() => {
    if (!showSource || !hostRef.current || viewRef.current) return
    const getData = () => dataRef.current
    const state = EditorState.create({
      doc: value,
      extensions: [
        history(),
        drawSelection(),
        highlightSpecialChars(),
        highlightActiveLine(),
        indentOnInput(),
        EditorView.lineWrapping,
        closeBrackets(),
        syntaxHighlighting(noteHighlightStyle, { fallback: true }),
        // GFM base — enables strikethrough / task lists / tables / autolinks in
        // the parse tree (live-preview decorations render these). `codeLanguages`
        // lazy-loads a per-language parser for fenced code blocks (```lang) so
        // `syntaxHighlighting` colors them by language in every mode.
        markdown({ base: markdownLanguage, codeLanguages }),
        wikilinkTheme,
        wikilinkDecorations(getData),
        wikilinkCompletion(getData),
        brokenLinkLinter(getData),
        wikilinkHover(
          () => kbIdRef.current ?? null,
          (kb, reference) =>
            // Drop `|alias` / `#anchor` so the shared ref cache keys match the
            // transclusion view's (both resolve to the same target note).
            fetchNoteRef(kb, cleanEmbedRef(reference), bustRef.current).then((res) =>
              res.status === "resolved"
                ? { title: res.note.title, excerpt: noteExcerpt(res.note.content) }
                : null,
            ),
        ),
        notePreviewTheme,
        notePreviewWidgets(),
        noteLiveTheme,
        liveComp.current.of(mode === "live" ? [noteLiveDecorations()] : []),
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...completionKeymap,
          ...lintKeymap,
          ...searchKeymap,
        ]),
        editorTheme,
        readOnlyComp.current.of([
          EditorState.readOnly.of(readOnly),
          EditorView.editable.of(!readOnly),
        ]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged && !applyingExternalRef.current) {
            onChangeRef.current(u.state.doc.toString())
          }
          // Drive the floating selection toolbar on selection / doc change.
          if (u.selectionSet || u.docChanged) {
            const sel = u.state.selection.main
            if (sel.empty) {
              setSelBar(null)
            } else {
              const text = u.state.doc.sliceString(sel.from, sel.to)
              const c = text.trim() ? u.view.coordsAtPos(sel.from) : null
              setSelBar(
                c
                  ? {
                      top: Math.max(8, c.top - 40),
                      left: Math.min(c.left, window.innerWidth - 200),
                    }
                  : null,
              )
            }
          }
        }),
        // Hide the floating toolbar on scroll (its fixed coords would go stale).
        EditorView.domEventHandlers({
          scroll() {
            setSelBar(null)
          },
        }),
      ],
    })
    const view = new EditorView({ state, parent: hostRef.current })
    viewRef.current = view
    return () => {
      view.destroy()
      viewRef.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showSource])

  // Sync external value changes into the editor.
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const current = view.state.doc.toString()
    if (value !== current) {
      // Guard the listener so this programmatic swap isn't seen as a user edit.
      applyingExternalRef.current = true
      try {
        view.dispatch({ changes: { from: 0, to: current.length, insert: value } })
      } finally {
        applyingExternalRef.current = false
      }
    }
  }, [value])

  // Scroll + place the cursor at the reveal target (backlink / search precision
  // nav, G3). Declared after the value-sync effect so the doc is already updated
  // when we resolve the line. Clamps to valid bounds; no-op if no editor view.
  useEffect(() => {
    if (!revealTarget) return
    const view = viewRef.current
    if (!view) return
    const lineCount = view.state.doc.lines
    const lineNo = Math.min(Math.max(revealTarget.line, 1), lineCount)
    const line = view.state.doc.line(lineNo)
    const pos = Math.min(line.from + Math.max(revealTarget.col ?? 0, 0), line.to)
    view.dispatch({
      selection: EditorSelection.cursor(pos),
      effects: EditorView.scrollIntoView(pos, { y: "center" }),
    })
  }, [revealTarget])

  // Reconfigure read-only without rebuilding the editor.
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: readOnlyComp.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    })
  }, [readOnly])

  // Toggle live-preview decorations on mode change without rebuilding the editor.
  // (The build effect only re-runs on a source-visibility flip; source↔live↔split
  // all keep the source pane mounted, so reconfigure the compartment instead.)
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: liveComp.current.reconfigure(mode === "live" ? [noteLiveDecorations()] : []),
    })
  }, [mode])

  // Sync scroll between the source and preview panes in split mode (WS9): map by
  // scroll fraction, with a one-frame lock so the programmatic scroll on one pane
  // doesn't echo back from the other. Re-binds whenever the editor is (re)built.
  useEffect(() => {
    if (mode !== "split") return
    const view = viewRef.current
    const preview = previewRef.current
    if (!view || !preview) return
    const source = view.scrollDOM
    let locked = false
    const fraction = (el: HTMLElement) => {
      const max = el.scrollHeight - el.clientHeight
      return max > 0 ? el.scrollTop / max : 0
    }
    const sync = (from: HTMLElement, to: HTMLElement) => {
      if (locked) return
      locked = true
      to.scrollTop = fraction(from) * (to.scrollHeight - to.clientHeight)
      requestAnimationFrame(() => {
        locked = false
      })
    }
    const onSource = () => sync(source, preview)
    const onPreview = () => sync(preview, source)
    source.addEventListener("scroll", onSource, { passive: true })
    preview.addEventListener("scroll", onPreview, { passive: true })
    return () => {
      source.removeEventListener("scroll", onSource)
      preview.removeEventListener("scroll", onPreview)
    }
    // Only `mode` — the editor is rebuilt solely on a source-visibility toggle, so
    // scrollDOM is stable across edits; no need to re-bind on every keystroke.
  }, [mode])

  // Imperative API for AI rewrite (WS9). Splicing via dispatch goes through the
  // updateListener, so the host's onChange fires (marks the note dirty) naturally.
  useImperativeHandle(ref, () => ({
    getSelection: () => {
      const view = viewRef.current
      if (!view) return null
      const { from, to } = view.state.selection.main
      return { from, to, text: view.state.sliceDoc(from, to) }
    },
    replaceRange: (from, to, text) => {
      const view = viewRef.current
      if (!view) return
      const len = view.state.doc.length
      const f = Math.max(0, Math.min(from, len))
      const tt = Math.max(f, Math.min(to, len))
      view.dispatch({
        changes: { from: f, to: tt, insert: text },
        selection: EditorSelection.cursor(f + text.length),
      })
      view.focus()
    },
    docLength: () => viewRef.current?.state.doc.length ?? 0,
    getText: () => viewRef.current?.state.doc.toString() ?? "",
  }))

  // Dismiss the floating selection toolbar on any outside mousedown.
  useEffect(() => {
    if (!selBar) return
    const onDown = (e: MouseEvent) => {
      if (selBarRef.current?.contains(e.target as Node)) return
      setSelBar(null)
    }
    document.addEventListener("mousedown", onDown)
    return () => document.removeEventListener("mousedown", onDown)
  }, [selBar])

  if (showOutline) {
    return <OutlineView content={value} onJump={onOutlineJump} />
  }

  const splitActive = showSource && showPreview
  const showSelBar = selBar && (onReferenceSelection || (!readOnly && onRewriteSelection))
  const renderedSelBar = selBar ?? lastSelBarRef.current

  return (
    <div ref={splitContainerRef} className="flex h-full min-h-0 w-full">
      {showSource && (
        <div
          ref={hostRef}
          className={`h-full min-h-0 overflow-hidden ${splitActive ? "shrink-0" : "w-full"}`}
          style={splitActive ? { width: `${splitRatio * 100}%` } : undefined}
        />
      )}
      {splitActive && (
        <div
          className={`relative w-px shrink-0 cursor-col-resize transition-colors ${isSplitResizing ? "bg-primary/50" : "bg-border-soft/60 hover:bg-primary/35"}`}
          onMouseDown={onSplitDragStart}
          role="separator"
          aria-orientation="vertical"
          aria-label={t("knowledge.resizeSplit", "Resize split")}
        >
          {/* Wider invisible hit area around the 1px divider. */}
          <div className="absolute inset-y-0 -left-1 -right-1" />
        </div>
      )}
      {showPreview && (
        <div
          ref={previewRef}
          className={`h-full min-h-0 overflow-auto p-4 ${showSource ? "min-w-0 flex-1" : "w-full"}`}
        >
          {kbId ? (
            <NoteTransclusionView
              kbId={kbId}
              content={value}
              cacheBustKey={embedCacheKey}
              onOpenNote={onOpenNote}
              seen={embedSeen}
              highlightLine={revealTarget?.line}
              highlightToken={revealTarget}
            />
          ) : (
            <MarkdownRenderer content={value} />
          )}
        </div>
      )}

      {/* Floating selection toolbar (Cursor-style) — appears on text selection. */}
      {renderedSelBar ? (
        <FloatingMenu
          open={Boolean(showSelBar)}
          strategy="fixed"
          portal
          positionClassName=""
          originClassName="origin-bottom-left"
          className="p-0.5"
          style={{ top: renderedSelBar.top, left: renderedSelBar.left }}
        >
          <div ref={selBarRef} className="flex items-center gap-0.5">
          {onReferenceSelection && (
            <button
              type="button"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                onReferenceSelection()
                setSelBar(null)
              }}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-foreground/90 transition-colors hover:bg-secondary"
            >
              <MessageSquareQuote className="h-3.5 w-3.5 text-muted-foreground" />
              {t("knowledge.chatPanel.addToChat", "Add to AI chat")}
            </button>
          )}
          {!readOnly && onRewriteSelection && (
            <button
              type="button"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                onRewriteSelection()
                setSelBar(null)
              }}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-foreground/90 transition-colors hover:bg-secondary"
            >
              <Sparkles className="h-3.5 w-3.5 text-muted-foreground" />
              {t("knowledge.quickRewrite.title", "Quick rewrite")}
            </button>
          )}
          </div>
        </FloatingMenu>
      ) : null}
    </div>
  )
})

export default memo(NoteEditor)
