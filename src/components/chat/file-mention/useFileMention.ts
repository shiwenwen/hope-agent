/**
 * Caret-aware mention popper state for the chat textarea.
 *
 * ChatInput owns the textarea + input string and delegates to this hook for
 * the popper. Keyboard handling mirrors {@link useSlashCommands}: the parent
 * `onKeyDown` chains slash → mention; the first to return `true` consumes
 * the event. Slash menu owns `Enter` while it is open, so mention popper
 * only sees `Enter` when slash is closed.
 */

import React, { useEffect, useRef, useState, useEffectEvent } from "react"
import { getTransport } from "@/lib/transport-provider"
import { detectActiveMention, formatMentionInsertion } from "./mentionTokens"
import { entryFromDir, entryFromMatch, joinAbs, type MentionEntry, type MentionMode } from "./types"

const SEARCH_DEBOUNCE_MS = 180

interface ActiveMention {
  anchor: number
  caret: number
  token: string
}

export interface UseFileMentionReturn {
  isOpen: boolean
  entries: MentionEntry[]
  selectedIndex: number
  mode: MentionMode
  /** Absolute path of the directory currently being listed (list mode). */
  dirPath: string | null
  loading: boolean
  error: string | null
  /** Server reported it capped the list/search; surface a hint in the UI. */
  truncated: boolean
  /** ChatInput's onKeyDown should delegate here; returns true if consumed. */
  handleKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => boolean
  applyEntry: (entry: MentionEntry) => void
  /** Remove a mention by its raw `@...` substring (chip X-button click). */
  removeMention: (raw: string) => void
  /** Re-evaluate the caret context after `onSelect` / `onClick` / paste. */
  recheckTrigger: () => void
  setSelectedIndex: (i: number) => void
}

export function useFileMention(
  input: string,
  setInput: (next: string) => void,
  textareaRef: React.RefObject<HTMLTextAreaElement | null>,
  workingDir: string | null,
): UseFileMentionReturn {
  const [mode, setMode] = useState<MentionMode>("list")
  const [entries, setEntries] = useState<MentionEntry[]>([])
  const [dirPath, setDirPath] = useState<string | null>(null)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [truncated, setTruncated] = useState(false)

  const [active, setActive] = useState<ActiveMention | null>(null)
  const isOpen = active !== null

  const requestSeqRef = useRef(0)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inputRef = useRef(input)
  inputRef.current = input
  const workingDirRef = useRef(workingDir)
  workingDirRef.current = workingDir

  const reset = () => {
    setEntries([])
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
  }
  const resetEffectEvent = useEffectEvent(reset)

  useEffect(() => {
    resetEffectEvent()
  }, [workingDir])

  const recheckTrigger = () => {
    if (!workingDirRef.current) {
      setActive((prev) => (prev ? null : prev))
      return
    }
    const ta = textareaRef.current
    if (!ta) return
    const caret = ta.selectionStart ?? ta.value.length
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
  }

  useEffect(() => {
    recheckTrigger()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [input])

  useEffect(() => {
    if (!active || !workingDir) return
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
          // Local prefix filter avoids a round-trip per keystroke within the
          // same directory; the server already capped at 5000 entries.
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

    // Debounce search mode (one RPC per fast keystroke); list mode only
    // changes when the user types `/`, so it's already low-rate.
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

  const applyEntry = (entry: MentionEntry) => {
    if (!active) return
    // Directory: trailing `/` keeps the popper open for the next level.
    // For paths with whitespace, formatMentionInsertion quotes the value
    // (`@"with space/"`); the closing quote ends the mention so the popper
    // naturally closes on those — accepted v1 limitation.
    const insertion = entry.isDir
      ? formatMentionInsertion(entry.relPath + "/")
      : formatMentionInsertion(entry.relPath) + " "
    const before = inputRef.current.slice(0, active.anchor)
    const after = inputRef.current.slice(active.caret)
    const next = before + insertion + after
    const newCaret = (before + insertion).length
    setInput(next)
    requestAnimationFrame(() => {
      const t = textareaRef.current
      if (t) {
        t.focus()
        t.setSelectionRange(newCaret, newCaret)
      }
    })
    if (!entry.isDir) {
      reset()
    }
  }

  const removeMention = (raw: string) => {
    const current = inputRef.current
    const idx = current.indexOf(raw)
    if (idx < 0) return
    const tail = current[idx + raw.length] === " " ? 1 : 0
    const next = current.slice(0, idx) + current.slice(idx + raw.length + tail)
    setInput(next)
    requestAnimationFrame(() => {
      const t = textareaRef.current
      if (t) {
        t.focus()
        t.setSelectionRange(idx, idx)
      }
    })
  }

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!isOpen || entries.length === 0) {
      if (isOpen && e.key === "Escape") {
        e.preventDefault()
        reset()
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
      case "Tab":
        e.preventDefault()
        applyEntry(entries[selectedIndex])
        return true
      case "Escape":
        e.preventDefault()
        reset()
        return true
      default:
        return false
    }
  }

  return {
    isOpen,
    entries,
    selectedIndex,
    mode,
    dirPath,
    loading,
    error,
    truncated,
    handleKeyDown,
    applyEntry,
    removeMention,
    recheckTrigger,
    setSelectedIndex,
  }
}
