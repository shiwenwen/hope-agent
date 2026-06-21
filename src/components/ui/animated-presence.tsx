import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type AriaRole,
  type CSSProperties,
  type MouseEventHandler,
  type ReactNode,
} from "react"

import { cn } from "@/lib/utils"
import { UI_EASING, UI_MOTION } from "@/components/ui/motion"

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
  durationMs = UI_MOTION.collapse,
  unmountOnExit = true,
}: AnimatedCollapseProps) {
  const [present, setPresent] = useState(open || !unmountOnExit)
  const [visible, setVisible] = useState(open)
  const [height, setHeight] = useState<number | "auto">(open ? "auto" : 0)
  const [renderedChildren, setRenderedChildren] = useState(children)
  const contentRef = useRef<HTMLDivElement>(null)
  const timerRef = useRef<number | null>(null)
  const frameRef = useRef<number | null>(null)
  const childrenTimerRef = useRef<number | null>(null)
  const previousOpenRef = useRef(open)

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

  useLayoutEffect(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }

    const wasOpen = previousOpenRef.current
    previousOpenRef.current = open

    if (open) {
      frameRef.current = window.requestAnimationFrame(() => {
        setPresent(true)
        const measured = contentRef.current?.scrollHeight ?? 0
        setHeight(measured)
        frameRef.current = window.requestAnimationFrame(() => {
          setVisible(true)
          timerRef.current = window.setTimeout(() => {
            setHeight("auto")
            timerRef.current = null
          }, durationMs)
          frameRef.current = null
        })
      })
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
    }

    if (!wasOpen) {
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }

    frameRef.current = window.requestAnimationFrame(() => {
      setHeight(contentRef.current?.scrollHeight ?? 0)
      setVisible(true)
      frameRef.current = window.requestAnimationFrame(() => {
        setVisible(false)
        setHeight(0)
        frameRef.current = null
      })
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

  useEffect(() => {
    if (
      !open ||
      height === "auto" ||
      !contentRef.current ||
      typeof ResizeObserver === "undefined"
    ) {
      return
    }
    const observer = new ResizeObserver(() => {
      setHeight(contentRef.current?.scrollHeight ?? 0)
    })
    observer.observe(contentRef.current)
    return () => observer.disconnect()
  }, [height, open])

  if (!present && !open && unmountOnExit) return null

  const allowOverflow = overflow === "visible-when-open" && visible
  const timingFunction = visible ? UI_EASING.emphasized : UI_EASING.accelerate

  return (
    <div
      className={cn(
        "transition-[height,opacity] motion-reduce:transition-none",
        allowOverflow ? "overflow-visible" : "overflow-hidden",
        visible ? "opacity-100" : "opacity-0 pointer-events-none",
        className,
      )}
      style={{
        height: height === "auto" ? "auto" : `${height}px`,
        transitionDuration: `${durationMs}ms`,
        transitionTimingFunction: timingFunction,
        willChange: "height, opacity",
      }}
      aria-hidden={open ? undefined : true}
      inert={open ? undefined : true}
    >
      <div
        ref={contentRef}
        className={cn(
          "min-h-0",
          allowOverflow ? "overflow-visible" : "overflow-hidden",
          innerClassName,
        )}
      >
        {open || !unmountOnExit ? children : renderedChildren}
      </div>
    </div>
  )
}

interface AnimatedPresenceBoxProps {
  open: boolean
  children: ReactNode
  className?: string
  enterFromClassName?: string
  enterClassName?: string
  exitClassName?: string
  durationMs?: number
  enterDurationMs?: number
  exitDurationMs?: number
  easing?: string
  enterEasing?: string
  exitEasing?: string
  unmountOnExit?: boolean
  onClick?: MouseEventHandler<HTMLDivElement>
  role?: AriaRole
}

export function AnimatedPresenceBox({
  open,
  children,
  className,
  enterFromClassName,
  enterClassName = "translate-y-0 scale-100 opacity-100",
  exitClassName = "translate-y-1 scale-[0.98] opacity-0 pointer-events-none",
  durationMs = UI_MOTION.popover,
  enterDurationMs,
  exitDurationMs,
  easing,
  enterEasing,
  exitEasing,
  unmountOnExit = true,
  onClick,
  role,
}: AnimatedPresenceBoxProps) {
  const resolvedEnterDurationMs = enterDurationMs ?? durationMs
  const resolvedExitDurationMs = exitDurationMs ?? durationMs
  const resolvedEnterEasing = enterEasing ?? easing ?? UI_EASING.emphasized
  const resolvedExitEasing = exitEasing ?? easing ?? UI_EASING.accelerate
  const [present, setPresent] = useState(open || !unmountOnExit)
  const [visible, setVisible] = useState(open)
  const [renderedChildren, setRenderedChildren] = useState(children)
  const timerRef = useRef<number | null>(null)
  const frameRef = useRef<number | null>(null)
  const childrenTimerRef = useRef<number | null>(null)
  const presentRef = useRef(present)

  useEffect(() => {
    presentRef.current = present
  }, [present])

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

  useLayoutEffect(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }

    if (open) {
      const wasPresent = presentRef.current

      const show = () => {
        setVisible(true)
        frameRef.current = null
      }

      frameRef.current = window.requestAnimationFrame(() => {
        if (!wasPresent) {
          presentRef.current = true
          setPresent(true)
          setVisible(false)
        }
        if (wasPresent) {
          show()
          return
        }
        frameRef.current = window.requestAnimationFrame(() => {
          show()
        })
      })
      return () => {
        if (frameRef.current !== null) {
          window.cancelAnimationFrame(frameRef.current)
          frameRef.current = null
        }
      }
    }

    if (!presentRef.current && unmountOnExit) return
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
      presentRef.current = false
      setPresent(false)
      timerRef.current = null
    }, resolvedExitDurationMs)

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
  }, [
    open,
    resolvedEnterDurationMs,
    resolvedEnterEasing,
    resolvedExitDurationMs,
    resolvedExitEasing,
    unmountOnExit,
  ])

  if (!present && !open && unmountOnExit) return null
  const activeDurationMs = visible ? resolvedEnterDurationMs : resolvedExitDurationMs
  const activeTimingFunction = visible ? resolvedEnterEasing : resolvedExitEasing
  const hiddenClassName = open ? (enterFromClassName ?? exitClassName) : exitClassName

  return (
    <div
      className={cn(
        "transition-[opacity,transform,filter] ease-out motion-reduce:transition-none",
        visible ? enterClassName : hiddenClassName,
        className,
      )}
      style={
        {
          transitionDuration: `${activeDurationMs}ms`,
          transitionTimingFunction: activeTimingFunction,
          "--ha-presence-enter-duration": `${resolvedEnterDurationMs}ms`,
          "--ha-presence-exit-duration": `${resolvedExitDurationMs}ms`,
          "--ha-presence-enter-easing": resolvedEnterEasing,
          "--ha-presence-exit-easing": resolvedExitEasing,
        } as CSSProperties
      }
      aria-hidden={visible ? undefined : true}
      inert={visible ? undefined : true}
      onClick={onClick}
      role={role}
    >
      {open ? children : renderedChildren}
    </div>
  )
}
