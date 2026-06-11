/**
 * Caret-aware mention popper state for the chat textarea.
 *
 * ChatInput owns the textarea + input string and delegates to this hook for
 * the popper. Keyboard handling mirrors {@link useSlashCommands}: the parent
 * `onKeyDown` chains slash → mention; the first to return `true` consumes
 * the event. Slash menu owns `Enter` while it is open, so mention popper
 * only sees `Enter` when slash is closed.
 *
 * The `@` popper is the unified entry point (design: `@` = files + knowledge
 * notes + built-in skills). It shows three sections — working dir **files**,
 * reachable knowledge **notes**, and curated **skills** (office trio + browser
 * + mac control) — over a single flattened keyboard cursor. Files insert
 * `@path`; notes insert `[[name]]`; skills insert `@skill:<name>`. The backend
 * resolves notes and skills at send time (`knowledge::inject` /
 * `skills::mention`); only files become attachments client-side.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { detectActiveMention, formatMentionInsertion } from "./mentionTokens"
import { entryFromDir, entryFromMatch, joinAbs, type MentionEntry, type MentionMode } from "./types"
import { formatNoteInsertion, relPathToken } from "../note-mention/noteTokens"
import {
  formatSkillInsertion,
  skillMatchesQuery,
  skillMentionMeta,
  skillQueryFromToken,
  type MentionableSkill,
} from "../skill-mention/skillTokens"
import type { KbDraftAttachment, ReferenceableNote } from "@/types/knowledge"
import type { ComposerInputHandle } from "../input/composerInputHandle"

const SEARCH_DEBOUNCE_MS = 180
const MAX_NOTE_ROWS = 50

interface ActiveMention {
  anchor: number
  caret: number
  token: string
}

/** Context the note section needs; absent → `@` is files-only (back-compat). */
export interface MentionNoteContext {
  sessionId: string | null
  projectId: string | null
  draftKbAttachments: KbDraftAttachment[]
}

export interface UseFileMentionReturn {
  isOpen: boolean
  entries: MentionEntry[]
  /** Knowledge-note section rows (already filtered by the `@` token). */
  noteEntries: ReferenceableNote[]
  /** Knowledge-note section fetch in flight (drives its loading spinner). */
  notesLoading: boolean
  /** Whether a note source exists (drives the note section header/empty state). */
  noteCapable: boolean
  /** Built-in skill section rows (already filtered by the `@` token). */
  skillEntries: MentionableSkill[]
  /** Whether the skill section is enabled (drives its header/empty state). */
  skillCapable: boolean
  /** Flat cursor over `[...entries, ...noteEntries, ...skillEntries]`. */
  selectedIndex: number
  mode: MentionMode
  /** Absolute path of the directory currently being listed (list mode). */
  dirPath: string | null
  loading: boolean
  error: string | null
  /** Server reported it capped the list/search; surface a hint in the UI. */
  truncated: boolean
  /** Current `@` query text (without the trigger). */
  query: string
  /** File rows are intentionally hidden for a bare `@`. */
  hasFileQuery: boolean
  /** ChatInput's onKeyDown should delegate here; returns true if consumed. */
  handleKeyDown: (e: React.KeyboardEvent<HTMLElement>) => boolean
  applyEntry: (entry: MentionEntry) => void
  /** Pick a knowledge note from the `@` menu — inserts `[[name]]`. */
  applyNote: (note: ReferenceableNote) => void
  /** Pick a built-in skill from the `@` menu — inserts `@skill:<name>`. */
  applySkill: (skill: MentionableSkill) => void
  /** Remove a mention by its raw `@...` substring (chip X-button click). */
  removeMention: (raw: string) => void
  /** Re-evaluate the caret context after `onSelect` / `onClick` / paste. */
  recheckTrigger: () => void
  setSelectedIndex: (i: number) => void
}

export function useFileMention(
  input: string,
  setInput: (next: string) => void,
  inputHandleRef: React.RefObject<ComposerInputHandle | null>,
  workingDir: string | null,
  noteCtx?: MentionNoteContext,
  skillEnabled = false,
): UseFileMentionReturn {
  const { t } = useTranslation()
  const [mode, setMode] = useState<MentionMode>("list")
  const [entries, setEntries] = useState<MentionEntry[]>([])
  const [dirPath, setDirPath] = useState<string | null>(null)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [truncated, setTruncated] = useState(false)
  const [allNotes, setAllNotes] = useState<ReferenceableNote[]>([])
  const [notesLoading, setNotesLoading] = useState(false)
  // Built-in skill catalog: fetched once when enabled (static per session,
  // OS-gated server-side), then filtered client-side by the `@` token.
  const [allSkills, setAllSkills] = useState<MentionableSkill[]>([])

  const [active, setActive] = useState<ActiveMention | null>(null)
  const isOpen = active !== null

  const requestSeqRef = useRef(0)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inputRef = useRef(input)
  inputRef.current = input
  const workingDirRef = useRef(workingDir)
  workingDirRef.current = workingDir
  const noteCtxRef = useRef(noteCtx)
  noteCtxRef.current = noteCtx
  const skillEnabledRef = useRef(skillEnabled)
  skillEnabledRef.current = skillEnabled

  const sessionId = noteCtx?.sessionId ?? null
  const projectId = noteCtx?.projectId ?? null
  const draftKbIds = (noteCtx?.draftKbAttachments ?? []).map((a) => a.kbId)
  const draftKey = draftKbIds.join(",")
  const noteCapable = !!noteCtx && (sessionId != null || draftKbIds.length > 0)

  const reset = useCallback(() => {
    setEntries([])
    setAllNotes([])
    setSelectedIndex(0)
    setActive(null)
    setError(null)
    setTruncated(false)
    setMode("list")
    setDirPath(null)
    if (debounceRef.current) {
      clearTimeout(debounceRef.current)
      debounceRef.current = null
    }
  }, [])

  useEffect(() => {
    reset()
  }, [workingDir, reset])

  const recheckTrigger = useCallback(() => {
    // `@` opens when there's a file source (working dir), a note source
    // (a session / staged draft attaches), OR the skill section is enabled
    // (the curated built-in set is always available). Files-only when neither
    // notes nor skills apply.
    const canFile = !!workingDirRef.current
    const ctx = noteCtxRef.current
    const canNote = !!ctx && (ctx.sessionId != null || ctx.draftKbAttachments.length > 0)
    const canSkill = skillEnabledRef.current
    if (!canFile && !canNote && !canSkill) {
      setActive((prev) => (prev ? null : prev))
      return
    }
    const inputHandle = inputHandleRef.current
    if (!inputHandle) return
    const caret = inputHandle.getSelectionRange().start
    const result = detectActiveMention(inputRef.current, caret)
    if (!result) {
      setActive((prev) => (prev ? null : prev))
      return
    }
    setActive((prev) =>
      prev &&
      prev.anchor === result.anchor &&
      prev.caret === result.caret &&
      prev.token === result.token
        ? prev
        : { anchor: result.anchor, caret: result.caret, token: result.token },
    )
  }, [inputHandleRef])

  useEffect(() => {
    recheckTrigger()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [input])

  // ── File section: list / search the working dir. Suppressed for an explicit
  // `@skill:` query (the user is clearly after a skill, not a file). ──
  useEffect(() => {
    const tokenIsSkill = active?.token.trim().toLowerCase().startsWith("skill:") ?? false
    if (!active || !workingDir || active.token.trim().length === 0 || tokenIsSkill) {
      requestSeqRef.current++
      setEntries([])
      setLoading(false)
      setError(null)
      setTruncated(false)
      return
    }
    const seq = ++requestSeqRef.current
    const token = active.token
    const transport = getTransport()
    const isSearch = token.length > 0 && !token.includes("/")

    if (debounceRef.current) {
      clearTimeout(debounceRef.current)
      debounceRef.current = null
    }

    const run = async () => {
      try {
        setLoading(true)
        setError(null)
        if (isSearch) {
          const res = await transport.searchFiles(workingDir, token, 50)
          if (seq !== requestSeqRef.current) return
          setMode("search")
          setDirPath(workingDir)
          setEntries(res.matches.map(entryFromMatch))
          setTruncated(res.truncated)
          setSelectedIndex(0)
        } else {
          const slashIdx = token.lastIndexOf("/")
          const dirPart = slashIdx >= 0 ? token.slice(0, slashIdx) : ""
          const namePrefix = slashIdx >= 0 ? token.slice(slashIdx + 1) : token
          const target = joinAbs(workingDir, dirPart)
          const res = await transport.listServerDirectory(target)
          if (seq !== requestSeqRef.current) return
          const filtered = namePrefix
            ? res.entries.filter((e) => e.name.toLowerCase().startsWith(namePrefix.toLowerCase()))
            : res.entries
          setMode("list")
          setDirPath(res.path)
          setEntries(filtered.map((e) => entryFromDir(workingDir, e)))
          setTruncated(res.truncated)
          setSelectedIndex(0)
        }
      } catch (err) {
        if (seq !== requestSeqRef.current) return
        setError(err instanceof Error ? err.message : String(err))
        setEntries([])
        setTruncated(false)
      } finally {
        if (seq === requestSeqRef.current) setLoading(false)
      }
    }

    if (isSearch) {
      debounceRef.current = setTimeout(run, SEARCH_DEBOUNCE_MS)
    } else {
      void run()
    }

    return () => {
      if (debounceRef.current) {
        clearTimeout(debounceRef.current)
        debounceRef.current = null
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.token, workingDir])

  // ── Note section: load the reachable set once per open, filter client-side. ──
  useEffect(() => {
    if (!isOpen || !noteCapable) {
      setAllNotes([])
      setNotesLoading(false)
      return
    }
    let cancelled = false
    setNotesLoading(true)
    getTransport()
      .call<ReferenceableNote[]>("list_referenceable_notes_cmd", {
        sessionId: sessionId ?? undefined,
        projectId: sessionId ? (projectId ?? undefined) : undefined,
        draftKbIds: sessionId ? undefined : draftKbIds,
      })
      .then((notes) => {
        if (!cancelled) setAllNotes(notes)
      })
      .catch(() => {
        if (!cancelled) setAllNotes([])
      })
      .finally(() => {
        if (!cancelled) setNotesLoading(false)
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, noteCapable, sessionId, projectId, draftKey])

  // ── Skill section: fetch the curated built-in catalog once when enabled. ──
  useEffect(() => {
    if (!skillEnabled) {
      setAllSkills([])
      return
    }
    let cancelled = false
    getTransport()
      .call<MentionableSkill[]>("list_mentionable_skills")
      .then((skills) => {
        if (!cancelled) setAllSkills(skills)
      })
      .catch(() => {
        if (!cancelled) setAllSkills([])
      })
    return () => {
      cancelled = true
    }
  }, [skillEnabled])

  // An explicit `@skill:` query targets the skill section only — suppress the
  // file + note sections so the menu doesn't show an empty "Files" header or
  // notes whose text merely contains "skill:".
  const tokenIsSkillQuery = (active?.token ?? "").trim().toLowerCase().startsWith("skill:")

  const noteEntries = useMemo(() => {
    if (!noteCapable || tokenIsSkillQuery) return []
    const q = active?.token.trim().toLowerCase() ?? ""
    const matched = q
      ? allNotes.filter(
          (n) => n.title.toLowerCase().includes(q) || n.relPath.toLowerCase().includes(q),
        )
      : allNotes
    return matched.slice(0, MAX_NOTE_ROWS)
  }, [allNotes, active, noteCapable, tokenIsSkillQuery])

  const skillEntries = useMemo(() => {
    if (!skillEnabled) return []
    const q = skillQueryFromToken(active?.token ?? "")
    return allSkills.filter((s) => skillMatchesQuery(s.name, q))
  }, [allSkills, active, skillEnabled])

  const total = entries.length + noteEntries.length + skillEntries.length
  const hasFileQuery = (active?.token.trim().length ?? 0) > 0 && !tokenIsSkillQuery
  const fileSectionVisible = !!workingDir && hasFileQuery
  const emptyMenuVisible = fileSectionVisible || !!error || notesLoading || skillEntries.length > 0

  // Reset the cursor to the top when the query changes (fresh result set).
  useEffect(() => {
    setSelectedIndex(0)
  }, [active?.token])

  // Keep the flat cursor in range as either section's length changes.
  useEffect(() => {
    setSelectedIndex((i) => (total === 0 ? 0 : Math.min(i, total - 1)))
  }, [total])

  const applyEntry = useCallback(
    (entry: MentionEntry) => {
      if (!active) return
      // Directory: trailing `/` keeps the popper open for the next level.
      const insertion = entry.isDir
        ? formatMentionInsertion(entry.relPath + "/")
        : formatMentionInsertion(entry.relPath) + " "
      const before = inputRef.current.slice(0, active.anchor)
      const after = inputRef.current.slice(active.caret)
      const next = before + insertion + after
      const newCaret = (before + insertion).length
      setInput(next)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (inputHandle) {
          inputHandle.focus()
          inputHandle.setSelectionRange(newCaret, newCaret)
        }
      })
      if (!entry.isDir) {
        reset()
      }
    },
    [active, setInput, inputHandleRef, reset],
  )

  const applyNote = useCallback(
    (note: ReferenceableNote) => {
      if (!active) return
      // Title token by default; rel-path (sans .md) on title collision, so the
      // inserted `[[ ]]` resolves to this exact note (matches the `[[` picker).
      const titleKey = note.title.trim().toLowerCase()
      const collision = allNotes.filter((n) => n.title.trim().toLowerCase() === titleKey).length > 1
      const inner = collision
        ? relPathToken(note.relPath)
        : note.title.trim() || relPathToken(note.relPath)
      const insertion = formatNoteInsertion(inner) + " "
      const before = inputRef.current.slice(0, active.anchor)
      const after = inputRef.current.slice(active.caret)
      const newCaret = (before + insertion).length
      setInput(before + insertion + after)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (inputHandle) {
          inputHandle.focus()
          inputHandle.setSelectionRange(newCaret, newCaret)
        }
      })
      reset()
    },
    [active, allNotes, setInput, inputHandleRef, reset],
  )

  const applySkill = useCallback(
    (skill: MentionableSkill) => {
      if (!active) return
      const meta = skillMentionMeta(skill.name)
      const label = meta ? t(meta.labelKey) : skill.name
      const insertion = formatSkillInsertion(skill.name, label) + " "
      const before = inputRef.current.slice(0, active.anchor)
      const after = inputRef.current.slice(active.caret)
      const newCaret = (before + insertion).length
      setInput(before + insertion + after)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (inputHandle) {
          inputHandle.focus()
          inputHandle.setSelectionRange(newCaret, newCaret)
        }
      })
      reset()
    },
    [active, setInput, inputHandleRef, reset, t],
  )

  const applyAtIndex = useCallback(
    (i: number) => {
      if (i < entries.length) {
        applyEntry(entries[i])
      } else if (i < entries.length + noteEntries.length) {
        const note = noteEntries[i - entries.length]
        if (note) applyNote(note)
      } else {
        const skill = skillEntries[i - entries.length - noteEntries.length]
        if (skill) applySkill(skill)
      }
    },
    [entries, noteEntries, skillEntries, applyEntry, applyNote, applySkill],
  )

  const removeMention = useCallback(
    (raw: string) => {
      const current = inputRef.current
      const idx = current.indexOf(raw)
      if (idx < 0) return
      const tail = current[idx + raw.length] === " " ? 1 : 0
      const next = current.slice(0, idx) + current.slice(idx + raw.length + tail)
      setInput(next)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (inputHandle) {
          inputHandle.focus()
          inputHandle.setSelectionRange(idx, idx)
        }
      })
    },
    [setInput, inputHandleRef],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLElement>): boolean => {
      if (!isOpen || total === 0) {
        // Popper open but empty (loading / no rows) — swallow nav + commit keys
        // so Enter doesn't send the half-typed `@` token.
        if (
          isOpen &&
          emptyMenuVisible &&
          (e.key === "Escape" ||
            e.key === "Enter" ||
            e.key === "Tab" ||
            e.key === "ArrowDown" ||
            e.key === "ArrowUp")
        ) {
          e.preventDefault()
          if (e.key === "Escape") reset()
          return true
        }
        return false
      }
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault()
          setSelectedIndex((i) => (i + 1) % total)
          return true
        case "ArrowUp":
          e.preventDefault()
          setSelectedIndex((i) => (i - 1 + total) % total)
          return true
        case "Enter":
        case "Tab":
          e.preventDefault()
          applyAtIndex(selectedIndex)
          return true
        case "Escape":
          e.preventDefault()
          reset()
          return true
        default:
          return false
      }
    },
    [isOpen, total, emptyMenuVisible, selectedIndex, applyAtIndex, reset],
  )

  return {
    isOpen,
    entries,
    noteEntries,
    notesLoading,
    noteCapable,
    skillEntries,
    skillCapable: skillEnabled,
    selectedIndex,
    mode,
    dirPath,
    loading,
    error,
    truncated,
    query: active?.token ?? "",
    hasFileQuery,
    handleKeyDown,
    applyEntry,
    applyNote,
    applySkill,
    removeMention,
    recheckTrigger,
    setSelectedIndex,
  }
}
