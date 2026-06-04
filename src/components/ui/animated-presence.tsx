import { useEffect, useRef, useState, type ReactNode } from "react"

import { cn } from "@/lib/utils"

interface AnimatedCollapseProps {
  open: boolean
  children: ReactNode
  className?: string
  innerClassName?: string
  overflow?: "hidden" | "visible-when-open"
  durationMs?: number
  unmountOnExit?: boolean
}

export function AnimatedCollapse({
  open,
  children,
  className,
  innerClassName,
  overflow = "hidden",
  durationMs = 200,
  unmountOnExit = true,
}: AnimatedCollapseProps) {
  const [present, setPresent] = useState(open || !unmountOnExit)
  const [visible, setVisible] = useState(open)
  const [renderedChildren, setRenderedChildren] = useState(children)
  const timerRef = useRef<number | null>(null)
  const frameRef = useRef<number | null>(null)
  const childrenTimerRef = useRef<number | null>(null)

  useEffect(() => {
    if (!open) return
    if (childrenTimerRef.current !== null) {
      window.clearTimeout(childrenTimerRef.current)
    }
    childrenTimerRef.current = window.setTimeout(() => {
      setRenderedChildren(children)
      childrenTimerRef.current = null
    }, 0)
    return () => {
      if (childrenTimerRef.current !== null) {
        window.clearTimeout(childrenTimerRef.current)
        childrenTimerRef.current = null
      }
    }
  }, [children, open])

  useEffect(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }

    if (open) {
      frameRef.current = window.requestAnimationFrame(() => {
        setPresent(true)
        frameRef.current = window.requestAnimationFrame(() => {
          setVisible(true)
          frameRef.current = null
        })
      })
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }

    frameRef.current = window.requestAnimationFrame(() => {
      setVisible(false)
      frameRef.current = null
    })
    if (!unmountOnExit) {
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }
    timerRef.current = window.setTimeout(() => {
      setPresent(false)
      timerRef.current = null
    }, durationMs)

    return () => {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current)
        timerRef.current = null
      }
    }
  }, [durationMs, open, unmountOnExit])

  if (!present && !open && unmountOnExit) return null

  const allowOverflow = overflow === "visible-when-open" && visible

  return (
    <div
      className={cn(
        "grid transition-[grid-template-rows,opacity] ease-out motion-reduce:transition-none",
        allowOverflow ? "overflow-visible" : "overflow-hidden",
        visible
          ? "grid-rows-[1fr] opacity-100"
          : "grid-rows-[0fr] opacity-0 pointer-events-none",
        className,
      )}
      style={{ transitionDuration: `${durationMs}ms` }}
      aria-hidden={open ? undefined : true}
    >
      <div
        className={cn(
          "min-h-0",
          allowOverflow ? "overflow-visible" : "overflow-hidden",
          innerClassName,
        )}
      >
        {open ? children : renderedChildren}
      </div>
    </div>
  )
}

interface AnimatedPresenceBoxProps {
  open: boolean
  children: ReactNode
  className?: string
  enterClassName?: string
  exitClassName?: string
  durationMs?: number
  unmountOnExit?: boolean
}

export function AnimatedPresenceBox({
  open,
  children,
  className,
  enterClassName = "translate-y-0 scale-100 opacity-100",
  exitClassName = "translate-y-1 scale-[0.98] opacity-0 pointer-events-none",
  durationMs = 180,
  unmountOnExit = true,
}: AnimatedPresenceBoxProps) {
  const [present, setPresent] = useState(open || !unmountOnExit)
  const [visible, setVisible] = useState(open)
  const [renderedChildren, setRenderedChildren] = useState(children)
  const timerRef = useRef<number | null>(null)
  const frameRef = useRef<number | null>(null)
  const childrenTimerRef = useRef<number | null>(null)

  useEffect(() => {
    if (!open) return
    if (childrenTimerRef.current !== null) {
      window.clearTimeout(childrenTimerRef.current)
    }
    childrenTimerRef.current = window.setTimeout(() => {
      setRenderedChildren(children)
      childrenTimerRef.current = null
    }, 0)
    return () => {
      if (childrenTimerRef.current !== null) {
        window.clearTimeout(childrenTimerRef.current)
        childrenTimerRef.current = null
      }
    }
  }, [children, open])

  useEffect(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }

    if (open) {
      frameRef.current = window.requestAnimationFrame(() => {
        setPresent(true)
        frameRef.current = window.requestAnimationFrame(() => {
          setVisible(true)
          frameRef.current = null
        })
      })
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }

    frameRef.current = window.requestAnimationFrame(() => {
      setVisible(false)
      frameRef.current = null
    })
    if (!unmountOnExit) {
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }
    timerRef.current = window.setTimeout(() => {
      setPresent(false)
      timerRef.current = null
    }, durationMs)

    return () => {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current)
        timerRef.current = null
      }
    }
  }, [durationMs, open, unmountOnExit])

  if (!present && !open && unmountOnExit) return null

  return (
    <div
      className={cn(
        "transition-[opacity,transform] ease-out motion-reduce:transition-none",
        visible ? enterClassName : exitClassName,
        className,
      )}
      style={{ transitionDuration: `${durationMs}ms` }}
      aria-hidden={open ? undefined : true}
    >
      {open ? children : renderedChildren}
    </div>
  )
}
