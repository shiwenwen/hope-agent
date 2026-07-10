import { history, historyKeymap, insertNewline } from "@codemirror/commands"
import { Compartment, EditorState, RangeSetBuilder, type Extension } from "@codemirror/state"
import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  drawSelection,
  keymap,
  placeholder as cmPlaceholder,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view"
import {
  createElement,
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react"
import { createRoot, type Root } from "react-dom/client"
import { FileText, Folder } from "lucide-react"
import { useTranslation } from "react-i18next"

import { AgentAvatarBadge } from "@/components/common/AgentSelectDisplay"
import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import { useFileActions } from "@/components/chat/files/useFileActions"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import { basename } from "@/lib/path"
import { cn } from "@/lib/utils"
import type { AgentSummaryForSidebar } from "@/types/chat"
import { parseAgentMentions } from "../agent-mention/agentTokens"
import { parseMentions, parseNoteRefs } from "../file-mention/mentionTokens"
import { joinAbs } from "../file-mention/types"
import { SkillMentionIcon } from "../skill-mention/SkillMentionIcon"
import {
  isSkillMentionName,
  parseSkillMentions,
  skillMentionMeta,
} from "../skill-mention/skillTokens"
import type { ComposerInputHandle } from "./composerInputHandle"

type MentionSpan =
  | { kind: "file"; raw: string; relPath: string; start: number; end: number }
  | { kind: "note"; raw: string; start: number; end: number }
  | { kind: "skill"; raw: string; name: string; start: number; end: number }
  | { kind: "agent"; raw: string; agentId: string; start: number; end: number }

export interface ComposerPasteEvent {
  clipboardData: DataTransfer | null
  preventDefault: () => void
  defaultPrevented?: boolean
}

interface MentionComposerInputProps {
  value: string
  placeholder: string
  workingDir: string | null
  fileEnabled: boolean
  noteEnabled: boolean
  /** Render `@skill:<name>` (allowlisted built-ins) as rose chips. */
  skillEnabled?: boolean
  /** Render `@agent` delegation mentions as teal chips. */
  agentMentionEnabled?: boolean
  agents?: AgentSummaryForSidebar[]
  hero?: boolean
  readOnly?: boolean
  onChange: (value: string) => void
  onKeyDown: (e: React.KeyboardEvent<HTMLElement>) => void
  onPaste: (e: ComposerPasteEvent) => void
  onSelectionChange: () => void
}

interface MentionConfig {
  workingDir: string | null
  fileEnabled: boolean
  noteEnabled: boolean
  skillEnabled: boolean
  agentMentionEnabled: boolean
  /** Resolve a skill id → localized chip label (threaded from `t`). */
  skillLabel: (name: string) => string
  /** Resolve an agent id → current summary for avatar/name chips. */
  agentById: (id: string) => AgentSummaryForSidebar | undefined
}

const FILE_CHIP_CLASS =
  "cm-mention-chip cm-mention-file mx-0.5 inline-flex h-6 max-w-[16rem] align-baseline items-center gap-1 rounded-md border border-blue-500/20 bg-blue-500/10 px-1.5 text-sm font-medium text-blue-600 shadow-sm outline-none dark:border-blue-300/20 dark:bg-blue-300/15 dark:text-blue-200"
const NOTE_CHIP_CLASS =
  "cm-mention-chip cm-mention-note mx-0.5 inline-flex h-6 max-w-[16rem] align-baseline items-center rounded-md border border-violet-500/20 bg-violet-500/10 px-1.5 text-sm font-medium text-violet-600 shadow-sm dark:border-violet-300/20 dark:bg-violet-300/15 dark:text-violet-200"
const SKILL_CHIP_CLASS =
  "cm-mention-chip cm-mention-skill mx-0.5 inline-flex h-6 max-w-[16rem] align-baseline items-center gap-1 rounded-md border border-rose-500/20 bg-rose-500/10 px-1.5 text-sm font-medium text-rose-600 shadow-sm dark:border-rose-300/20 dark:bg-rose-300/15 dark:text-rose-200"
const AGENT_CHIP_CLASS =
  "cm-mention-chip cm-mention-agent mx-0.5 inline-flex h-6 max-w-[16rem] align-baseline items-center gap-1 rounded-md border border-teal-500/20 bg-teal-500/10 px-1.5 text-sm font-medium text-teal-700 shadow-sm dark:border-teal-300/20 dark:bg-teal-300/15 dark:text-teal-200"
const CHIP_ICON_CLASS = "h-4 w-4 shrink-0"
const CHIP_LABEL_CLASS = "truncate"
const widgetIconRoots = new WeakMap<HTMLElement, Root>()

function isCommittedMention(input: string, end: number): boolean {
  return end < input.length && /\s/.test(input[end] ?? "")
}

function mentionSpans(input: string, config: MentionConfig): MentionSpan[] {
  const spans: MentionSpan[] = []

  if (config.fileEnabled) {
    for (const mention of parseMentions(input)) {
      if (!isCommittedMention(input, mention.end)) continue
      // `plan:` belongs to its own picker, not file chips. (Skill mentions use
      // the `[@…](#skill:…)` link form — their `@` sits after `[`, so the bare
      // `@token` file grammar never matches them.)
      if (mention.relPath.startsWith("plan:")) continue
      spans.push({
        kind: "file",
        raw: mention.raw,
        relPath: mention.relPath,
        start: mention.start,
        end: mention.end,
      })
    }
  }

  if (config.noteEnabled) {
    for (const note of parseNoteRefs(input)) {
      spans.push({ kind: "note", raw: note.raw, start: note.start, end: note.end })
    }
  }

  if (config.skillEnabled) {
    // The `[@label](#skill:name)` link is self-delimiting — no trailing-space
    // commitment gate needed. Only allowlisted built-ins become chips.
    for (const skill of parseSkillMentions(input)) {
      if (!isSkillMentionName(skill.name)) continue
      spans.push({
        kind: "skill",
        raw: skill.raw,
        name: skill.name,
        start: skill.start,
        end: skill.end,
      })
    }
  }

  if (config.agentMentionEnabled) {
    // Same markdown-link token shape as `@skill`, so it is self-delimiting and
    // cannot collide with bare `@path` file mentions.
    for (const agent of parseAgentMentions(input)) {
      spans.push({
        kind: "agent",
        raw: agent.raw,
        agentId: agent.agentId,
        start: agent.start,
        end: agent.end,
      })
    }
  }

  spans.sort((a, b) => a.start - b.start)
  const out: MentionSpan[] = []
  let cursor = 0
  for (const span of spans) {
    if (span.start < cursor) continue
    out.push(span)
    cursor = span.end
  }
  return out
}

function appendText(parent: HTMLElement, className: string, text: string) {
  const el = document.createElement("span")
  el.className = className
  el.textContent = text
  parent.appendChild(el)
}

function appendIcon(parent: HTMLElement, icon: React.ReactNode) {
  const mount = document.createElement("span")
  mount.className = "inline-flex shrink-0 items-center justify-center"
  parent.appendChild(mount)
  const root = createRoot(mount)
  root.render(icon)
  widgetIconRoots.set(mount, root)
}

class MentionWidget extends WidgetType {
  private readonly span: MentionSpan
  private readonly workingDir: string | null
  private readonly skillLabel: (name: string) => string
  private readonly agentById: (id: string) => AgentSummaryForSidebar | undefined

  constructor(
    span: MentionSpan,
    workingDir: string | null,
    skillLabel: (name: string) => string,
    agentById: (id: string) => AgentSummaryForSidebar | undefined,
  ) {
    super()
    this.span = span
    this.workingDir = workingDir
    this.skillLabel = skillLabel
    this.agentById = agentById
  }

  eq(other: MentionWidget): boolean {
    return (
      other.span.kind === this.span.kind &&
      other.span.raw === this.span.raw &&
      (other.span.kind !== "file" ||
        (this.span.kind === "file" && other.span.relPath === this.span.relPath)) &&
      (other.span.kind !== "agent" ||
        (this.span.kind === "agent" && other.span.agentId === this.span.agentId)) &&
      other.workingDir === this.workingDir &&
      other.skillLabel === this.skillLabel &&
      other.agentById === this.agentById
    )
  }

  toDOM(): HTMLElement {
    const root = document.createElement("span")
    root.dataset.mentionKind = this.span.kind
    root.dataset.mentionRaw = this.span.raw
    root.setAttribute("contenteditable", "false")

    if (this.span.kind === "note") {
      const title = this.span.raw.slice(2, -2)
      root.className = NOTE_CHIP_CLASS
      root.title = this.span.raw
      appendIcon(root, createElement(FileText, { className: CHIP_ICON_CLASS }))
      appendText(root, CHIP_LABEL_CLASS, title)
      return root
    }

    if (this.span.kind === "skill") {
      const meta = skillMentionMeta(this.span.name)
      root.className = SKILL_CHIP_CLASS
      root.title = this.span.raw
      if (meta) {
        appendIcon(
          root,
          createElement(SkillMentionIcon, { kind: meta.iconKind, className: CHIP_ICON_CLASS }),
        )
      }
      appendText(root, CHIP_LABEL_CLASS, this.skillLabel(this.span.name))
      return root
    }

    if (this.span.kind === "agent") {
      const agent = this.agentById(this.span.agentId)
      const label = agent?.name || this.span.agentId
      root.className = AGENT_CHIP_CLASS
      root.title = label
      appendIcon(
        root,
        createElement(AgentAvatarBadge, {
          agent: agent ?? { id: this.span.agentId, name: label },
          size: "xs",
        }),
      )
      appendText(root, CHIP_LABEL_CLASS, label)
      return root
    }

    const isDir = /[\\/]$/.test(this.span.relPath)
    const normalizedRel = this.span.relPath.replace(/[\\/]+$/, "")
    const name = basename(normalizedRel || this.span.relPath)
    const fullPath = this.workingDir
      ? joinAbs(this.workingDir, normalizedRel || this.span.relPath)
      : this.span.relPath
    root.className = FILE_CHIP_CLASS
    root.dataset.mentionRelPath = this.span.relPath
    root.setAttribute("role", "button")
    root.setAttribute("aria-label", fullPath)
    root.title = fullPath
    appendIcon(
      root,
      isDir
        ? createElement(Folder, { className: `${CHIP_ICON_CLASS} text-blue-500` })
        : createElement(FileTypeIcon, { name, className: CHIP_ICON_CLASS }),
    )
    appendText(root, CHIP_LABEL_CLASS, name)
    return root
  }

  destroy(dom: HTMLElement): void {
    for (const child of Array.from(dom.children)) {
      const root = widgetIconRoots.get(child as HTMLElement)
      if (root) {
        root.unmount()
        widgetIconRoots.delete(child as HTMLElement)
      }
    }
  }

  ignoreEvent() {
    return true
  }
}

function mentionDecorations(getConfig: () => MentionConfig) {
  const build = (view: EditorView): DecorationSet => {
    const config = getConfig()
    const text = view.state.doc.toString()
    const builder = new RangeSetBuilder<Decoration>()
    for (const span of mentionSpans(text, config)) {
      builder.add(
        span.start,
        span.end,
        Decoration.replace({
          widget: new MentionWidget(span, config.workingDir, config.skillLabel, config.agentById),
          inclusive: false,
        }),
      )
    }
    return builder.finish()
  }

  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet

      constructor(view: EditorView) {
        this.decorations = build(view)
      }

      update(update: ViewUpdate) {
        // Never rebuild decorations mid-IME-composition. Swapping the decoration
        // set (and its atomic ranges) forces CM6 to redraw the line holding the
        // active composition, which aborts third-party IMEs (e.g. Sogou Pinyin)
        // on Chromium/WebView2. Keep existing chips aligned by mapping them
        // through the change; do a full rebuild once composition ends.
        if (update.view.composing) {
          if (update.docChanged) this.decorations = this.decorations.map(update.changes)
          return
        }
        if (update.docChanged || update.viewportChanged || update.transactions.length > 0) {
          this.decorations = build(update.view)
        }
      }
    },
    {
      decorations: (value) => value.decorations,
      provide: (self) =>
        EditorView.atomicRanges.of((view) => view.plugin(self)?.decorations ?? Decoration.none),
    },
  )

  return plugin
}

function adjacentMentionDeletion(
  value: string,
  caret: number,
  key: "Backspace" | "Delete",
  config: MentionConfig,
) {
  const spans = mentionSpans(value, config)
  for (const span of spans) {
    if (key === "Backspace" && (caret === span.end || caret === span.end + 1)) {
      return { from: span.start, to: caret === span.end + 1 ? caret : span.end }
    }
    if (key === "Delete" && caret === span.start) {
      return { from: span.start, to: value[span.end] === " " ? span.end + 1 : span.end }
    }
  }
  return null
}

function atomicDeleteExtension(getConfig: () => MentionConfig) {
  return EditorView.domEventHandlers({
    keydown(event, view) {
      // Never intercept deletes during an IME composition. preventDefault() +
      // dispatch here would abort the composition for third-party Windows IMEs.
      if (event.isComposing || event.keyCode === 229) return false
      if (
        (event.key !== "Backspace" && event.key !== "Delete") ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey
      ) {
        return false
      }

      const selection = view.state.selection.main
      if (!selection.empty) return false
      const deletion = adjacentMentionDeletion(
        view.state.doc.toString(),
        selection.from,
        event.key,
        getConfig(),
      )
      if (!deletion) return false

      event.preventDefault()
      view.dispatch({
        changes: { from: deletion.from, to: deletion.to },
        selection: { anchor: deletion.from },
        scrollIntoView: true,
      })
      return true
    },
  })
}

function pasteExtension(getOnPaste: () => (event: ComposerPasteEvent) => void) {
  return EditorView.domEventHandlers({
    paste(event) {
      getOnPaste()(event)
      return event.defaultPrevented
    },
  })
}

const CODEMIRROR_EDIT_CONTEXT_FLAG = "__HOPE_AGENT_CODEMIRROR_EDIT_CONTEXT"

type CodeMirrorEditContextWindow = Window & {
  __HOPE_AGENT_CODEMIRROR_EDIT_CONTEXT?: boolean
}

function shouldForceCodeMirrorEditContext(): boolean {
  if (typeof window === "undefined" || typeof navigator === "undefined") return false
  if (!("EditContext" in window)) return false
  const ua = navigator.userAgent
  if (!/\bWindows NT\b/.test(ua)) return false
  const version = ua.match(/\b(?:Chrome|Chromium|Edg)\/(\d+)/)?.[1]
  return version ? Number(version) >= 126 : false
}

function withCodeMirrorEditContext<T>(enabled: boolean, create: () => T): T {
  if (!enabled) return create()
  const target = window as unknown as CodeMirrorEditContextWindow
  const previous = target[CODEMIRROR_EDIT_CONTEXT_FLAG]
  target[CODEMIRROR_EDIT_CONTEXT_FLAG] = true
  try {
    return create()
  } finally {
    if (previous === undefined) {
      delete target[CODEMIRROR_EDIT_CONTEXT_FLAG]
    } else {
      target[CODEMIRROR_EDIT_CONTEXT_FLAG] = previous
    }
  }
}

// CodeMirror ships an EditContext input path, but gates it to Android. The
// postinstall patch in scripts/patch-codemirror-edit-context.mjs adds this
// scoped opt-in so the Windows chat composer can bypass Chrome/WebView2's
// legacy contenteditable IME path without changing the editor surface.
const FORCE_CODEMIRROR_EDIT_CONTEXT = shouldForceCodeMirrorEditContext()

const baseTheme = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "inherit",
    fontFamily: "inherit",
    fontSize: "inherit",
  },
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-scroller": {
    maxHeight: "40vh",
    overflow: "auto",
    fontFamily: "inherit",
    lineHeight: "1.5",
  },
  ".cm-content": {
    // Native caret hidden — drawSelection() paints `.cm-cursor` instead so the
    // caret shows even on an empty doc under WebKit.
    caretColor: "transparent",
    fontFamily: "inherit",
    minHeight: "42px",
    outline: "none",
    padding: "12px 16px 4px",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "currentColor",
    borderLeftWidth: "1.5px",
  },
  // drawSelection() paints its own selection layer; give it a visible tint
  // (CM6's gray default clashes with the app theme). Match CM6's focused
  // selector specificity so the focused state isn't left at the default.
  ".cm-selectionBackground": {
    background: "color-mix(in srgb, var(--primary, #3b82f6) 22%, transparent)",
  },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    background: "color-mix(in srgb, var(--primary, #3b82f6) 28%, transparent)",
  },
  ".cm-line": {
    padding: "0",
  },
  ".cm-mention-chip": {
    verticalAlign: "baseline",
    whiteSpace: "nowrap",
  },
  ".cm-mention-file:hover": {
    backgroundColor: "color-mix(in srgb, var(--primary, #3b82f6) 15%, transparent)",
  },
})

function sizeTheme(hero: boolean): Extension {
  return EditorView.theme({
    ".cm-content": {
      minHeight: hero ? "72px" : "42px",
      paddingTop: hero ? "16px" : "12px",
      paddingBottom: hero ? "8px" : "4px",
    },
  })
}

function composerPlaceholder(text: string): Extension {
  return [
    cmPlaceholder(() => {
      const span = document.createElement("span")
      span.textContent = text
      span.style.color = "hsl(var(--muted-foreground) / 0.24)"
      return span
    }),
    EditorView.contentAttributes.of({ "aria-placeholder": text }),
  ]
}

const MentionComposerInput = forwardRef<ComposerInputHandle, MentionComposerInputProps>(
  function MentionComposerInput(
    {
      value,
      placeholder,
      workingDir,
      fileEnabled,
      noteEnabled,
      skillEnabled = false,
      agentMentionEnabled = false,
      agents = [],
      hero = false,
      readOnly = false,
      onChange,
      onKeyDown,
      onPaste,
      onSelectionChange,
    },
    ref,
  ) {
    const { t } = useTranslation()
    // Resolve a skill id → localized chip label. Stable per language so the
    // widget's `eq` can treat it as constant between renders.
    const skillLabel = useCallback(
      (name: string) => {
        const meta = skillMentionMeta(name)
        return meta ? t(meta.labelKey) : name
      },
      [t],
    )
    const agentById = useCallback((id: string) => agents.find((agent) => agent.id === id), [agents])
    const hostRef = useRef<HTMLDivElement | null>(null)
    const viewRef = useRef<EditorView | null>(null)
    const valueRef = useRef(value)
    const onChangeRef = useRef(onChange)
    const onPasteRef = useRef(onPaste)
    const onSelectionChangeRef = useRef(onSelectionChange)
    const configRef = useRef<MentionConfig>({
      workingDir,
      fileEnabled,
      noteEnabled,
      skillEnabled,
      agentMentionEnabled,
      skillLabel,
      agentById,
    })
    const readOnlyComp = useRef(new Compartment())
    const placeholderComp = useRef(new Compartment())
    const sizeComp = useRef(new Compartment())
    const syncingExternalRef = useRef(false)
    const nextFileActionIdRef = useRef(0)
    const handledFileActionIdRef = useRef(0)
    const [pendingFileAction, setPendingFileAction] = useState<{
      id: number
      target: PreviewTarget
    } | null>(null)

    valueRef.current = value
    onChangeRef.current = onChange
    onPasteRef.current = onPaste
    onSelectionChangeRef.current = onSelectionChange
    configRef.current = {
      workingDir,
      fileEnabled,
      noteEnabled,
      skillEnabled,
      agentMentionEnabled,
      skillLabel,
      agentById,
    }

    const { primary: pendingFilePrimary, run: runPendingFileAction } = useFileActions(
      pendingFileAction?.target ?? null,
    )

    useEffect(() => {
      if (!pendingFileAction || handledFileActionIdRef.current === pendingFileAction.id) return
      handledFileActionIdRef.current = pendingFileAction.id
      runPendingFileAction(pendingFilePrimary)
      setPendingFileAction(null)
    }, [pendingFileAction, pendingFilePrimary, runPendingFileAction])

    useLayoutEffect(() => {
      const parent = hostRef.current
      if (!parent || viewRef.current) return

      const view = withCodeMirrorEditContext(
        FORCE_CODEMIRROR_EDIT_CONTEXT,
        () =>
          new EditorView({
            parent,
            state: EditorState.create({
              doc: valueRef.current,
              extensions: [
                history(),
                // Shift+Enter inserts a soft line break (standard IM
                // convention). Plain Enter is deliberately left unbound so it
                // bubbles to ChatInput's onKeyDown, which sends the message —
                // CM6 without a binding swallows the structural edit, which is
                // also why the editor could never insert a newline before.
                keymap.of([{ key: "Shift-Enter", run: insertNewline }, ...historyKeymap]),
                EditorView.lineWrapping,
                // WebKit (Tauri) doesn't paint the native caret in an empty
                // contenteditable; CM6 draws its own reliable, blinking cursor.
                drawSelection(),
                baseTheme,
                sizeComp.current.of(sizeTheme(hero)),
                placeholderComp.current.of(composerPlaceholder(placeholder)),
                readOnlyComp.current.of([
                  EditorState.readOnly.of(readOnly),
                  EditorView.editable.of(!readOnly),
                ]),
                mentionDecorations(() => configRef.current),
                atomicDeleteExtension(() => configRef.current),
                pasteExtension(() => onPasteRef.current),
                EditorView.updateListener.of((update) => {
                  if (update.docChanged) {
                    const next = update.state.doc.toString()
                    valueRef.current = next
                    if (!syncingExternalRef.current) onChangeRef.current(next)
                  }
                  if (update.docChanged || update.selectionSet) {
                    onSelectionChangeRef.current()
                  }
                }),
              ],
            }),
          }),
      )

      viewRef.current = view
      return () => {
        view.destroy()
        viewRef.current = null
      }
      // Create once. Mutable refs above provide live callbacks/config.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    useLayoutEffect(() => {
      const view = viewRef.current
      if (!view) return
      // Never reconcile the external value while an IME composition is active.
      // Dispatching a doc-replacing transaction mid-composition aborts the
      // composition. CM6 fires its own docChanged on compositionend, after which
      // value/doc reconverge and any genuinely-needed sync runs later.
      if (view.composing) return
      const current = view.state.doc.toString()
      if (value === current) return

      syncingExternalRef.current = true
      const selection = view.state.selection.main
      const anchor = Math.min(selection.anchor, value.length)
      const head = Math.min(selection.head, value.length)
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
        selection: { anchor, head },
      })
      syncingExternalRef.current = false
    }, [value])

    useEffect(() => {
      const view = viewRef.current
      if (!view) return
      view.dispatch({
        effects: [
          readOnlyComp.current.reconfigure([
            EditorState.readOnly.of(readOnly),
            EditorView.editable.of(!readOnly),
          ]),
          placeholderComp.current.reconfigure(composerPlaceholder(placeholder)),
          sizeComp.current.reconfigure(sizeTheme(hero)),
        ],
      })
    }, [hero, placeholder, readOnly])

    useEffect(() => {
      const view = viewRef.current
      if (!view) return
      view.dispatch({})
    }, [
      agentById,
      agentMentionEnabled,
      fileEnabled,
      noteEnabled,
      skillEnabled,
      skillLabel,
      workingDir,
    ])

    useImperativeHandle(
      ref,
      () => ({
        focus: () => viewRef.current?.focus(),
        getValue: () => valueRef.current,
        getSelectionRange: () => {
          const selection = viewRef.current?.state.selection.main
          if (!selection) {
            const end = valueRef.current.length
            return { start: end, end }
          }
          return {
            start: Math.min(selection.from, selection.to),
            end: Math.max(selection.from, selection.to),
          }
        },
        setSelectionRange: (start: number, end: number) => {
          const view = viewRef.current
          if (!view) return
          const length = view.state.doc.length
          const anchor = Math.max(0, Math.min(start, length))
          const head = Math.max(0, Math.min(end, length))
          view.dispatch({ selection: { anchor, head }, scrollIntoView: true })
          view.focus()
        },
      }),
      [],
    )

    const fileChipFromEvent = useCallback((target: EventTarget | null): HTMLElement | null => {
      const host = hostRef.current
      if (!host || !(target instanceof Element)) return null
      const chip = target.closest<HTMLElement>('[data-mention-kind="file"]')
      return chip && host.contains(chip) ? chip : null
    }, [])

    const handleMouseDown = useCallback(
      (event: React.MouseEvent<HTMLDivElement>) => {
        if (fileChipFromEvent(event.target)) event.preventDefault()
      },
      [fileChipFromEvent],
    )

    const handleClick = useCallback(
      (event: React.MouseEvent<HTMLDivElement>) => {
        const chip = fileChipFromEvent(event.target)
        if (!chip) {
          onSelectionChange()
          return
        }

        event.preventDefault()
        event.stopPropagation()
        const relPath = chip.dataset.mentionRelPath
        if (!workingDir || !relPath) return
        const normalizedRel = relPath.replace(/[\\/]+$/, "")
        const name = basename(normalizedRel || relPath)
        setPendingFileAction({
          id: ++nextFileActionIdRef.current,
          target: { kind: "path", path: joinAbs(workingDir, normalizedRel || relPath), name },
        })
      },
      [fileChipFromEvent, onSelectionChange, workingDir],
    )

    return (
      <div
        ref={hostRef}
        role="textbox"
        data-chat-composer="true"
        aria-multiline="true"
        aria-readonly={readOnly}
        onKeyDown={onKeyDown}
        onMouseDown={handleMouseDown}
        onClick={handleClick}
        className={cn(
          "relative w-full cursor-text border-0 bg-transparent text-sm text-foreground outline-none focus-visible:ring-0",
          "min-h-[42px] max-h-[40vh] overflow-hidden",
          hero && "min-h-[72px]",
          readOnly && "opacity-80",
        )}
      />
    )
  },
)

export default MentionComposerInput
