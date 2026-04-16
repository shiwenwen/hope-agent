import { useRef, useEffect, useLayoutEffect, useCallback } from "react"
import type { Message } from "@/types/chat"

interface UseAutoScrollOptions {
  loading: boolean
  messages: Message[]
  currentSessionId: string | null
}

export function useAutoScroll({ loading, messages, currentSessionId }: UseAutoScrollOptions) {
  const scrollContainerRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const isUserScrolledUpRef = useRef(false)
  const rafIdRef = useRef<number | null>(null)
  // Track the last message count we handled to avoid duplicate scroll-to-bottom
  const lastHandledLengthRef = useRef(0)

  const scrollToBottom = useCallback((immediate?: boolean) => {
    const el = scrollContainerRef.current
    if (!el) return
    if (immediate) {
      el.scrollTop = el.scrollHeight
    } else {
      // Use rAF to ensure DOM has updated
      requestAnimationFrame(() => {
        el.scrollTop = el.scrollHeight
      })
    }
  }, [])

  // Scroll to bottom when switching to a session (after React renders the new messages)
  useLayoutEffect(() => {
    if (!currentSessionId) return
    scrollToBottom(true)
  }, [currentSessionId, scrollToBottom])

  // Detect user scrolling up to pause auto-scroll.
  // Use hysteresis: a larger threshold to ENTER scrolled-up state,
  // and a smaller one to LEAVE it (re-enable auto-scroll).
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    const handleScroll = () => {
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
      if (isUserScrolledUpRef.current) {
        // Already in scrolled-up state: only re-enable auto-scroll
        // when user scrolls very close to bottom
        if (distanceFromBottom <= 80) {
          isUserScrolledUpRef.current = false
        }
      } else {
        // Not scrolled up: only enter scrolled-up state on a decisive scroll
        if (distanceFromBottom > 300) {
          isUserScrolledUpRef.current = true
        }
      }
    }
    el.addEventListener("scroll", handleScroll, { passive: true })
    return () => el.removeEventListener("scroll", handleScroll)
  }, [])

  // rAF loop: smoothly follow content growth during streaming
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return

    if (loading) {
      // Reset scroll-up detection when new message starts
      isUserScrolledUpRef.current = false

      const tick = () => {
        if (!isUserScrolledUpRef.current) {
          const target = el.scrollHeight - el.clientHeight
          const diff = target - el.scrollTop
          if (diff > 1) {
            // Snap directly to bottom — eliminates lerp-induced jitter
            // when fighting with user scroll events
            el.scrollTop = target
          }
        }
        rafIdRef.current = requestAnimationFrame(tick)
      }
      rafIdRef.current = requestAnimationFrame(tick)

      return () => {
        if (rafIdRef.current !== null) {
          cancelAnimationFrame(rafIdRef.current)
          rafIdRef.current = null
        }
      }
    } else {
      // Streaming ended — snap to bottom if not scrolled up
      if (!isUserScrolledUpRef.current) {
        scrollToBottom()
      }
    }
  }, [loading, scrollToBottom])

  // When user sends a new message, immediately scroll to bottom
  useLayoutEffect(() => {
    const len = messages.length
    if (len === 0 || len === lastHandledLengthRef.current) return
    lastHandledLengthRef.current = len

    const lastMsg = messages[len - 1]
    // Scroll for user messages, and also for the first assistant message
    // that appears right after (the empty placeholder)
    if (lastMsg?.role === "user" || (len >= 2 && messages[len - 2]?.role === "user" && lastMsg?.role === "assistant")) {
      isUserScrolledUpRef.current = false
      scrollToBottom()
    }
  }, [messages.length, messages, scrollToBottom]) // eslint-disable-line react-hooks/exhaustive-deps

  return { scrollContainerRef, bottomRef }
}
