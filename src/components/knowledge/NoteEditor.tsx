import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete"
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands"
import { markdown } from "@codemirror/lang-markdown"
import { defaultHighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language"
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
import { memo, useEffect, useMemo, useRef } from "react"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import type { NoteEditorMode } from "@/types/knowledge"

import NoteTransclusionView from "./NoteTransclusionView"

import {
  brokenLinkLinter,
  wikilinkCompletion,
  wikilinkDecorations,
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
}

const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "13px",
    backgroundColor: "transparent",
  },
  ".cm-content": {
    fontFamily:
      "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace",
    padding: "12px 16px",
  },
  ".cm-scroller": { overflow: "auto" },
  "&.cm-focused": { outline: "none" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none" },
})

/**
 * CodeMirror 6 markdown note editor (design D13). Three modes: Source (CM6),
 * Preview (streamdown), Split. The document is always plain `.md` text — wikilink
 * chip decorations / autocomplete / broken-link lint operate on top of it.
 */
function NoteEditor({
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
}: NoteEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null)
  const viewRef = useRef<EditorView | null>(null)
  const onChangeRef = useRef(onChange)
  const dataRef = useRef<WikilinkData>(data)
  const readOnlyComp = useRef(new Compartment())
  // True only while we push an external `value` into the editor programmatically
  // (opening / switching a note). The updateListener checks it so a programmatic
  // doc swap is NOT reported as a user edit — otherwise just opening a note marks
  // it dirty.
  const applyingExternalRef = useRef(false)

  onChangeRef.current = onChange
  dataRef.current = data

  const showSource = mode === "source" || mode === "split"
  const showPreview = mode === "preview" || mode === "split"

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
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        markdown(),
        wikilinkTheme,
        wikilinkDecorations(getData),
        wikilinkCompletion(getData),
        brokenLinkLinter(getData),
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

  return (
    <div className="flex h-full min-h-0 w-full">
      {showSource && (
        <div
          ref={hostRef}
          className={`h-full min-h-0 overflow-hidden ${showPreview ? "w-1/2 border-r border-border-soft/60" : "w-full"}`}
        />
      )}
      {showPreview && (
        <div
          className={`h-full min-h-0 overflow-auto p-4 ${showSource ? "w-1/2" : "w-full"}`}
        >
          {kbId ? (
            <NoteTransclusionView
              kbId={kbId}
              content={value}
              cacheBustKey={embedCacheKey}
              onOpenNote={onOpenNote}
              seen={embedSeen}
            />
          ) : (
            <MarkdownRenderer content={value} />
          )}
        </div>
      )}
    </div>
  )
}

export default memo(NoteEditor)
