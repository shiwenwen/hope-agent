import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { ComposerInputHandle } from "@/components/chat/input/composerInputHandle"
import type { QuickPromptItem } from "@/types/quickPrompts"
import { detectActiveQuickPrompt } from "./quickPromptTokens"

interface ActiveQuickPrompt {
  anchor: number
  caret: number
  token: string
}

export interface UseQuickPromptsReturn {
  isOpen: boolean
  entries: QuickPromptItem[]
  selectedIndex: number
  query: string
  handleKeyDown: (e: React.KeyboardEvent<HTMLElement>) => boolean
  applyEntry: (entry: QuickPromptItem) => void
  recheckTrigger: () => void
  reset: () => void
  setSelectedIndex: (index: number) => void
}

function matchesPrompt(prompt: QuickPromptItem, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return (
    prompt.title.toLowerCase().includes(q) ||
    prompt.content.toLowerCase().includes(q)
  )
}

export function useQuickPrompts(
  input: string,
  setInput: (next: string) => void,
  inputHandleRef: React.RefObject<ComposerInputHandle | null>,
  quickPrompts: QuickPromptItem[],
): UseQuickPromptsReturn {
  const [active, setActive] = useState<ActiveQuickPrompt | null>(null)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const inputRef = useRef(input)
  inputRef.current = input
  const reset = useCallback(() => {
    setActive(null)
    setSelectedIndex(0)
  }, [])

  const recheckTrigger = useCallback(() => {
    const inputHandle = inputHandleRef.current
    if (!inputHandle) return
    const selection = inputHandle.getSelectionRange()
    if (selection.start !== selection.end) {
      setActive((prev) => (prev ? null : prev))
      return
    }
    const result = detectActiveQuickPrompt(inputRef.current, selection.start)
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
  }, [input, quickPrompts])

  const entries = useMemo(() => {
    if (!active) return []
    return quickPrompts.filter((prompt) => matchesPrompt(prompt, active.token)).slice(0, 50)
  }, [active, quickPrompts])

  useEffect(() => {
    setSelectedIndex(0)
  }, [active?.token])

  useEffect(() => {
    setSelectedIndex((index) =>
      entries.length === 0 ? 0 : Math.min(index, entries.length - 1),
    )
  }, [entries.length])

  const applyEntry = useCallback(
    (entry: QuickPromptItem) => {
      if (!active) return
      const before = inputRef.current.slice(0, active.anchor)
      const after = inputRef.current.slice(active.caret)
      const insert = entry.content
      const next = before + insert + after
      const newCaret = (before + insert).length
      setInput(next)
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        if (!inputHandle) return
        inputHandle.focus()
        inputHandle.setSelectionRange(newCaret, newCaret)
      })
      reset()
    },
    [active, inputHandleRef, reset, setInput],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLElement>): boolean => {
      if (!active) return false
      if (entries.length === 0) {
        if (
          e.key === "Escape" ||
          e.key === "Enter" ||
          e.key === "Tab" ||
          e.key === "ArrowDown" ||
          e.key === "ArrowUp"
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
          setSelectedIndex((index) => (index + 1) % entries.length)
          return true
        case "ArrowUp":
          e.preventDefault()
          setSelectedIndex((index) => (index - 1 + entries.length) % entries.length)
          return true
        case "Enter":
        case "Tab":
          e.preventDefault()
          {
            const entry = entries[selectedIndex]
            if (entry) applyEntry(entry)
          }
          return true
        case "Escape":
          e.preventDefault()
          reset()
          return true
        default:
          return false
      }
    },
    [active, applyEntry, entries, reset, selectedIndex],
  )

  return {
    isOpen: active !== null,
    entries,
    selectedIndex,
    query: active?.token ?? "",
    handleKeyDown,
    applyEntry,
    recheckTrigger,
    reset,
    setSelectedIndex,
  }
}
