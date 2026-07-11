import { useState, useRef, useEffect, useCallback, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ReferenceableNote, KbDraftAttachment } from "@/types/knowledge"
import { detectActiveNoteRef, formatNoteInsertion, relPathToken } from "./noteTokens"
import { noteMentionErrorDetail } from "./noteMentionFeedback"
import type { ComposerInputHandle } from "../input/composerInputHandle"

const MAX_ROWS = 50

export interface NoteMentionState {
  isOpen: boolean
  entries: ReferenceableNote[]
  selectedIndex: number
  loading: boolean
  loadErrorDetail: string | null
  setSelectedIndex: (i: number) => void
  applyEntry: (entry: ReferenceableNote) => void
  handleKeyDown: (e: React.KeyboardEvent<HTMLElement>) => boolean
  recheckTrigger: () => void
}

/**
 * Chat-composer `[[note]]` picker, parallel to {@link useFileMention}. Typing
 * `[[` opens a popper of notes reachable from the current chat (attached KBs, or
 * staged draft attaches for a brand-new chat); picking one splices `[[name]]`
 * into the textarea. The backend resolves the literal `[[ ]]` at send time
 * (inject.rs) — no send-time expansion here.
 *
 * `enabled=false` (e.g. QuickChat) makes this a no-op so those surfaces keep
 * their prior composer behavior.
 */
export function useNoteMention(
  input: string,
  setInput: (value: string) => void,
  inputHandleRef: React.RefObject<ComposerInputHandle | null>,
  sessionId: string | null,
  projectId: string | null,
  draftKbAttachments: KbDraftAttachment[],
  enabled: boolean,
): NoteMentionState {
  const [active, setActive] = useState<ActiveState | null>(null)
  const [allNotes, setAllNotes] = useState<ReferenceableNote[]>([])
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [loading, setLoading] = useState(false)
  const [loadErrorDetail, setLoadErrorDetail] = useState<string | null>(null)
  const inputRef = useRef(input)
  inputRef.current = input
  const enabledRef = useRef(enabled)
  enabledRef.current = enabled

  // Stable string key for the draft kb-id set (array identity changes each render).
  const draftKbIds = draftKbAttachments.map((a) => a.kbId)
  const draftKey = draftKbIds.join(",")

  const recheckTrigger = useCallback(() => {
    if (!enabledRef.current) {
      setActive((prev) => (prev ? null : prev))
      return
    }
    const inputHandle = inputHandleRef.current
    const caret = inputHandle?.getSelectionRange().start ?? inputRef.current.length
    const next = detectActiveNoteRef(inputRef.current, caret)
    // Preserve identity when unchanged to avoid churning the entries memo + reset.
    setActive((prev) => {
      if (!next) return prev ? null : prev
      return prev &&
        prev.anchor === next.anchor &&
        prev.caret === next.caret &&
        prev.token === next.token
        ? prev
        : next
    })
  }, [inputHandleRef])

  useEffect(() => {
    recheckTrigger()
  }, [input, enabled, recheckTrigger])

  const isOpen = active !== null

  // Load the referenceable set when the popper opens (re-fetch if the chat
  // context changes). Token filtering happens client-side, so growing the query
  // does not re-fetch.
  useEffect(() => {
    if (!isOpen) {
      setAllNotes([])
      setLoadErrorDetail(null)
      return
    }
    setLoading(true)
    setLoadErrorDetail(null)
    let cancelled = false
    getTransport()
      .call<ReferenceableNote[]>("list_referenceable_notes_cmd", {
        sessionId: sessionId ?? undefined,
        projectId: sessionId ? (projectId ?? undefined) : undefined,
        draftKbIds: sessionId ? undefined : draftKbIds,
      })
      .then((notes) => {
        if (!cancelled) {
          setAllNotes(notes)
          setLoadErrorDetail(null)
        }
      })
      .catch((e) => {
        if (!cancelled) {
          logger.error("chat", "useNoteMention::load", "load referenceable notes failed", e)
          setAllNotes([])
          setLoadErrorDetail(noteMentionErrorDetail(e))
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
    // draftKbIds is captured via the stable draftKey; isOpen/session drive refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, sessionId, projectId, draftKey])

  const entries = useMemo(() => {
    const q = active?.token.trim().toLowerCase() ?? ""
    const matched = q
      ? allNotes.filter(
          (n) => n.title.toLowerCase().includes(q) || n.relPath.toLowerCase().includes(q),
        )
      : allNotes
    return matched.slice(0, MAX_ROWS)
  }, [allNotes, active])

  // Reset the cursor to the top when the query changes (fresh result set).
  useEffect(() => {
    setSelectedIndex(0)
  }, [active?.token])

  // Keep the cursor in range as the filtered set shrinks.
  useEffect(() => {
    setSelectedIndex((i) => (entries.length === 0 ? 0 : Math.min(i, entries.length - 1)))
  }, [entries.length])

  const applyEntry = useCallback(
    (entry: ReferenceableNote) => {
      const a = active
      if (!a) return
      // Title token by default; rel-path (sans .md) when the title collides
      // across the referenceable set, so the inserted `[[ ]]` resolves to this
      // exact note rather than the resolver's tie-break winner.
      const titleKey = entry.title.trim().toLowerCase()
      const collision = allNotes.filter((n) => n.title.trim().toLowerCase() === titleKey).length > 1
      const inner = collision
        ? relPathToken(entry.relPath)
        : entry.title.trim() || relPathToken(entry.relPath)
      const insertion = formatNoteInsertion(inner) + " "
      const before = inputRef.current.slice(0, a.anchor)
      const after = inputRef.current.slice(a.caret)
      const newCaret = (before + insertion).length
      setInput(before + insertion + after)
      setActive(null)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (inputHandle) {
          inputHandle.focus()
          inputHandle.setSelectionRange(newCaret, newCaret)
        }
      })
    },
    [active, allNotes, setInput, inputHandleRef],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLElement>): boolean => {
      if (!isOpen) return false
      if (entries.length === 0) {
        // Popper is visibly open (loading / "no notes") — swallow nav + commit
        // keys so Enter doesn't send the half-typed `[[` token.
        if (
          e.key === "Escape" ||
          e.key === "Enter" ||
          e.key === "Tab" ||
          e.key === "ArrowDown" ||
          e.key === "ArrowUp"
        ) {
          e.preventDefault()
          if (e.key === "Escape") setActive(null)
          return true
        }
        return false
      }
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault()
          setSelectedIndex((i) => (i + 1) % entries.length)
          return true
        case "ArrowUp":
          e.preventDefault()
          setSelectedIndex((i) => (i - 1 + entries.length) % entries.length)
          return true
        case "Enter":
        case "Tab": {
          e.preventDefault()
          const entry = entries[selectedIndex]
          if (entry) applyEntry(entry)
          return true
        }
        case "Escape":
          e.preventDefault()
          setActive(null)
          return true
        default:
          return false
      }
    },
    [isOpen, entries, selectedIndex, applyEntry],
  )

  return {
    isOpen,
    entries,
    selectedIndex,
    loading,
    loadErrorDetail,
    setSelectedIndex,
    applyEntry,
    handleKeyDown,
    recheckTrigger,
  }
}

interface ActiveState {
  anchor: number
  caret: number
  token: string
}
