import {
  AlertTriangle,
  Archive,
  ArchiveRestore,
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  Library,
  Link2,
  Loader2,
  Lock,
  FolderInput,
  MessageSquareQuote,
  PanelLeft,
  PanelLeftDashed,
  PanelRight,
  PanelRightDashed,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Settings,
  Sparkles,
  Trash2,
  Waypoints,
  X,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { IconTip } from "@/components/ui/tooltip"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"
import { logger } from "@/lib/logger"
import { isTauriMode } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type {
  KnowledgeBaseMeta,
  Note,
  NoteEditorMode,
  NoteReadResult,
  NoteSearchHit,
  RenameOutcome,
} from "@/types/knowledge"
import type { PendingFileQuote } from "@/types/chat"

import { useReembedJob } from "@/hooks/useReembedJob"
import { useDragWidth } from "@/hooks/useDragWidth"
import { useViewportMediaQuery } from "@/hooks/useViewportMediaQuery"
import { isLocalModelJobActive } from "@/types/local-model-jobs"

import HeadingOutline from "./HeadingOutline"
import KnowledgeEmbeddingBadge from "./KnowledgeEmbeddingBadge"
import KnowledgeGraphView from "./KnowledgeGraphView"
import KnowledgeJobsButton from "./KnowledgeJobsButton"
import KnowledgeMaintenanceButton from "./KnowledgeMaintenanceButton"
import NoteEditor, { type NoteEditorHandle } from "./NoteEditor"
import {
  KnowledgeChatPanel,
  type KnowledgeChatPanelHandle,
} from "./chat/KnowledgeChatPanel"
import { QuickRewriteBar } from "./chat/QuickRewriteBar"
import { buildKnownTargets, type WikilinkData } from "./cm/wikilinkExtensions"
import { parseHeadings } from "./outline"
import { formatNoteInsertion, relPathToken } from "@/components/chat/note-mention/noteTokens"

interface KnowledgeViewProps {
  onBack: () => void
  /** Jump to Settings → Knowledge (embedding / retrieval config). */
  onOpenSettings?: () => void
}

type SaveStatus = "idle" | "saved" | "failed"

// ── Layout: collapsible + resizable side panes (mirrors the chat view) ──────
const KB_LEFT_WIDTH_KEY = "hope.knowledge.leftWidth"
const KB_RIGHT_WIDTH_KEY = "hope.knowledge.rightWidth"
const KB_LEFT_COLLAPSED_KEY = "hope.knowledge.leftCollapsed"
const KB_RIGHT_COLLAPSED_KEY = "hope.knowledge.rightCollapsed"
const KB_LEFT_DEFAULT_WIDTH = 256 // = old w-64
const KB_LEFT_MIN_WIDTH = 200
const KB_LEFT_MAX_WIDTH = 420
const KB_RIGHT_DEFAULT_WIDTH = 288 // = old w-72
const KB_RIGHT_MIN_WIDTH = 220
const KB_RIGHT_MAX_WIDTH = 480
const KB_CONTENT_MIN_WIDTH = 360 // min usable editor column
const KB_SPLIT_MIN_WIDTH = 680 // editor area needs this for two side-by-side panes
// Below this editor-column width the crowded note toolbar can't fit the 5-button
// mode segmented control, so it collapses to a compact dropdown.
const KB_TOOLBAR_COMPACT_WIDTH = 560
const KB_LEFT_AUTO_COLLAPSE_GUTTER = 120
const KB_RESPONSIVE_HYSTERESIS = 120 // gap between collapse-at and expand-at (anti-flap)
// Collapse animation (must match the chat sidebar / RightPanelShell exactly).
const PANE_WIDTH_TRANSITION =
  "transition-[width] duration-[250ms] ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[width] motion-reduce:transition-none"
const PANE_SURFACE_TRANSITION =
  "transition-[opacity,transform] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] will-change-[opacity,transform] [contain:layout_paint] motion-reduce:transition-none"
const PANE_HANDLE_BASE =
  "absolute inset-y-0 z-20 cursor-col-resize transition-[width,opacity,background-color] duration-200 ease-out hover:bg-primary/30 active:bg-primary/50"

function readStoredBool(key: string): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(key) === "true"
  } catch {
    return false
  }
}

function clampPaneWidth(w: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, w))
}

function readStoredWidth(key: string, def: number, min: number, max: number): number {
  if (typeof window === "undefined") return def
  try {
    const n = Number(window.localStorage.getItem(key))
    return Number.isFinite(n) && n > 0 ? clampPaneWidth(n, min, max) : def
  } catch {
    return def
  }
}

export default function KnowledgeView({ onBack, onOpenSettings }: KnowledgeViewProps) {
  const { t } = useTranslation()
  const tx = getTransport()
  // Desktop can reveal real files in the OS file manager; HTTP/Web cannot.
  const isLocal = tx.supportsLocalFileOps()

  const [kbs, setKbs] = useState<KnowledgeBaseMeta[]>([])
  const [activeKbId, setActiveKbId] = useState<string | null>(null)
  const [notes, setNotes] = useState<Note[]>([])
  // Real directories under the KB root (incl. empty ones) — the index only tracks
  // .md, so empty folders come from a separate disk listing.
  const [dirs, setDirs] = useState<string[]>([])
  const [kbTags, setKbTags] = useState<string[]>([])
  const [openPath, setOpenPath] = useState<string | null>(null)
  // Which KB the open note / draft belongs to — guards against the active KB
  // being repicked (archive/delete/external) out from under the editor.
  const [openKbId, setOpenKbId] = useState<string | null>(null)
  const [noteData, setNoteData] = useState<NoteReadResult | null>(null)
  // Pending editor scroll target (1-based line / 0-based col) for backlink +
  // search precision navigation (G3). New object identity per request re-fires.
  const [revealTarget, setRevealTarget] = useState<{ line: number; col?: number } | null>(null)
  const [editorValue, setEditorValue] = useState("")
  // Bumped only on *user* edits (NoteEditor.onChange fires only for user doc
  // changes, not external loads) — drives the sprite edit-idle trigger.
  const [editorRevision, setEditorRevision] = useState(0)
  const handleEditorChange = useCallback((v: string) => {
    setEditorValue(v)
    setEditorRevision((r) => r + 1)
  }, [])
  const [baseHash, setBaseHash] = useState<string | null>(null)
  const [dirty, setDirty] = useState(false)
  // Fresh disk content captured when the open note changed externally *while*
  // the user has unsaved edits — drives the conflict banner (reload / keep mine)
  // instead of silently clobbering or only failing at save time.
  const [externalConflict, setExternalConflict] = useState<NoteReadResult | null>(null)
  const [mode, setMode] = useState<NoteEditorMode>("split")
  // Imperative handle into the saved-note editor (selection / range splice),
  // shared by the floating quick-rewrite bar and the "add selection to chat".
  const editorRef = useRef<NoteEditorHandle>(null)
  // Floating one-shot quick-rewrite bar: the captured range (selection or whole
  // doc). Replaces the old modal AI-rewrite dialog.
  const [quickRewrite, setQuickRewrite] = useState<{
    before: string
    from: number
    to: number
  } | null>(null)
  // Right panel: embedded AI chat ↔ backlinks view. The chat panel is anchored
  // to the open note + bound to the active KB, and is the default right-pane tab.
  const [rightMode, setRightMode] = useState<"links" | "chat">("chat")
  const chatPanelRef = useRef<KnowledgeChatPanelHandle>(null)
  // Live editor text for the chat panel's per-turn current-note context.
  const editorValueRef = useRef("")
  const getEditorValue = useCallback(() => editorValueRef.current, [])
  // Whole-KB graph view (WS1) — a per-KB toggle, orthogonal to the per-note
  // source/split/preview mode.
  const [graphMode, setGraphMode] = useState(false)
  // Bumped on knowledge:changed to bust the `![[ ]]` transclusion embed cache.
  const [embedCacheKey, setEmbedCacheKey] = useState(0)

  // ── Collapsible + resizable side panes (mirrors the chat view) ──────────
  const [leftCollapsed, setLeftCollapsed] = useState(() => readStoredBool(KB_LEFT_COLLAPSED_KEY))
  const [rightCollapsed, setRightCollapsed] = useState(() => readStoredBool(KB_RIGHT_COLLAPSED_KEY))
  const [leftWidth, setLeftWidth] = useState(() =>
    readStoredWidth(KB_LEFT_WIDTH_KEY, KB_LEFT_DEFAULT_WIDTH, KB_LEFT_MIN_WIDTH, KB_LEFT_MAX_WIDTH),
  )
  const [rightWidth, setRightWidth] = useState(() =>
    readStoredWidth(
      KB_RIGHT_WIDTH_KEY,
      KB_RIGHT_DEFAULT_WIDTH,
      KB_RIGHT_MIN_WIDTH,
      KB_RIGHT_MAX_WIDTH,
    ),
  )
  // Suppress the width CSS transition during a drag so the pane tracks the cursor.
  const [isResizingLeft, setIsResizingLeft] = useState(false)
  const [isResizingRight, setIsResizingRight] = useState(false)
  // Responsive-collapse intent tracking (per side): distinguishes a viewport-driven
  // collapse from a deliberate user one so auto-expand never fights the user.
  const autoCollapsedLeftRef = useRef(false)
  const manualLeftExpandedOverrideRef = useRef(false)
  const userLeftCollapsedPrefRef = useRef(leftCollapsed)
  const autoCollapsedRightRef = useRef(false)
  const manualRightExpandedOverrideRef = useRef(false)
  const userRightCollapsedPrefRef = useRef(rightCollapsed)
  // Responsive split→live auto-switch (analogous to right-panel auto-collapse).
  const autoSwitchedToLiveRef = useRef(false)
  const userModeOverrideRef = useRef(false)

  // Persist widths; persist collapse only when it's a deliberate (non-auto) change
  // so a transient responsive collapse isn't remembered as user intent.
  useEffect(() => {
    if (typeof window === "undefined") return
    try {
      window.localStorage.setItem(KB_LEFT_WIDTH_KEY, String(Math.round(leftWidth)))
    } catch {
      /* quota / private mode */
    }
  }, [leftWidth])
  useEffect(() => {
    if (typeof window === "undefined") return
    try {
      window.localStorage.setItem(KB_RIGHT_WIDTH_KEY, String(Math.round(rightWidth)))
    } catch {
      /* quota / private mode */
    }
  }, [rightWidth])
  useEffect(() => {
    if (typeof window === "undefined" || autoCollapsedLeftRef.current) return
    try {
      window.localStorage.setItem(KB_LEFT_COLLAPSED_KEY, String(leftCollapsed))
    } catch {
      /* quota / private mode */
    }
  }, [leftCollapsed])
  useEffect(() => {
    if (typeof window === "undefined" || autoCollapsedRightRef.current) return
    try {
      window.localStorage.setItem(KB_RIGHT_COLLAPSED_KEY, String(rightCollapsed))
    } catch {
      /* quota / private mode */
    }
  }, [rightCollapsed])

  const handleLeftCollapsedChange = useCallback((collapsed: boolean) => {
    autoCollapsedLeftRef.current = false
    manualLeftExpandedOverrideRef.current = !collapsed
    userLeftCollapsedPrefRef.current = collapsed
    setLeftCollapsed(collapsed)
  }, [])
  const handleRightCollapsedChange = useCallback((collapsed: boolean) => {
    autoCollapsedRightRef.current = false
    manualRightExpandedOverrideRef.current = !collapsed
    userRightCollapsedPrefRef.current = collapsed
    setRightCollapsed(collapsed)
  }, [])
  const handleModeChange = useCallback((m: NoteEditorMode) => {
    userModeOverrideRef.current = true
    autoSwitchedToLiveRef.current = false
    setMode(m)
  }, [])

  const onDragLeft = useDragWidth({
    width: leftWidth,
    min: KB_LEFT_MIN_WIDTH,
    max: KB_LEFT_MAX_WIDTH,
    onChange: setLeftWidth,
    direction: "ltr",
    onResizingChange: setIsResizingLeft,
  })
  const onDragRight = useDragWidth({
    width: rightWidth,
    min: KB_RIGHT_MIN_WIDTH,
    max: KB_RIGHT_MAX_WIDTH,
    onChange: setRightWidth,
    direction: "rtl",
    onResizingChange: setIsResizingRight,
  })

  // Derived side-pane breakpoints (recomputed per render from live widths; the
  // media-query strings are the effect deps so width changes re-bind listeners).
  // `preferredLeftWidth` keys off user intent (not the live collapse) so an
  // auto-collapse never feeds back into the breakpoint that triggered it.
  const preferredLeftWidth = userLeftCollapsedPrefRef.current ? 0 : leftWidth
  const rightCollapseAt = preferredLeftWidth + KB_CONTENT_MIN_WIDTH + rightWidth
  const rightExpandAt = rightCollapseAt + KB_RESPONSIVE_HYSTERESIS
  const leftCollapseAt = leftWidth + KB_CONTENT_MIN_WIDTH + KB_LEFT_AUTO_COLLAPSE_GUTTER
  const leftExpandAt = leftCollapseAt + KB_RESPONSIVE_HYSTERESIS

  const shouldAutoCollapseRight = useViewportMediaQuery(`(max-width: ${rightCollapseAt}px)`)
  const shouldAutoExpandRight = useViewportMediaQuery(`(min-width: ${rightExpandAt}px)`)
  const shouldAutoCollapseLeft = useViewportMediaQuery(`(max-width: ${leftCollapseAt}px)`)
  const shouldAutoExpandLeft = useViewportMediaQuery(`(min-width: ${leftExpandAt}px)`)

  useEffect(() => {
    // Room returned → drop manual-expand overrides so auto-collapse can resume.
    if (shouldAutoExpandRight) manualRightExpandedOverrideRef.current = false
    if (shouldAutoExpandLeft) manualLeftExpandedOverrideRef.current = false

    if (
      shouldAutoCollapseRight &&
      !rightCollapsed &&
      !userRightCollapsedPrefRef.current &&
      !manualRightExpandedOverrideRef.current
    ) {
      autoCollapsedRightRef.current = true
      setRightCollapsed(true)
    } else if (
      shouldAutoExpandRight &&
      rightCollapsed &&
      autoCollapsedRightRef.current &&
      !userRightCollapsedPrefRef.current
    ) {
      autoCollapsedRightRef.current = false
      setRightCollapsed(false)
    }

    if (
      shouldAutoCollapseLeft &&
      !leftCollapsed &&
      !userLeftCollapsedPrefRef.current &&
      !manualLeftExpandedOverrideRef.current
    ) {
      autoCollapsedLeftRef.current = true
      setLeftCollapsed(true)
    } else if (
      shouldAutoExpandLeft &&
      leftCollapsed &&
      autoCollapsedLeftRef.current &&
      !userLeftCollapsedPrefRef.current
    ) {
      autoCollapsedLeftRef.current = false
      setLeftCollapsed(false)
    }
  }, [
    shouldAutoCollapseRight,
    shouldAutoExpandRight,
    shouldAutoCollapseLeft,
    shouldAutoExpandLeft,
    rightCollapsed,
    leftCollapsed,
  ])

  // Split ⇄ live is driven by the editor pane's ACTUAL measured width (not derived
  // viewport math) so collapsing a side pane — which grows the editor — naturally
  // re-credits the freed space, with no breakpoint feedback loop. Not observed in
  // graph mode (the editor pane isn't mounted), so it can't mutate mode then.
  const centerRef = useRef<HTMLDivElement | null>(null)
  // Collapse the note toolbar's mode switch to a dropdown once the editor column
  // is too narrow for the 5-button segmented control. setState bails on an equal
  // value, so this only re-renders when crossing the threshold (no flap).
  const [compactToolbar, setCompactToolbar] = useState(false)
  useEffect(() => {
    const el = centerRef.current
    if (!el || typeof ResizeObserver === "undefined") return
    const evaluate = (width: number) => {
      if (width <= 0) return
      setCompactToolbar(width < KB_TOOLBAR_COMPACT_WIDTH)
      if (width < KB_SPLIT_MIN_WIDTH) {
        if (mode === "split" && !userModeOverrideRef.current) {
          autoSwitchedToLiveRef.current = true
          setMode("live")
        }
      } else if (width >= KB_SPLIT_MIN_WIDTH + KB_RESPONSIVE_HYSTERESIS) {
        // Room returned → allow auto-switch again; restore a prior auto-switch.
        userModeOverrideRef.current = false
        if (mode === "live" && autoSwitchedToLiveRef.current) {
          autoSwitchedToLiveRef.current = false
          setMode("split")
        }
      }
    }
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) evaluate(e.contentRect.width)
    })
    ro.observe(el)
    evaluate(el.getBoundingClientRect().width)
    return () => ro.disconnect()
  }, [mode, graphMode])

  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle")

  const [query, setQuery] = useState("")
  const [hits, setHits] = useState<NoteSearchHit[]>([])

  const [createOpen, setCreateOpen] = useState(false)
  const [newKbName, setNewKbName] = useState("")
  const [newKbRoot, setNewKbRoot] = useState("")
  // Draft note: a blank, in-memory note being composed. No file exists until
  // the first save, which derives the filename from the title.
  const [draftMode, setDraftMode] = useState(false)
  const [draftTitle, setDraftTitle] = useState("")
  // Fallback name prompt — only shown when a draft has neither a title nor a
  // leading H1 to derive the filename from.
  const [namePromptOpen, setNamePromptOpen] = useState(false)
  const [namePromptValue, setNamePromptValue] = useState("")

  // Inline rename: in the note list (`renamingPath`) and in the open-note header
  // (`titleEditing`). Both edit the file's rel path and commit a rename.
  const [renamingPath, setRenamingPath] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState("")
  const [titleEditing, setTitleEditing] = useState(false)
  const [titleValue, setTitleValue] = useState("")
  const [deleteConfirmPath, setDeleteConfirmPath] = useState<string | null>(null)

  // Folder tree state. Folders are implicit (derived from note paths); collapse
  // is tracked as an opt-in set so new folders start expanded.
  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(new Set())
  const [draftFolder, setDraftFolder] = useState("")
  const [newFolderOpen, setNewFolderOpen] = useState(false)
  const [newFolderParent, setNewFolderParent] = useState("")
  const [newFolderValue, setNewFolderValue] = useState("")
  const [renameFolderPath, setRenameFolderPath] = useState<string | null>(null)
  const [renameFolderValue, setRenameFolderValue] = useState("")
  const [deleteFolderPath, setDeleteFolderPath] = useState<string | null>(null)
  // "Move to…" picker: the note/folder being moved (null = closed).
  const [moveItem, setMoveItem] = useState<{ type: "note" | "folder"; path: string } | null>(null)

  // Space (KB) management.
  const [includeArchived, setIncludeArchived] = useState(false)
  const [kbBusy, setKbBusy] = useState(false) // in-flight guard for create/edit KB

  const [editKb, setEditKb] = useState<KnowledgeBaseMeta | null>(null)
  const [editKbName, setEditKbName] = useState("")
  const [editKbEmoji, setEditKbEmoji] = useState("")
  const [editKbAllowExternal, setEditKbAllowExternal] = useState(false)
  const [deleteKb, setDeleteKb] = useState<KnowledgeBaseMeta | null>(null)

  // Drag-to-move within the note tree.
  const [dragItem, setDragItem] = useState<{ type: "note" | "folder"; path: string } | null>(null)
  const [dragOver, setDragOver] = useState<string | null>(null)
  // Synchronous mirror of dragItem: drop targets read this in onDragOver/onDrop so
  // they don't depend on the async setDragItem state landing before the dragover
  // events fire (otherwise the row never becomes a valid drop target).
  const dragItemRef = useRef<{ type: "note" | "folder"; path: string } | null>(null)

  // Unsaved-changes guard: a navigation intent parked until the user resolves it.
  const [pendingNav, setPendingNav] = useState<(() => void) | null>(null)
  // A nav intent to resume after a headless draft gets named + saved (#7).
  const resumeNavRef = useRef<(() => void) | null>(null)

  const noteTree = useMemo(() => buildNoteTree(notes, dirs), [notes, dirs])

  // Full folder hierarchy for the "Move to…" picker (folders only — reuse the
  // note-tree builder with no notes).
  const moveTree = useMemo(() => buildNoteTree([], dirs), [dirs])

  const activeKb = useMemo(() => kbs.find((k) => k.id === activeKbId) ?? null, [kbs, activeKbId])
  // External vaults are read-only unless the KB opted into external writes (WS7).
  // Internal KBs (`external === false`) are always writable.
  const readOnly = (activeKb?.external ?? false) && !(activeKb?.allowExternalWrites ?? false)
  // Split the open note path into a folder prefix (shown muted, non-editable) and
  // the filename (the editable "title") so renaming never touches the folder.
  const openDir = openPath && openPath.includes("/") ? openPath.slice(0, openPath.lastIndexOf("/")) : ""
  const openBase = openPath ? openPath.slice(openDir ? openDir.length + 1 : 0) : ""

  // ── Loaders ──
  const loadKbs = useCallback(async () => {
    try {
      const list = await tx.call<KnowledgeBaseMeta[]>("list_kbs_cmd", { includeArchived })
      setKbs(list)
      // Keep the current selection only if it's still visible; otherwise repick a
      // non-archived space (covers delete / archive-active / hide-archived).
      setActiveKbId((cur) =>
        cur && list.some((k) => k.id === cur)
          ? cur
          : (list.find((k) => !k.archived)?.id ?? list[0]?.id ?? null),
      )
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::loadKbs", "list_kbs failed", e)
    }
  }, [tx, includeArchived])

  const loadNotes = useCallback(
    async (kbId: string) => {
      try {
        const list = await tx.call<Note[]>("list_kb_notes_cmd", { kbId })
        setNotes(list)
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::loadNotes", "list_kb_notes failed", e)
        setNotes([])
      }
    },
    [tx],
  )

  const loadDirs = useCallback(
    async (kbId: string) => {
      try {
        setDirs(await tx.call<string[]>("kb_list_dirs_cmd", { kbId }))
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::loadDirs", "kb_list_dirs failed", e)
        setDirs([])
      }
    },
    [tx],
  )

  // Tag vocabulary for the editor `#tag` autocomplete (design D13).
  const loadTags = useCallback(
    async (kbId: string) => {
      try {
        setKbTags(await tx.call<string[]>("kb_list_tags_cmd", { kbId }))
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::loadTags", "kb_list_tags failed", e)
        setKbTags([])
      }
    },
    [tx],
  )

  const openNote = useCallback(
    // `reveal` (optional) scrolls the editor to a 1-based line / 0-based col on
    // open — used by backlink + search clicks for precision navigation (G3).
    async (kbId: string, path: string, reveal?: { line: number; col?: number }) => {
      try {
        const data = await tx.call<NoteReadResult>("kb_note_read_cmd", { kbId, path })
        setGraphMode(false) // opening a note leaves graph view
        setDraftMode(false)
        setTitleEditing(false)
        setNoteData(data)
        setEditorValue(data.content)
        setBaseHash(data.contentHash)
        setOpenPath(path)
        setOpenKbId(kbId)
        setDirty(false)
        // Clear any conflict banner here too — re-opening the *same* path won't
        // change `openPath`, so the [openPath] clearing effect wouldn't fire.
        setExternalConflict(null)
        setSaveStatus("idle")
        setHits([]) // opening a note dismisses the search-results panel (#10)
        // A fresh object each call so re-clicking the same target re-triggers the
        // editor's reveal effect (identity-compared); null clears it.
        setRevealTarget(reveal ? { ...reveal } : null)
        resumeNavRef.current = null // drop any stale parked nav from a prior draft
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::openNote", "kb_note_read failed", e)
      }
    },
    [tx],
  )

  // "Add to AI chat": switch the right panel to the chat view and either stage
  // the current selection as a removable quote chip, or — with no selection —
  // insert a `[[note#heading]]` reference token into the composer. The chat
  // panel stays mounted (hidden in links mode) so its ref is always ready.
  const referenceCurrentSelectionInChat = useCallback(() => {
    if (!openPath) return
    setRightMode("chat")
    setRightCollapsed(false)
    const sel = editorRef.current?.getSelection()
    if (sel && sel.from !== sel.to && sel.text.trim()) {
      const startLine = (editorValue.slice(0, sel.from).match(/\n/g)?.length ?? 0) + 1
      const endLine = (editorValue.slice(0, sel.to).match(/\n/g)?.length ?? 0) + 1
      chatPanelRef.current?.addQuote({
        path: openPath,
        name: openPath.split("/").pop() ?? openPath,
        startLine,
        endLine,
        content: sel.text,
        kbId: openKbId ?? activeKbId ?? undefined,
      })
    } else {
      // Whole-note reference: `[[relPath]]`, with the nearest heading above the
      // cursor as a readable anchor when available.
      let inner = relPathToken(openPath)
      const sel2 = editorRef.current?.getSelection()
      if (sel2) {
        const selLine = (editorValue.slice(0, sel2.from).match(/\n/g)?.length ?? 0) + 1
        let nearest: string | undefined
        for (const h of parseHeadings(editorValue)) {
          if (h.line <= selLine) nearest = h.text
          else break
        }
        const anchor = nearest?.replace(/[[\]|#\n\r]/g, "").trim()
        if (anchor) inner = `${inner}#${anchor}`
      }
      chatPanelRef.current?.insertToken(formatNoteInsertion(inner))
    }
  }, [openPath, editorValue, openKbId, activeKbId])

  // Click a staged quote chip → scroll the editor to that selection's start
  // line. Same note: re-trigger the reveal effect with a fresh target (switching
  // to source first if we're in a CM6-less mode, else reveal is a no-op). Other
  // note: open it in the quote's own KB (it carries `kbId`), guarded so we don't
  // silently discard unsaved edits in the current note. Plain function (not
  // memoized) so it always closes over the latest `guardNavigation` / `hasUnsaved`
  // / `mode` rather than a stale snapshot.
  const jumpToQuoteInEditor = (q: PendingFileQuote) => {
    const kb = q.kbId ?? openKbId ?? activeKbId
    if (!kb) return
    if (kb === (openKbId ?? activeKbId) && q.path === openPath) {
      if (mode === "preview" || mode === "outline") handleModeChange("source")
      setRevealTarget({ line: q.startLine })
    } else {
      // Switch the active KB first when jumping into a different space, else the
      // orphan-guard effect (openKbId !== activeKbId) clears the note the instant
      // it opens. Mirrors the search-result cross-KB jump.
      guardNavigation(() => {
        setActiveKbId(kb)
        void openNote(kb, q.path, { line: q.startLine })
      })
    }
  }

  // Quick-rewrite the current selection (or the whole note when nothing is
  // selected). Shared by the editor toolbar button + the floating selection bar.
  const quickRewriteSelection = useCallback(() => {
    const sel = editorRef.current?.getSelection()
    if (sel && sel.from !== sel.to && sel.text.trim()) {
      setQuickRewrite({ before: sel.text, from: sel.from, to: sel.to })
    } else {
      // Whole-note rewrite: read `before` + `to` from the SAME live source
      // (getText), not the React-mirrored editorValue, so they can't be a
      // frame out of sync and leave the apply re-anchor replacing only a prefix.
      const text = editorRef.current?.getText() ?? editorValue
      setQuickRewrite({ before: text, from: 0, to: text.length })
    }
  }, [editorValue])

  // setState in these loaders is deferred behind an `await` (async fetch), so
  // there's no synchronous cascading render.
  useEffect(() => {
    void loadKbs()
  }, [loadKbs])

  useEffect(() => {
    // Clear the previous KB's tree immediately so we never render KB-A's notes/
    // folders under KB-B while the new loaders are in flight (#6).
    setNotes([])
    setDirs([])
    setKbTags([])
    if (activeKbId) {
      void loadNotes(activeKbId)
      void loadDirs(activeKbId)
      void loadTags(activeKbId)
    }
  }, [activeKbId, loadNotes, loadDirs, loadTags])

  // If the active KB was repicked out from under the editor (archive/delete the
  // active space, hide-archived, or an external change), drop the now-orphaned
  // open note/draft so we never desync or save into the wrong KB (#6/#8/#11/#12).
  useEffect(() => {
    if (openKbId != null && openKbId !== activeKbId) {
      setOpenPath(null)
      setOpenKbId(null)
      setNoteData(null)
      setEditorValue("")
      setBaseHash(null)
      setDirty(false)
      setDraftMode(false)
      setTitleEditing(false)
    }
  }, [activeKbId, openKbId])

  // Graph view is per-KB and transient: leave it whenever the active space
  // changes (sidebar switch, or a delete/archive that re-picks another space),
  // so a lingering toggle never drops the user into the next KB's graph. Resets
  // to the same value (false) bail out of re-render, so this is cheap.
  useEffect(() => setGraphMode(false), [activeKbId])

  // If the open note vanished from the active KB's list (deleted/moved/renamed
  // outside the app — agent tools, external vault watcher, another window), drop
  // the editor so a save can't resurrect it at the stale path (#2). Gated on the
  // notes list being confirmed for the active KB to avoid clearing mid-load.
  useEffect(() => {
    if (!openPath || draftMode || openKbId !== activeKbId) return
    if (notes.length === 0 || notes[0].kbId !== activeKbId) return
    if (notes.some((n) => n.relPath === openPath)) return
    setOpenPath(null)
    setOpenKbId(null)
    setNoteData(null)
    setEditorValue("")
    setBaseHash(null)
    setDirty(false)
    toast.error(t("knowledge.noteRemovedExternally", "This note was removed or moved outside the app."))
  }, [notes, openPath, draftMode, openKbId, activeKbId, t])

  // Mirror the live editor text into a ref for the chat panel's per-turn
  // current-note context (read lazily at send time, no re-render coupling).
  useEffect(() => {
    editorValueRef.current = editorValue
  }, [editorValue])

  // Refresh on backend knowledge changes (own writes, watcher, reindex). When
  // the AI chat / quick-rewrite writes the currently-open note, reload its
  // content so the editor reflects the change — unless the user has unsaved
  // edits, where clobbering would lose their work (the stale-write guard still
  // protects the file).
  useEffect(() => {
    return tx.listen("knowledge:changed", () => {
      void loadKbs()
      setEmbedCacheKey((n) => n + 1) // invalidate transclusion embed cache
      if (activeKbId) {
        void loadNotes(activeKbId)
        void loadDirs(activeKbId)
        void loadTags(activeKbId)
      }
      if (openKbId && openPath && !dirty && !draftMode) {
        void (async () => {
          try {
            const data = await tx.call<NoteReadResult>("kb_note_read_cmd", {
              kbId: openKbId,
              path: openPath,
            })
            // Only apply when the content actually changed on disk (avoid a
            // needless editor reset / cursor jump on unrelated KB events).
            if (data.contentHash !== baseHash) {
              setNoteData(data)
              setEditorValue(data.content)
              setBaseHash(data.contentHash)
              setDirty(false)
            }
          } catch {
            /* note may have been removed; the other listeners handle that */
          }
        })()
      } else if (openKbId && openPath && dirty && !draftMode) {
        // Unsaved local edits + the file changed underneath us: surface a
        // conflict banner rather than clobbering. The stale-write guard still
        // blocks a blind save; this just lets the user reload / keep-mine first.
        void (async () => {
          try {
            const data = await tx.call<NoteReadResult>("kb_note_read_cmd", {
              kbId: openKbId,
              path: openPath,
            })
            // Clear a stale banner if the disk reverted back to our base
            // (e.g. the external editor undid its change).
            setExternalConflict(data.contentHash !== baseHash ? data : null)
          } catch {
            /* note may have been removed; the other listeners handle that */
          }
        })()
      }
    })
  }, [tx, loadKbs, loadNotes, loadDirs, loadTags, activeKbId, openKbId, openPath, dirty, draftMode, baseHash])

  // Switching / closing the note invalidates any pending external-change
  // conflict (a fresh open re-establishes baseHash from disk).
  useEffect(() => {
    setExternalConflict(null)
  }, [openPath])

  // ── Wikilink editor data ──
  const wikilinkData: WikilinkData = useMemo(
    () => ({
      notes: notes.map((n) => ({
        label: n.relPath.replace(/\.(md|markdown)$/i, "").split("/").pop() ?? n.relPath,
        detail: n.relPath,
      })),
      tags: kbTags,
      knownTargets: buildKnownTargets(notes),
    }),
    [notes, kbTags],
  )

  // ── Actions ──
  const handleSave = useCallback(async (): Promise<boolean> => {
    // openKbId !== activeKbId guards against saving into the wrong KB after the
    // active space was repicked out from under the editor (#6/#8).
    if (!activeKbId || !openPath || readOnly || openKbId !== activeKbId) return false
    setSaving(true)
    try {
      const newHash = await tx.call<string>("kb_note_save_cmd", {
        kbId: activeKbId,
        path: openPath,
        content: editorValue,
        expectedFileHash: baseHash,
      })
      setBaseHash(newHash)
      setDirty(false)
      setExternalConflict(null)
      setSaving(false)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
      return true
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::handleSave", "kb_note_save failed", e)
      if (isRemoteWriteBlocked(e))
        toast.error(t("knowledge.remoteWritesDisabled", "Remote file writing is off."))
      setSaving(false)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      return false
    }
  }, [tx, activeKbId, openKbId, openPath, readOnly, editorValue, baseHash, t])

  const runSearch = useCallback(async () => {
    const q = query.trim()
    if (!q) {
      setHits([])
      return
    }
    try {
      const res = await tx.call<NoteSearchHit[]>("kb_search_cmd", {
        query: q,
        kbId: activeKbId ?? undefined,
        limit: 25,
      })
      setHits(res)
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::runSearch", "kb_search failed", e)
    }
  }, [tx, query, activeKbId])

  const createKb = useCallback(async () => {
    const name = newKbName.trim()
    if (!name || kbBusy) return // re-entrancy guard: no duplicate spaces (#7)
    setKbBusy(true)
    try {
      const kb = await tx.call<KnowledgeBaseMeta>("create_kb_cmd", {
        input: { name, rootDir: newKbRoot.trim() || null },
      })
      setCreateOpen(false)
      setNewKbName("")
      setNewKbRoot("")
      await loadKbs()
      setActiveKbId(kb.id)
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::createKb", "create_kb failed", e)
    } finally {
      setKbBusy(false)
    }
  }, [tx, newKbName, newKbRoot, kbBusy, loadKbs])

  // External-vault folder picker for the New Space dialog: native dialog on
  // desktop, server-side directory browser on web/HTTP (shared choreography).
  const {
    pick: pickKbRoot,
    browserOpen: rootBrowserOpen,
    setBrowserOpen: setRootBrowserOpen,
    handleBrowserSelect: handleKbRootSelect,
  } = useDirectoryPicker({
    onPicked: setNewKbRoot,
    errorTitle: t("knowledge.kbRootInvalid", "Couldn't select that folder"),
    loggerSource: "KnowledgeView::pickKbRoot",
  })

  // Open a blank draft straight away — no dialog. The title is composed in the
  // editor header; the filename is derived on first save. `folder` (optional)
  // scopes the new note under a subfolder.
  const startDraft = useCallback(
    (folder = "") => {
      if (!activeKbId || readOnly) return
      setDraftFolder(folder.replace(/\/+$/, ""))
      setDraftMode(true)
      setDraftTitle("")
      setEditorValue("")
      setOpenPath(null)
      setOpenKbId(activeKbId) // the draft belongs to the active KB
      setNoteData(null)
      setBaseHash(null)
      setDirty(false)
      setSaveStatus("idle")
      setHits([])
      resumeNavRef.current = null // a fresh draft drops any stale parked nav
    },
    [activeKbId, readOnly],
  )

  // Persist a draft under `title`. `prependHeading` controls whether the title
  // is written as a leading H1 — skipped when the body already starts with one.
  const commitDraft = useCallback(
    async (title: string, prependHeading: boolean): Promise<boolean> => {
      if (!activeKbId || readOnly) return false
      // Derive a flat, traversal-safe filename from the title (backend re-checks),
      // then scope it under the draft's folder.
      const base = title.replace(/[/\\]+/g, "-").replace(/^\.+/, "").trim() || "untitled"
      const dir = draftFolder ? `${draftFolder}/` : ""
      const existing = new Set(notes.map((n) => n.relPath.toLowerCase()))
      let rel = `${dir}${base}.md`
      for (let i = 2; existing.has(rel.toLowerCase()); i++) rel = `${dir}${base}-${i}.md`
      const content = prependHeading ? `# ${title}\n\n${editorValue}` : editorValue
      setSaving(true)
      try {
        await tx.call("kb_note_save_cmd", {
          kbId: activeKbId,
          path: rel,
          content,
          createOnly: true,
        })
        setSaving(false)
        setDraftMode(false)
        setNamePromptOpen(false)
        setNamePromptValue("")
        // Capture the parked nav BEFORE openNote (which also clears the ref) so a
        // navigation parked while the draft awaited a name still resumes (#7).
        const resume = resumeNavRef.current
        resumeNavRef.current = null
        await loadNotes(activeKbId)
        await openNote(activeKbId, rel)
        resume?.()
        return true
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::commitDraft", "create note failed", e)
        if (isRemoteWriteBlocked(e))
          toast.error(t("knowledge.remoteWritesDisabled", "Remote file writing is off."))
        setSaving(false)
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 2000)
        return false
      }
    },
    [tx, activeKbId, readOnly, draftFolder, editorValue, notes, loadNotes, openNote, t],
  )

  // Resolve a broken outgoing `[[ref]]` in one click: create the missing note at
  // the link's target path (alias/anchor stripped) and open it. Design D13's
  // "create this note" affordance for dangling links.
  const createNoteFromRef = useCallback(
    async (ref: string) => {
      if (!activeKbId || readOnly) return
      let p = ref.trim()
      const pipe = p.indexOf("|")
      if (pipe >= 0) p = p.slice(0, pipe)
      const hash = p.indexOf("#")
      if (hash >= 0) p = p.slice(0, hash)
      p = p.trim().replace(/\\/g, "/").replace(/^\/+/, "")
      if (!p) return
      const title = p.split("/").pop() ?? p
      if (!/\.(md|markdown)$/i.test(p)) p = `${p}.md`
      // Already exists (e.g. a basename-only link to an existing path) — just open.
      const existing = notes.find((n) => n.relPath.toLowerCase() === p.toLowerCase())
      if (existing) {
        await openNote(activeKbId, existing.relPath)
        return
      }
      try {
        await tx.call("kb_note_save_cmd", {
          kbId: activeKbId,
          path: p,
          content: `# ${title}\n\n`,
          createOnly: true,
        })
        await loadNotes(activeKbId)
        await openNote(activeKbId, p)
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::createNoteFromRef", "create note from link failed", e)
        if (isRemoteWriteBlocked(e))
          toast.error(t("knowledge.remoteWritesDisabled", "Remote file writing is off."))
      }
    },
    [activeKbId, readOnly, notes, tx, loadNotes, openNote, t],
  )

  const saveDraft = useCallback(async () => {
    if (!activeKbId || readOnly) return
    const title = draftTitle.trim()
    if (title) {
      await commitDraft(title, true)
      return
    }
    // No explicit title — fall back to the body's leading H1, else ask.
    const h1 = firstHeading(editorValue)
    if (h1) {
      await commitDraft(h1, false)
      return
    }
    setNamePromptValue("")
    setNamePromptOpen(true)
  }, [activeKbId, readOnly, draftTitle, editorValue, commitDraft])

  const reindex = useCallback(async () => {
    if (!activeKbId) return
    try {
      // Runs through the KnowledgeReembed job (progress-tracked below). The await
      // resolves once the job is *spawned*, not when it finishes — completion
      // feedback comes from `onReindexDone` (toast) + the spinning 🔄.
      await tx.call("reindex_kb_cmd", { id: activeKbId })
    } catch (e) {
      // Surface spawn failures (e.g. vector search on but no embedding model
      // configured) instead of silently swallowing them — otherwise the click
      // looks dead.
      logger.error("knowledge", "KnowledgeView::reindex", "reindex failed", e)
      toast.error(String(e))
    }
  }, [tx, activeKbId])

  // Toast on reindex/reembed completion so a fast single-KB rebuild (where the
  // 🔄 spin is too brief to notice) still gives visible feedback. Stable identity
  // so useReembedJob doesn't re-subscribe every render.
  const onReindexDone = useCallback(() => {
    toast.success(t("knowledge.reindexDone", "Index rebuilt"))
  }, [t])

  // Track the global knowledge reindex/reembed job so the toolbar 🔄 shows
  // progress (spins + shows N/M while a rebuild — single-KB or full — runs).
  const { job: reindexJob } = useReembedJob({
    kind: "knowledge_reembed",
    onCompleted: onReindexDone,
  })
  const reindexActive = !!reindexJob && isLocalModelJobActive(reindexJob)
  const reindexProgress =
    reindexActive && reindexJob
      ? ` (${Number(reindexJob.bytesCompleted ?? 0)}/${Number(reindexJob.bytesTotal ?? 0)})`
      : ""

  // Per-note / per-folder / per-space "rebuild index" (context menus). Index is
  // an app-side cache (not the vault files), so these work even on read-only
  // external vaults. Note/folder run synchronously; space runs via the
  // progress-tracked job (surfaces on the toolbar 🔄).
  const reindexNote = useCallback(
    async (relPath: string) => {
      if (!activeKbId) return
      try {
        await tx.call("reindex_note_cmd", { kbId: activeKbId, path: relPath })
        toast.success(t("knowledge.reindexDone", "Index rebuilt"))
      } catch (e) {
        toast.error(String(e))
      }
    },
    [tx, activeKbId, t],
  )

  const reindexDir = useCallback(
    async (dirPath: string) => {
      if (!activeKbId) return
      try {
        await tx.call("reindex_dir_cmd", { kbId: activeKbId, path: dirPath })
        toast.success(t("knowledge.reindexDone", "Index rebuilt"))
      } catch (e) {
        toast.error(String(e))
      }
    },
    [tx, activeKbId, t],
  )

  const reindexSpace = useCallback(
    async (kbId: string) => {
      try {
        await tx.call("reindex_kb_cmd", { id: kbId })
      } catch (e) {
        toast.error(String(e))
      }
    },
    [tx],
  )

  // Rename/move a note's file. Backend guards traversal and re-resolves links.
  const renameNote = useCallback(
    async (fromRel: string, toRaw: string) => {
      if (!activeKbId || readOnly) return
      let to = toRaw.trim()
      if (!to || to === fromRel) return
      if (!/\.(md|markdown)$/i.test(to)) to = `${to}.md`
      if (to === fromRel) return
      try {
        const outcome = await tx.call<RenameOutcome>("kb_note_rename_cmd", {
          kbId: activeKbId,
          path: fromRel,
          newPath: to,
        })
        await loadNotes(activeKbId)
        if (openPath === fromRel) await openNote(activeKbId, outcome.newRel)
        if (outcome.linksRewritten > 0)
          toast.success(t("knowledge.linksRewritten", { count: outcome.linksRewritten }))
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::renameNote", "rename note failed", e)
        toast.error(
          isRemoteWriteBlocked(e)
            ? t("knowledge.remoteWritesDisabled", "Remote file writing is off.")
            : t("knowledge.renameMoveFailed", { name: to }),
        )
      }
    },
    [tx, activeKbId, readOnly, openPath, loadNotes, openNote, t],
  )

  const deleteNote = useCallback(
    async (rel: string) => {
      if (!activeKbId || readOnly) return
      try {
        await tx.call("kb_note_delete_cmd", { kbId: activeKbId, path: rel })
        if (openPath === rel) {
          setOpenPath(null)
          setOpenKbId(null)
          setNoteData(null)
        }
        await loadNotes(activeKbId)
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::deleteNote", "delete note failed", e)
        if (isRemoteWriteBlocked(e))
          toast.error(t("knowledge.remoteWritesDisabled", "Remote file writing is off."))
      } finally {
        setDeleteConfirmPath(null)
      }
    },
    [tx, activeKbId, readOnly, openPath, loadNotes, t],
  )

  // Desktop-only: resolve the note to an absolute path and reveal it in the OS
  // file manager (same dispatch as the chat workspace's "Reveal in folder").
  const revealNote = useCallback(
    async (rel: string) => {
      if (!activeKbId || !isLocal) return
      try {
        const abs = await tx.call<string>("kb_file_resolve_cmd", { kbId: activeKbId, path: rel })
        await tx.call("reveal_in_folder", { path: abs })
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::revealNote", "reveal note failed", e)
      }
    },
    [tx, activeKbId, isLocal],
  )

  const toggleFolder = useCallback((path: string) => {
    setCollapsedFolders((prev) => {
      const next = new Set(prev)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  }, [])

  // Rename/move a real folder (one fs rename of the directory + its contents);
  // the backend reconciles the index + re-resolves links. Refuses self/descendant.
  const applyFolderMove = useCallback(
    async (oldPath: string, newPath: string) => {
      if (!activeKbId || readOnly) return
      if (newPath === oldPath || newPath.startsWith(`${oldPath}/`)) return
      try {
        const outcome = await tx.call<RenameOutcome>("kb_rename_dir_cmd", {
          kbId: activeKbId,
          path: oldPath,
          newPath,
        })
        await Promise.all([loadNotes(activeKbId), loadDirs(activeKbId)])
        if (openPath && openPath.startsWith(`${oldPath}/`)) {
          await openNote(activeKbId, `${newPath}${openPath.slice(oldPath.length)}`)
        }
        if (outcome.linksRewritten > 0)
          toast.success(t("knowledge.linksRewritten", { count: outcome.linksRewritten }))
        // Keep a draft scoped to the moved folder pointing at the new path so it
        // doesn't resurrect the old one on save (#5).
        setDraftFolder((cur) =>
          cur === oldPath || cur.startsWith(`${oldPath}/`)
            ? `${newPath}${cur.slice(oldPath.length)}`
            : cur,
        )
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::applyFolderMove", "rename/move folder failed", e)
        toast.error(
          isRemoteWriteBlocked(e)
            ? t("knowledge.remoteWritesDisabled", "Remote file writing is off.")
            : t("knowledge.renameMoveFailed", { name: oldPath }),
        )
      }
    },
    [tx, activeKbId, readOnly, openPath, loadNotes, loadDirs, openNote, t],
  )

  // Rename a folder in place (same parent, new last segment).
  const renameFolder = useCallback(
    async (oldPath: string, rawName: string) => {
      const name = rawName.trim().replace(/[/\\]+/g, "-").replace(/^\.+/, "")
      if (!name) return
      const parent = oldPath.includes("/") ? oldPath.slice(0, oldPath.lastIndexOf("/")) : ""
      try {
        await applyFolderMove(oldPath, parent ? `${parent}/${name}` : name)
      } finally {
        setRenameFolderPath(null)
      }
    },
    [applyFolderMove],
  )

  // Move a folder under a new parent ("" = root), keeping its own name.
  const moveFolder = useCallback(
    async (oldPath: string, newParent: string) => {
      const base = oldPath.split("/").pop() ?? oldPath
      await applyFolderMove(oldPath, newParent ? `${newParent}/${base}` : base)
    },
    [applyFolderMove],
  )

  // Delete a folder = remove the directory and everything under it.
  const deleteFolder = useCallback(
    async (path: string) => {
      if (!activeKbId || readOnly) return
      try {
        await tx.call("kb_delete_dir_cmd", { kbId: activeKbId, path })
        if (openPath && openPath.startsWith(`${path}/`)) {
          setOpenPath(null)
          setOpenKbId(null)
          setNoteData(null)
        }
        // A draft scoped to the deleted folder falls back to root (#5).
        setDraftFolder((cur) => (cur === path || cur.startsWith(`${path}/`) ? "" : cur))
        await Promise.all([loadNotes(activeKbId), loadDirs(activeKbId)])
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::deleteFolder", "delete folder failed", e)
        toast.error(
          isRemoteWriteBlocked(e)
            ? t("knowledge.remoteWritesDisabled", "Remote file writing is off.")
            : t("knowledge.renameMoveFailed", { name: path }),
        )
      } finally {
        setDeleteFolderPath(null)
      }
    },
    [tx, activeKbId, readOnly, openPath, loadNotes, loadDirs, t],
  )

  // ── Space (KB) management ──
  const openEditKb = useCallback((kb: KnowledgeBaseMeta) => {
    setEditKb(kb)
    setEditKbName(kb.name)
    setEditKbEmoji(kb.emoji ?? "")
    setEditKbAllowExternal(kb.allowExternalWrites)
  }, [])

  const saveEditKb = useCallback(async () => {
    if (!editKb) return
    const name = editKbName.trim()
    if (!name || kbBusy) return // re-entrancy guard (#9)
    setKbBusy(true)
    try {
      await tx.call("update_kb_cmd", {
        id: editKb.id,
        // Send the trimmed string (possibly "") — the backend clears emoji to
        // NULL on empty; sending null would be treated as "no change" (#9).
        // `allowExternalWrites` only matters for external roots (WS7); the
        // backend ignores it for internal KBs, but only send it for external
        // ones to keep the patch minimal.
        patch: {
          name,
          emoji: editKbEmoji.trim(),
          ...(editKb.external ? { allowExternalWrites: editKbAllowExternal } : {}),
        },
      })
      setEditKb(null)
      await loadKbs()
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::saveEditKb", "update kb failed", e)
    } finally {
      setKbBusy(false)
    }
  }, [tx, editKb, editKbName, editKbEmoji, editKbAllowExternal, kbBusy, loadKbs])

  const toggleArchiveKb = useCallback(
    async (kb: KnowledgeBaseMeta) => {
      try {
        await tx.call("update_kb_cmd", { id: kb.id, patch: { archived: !kb.archived } })
        await loadKbs()
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::toggleArchiveKb", "archive kb failed", e)
      }
    },
    [tx, loadKbs],
  )

  const deleteKbConfirm = useCallback(async () => {
    if (!deleteKb) return
    const id = deleteKb.id
    try {
      await tx.call("delete_kb_cmd", { id })
      if (activeKbId === id) {
        setActiveKbId(null)
        setOpenPath(null)
        setOpenKbId(null)
        setNoteData(null)
        setDraftMode(false)
        setHits([]) // drop stale search hits that point at the deleted KB (#11)
        setQuery("")
      }
      setDeleteKb(null)
      await loadKbs()
    } catch (e) {
      logger.error("knowledge", "KnowledgeView::deleteKbConfirm", "delete kb failed", e)
    }
  }, [tx, deleteKb, activeKbId, loadKbs])

  // ⌘S / Ctrl+S saves the draft or the open note (intercepts the webview's
  // default "save page" so it never bubbles out of the Knowledge view).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "s") return
      e.preventDefault()
      if (namePromptOpen) return // let the prompt dialog own the keystroke
      if (draftMode) void saveDraft()
      else if (openPath && noteData && dirty && !readOnly) void handleSave()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [namePromptOpen, draftMode, saveDraft, openPath, noteData, dirty, readOnly, handleSave])

  // ── Unsaved-changes guard (plain closures over current state) ──
  const hasUnsaved = draftMode
    ? draftTitle.trim().length > 0 || editorValue.trim().length > 0
    : !!openPath && dirty && !readOnly

  // Persist whatever is open. Returns true if it actually saved (false if a draft
  // still needs a name — the name prompt is opened and the navigation abandoned).
  const persistCurrent = async (): Promise<boolean> => {
    if (draftMode) {
      const title = draftTitle.trim()
      if (title) return commitDraft(title, true)
      const h1 = firstHeading(editorValue)
      if (h1) return commitDraft(h1, false)
      setNamePromptValue("")
      setNamePromptOpen(true)
      return false
    }
    if (openPath && noteData && dirty && !readOnly) return handleSave()
    return true
  }

  // Run `action` now if nothing is unsaved, otherwise park it behind the guard.
  const guardNavigation = (action: () => void) => {
    if (!hasUnsaved) {
      action()
      return
    }
    setPendingNav(() => action)
  }

  // Whether `path` is (or contains) the currently open note — used to decide if a
  // rename/move would clobber unsaved edits on the open note.
  const affectsOpenNote = (path: string) =>
    !draftMode && !!openPath && (openPath === path || openPath.startsWith(`${path}/`))

  // Guard a rename/move: only prompt when it would discard unsaved edits to the
  // OPEN note (renaming/moving other notes mustn't false-prompt) (#1).
  const guardEdit = (path: string, action: () => void) => {
    if (dirty && !readOnly && affectsOpenNote(path)) setPendingNav(() => action)
    else action()
  }

  // ── Drag-to-move within the note tree ("" target = root) ──
  const handleDropOn = (target: string) => {
    const d = dragItemRef.current
    dragItemRef.current = null
    setDragItem(null)
    setDragOver(null)
    if (!d || readOnly) return
    guardEdit(d.path, () => {
      if (d.type === "note") {
        const filename = d.path.split("/").pop() ?? d.path
        const dest = target ? `${target}/${filename}` : filename
        if (dest !== d.path) void renameNote(d.path, dest)
      } else {
        void moveFolder(d.path, target)
      }
    })
  }

  // "Move to…" picker commit ("" target = root). Mirrors drag-move.
  const performMove = (target: string) => {
    const it = moveItem
    setMoveItem(null)
    if (!it || readOnly) return
    if (it.type === "note") {
      const filename = it.path.split("/").pop() ?? it.path
      const dest = target ? `${target}/${filename}` : filename
      if (dest !== it.path) guardEdit(it.path, () => void renameNote(it.path, dest))
    } else {
      guardEdit(it.path, () => void moveFolder(it.path, target))
    }
  }

  // Create a real (empty) directory and refresh — the folder just appears, no
  // draft document is opened.
  const confirmNewFolder = () => {
    const name = newFolderValue.trim().replace(/[/\\]+/g, "-").replace(/^\.+/, "")
    if (!name || !activeKbId) return
    const parent = newFolderParent
    const folder = parent ? `${parent}/${name}` : name
    setNewFolderOpen(false)
    setNewFolderValue("")
    setNewFolderParent("")
    if (parent) {
      setCollapsedFolders((prev) => {
        const next = new Set(prev)
        next.delete(parent)
        return next
      })
    }
    void (async () => {
      try {
        await tx.call("kb_mkdir_cmd", { kbId: activeKbId, path: folder })
        await loadDirs(activeKbId)
      } catch (e) {
        logger.error("knowledge", "KnowledgeView::confirmNewFolder", "mkdir failed", e)
        toast.error(
          isRemoteWriteBlocked(e)
            ? t("knowledge.remoteWritesDisabled", "Remote file writing is off.")
            : t("knowledge.renameMoveFailed", { name: folder }),
        )
      }
    })()
  }

  // ── Tree renderers (closures over the state/handlers above) ──
  const renderNote = (node: Extract<TreeNode, { type: "note" }>, depth: number) => {
    const n = node.note
    const pad = { paddingLeft: depth * 14 + 8 }
    const noteParent = n.relPath.includes("/")
      ? n.relPath.slice(0, n.relPath.lastIndexOf("/"))
      : ""
    if (renamingPath === n.relPath) {
      return (
        <div key={n.id} className="py-0.5 pr-2" style={pad}>
          <Input
            value={renameValue}
            autoFocus
            onChange={(e) => setRenameValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                const dir = n.relPath.includes("/")
                  ? n.relPath.slice(0, n.relPath.lastIndexOf("/") + 1)
                  : ""
                const target = dir + renameValue
                setRenamingPath(null)
                guardEdit(n.relPath, () => void renameNote(n.relPath, target))
              } else if (e.key === "Escape") {
                setRenamingPath(null)
              }
            }}
            onBlur={() => setRenamingPath(null)}
            className="h-6 text-xs"
          />
        </div>
      )
    }
    return (
      <ContextMenu key={n.id}>
        <ContextMenuTrigger asChild>
          <button
            onClick={() =>
              guardNavigation(() => {
                if (activeKbId) void openNote(activeKbId, n.relPath)
              })
            }
            draggable={!readOnly}
            onDragStart={(e) => {
              e.dataTransfer.setData("text/plain", n.relPath)
              e.dataTransfer.effectAllowed = "move"
              dragItemRef.current = { type: "note", path: n.relPath }
              setDragItem({ type: "note", path: n.relPath })
            }}
            onDragEnd={() => {
              dragItemRef.current = null
              setDragItem(null)
              setDragOver(null)
            }}
            onDragOver={(e) => {
              // A drop on a note targets its parent folder (never falls through to
              // root). Without this, the drop bubbles to the root container (#3).
              if (!dragItemRef.current || readOnly) return
              e.preventDefault()
              e.stopPropagation()
              setDragOver(noteParent)
            }}
            onDrop={(e) => {
              e.preventDefault()
              e.stopPropagation()
              handleDropOn(noteParent)
            }}
            style={pad}
            className={cn(
              "flex w-full items-center gap-2 py-1 pr-2 text-left text-xs hover:bg-muted/50",
              openPath === n.relPath && "bg-muted",
              dragItem?.path === n.relPath && "opacity-40",
            )}
          >
            <FileText className="h-3 w-3 shrink-0 text-muted-foreground" />
            <span className="flex-1 truncate" title={n.relPath}>
              {node.name}
            </span>
            {/* Unsaved marker — only the currently-open note can be dirty
                (single editor / single `dirty` flag), so a per-row check is
                exact without a per-path dirty map. */}
            {openPath === n.relPath && dirty && (
              <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />
            )}
          </button>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem
            disabled={readOnly}
            onClick={() => {
              setRenameValue(node.name)
              setRenamingPath(n.relPath)
            }}
          >
            <Pencil className="mr-2 h-3.5 w-3.5" />
            {t("common.rename", "Rename")}
          </ContextMenuItem>
          <ContextMenuItem
            disabled={readOnly}
            onClick={() => setMoveItem({ type: "note", path: n.relPath })}
          >
            <FolderInput className="mr-2 h-3.5 w-3.5" />
            {t("knowledge.moveTo", "Move to…")}
          </ContextMenuItem>
          {isLocal && (
            <ContextMenuItem onClick={() => void revealNote(n.relPath)}>
              <FolderOpen className="mr-2 h-3.5 w-3.5" />
              {t("fileActions.revealInFolder", "Reveal in folder")}
            </ContextMenuItem>
          )}
          <ContextMenuItem onClick={() => void reindexNote(n.relPath)}>
            <RefreshCw className="mr-2 h-3.5 w-3.5" />
            {t("knowledge.reindex", "Reindex")}
          </ContextMenuItem>
          <ContextMenuItem
            disabled={readOnly}
            className="text-destructive focus:text-destructive"
            onClick={() => setDeleteConfirmPath(n.relPath)}
          >
            <Trash2 className="mr-2 h-3.5 w-3.5" />
            {t("common.delete", "Delete")}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
    )
  }

  const renderFolder = (node: Extract<TreeNode, { type: "folder" }>, depth: number) => {
    const collapsed = collapsedFolders.has(node.path)
    return (
      <div key={`f:${node.path}`}>
        <ContextMenu>
          <ContextMenuTrigger asChild>
            <button
              onClick={() => toggleFolder(node.path)}
              draggable={!readOnly}
              onDragStart={(e) => {
                e.dataTransfer.setData("text/plain", node.path)
                e.dataTransfer.effectAllowed = "move"
                dragItemRef.current = { type: "folder", path: node.path }
                setDragItem({ type: "folder", path: node.path })
              }}
              onDragEnd={() => {
                dragItemRef.current = null
                setDragItem(null)
                setDragOver(null)
              }}
              onDragOver={(e) => {
                if (!dragItemRef.current || readOnly) return
                e.preventDefault()
                e.stopPropagation()
                setDragOver(node.path)
              }}
              onDrop={(e) => {
                e.preventDefault()
                e.stopPropagation()
                handleDropOn(node.path)
              }}
              style={{ paddingLeft: depth * 14 + 8 }}
              className={cn(
                "flex w-full items-center gap-1 py-1 pr-2 text-left text-xs hover:bg-muted/50",
                dragOver === node.path && "bg-primary/10 ring-1 ring-inset ring-primary/40",
                dragItem?.type === "folder" && dragItem.path === node.path && "opacity-40",
              )}
            >
              {collapsed ? (
                <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
              ) : (
                <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
              )}
              <Folder className="h-3 w-3 shrink-0 text-muted-foreground" />
              <span className="flex-1 truncate font-medium" title={node.path}>
                {node.name}
              </span>
            </button>
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem
              disabled={readOnly}
              onClick={() => guardNavigation(() => startDraft(node.path))}
            >
              <Plus className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.newNote", "New note")}
            </ContextMenuItem>
            <ContextMenuItem
              disabled={readOnly}
              onClick={() => {
                setNewFolderParent(node.path)
                setNewFolderValue("")
                setNewFolderOpen(true)
              }}
            >
              <FolderPlus className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.newSubfolder", "New subfolder")}
            </ContextMenuItem>
            <ContextMenuItem
              disabled={readOnly}
              onClick={() => {
                setRenameFolderValue(node.name)
                setRenameFolderPath(node.path)
              }}
            >
              <Pencil className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.renameFolder", "Rename folder")}
            </ContextMenuItem>
            <ContextMenuItem
              disabled={readOnly}
              onClick={() => setMoveItem({ type: "folder", path: node.path })}
            >
              <FolderInput className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.moveTo", "Move to…")}
            </ContextMenuItem>
            <ContextMenuItem onClick={() => void reindexDir(node.path)}>
              <RefreshCw className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.reindex", "Reindex")}
            </ContextMenuItem>
            <ContextMenuItem
              disabled={readOnly}
              className="text-destructive focus:text-destructive"
              onClick={() => setDeleteFolderPath(node.path)}
            >
              <Trash2 className="mr-2 h-3.5 w-3.5" />
              {t("knowledge.deleteFolder", "Delete folder")}
            </ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
        {!collapsed &&
          node.children.map((c) =>
            c.type === "folder" ? renderFolder(c, depth + 1) : renderNote(c, depth + 1),
          )}
      </div>
    )
  }

  const renderNodes = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((node) =>
      node.type === "folder" ? renderFolder(node, depth) : renderNote(node, depth),
    )

  // ── "Move to…" folder-tree picker ──
  // A destination is invalid when it's the item's current parent (no-op) or, for
  // a folder, the folder itself or one of its descendants.
  const isInvalidMoveTarget = (dest: string): boolean => {
    if (!moveItem) return true
    const curParent = moveItem.path.includes("/")
      ? moveItem.path.slice(0, moveItem.path.lastIndexOf("/"))
      : ""
    if (dest === curParent) return true
    return moveItem.type === "folder" && (dest === moveItem.path || dest.startsWith(`${moveItem.path}/`))
  }

  const renderMoveRow = (label: string, path: string, depth: number) => {
    const disabled = isInvalidMoveTarget(path)
    return (
      <button
        type="button"
        disabled={disabled}
        onClick={() => performMove(path)}
        style={{ paddingLeft: depth * 14 + 8 }}
        className={cn(
          "flex w-full items-center gap-2 py-1.5 pr-2 text-left text-xs",
          disabled ? "opacity-40" : "hover:bg-muted/50",
        )}
      >
        <Folder className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="truncate">{label}</span>
      </button>
    )
  }

  const renderMoveTree = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((node) =>
      node.type === "folder" ? (
        <div key={node.path}>
          {renderMoveRow(node.name, node.path, depth)}
          {renderMoveTree(node.children, depth + 1)}
        </div>
      ) : null,
    )

  // ── Render ──
  return (
    // `min-w-0`: this is a flex item in the app body's flex row (which is
    // `overflow-hidden`). Without it the root won't shrink below its content's
    // min-content (the editor's 360px floor + panes), so it overflows and the
    // body hard-clips the right pane off-screen instead of the panes compressing.
    <div className="flex flex-1 min-h-0 min-w-0 flex-col bg-background">
      {/* Header */}
      <div
        className="flex items-center gap-2 border-b border-border-soft/60 px-3 py-2"
        data-tauri-drag-region
      >
        <IconTip
          label={
            leftCollapsed
              ? t("knowledge.expandLeft", "Expand sidebar")
              : t("knowledge.collapseLeft", "Collapse sidebar")
          }
          side="bottom"
        >
          <Button
            variant="ghost"
            size="icon"
            className={cn("h-8 w-8", leftCollapsed ? "text-muted-foreground" : "text-foreground")}
            aria-label={
              leftCollapsed
                ? t("knowledge.expandLeft", "Expand sidebar")
                : t("knowledge.collapseLeft", "Collapse sidebar")
            }
            aria-expanded={!leftCollapsed}
            onClick={() => handleLeftCollapsedChange(!leftCollapsed)}
          >
            {leftCollapsed ? (
              <PanelLeftDashed className="h-4 w-4" />
            ) : (
              <PanelLeft className="h-4 w-4" />
            )}
          </Button>
        </IconTip>
        <IconTip label={t("common.back", "Back")} side="bottom">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={() => guardNavigation(onBack)}
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
        </IconTip>
        <Library className="h-4 w-4 text-primary" />
        <span className="text-sm font-medium">{t("knowledge.title", "Knowledge Space")}</span>
        <Button
          variant="outline"
          size="sm"
          className="ml-2 h-8"
          onClick={() => setCreateOpen(true)}
        >
          <Plus className="mr-1 h-3.5 w-3.5" />
          {t("knowledge.newKb", "New space")}
        </Button>
        {/* Container fills the middle (pinning Settings + tasks flush right); the
            input stays capped + left-anchored via max-w-md so it isn't too wide.
            `min-w-0` lets it absorb ALL shrinkage on a narrow window — the input
            compresses first instead of the right-side controls being clipped.
            data-tauri-drag-region keeps the empty area right of the input draggable
            (the input is a child, so clicking it still types). */}
        <div className="relative flex-1 min-w-0" data-tauri-drag-region>
          <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void runSearch()
            }}
            placeholder={t("knowledge.searchPlaceholder", "Search notes…")}
            className="h-8 max-w-md pl-7 text-xs"
          />
        </div>
        {/* Right cluster — shrink-0 so it's never clipped; the search input above
            absorbs window shrinkage instead. */}
        <div className="flex shrink-0 items-center gap-2">
        {onOpenSettings && <KnowledgeEmbeddingBadge onOpenSettings={onOpenSettings} />}
        {onOpenSettings && (
          <IconTip label={t("knowledge.openSettings", "Knowledge space settings")} side="bottom">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={onOpenSettings}
            >
              <Settings className="h-4 w-4" />
            </Button>
          </IconTip>
        )}
        <IconTip label={t("knowledge.graph.toggle", "Graph view")} side="bottom">
          <Button
            variant="ghost"
            size="icon"
            className={cn("h-7 w-7", graphMode && "bg-primary/15 text-primary")}
            disabled={!activeKbId}
            onClick={() => setGraphMode((g) => !g)}
          >
            <Waypoints className="h-4 w-4" />
          </Button>
        </IconTip>
        <KnowledgeMaintenanceButton
          // Remount per space so a previous KB's broken/orphan lists never render
          // under the new one, and an in-flight refresh for the old KB can't
          // overwrite the new KB's state (it resolves on the unmounted instance).
          key={activeKbId ?? "none"}
          kbId={activeKbId}
          onOpenNote={(path, line) => {
            if (activeKbId)
              guardNavigation(() => void openNote(activeKbId, path, line ? { line } : undefined))
          }}
        />
        <KnowledgeJobsButton />
        {!graphMode && (
          <IconTip
            label={
              rightCollapsed
                ? t("knowledge.expandRight", "Expand panel")
                : t("knowledge.collapseRight", "Collapse panel")
            }
            side="bottom"
          >
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "h-8 w-8",
                rightCollapsed ? "text-muted-foreground" : "text-foreground",
              )}
              aria-label={
                rightCollapsed
                  ? t("knowledge.expandRight", "Expand panel")
                  : t("knowledge.collapseRight", "Collapse panel")
              }
              aria-expanded={!rightCollapsed}
              onClick={() => handleRightCollapsedChange(!rightCollapsed)}
            >
              {rightCollapsed ? (
                <PanelRightDashed className="h-4 w-4" />
              ) : (
                <PanelRight className="h-4 w-4" />
              )}
            </Button>
          </IconTip>
        )}
        </div>
      </div>

      <div className="flex flex-1 min-h-0">
        {/* Left: KB list + notes — collapsible + resizable (mirrors chat) */}
        <div
          style={{ width: leftCollapsed ? 0 : leftWidth }}
          className={cn("relative h-full min-w-0", !isResizingLeft && PANE_WIDTH_TRANSITION)}
        >
          <div className="h-full overflow-hidden">
            <div
              // `width` is the preferred size; `maxWidth: 100%` lets the content
              // reflow/compress (note rows truncate) when the row can't grant the
              // full pane width (narrow window) instead of being hard-clipped by
              // the overflow. Mirrors the right pane.
              style={{ width: leftWidth, maxWidth: "100%" }}
              aria-hidden={leftCollapsed}
              inert={leftCollapsed ? true : undefined}
              className={cn(
                "flex h-full min-w-0 flex-col border-r border-border-soft/60",
                PANE_SURFACE_TRANSITION,
                leftCollapsed
                  ? "pointer-events-none -translate-x-4 opacity-0"
                  : "translate-x-0 opacity-100",
              )}
            >
          <div className="flex items-center justify-between border-b border-border-soft/60 px-2 py-1.5">
            <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("knowledge.spaces", "Spaces")}
            </span>
            <IconTip label={t("knowledge.showArchived", "Show archived")} side="bottom">
              <Button
                variant="ghost"
                size="icon"
                className={cn(
                  "h-6 w-6",
                  includeArchived ? "text-primary" : "text-muted-foreground",
                )}
                onClick={() => setIncludeArchived((v) => !v)}
              >
                <Archive className="h-3 w-3" />
              </Button>
            </IconTip>
          </div>
          <div className="max-h-48 overflow-auto">
            {kbs.length === 0 && (
              <div className="px-3 py-2 text-xs text-muted-foreground">
                {t("knowledge.noSpaces", "No knowledge spaces yet.")}
              </div>
            )}
            {kbs.map((kb) => (
              <ContextMenu key={kb.id}>
                <ContextMenuTrigger asChild>
                  <button
                    onClick={() =>
                      guardNavigation(() => {
                        setActiveKbId(kb.id)
                        setOpenPath(null)
                        setNoteData(null)
                        setDraftMode(false)
                      })
                    }
                    className={cn(
                      "flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-muted/50",
                      kb.id === activeKbId && "bg-primary/10 text-primary",
                      kb.archived && "opacity-60",
                    )}
                  >
                    <span className="shrink-0">{kb.emoji || "📚"}</span>
                    <span className="flex-1 truncate">{kb.name}</span>
                    {kb.archived && (
                      <Archive className="h-3 w-3 shrink-0 text-muted-foreground" />
                    )}
                    {kb.external &&
                      (kb.allowExternalWrites ? (
                        // External vault with editing unlocked (WS7).
                        <Pencil className="h-3 w-3 shrink-0 text-muted-foreground" />
                      ) : (
                        <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />
                      ))}
                    <span className="shrink-0 text-[10px] text-muted-foreground">
                      {kb.noteCount}
                    </span>
                  </button>
                </ContextMenuTrigger>
                <ContextMenuContent>
                  <ContextMenuItem onClick={() => openEditKb(kb)}>
                    <Pencil className="mr-2 h-3.5 w-3.5" />
                    {t("knowledge.editSpace", "Edit space")}
                  </ContextMenuItem>
                  <ContextMenuItem
                    onClick={() =>
                      // Archiving the active space drops its editor — guard unsaved
                      // edits; archiving any other space can't affect the editor.
                      kb.id === activeKbId
                        ? guardNavigation(() => void toggleArchiveKb(kb))
                        : void toggleArchiveKb(kb)
                    }
                  >
                    {kb.archived ? (
                      <>
                        <ArchiveRestore className="mr-2 h-3.5 w-3.5" />
                        {t("knowledge.unarchive", "Unarchive")}
                      </>
                    ) : (
                      <>
                        <Archive className="mr-2 h-3.5 w-3.5" />
                        {t("knowledge.archive", "Archive")}
                      </>
                    )}
                  </ContextMenuItem>
                  <ContextMenuItem onClick={() => void reindexSpace(kb.id)}>
                    <RefreshCw className="mr-2 h-3.5 w-3.5" />
                    {t("knowledge.reindex", "Reindex")}
                  </ContextMenuItem>
                  <ContextMenuItem
                    className="text-destructive focus:text-destructive"
                    onClick={() => setDeleteKb(kb)}
                  >
                    <Trash2 className="mr-2 h-3.5 w-3.5" />
                    {t("knowledge.deleteSpace", "Delete space")}
                  </ContextMenuItem>
                </ContextMenuContent>
              </ContextMenu>
            ))}
          </div>

          <div className="flex items-center justify-between border-b border-t border-border-soft/60 px-2 py-1.5">
            <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("knowledge.notes", "Notes")}
            </span>
            <div className="flex items-center gap-1">
              <IconTip
                label={
                  reindexActive
                    ? `${t("knowledge.reindexing", "Reindexing…")}${reindexProgress}`
                    : t("knowledge.reindex", "Reindex")
                }
                side="bottom"
              >
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6"
                  onClick={reindex}
                  disabled={reindexActive}
                >
                  <RefreshCw className={cn("h-3 w-3", reindexActive && "animate-spin")} />
                </Button>
              </IconTip>
              {!readOnly && activeKbId && (
                <>
                  <IconTip label={t("knowledge.newFolder", "New folder")} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      onClick={() => {
                        setNewFolderParent("")
                        setNewFolderValue("")
                        setNewFolderOpen(true)
                      }}
                    >
                      <FolderPlus className="h-3 w-3" />
                    </Button>
                  </IconTip>
                  <IconTip label={t("knowledge.newNote", "New note")} side="bottom">
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      onClick={() => guardNavigation(() => startDraft())}
                    >
                      <Plus className="h-3 w-3" />
                    </Button>
                  </IconTip>
                </>
              )}
            </div>
          </div>
          <div
            className={cn(
              "flex-1 overflow-auto py-0.5",
              dragOver === "" && dragItem && "bg-primary/5",
            )}
            onDragOver={(e) => {
              if (!dragItemRef.current || readOnly) return
              e.preventDefault()
              setDragOver("")
            }}
            onDrop={(e) => {
              e.preventDefault()
              handleDropOn("")
            }}
          >
            {renderNodes(noteTree, 0)}
          </div>
            </div>
          </div>
          <div
            className={cn(
              PANE_HANDLE_BASE,
              "right-0",
              leftCollapsed ? "w-0 pointer-events-none opacity-0" : "w-1 opacity-100",
            )}
            onMouseDown={onDragLeft}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("knowledge.resizeLeft", "Resize sidebar")}
          />
        </div>

        {graphMode && activeKbId ? (
          <KnowledgeGraphView
            key={activeKbId}
            kbId={activeKbId}
            activePath={openPath}
            refreshKey={embedCacheKey}
            onOpenNote={(rel) => guardNavigation(() => void openNote(activeKbId, rel))}
          />
        ) : (
          <>
        {/* Center: editor */}
        <div
          ref={centerRef}
          className="flex flex-1 min-w-0 flex-col"
          style={{ minWidth: `min(100%, ${KB_CONTENT_MIN_WIDTH}px)` }}
        >
          {draftMode ? (
            <>
              <div className="flex items-center gap-2 border-b border-border-soft/60 px-3 py-1.5">
                {draftFolder ? (
                  <span
                    className="flex shrink-0 items-center gap-1 truncate text-xs text-muted-foreground"
                    title={draftFolder}
                  >
                    <Folder className="h-3 w-3 shrink-0" />
                    {draftFolder}/
                  </span>
                ) : null}
                <Input
                  value={draftTitle}
                  onChange={(e) => setDraftTitle(e.target.value)}
                  placeholder={t("knowledge.titlePlaceholder", "Untitled")}
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void saveDraft()
                  }}
                  className="h-7 flex-1 border-0 bg-transparent px-1 text-sm font-medium shadow-none focus-visible:ring-0"
                />
                <ModeSwitch mode={mode} onChange={handleModeChange} compact={compactToolbar} />
                <Button variant="outline" size="sm" className="h-7" disabled={saving} onClick={saveDraft}>
                  {saving ? (
                    <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Save className="mr-1 h-3.5 w-3.5" />
                  )}
                  {t("common.save", "Save")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7"
                  onClick={() => guardNavigation(() => setDraftMode(false))}
                >
                  {t("common.cancel", "Cancel")}
                </Button>
              </div>
              <div className="flex-1 min-h-0">
                <NoteEditor
                  value={editorValue}
                  onChange={handleEditorChange}
                  readOnly={false}
                  mode={mode}
                  data={wikilinkData}
                  revealTarget={revealTarget}
                  kbId={activeKbId}
                  onOpenNote={(rel) => {
                    if (activeKbId) guardNavigation(() => void openNote(activeKbId, rel))
                  }}
                  embedCacheKey={embedCacheKey}
                  onOutlineJump={(line) => {
                    handleModeChange("source")
                    setRevealTarget({ line })
                  }}
                />
              </div>
            </>
          ) : openPath && noteData ? (
            <>
              <div className="flex items-center gap-2 border-b border-border-soft/60 px-3 py-1.5">
                {openDir && (
                  <span
                    className="flex shrink-0 items-center gap-1 truncate text-xs text-muted-foreground"
                    title={openDir}
                  >
                    <Folder className="h-3 w-3 shrink-0" />
                    {openDir}/
                  </span>
                )}
                {titleEditing && !readOnly ? (
                  <Input
                    value={titleValue}
                    autoFocus
                    onChange={(e) => setTitleValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        const from = openPath
                        // Edit only the filename; keep the note in its folder.
                        const to = openDir ? `${openDir}/${titleValue}` : titleValue
                        setTitleEditing(false)
                        if (from) guardEdit(from, () => void renameNote(from, to))
                      } else if (e.key === "Escape") {
                        setTitleEditing(false)
                      }
                    }}
                    onBlur={() => setTitleEditing(false)}
                    className="h-7 flex-1 text-xs"
                  />
                ) : (
                  <button
                    type="button"
                    disabled={readOnly}
                    title={readOnly ? (openPath ?? "") : t("knowledge.clickToRename", "Click to rename")}
                    onClick={() => {
                      setTitleValue(openBase)
                      setTitleEditing(true)
                    }}
                    className="flex-1 truncate text-left text-xs text-muted-foreground enabled:hover:text-foreground disabled:cursor-default"
                  >
                    {openBase}
                    {dirty && <span className="ml-1 text-amber-500">●</span>}
                  </button>
                )}
                {readOnly && (
                  <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                    <Lock className="h-3 w-3" />
                    {t("knowledge.readOnly", "Read-only (external vault)")}
                  </span>
                )}
                {mode !== "preview" && mode !== "outline" && (
                  <HeadingOutline
                    content={editorValue}
                    onJump={(line) => setRevealTarget({ line })}
                  />
                )}
                {!readOnly && mode !== "preview" && mode !== "outline" && (
                  <IconTip label={t("knowledge.quickRewrite.title", "Quick rewrite")}>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2"
                      onClick={() => quickRewriteSelection()}
                    >
                      <Sparkles className="h-3.5 w-3.5" />
                    </Button>
                  </IconTip>
                )}
                {openPath && (openKbId ?? activeKbId) && (
                  <IconTip label={t("knowledge.chatPanel.addToChat", "Add to AI chat")}>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2"
                      onClick={() => referenceCurrentSelectionInChat()}
                    >
                      <MessageSquareQuote className="h-3.5 w-3.5" />
                    </Button>
                  </IconTip>
                )}
                <ModeSwitch mode={mode} onChange={handleModeChange} compact={compactToolbar} />
                {!readOnly && (
                  <Button
                    variant="outline"
                    size="sm"
                    className={cn(
                      "h-7",
                      saveStatus === "saved" && "border-green-500 text-green-600",
                      saveStatus === "failed" && "border-red-500 text-red-600",
                    )}
                    disabled={saving || !dirty}
                    onClick={handleSave}
                  >
                    {saving ? (
                      <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                    ) : saveStatus === "saved" ? (
                      <Check className="mr-1 h-3.5 w-3.5" />
                    ) : (
                      <Save className="mr-1 h-3.5 w-3.5" />
                    )}
                    {t("common.save", "Save")}
                  </Button>
                )}
              </div>
              {externalConflict && (
                <div className="flex items-center gap-2 border-b border-amber-500/40 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-400">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                  <span className="flex-1">
                    {t(
                      "knowledge.externalChange.banner",
                      "This file was modified outside the editor. Saving will overwrite those changes.",
                    )}
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-amber-700 hover:text-amber-800 dark:text-amber-400"
                    onClick={() => {
                      const data = externalConflict
                      setNoteData(data)
                      setEditorValue(data.content)
                      setBaseHash(data.contentHash)
                      setDirty(false)
                      setExternalConflict(null)
                    }}
                  >
                    {t("knowledge.externalChange.reload", "Reload")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-amber-700 hover:text-amber-800 dark:text-amber-400"
                    onClick={() => {
                      // Rebase the expected hash so the next save passes the
                      // stale-write guard and overwrites the external version.
                      setBaseHash(externalConflict.contentHash)
                      setExternalConflict(null)
                    }}
                  >
                    {t("knowledge.externalChange.keepMine", "Keep mine")}
                  </Button>
                </div>
              )}
              <div className="flex-1 min-h-0">
                <NoteEditor
                  ref={editorRef}
                  value={editorValue}
                  onChange={(v) => {
                    handleEditorChange(v)
                    setDirty(true)
                  }}
                  readOnly={readOnly}
                  mode={mode}
                  data={wikilinkData}
                  revealTarget={revealTarget}
                  kbId={openKbId}
                  notePath={openPath}
                  onOpenNote={(rel) => {
                    const k = openKbId ?? activeKbId
                    if (k) guardNavigation(() => void openNote(k, rel))
                  }}
                  embedCacheKey={embedCacheKey}
                  onOutlineJump={(line) => {
                    // Outline has no CM6 view to scroll — switch to source first,
                    // then reveal the line (new object identity re-fires reveal).
                    handleModeChange("source")
                    setRevealTarget({ line })
                  }}
                  onReferenceSelection={referenceCurrentSelectionInChat}
                  onRewriteSelection={quickRewriteSelection}
                />
              </div>
            </>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
              <Library className="h-10 w-10 opacity-40" />
              <span className="text-sm">
                {t("knowledge.emptyEditor", "Select a note to view or edit.")}
              </span>
            </div>
          )}
        </div>

        {/* Right: backlinks / tags — collapsible + resizable (mirrors chat) */}
        <div
          style={{ width: rightCollapsed ? 0 : rightWidth }}
          className={cn("relative h-full min-w-0", !isResizingRight && PANE_WIDTH_TRANSITION)}
        >
          <div className="h-full overflow-hidden">
            <div
              // `width` is the preferred size; `maxWidth: 100%` lets the content
              // reflow/compress when the row can't grant the full pane width
              // (narrow window) instead of being hard-clipped by the overflow.
              style={{ width: rightWidth, maxWidth: "100%" }}
              aria-hidden={rightCollapsed}
              inert={rightCollapsed ? true : undefined}
              className={cn(
                "flex h-full min-w-0 flex-col border-l border-border-soft/60",
                PANE_SURFACE_TRANSITION,
                rightCollapsed
                  ? "pointer-events-none translate-x-4 opacity-0"
                  : "translate-x-0 opacity-100",
              )}
            >
          <RightPanelTabs mode={rightMode} onChange={setRightMode} />
          {/* Chat panel stays mounted (so its imperative ref is always ready for
              "add to chat") but only loads when actually shown. */}
          <div
            className={cn(
              "min-h-0 min-w-0 flex-1",
              rightMode === "chat" ? "flex flex-col" : "hidden",
            )}
          >
            <KnowledgeChatPanel
              ref={chatPanelRef}
              active={rightMode === "chat" && !rightCollapsed}
              kbId={openKbId ?? activeKbId}
              notePath={openPath}
              getEditorValue={getEditorValue}
              editorRevision={editorRevision}
              onJumpToQuote={jumpToQuoteInEditor}
            />
          </div>
          <div
            className={cn(
              "min-h-0 min-w-0 flex-1",
              rightMode === "chat" ? "hidden" : "flex flex-col",
            )}
          >
          {hits.length > 0 ? (
            <>
              <div className="flex items-center justify-between border-b border-border-soft/60 px-2 py-1.5">
                <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  {t("knowledge.searchResults", "Search results")}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 px-2 text-[11px]"
                  onClick={() => {
                    setHits([])
                    setQuery("")
                  }}
                >
                  {t("common.clear", "Clear")}
                </Button>
              </div>
              <div className="flex-1 overflow-auto">
                {hits.map((h) => (
                  <button
                    key={`${h.kbId}:${h.noteId}`}
                    onClick={() =>
                      guardNavigation(() => {
                        setActiveKbId(h.kbId)
                        void openNote(h.kbId, h.relPath, { line: h.startLine })
                      })
                    }
                    className="block w-full border-b border-border-soft/40 px-3 py-2 text-left hover:bg-muted/50"
                  >
                    <div className="truncate text-xs font-medium">{h.title}</div>
                    {h.headingPath && (
                      <div className="truncate text-[10px] text-muted-foreground">{h.headingPath}</div>
                    )}
                    <div className="mt-0.5 line-clamp-2 text-[11px] text-muted-foreground">
                      {h.snippet}
                    </div>
                  </button>
                ))}
              </div>
            </>
          ) : noteData ? (
            <div className="flex-1 overflow-auto p-3 text-xs">
              <BacklinksSection
                title={t("knowledge.backlinks", "Backlinks")}
                count={noteData.backlinks.length}
              >
                {noteData.backlinks.map((b, i) => (
                  <button
                    key={i}
                    onClick={() =>
                      guardNavigation(() => {
                        if (activeKbId)
                          void openNote(activeKbId, b.srcRelPath, {
                            line: b.srcStartLine,
                            col: b.srcStartCol,
                          })
                      })
                    }
                    className="block w-full rounded px-1 py-0.5 text-left hover:bg-muted/50"
                  >
                    <span className="text-primary">{b.srcTitle}</span>
                    {b.srcHeadingPath && (
                      <span className="text-muted-foreground"> · {b.srcHeadingPath}</span>
                    )}
                  </button>
                ))}
              </BacklinksSection>

              <BacklinksSection
                title={t("knowledge.outgoingLinks", "Links")}
                count={noteData.outgoingLinks.length}
              >
                {noteData.outgoingLinks.map((l, i) => {
                  const broken = l.targetNoteId == null
                  return (
                    <div key={i} className="flex items-center gap-1 px-1 py-0.5">
                      <Link2 className="h-3 w-3 shrink-0 text-muted-foreground" />
                      <button
                        type="button"
                        disabled={broken}
                        onClick={() => {
                          if (broken || !activeKbId) return
                          const target = notes.find((n) => n.id === l.targetNoteId)
                          if (target) guardNavigation(() => void openNote(activeKbId, target.relPath))
                        }}
                        className={cn(
                          "min-w-0 flex-1 truncate text-left",
                          broken ? "text-red-500" : "text-foreground hover:underline",
                        )}
                        title={l.rawText}
                      >
                        {l.alias || l.targetRef}
                      </button>
                      {broken &&
                        (readOnly ? (
                          <AlertTriangle className="h-3 w-3 shrink-0 text-red-500" />
                        ) : (
                          <IconTip label={t("knowledge.createMissingNote", "Create this note")}>
                            <button
                              type="button"
                              onClick={() =>
                                guardNavigation(() => void createNoteFromRef(l.targetRef))
                              }
                              className="shrink-0 rounded p-0.5 text-red-500 transition-colors hover:bg-red-500/10"
                            >
                              <Plus className="h-3 w-3" />
                            </button>
                          </IconTip>
                        ))}
                    </div>
                  )
                })}
              </BacklinksSection>

              {noteData.tags.length > 0 && (
                <div className="mt-3">
                  <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("knowledge.tags", "Tags")}
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {noteData.tags.map((tag) => (
                      <span
                        key={tag}
                        className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                      >
                        #{tag}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="flex flex-1 items-center justify-center p-3 text-center text-xs text-muted-foreground">
              {t("knowledge.backlinksHint", "Open a note to see its backlinks.")}
            </div>
          )}
          </div>
            </div>
          </div>
          <div
            className={cn(
              PANE_HANDLE_BASE,
              "left-0",
              rightCollapsed ? "w-0 pointer-events-none opacity-0" : "w-1 opacity-100",
            )}
            onMouseDown={onDragRight}
            role="separator"
            aria-orientation="vertical"
            aria-label={t("knowledge.resizeRight", "Resize panel")}
          />
        </div>
          </>
        )}
      </div>

      {/* Create KB dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.newKb", "New space")}</DialogTitle>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (newKbName.trim()) void createKb()
            }}
          >
            <div className="space-y-3 py-2">
              <Input
                value={newKbName}
                onChange={(e) => setNewKbName(e.target.value)}
                placeholder={t("knowledge.kbNamePlaceholder", "Space name")}
                autoFocus
              />
              <div className="space-y-1.5">
                <div className="flex items-center gap-2">
                  <Input
                    value={newKbRoot}
                    readOnly
                    placeholder={t(
                      "knowledge.kbRootPlaceholder",
                      "External vault folder (optional, read-only)",
                    )}
                    className="flex-1 font-mono text-xs"
                  />
                  <Button type="button" variant="outline" onClick={() => void pickKbRoot()}>
                    <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                    {t("knowledge.kbRootPick", "Choose…")}
                  </Button>
                  {newKbRoot && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-9 w-9"
                      onClick={() => setNewKbRoot("")}
                      aria-label={t("common.clear", "Clear")}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  )}
                </div>
                <p className="text-xs text-muted-foreground">
                  {t(
                    "knowledge.kbRootHint",
                    "Leave empty for an internal space. External vaults are read-only in Phase 1.",
                  )}
                </p>
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setCreateOpen(false)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={!newKbName.trim() || kbBusy}>
                {t("common.create", "Create")}
              </Button>
            </DialogFooter>
          </form>
          {!isTauriMode() && (
            <ServerDirectoryBrowser
              open={rootBrowserOpen}
              initialPath={newKbRoot || null}
              onOpenChange={setRootBrowserOpen}
              onSelect={handleKbRootSelect}
            />
          )}
        </DialogContent>
      </Dialog>

      {/* Name prompt — only when a draft has no title and no leading H1 */}
      <Dialog
        open={namePromptOpen}
        onOpenChange={(o) => {
          setNamePromptOpen(o)
          // Cancelling naming drops the parked nav so it can't fire on a later,
          // unrelated draft save (regression guard for #7).
          if (!o) resumeNavRef.current = null
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.nameNote", "Name this note")}</DialogTitle>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (namePromptValue.trim()) void commitDraft(namePromptValue.trim(), true)
            }}
          >
            <div className="py-2">
              <Input
                value={namePromptValue}
                onChange={(e) => setNamePromptValue(e.target.value)}
                placeholder={t("knowledge.titlePlaceholder", "Untitled")}
                autoFocus
              />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => {
                  setNamePromptOpen(false)
                  resumeNavRef.current = null
                }}
              >
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={!namePromptValue.trim() || saving}>
                {t("common.save", "Save")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete note confirmation */}
      <Dialog
        open={deleteConfirmPath != null}
        onOpenChange={(o) => !o && setDeleteConfirmPath(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.deleteNoteTitle", "Delete note")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.deleteNoteBody", {
                name: deleteConfirmPath ?? "",
                defaultValue:
                  'Delete "{{name}}"? The Markdown file will be removed from disk. This cannot be undone.',
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteConfirmPath(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteConfirmPath && void deleteNote(deleteConfirmPath)}
            >
              {t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* New folder — creates a real (empty) directory */}
      <Dialog open={newFolderOpen} onOpenChange={setNewFolderOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {newFolderParent
                ? t("knowledge.newSubfolder", "New subfolder")
                : t("knowledge.newFolder", "New folder")}
            </DialogTitle>
            {newFolderParent ? <DialogDescription>{newFolderParent}/</DialogDescription> : null}
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (newFolderValue.trim()) confirmNewFolder()
            }}
          >
            <div className="py-2">
              <Input
                value={newFolderValue}
                onChange={(e) => setNewFolderValue(e.target.value)}
                placeholder={t("knowledge.folderNamePlaceholder", "Folder name")}
                autoFocus
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setNewFolderOpen(false)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={!newFolderValue.trim()}>
                {t("common.create", "Create")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Rename folder — renames the directory and its contents */}
      <Dialog
        open={renameFolderPath != null}
        onOpenChange={(o) => !o && setRenameFolderPath(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.renameFolder", "Rename folder")}</DialogTitle>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (!renameFolderValue.trim() || !renameFolderPath) return
              const p = renameFolderPath
              const v = renameFolderValue
              // Close this dialog first so the unsaved-changes guard (if it parks
              // the rename) doesn't stack on top of it (#8).
              setRenameFolderPath(null)
              guardEdit(p, () => void renameFolder(p, v))
            }}
          >
            <div className="py-2">
              <Input
                value={renameFolderValue}
                onChange={(e) => setRenameFolderValue(e.target.value)}
                placeholder={t("knowledge.folderNamePlaceholder", "Folder name")}
                autoFocus
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setRenameFolderPath(null)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={!renameFolderValue.trim()}>
                {t("common.rename", "Rename")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete folder — deletes every note under the prefix */}
      <Dialog
        open={deleteFolderPath != null}
        onOpenChange={(o) => !o && setDeleteFolderPath(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.deleteFolderTitle", "Delete folder")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.deleteFolderBody", {
                name: deleteFolderPath ?? "",
                count: deleteFolderPath
                  ? notes.filter((n) => n.relPath.startsWith(`${deleteFolderPath}/`)).length
                  : 0,
                defaultValue:
                  'Delete folder "{{name}}" and its {{count}} note(s)? The Markdown files will be removed from disk. This cannot be undone.',
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteFolderPath(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteFolderPath && void deleteFolder(deleteFolderPath)}
            >
              {t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Move to… — pick a destination folder for a note/folder */}
      <Dialog open={moveItem != null} onOpenChange={(o) => !o && setMoveItem(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("knowledge.moveToTitle", {
                name: moveItem ? (moveItem.path.split("/").pop() ?? moveItem.path) : "",
                defaultValue: 'Move "{{name}}" to…',
              })}
            </DialogTitle>
          </DialogHeader>
          <div className="max-h-72 overflow-auto py-1">
            {renderMoveRow(t("knowledge.rootFolder", "Root"), "", 0)}
            {renderMoveTree(moveTree, 1)}
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setMoveItem(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit space — name + emoji */}
      <Dialog open={editKb != null} onOpenChange={(o) => !o && setEditKb(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.editSpace", "Edit space")}</DialogTitle>
          </DialogHeader>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              if (editKbName.trim()) void saveEditKb()
            }}
          >
            <div className="flex gap-2 py-2">
              <Input
                value={editKbEmoji}
                onChange={(e) => setEditKbEmoji(e.target.value)}
                placeholder={t("knowledge.emojiPlaceholder", "Emoji (optional)")}
                className="w-32 shrink-0 text-center"
              />
              <Input
                value={editKbName}
                onChange={(e) => setEditKbName(e.target.value)}
                placeholder={t("knowledge.kbNamePlaceholder", "Space name")}
                autoFocus
                className="flex-1"
              />
            </div>
            {editKb?.external && (
              <div className="flex items-start justify-between gap-4 rounded-md border border-border/60 p-3">
                <div className="space-y-1">
                  <div className="text-sm font-medium">
                    {t("knowledge.allowExternalWrites", "Allow editing this vault")}
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {t(
                      "knowledge.allowExternalWritesHint",
                      "External vaults are read-only by default. Enable to let editing and AI tools write to the bound folder. Background maintenance never writes external vaults.",
                    )}
                  </p>
                </div>
                <Switch
                  checked={editKbAllowExternal}
                  onCheckedChange={setEditKbAllowExternal}
                  className="mt-0.5 shrink-0"
                />
              </div>
            )}
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setEditKb(null)}>
                {t("common.cancel", "Cancel")}
              </Button>
              <Button type="submit" disabled={!editKbName.trim() || kbBusy}>
                {t("common.save", "Save")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete space confirmation */}
      <Dialog open={deleteKb != null} onOpenChange={(o) => !o && setDeleteKb(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.deleteSpaceTitle", "Delete space")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.deleteSpaceBody", {
                name: deleteKb?.name ?? "",
                defaultValue:
                  'Delete space "{{name}}" and all its notes? For internal spaces the notes are removed from disk; external vaults are left untouched. This cannot be undone.',
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteKb(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button variant="destructive" onClick={deleteKbConfirm}>
              {t("common.delete", "Delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Unsaved-changes guard */}
      <Dialog open={pendingNav != null} onOpenChange={(o) => !o && setPendingNav(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("knowledge.unsavedTitle", "Unsaved changes")}</DialogTitle>
            <DialogDescription>
              {t("knowledge.unsavedBody", "You have unsaved changes. Save before leaving?")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setPendingNav(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                const act = pendingNav
                setPendingNav(null)
                act?.()
              }}
            >
              {t("knowledge.discardChanges", "Discard")}
            </Button>
            <Button
              onClick={async () => {
                const act = pendingNav
                setPendingNav(null)
                const ok = await persistCurrent()
                if (ok) act?.()
                // Draft still needs a name: persistCurrent opened the name prompt.
                // Park the intent so it resumes once the draft is named+saved (#7).
                else resumeNavRef.current = act
              }}
            >
              {t("common.save", "Save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* One-shot floating quick-rewrite bar (replaces the old AI rewrite modal). */}
      {quickRewrite && (openKbId ?? activeKbId) && (
        <div className="fixed left-1/2 top-24 z-50 -translate-x-1/2">
          <QuickRewriteBar
            kbId={(openKbId ?? activeKbId) as string}
            notePath={openPath}
            before={quickRewrite.before}
            onApply={(after) => {
              const ed = editorRef.current
              if (!ed) return
              const { before, from, to } = quickRewrite
              const doc = ed.getText()
              // Fast path: the captured range still holds the original text
              // (no shifting edit happened during generation) — apply in place.
              if (doc.slice(from, to) === before) {
                ed.replaceRange(from, to, after)
                setQuickRewrite(null)
                return
              }
              // The buffer changed under us. Re-anchor by unique occurrence;
              // refuse on 0 / multiple matches rather than overwrite the wrong
              // text (mirrors note_patch's unique-hit guard, D14).
              const idx = before ? doc.indexOf(before) : -1
              if (idx >= 0 && doc.indexOf(before, idx + 1) === -1) {
                ed.replaceRange(idx, idx + before.length, after)
                setQuickRewrite(null)
                return
              }
              toast.error(t("knowledge.quickRewrite.staleSelection"))
            }}
            onClose={() => setQuickRewrite(null)}
          />
        </div>
      )}
    </div>
  )
}

// ── Note tree (implicit folders derived from note rel paths) ──
type TreeNode =
  | { type: "folder"; name: string; path: string; children: TreeNode[] }
  | { type: "note"; name: string; note: Note }

// Group notes into a folder tree by splitting their rel path on "/". `dirs` seeds
// real (possibly empty) folders from disk so they show even before they hold a
// note. Returns deepest folder node for a given "/"-joined path, creating chain.
function buildNoteTree(notes: Note[], dirs: string[]): TreeNode[] {
  const root: TreeNode[] = []
  const folders = new Map<string, Extract<TreeNode, { type: "folder" }>>()
  const ensureFolder = (path: string): Extract<TreeNode, { type: "folder" }> | null => {
    let siblings = root
    let curPath = ""
    let node: Extract<TreeNode, { type: "folder" }> | null = null
    for (const seg of path.split("/")) {
      if (!seg) continue
      curPath = curPath ? `${curPath}/${seg}` : seg
      let folder = folders.get(curPath)
      if (!folder) {
        folder = { type: "folder", name: seg, path: curPath, children: [] }
        folders.set(curPath, folder)
        siblings.push(folder)
      }
      siblings = folder.children
      node = folder
    }
    return node
  }
  for (const dir of dirs) ensureFolder(dir)
  for (const note of notes) {
    const parts = note.relPath.split("/")
    const fileName = parts.pop() ?? note.relPath
    const parent = parts.join("/")
    const siblings = parent ? (ensureFolder(parent)?.children ?? root) : root
    siblings.push({ type: "note", name: fileName, note })
  }
  const sort = (nodes: TreeNode[]) => {
    nodes.sort((a, b) =>
      a.type !== b.type
        ? a.type === "folder"
          ? -1
          : 1
        : a.name.localeCompare(b.name),
    )
    for (const n of nodes) if (n.type === "folder") sort(n.children)
  }
  sort(root)
  return root
}

// Detect the HTTP write-gate rejection (filesystem.allowRemoteWrites = false) so
// we can point the user at the toggle instead of a generic "failed".
function isRemoteWriteBlocked(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : String(e)
  return /allowremotewrites|remote file writes are disabled/i.test(msg)
}

// First ATX H1 in the body, skipping a leading YAML frontmatter block. Used to
// derive a draft's filename when the title field is left empty.
function firstHeading(md: string): string | null {
  let body = md
  if (body.startsWith("---\n") || body.startsWith("---\r\n")) {
    const close = body.indexOf("\n---", 3)
    if (close !== -1) {
      const nl = body.indexOf("\n", close + 1)
      body = nl !== -1 ? body.slice(nl + 1) : ""
    }
  }
  const m = body.match(/^#[ \t]+(.+?)[ \t]*$/m)
  return m ? m[1].trim() : null
}

function RightPanelTabs({
  mode,
  onChange,
}: {
  mode: "links" | "chat"
  onChange: (m: "links" | "chat") => void
}) {
  const { t } = useTranslation()
  const tabs: { key: "links" | "chat"; label: string }[] = [
    { key: "chat", label: t("knowledge.chatPanel.tab", "AI Agent") },
    { key: "links", label: t("knowledge.backlinks", "Backlinks") },
  ]
  return (
    <div className="flex shrink-0 border-b border-border-soft/60 px-1.5 py-1">
      <div className="flex w-full overflow-hidden rounded-md border border-border-soft/60">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            onClick={() => onChange(tab.key)}
            className={cn(
              "min-w-0 flex-1 truncate px-2 py-1 text-[11px] font-medium transition-colors",
              mode === tab.key
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground hover:bg-muted/50",
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>
    </div>
  )
}

const MODE_KEYS: NoteEditorMode[] = ["source", "live", "split", "preview", "outline"]

function ModeSwitch({
  mode,
  onChange,
  compact = false,
}: {
  mode: NoteEditorMode
  onChange: (m: NoteEditorMode) => void
  /** Narrow editor column: collapse the segmented control into a dropdown so the
   *  crowded toolbar doesn't wrap. Driven by the center-pane width measurement. */
  compact?: boolean
}) {
  const { t } = useTranslation()

  if (compact) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            size="sm"
            className="h-7 shrink-0 gap-1 px-2 text-[11px] font-normal"
          >
            {t(`knowledge.mode.${mode}`, mode)}
            <ChevronDown className="h-3 w-3 text-muted-foreground" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-[8rem]">
          {MODE_KEYS.map((m) => (
            <DropdownMenuItem key={m} onSelect={() => onChange(m)} className="gap-2 text-xs">
              <Check
                className={cn("h-3.5 w-3.5", mode === m ? "text-primary opacity-100" : "opacity-0")}
              />
              {t(`knowledge.mode.${m}`, m)}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    )
  }

  return (
    <div className="flex shrink-0 overflow-hidden rounded-md border border-border-soft/60">
      {MODE_KEYS.map((m) => (
        <button
          key={m}
          onClick={() => onChange(m)}
          className={cn(
            "whitespace-nowrap px-2 py-0.5 text-[11px]",
            mode === m
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:bg-muted/50",
          )}
        >
          {t(`knowledge.mode.${m}`, m)}
        </button>
      ))}
    </div>
  )
}

function BacklinksSection({
  title,
  count,
  children,
}: {
  title: string
  count: number
  children: React.ReactNode
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {title} ({count})
      </div>
      {count === 0 ? (
        <div className="px-1 text-[11px] text-muted-foreground/70">—</div>
      ) : (
        children
      )}
    </div>
  )
}
