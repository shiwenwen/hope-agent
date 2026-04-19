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
  // Previous scrollTop, used to detect scroll direction in handleScroll.
  const lastScrollTopRef = useRef(0)

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

  // Detect user scrolling to pause auto-scroll.
  //
  // Two detection paths:
  // 1. User-initiated input (wheel / touchmove / PageUp / ArrowUp / Home) —
  //    flips immediately on any upward intent.
  // 2. Plain scroll events — RESUME auto-scroll only when the user is actively
  //    scrolling DOWN (scrollTop increased) and is within 80px of the bottom.
  //    Requiring a downward delta is critical: a tiny upward wheel event leaves
  //    scrollTop within 80px of bottom, and resuming on distance alone would
  //    immediately flip the ref back to false, letting the rAF loop snap back
  //    to bottom — observed as jitter while the user tries to scroll up.
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    lastScrollTopRef.current = el.scrollTop

    const pauseAutoScroll = () => {
      isUserScrolledUpRef.current = true
    }

    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) pauseAutoScroll()
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "PageUp" || e.key === "ArrowUp" || e.key === "Home") {
        pauseAutoScroll()
      }
    }

    const handleScroll = () => {
      const currentTop = el.scrollTop
      const prevTop = lastScrollTopRef.current
      lastScrollTopRef.current = currentTop
      if (!isUserScrolledUpRef.current) return
      if (currentTop <= prevTop) return
      const distanceFromBottom = el.scrollHeight - currentTop - el.clientHeight
      if (distanceFromBottom <= 80) {
        isUserScrolledUpRef.current = false
      }
    }

    el.addEventListener("wheel", handleWheel, { passive: true })
    el.addEventListener("touchmove", pauseAutoScroll, { passive: true })
    el.addEventListener("keydown", handleKeyDown)
    el.addEventListener("scroll", handleScroll, { passive: true })
    return () => {
      el.removeEventListener("wheel", handleWheel)
      el.removeEventListener("touchmove", pauseAutoScroll)
      el.removeEventListener("keydown", handleKeyDown)
      el.removeEventListener("scroll", handleScroll)
    }
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
  }, [messages.length, messages, scrollToBottom])

  return { scrollContainerRef, bottomRef }
}
