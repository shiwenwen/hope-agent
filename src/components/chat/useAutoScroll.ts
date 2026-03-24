import { useRef, useEffect, useLayoutEffect } from "react"
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
  const prevScrollHeightRef = useRef(0)

  // Scroll to bottom when switching to a session (after React renders the new messages)
  useLayoutEffect(() => {
    if (!currentSessionId) return
    const el = scrollContainerRef.current
    if (el) {
      el.scrollTop = el.scrollHeight
    }
  }, [currentSessionId])

  // Detect user scrolling up to pause auto-scroll
  useEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    const handleScroll = () => {
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
      isUserScrolledUpRef.current = distanceFromBottom > 150
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
      prevScrollHeightRef.current = el.scrollHeight

      const tick = () => {
        if (!isUserScrolledUpRef.current) {
          // Lerp toward bottom for silky-smooth scrolling instead of snapping
          const target = el.scrollHeight - el.clientHeight
          const diff = target - el.scrollTop
          if (diff > 1) {
            // Interpolate: cover 25% of remaining distance per frame (~60fps → ~4 frames to settle)
            el.scrollTop += diff * 0.25
          } else if (diff > 0) {
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
      // Streaming ended — do a final smooth scroll to bottom
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [loading])

  // When user sends a new message, immediately scroll to bottom
  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) return
    // Only trigger on user messages being added
    const lastMsg = messages[messages.length - 1]
    if (lastMsg?.role === "user") {
      isUserScrolledUpRef.current = false
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
    }
  }, [messages.length]) // eslint-disable-line react-hooks/exhaustive-deps

  return { scrollContainerRef, bottomRef }
}
