import {
  useEffect,
  type AriaRole,
  type CSSProperties,
  type MouseEventHandler,
  type ReactNode,
  type Ref,
} from "react"
import { createPortal } from "react-dom"

import { AnimatedPresenceBox } from "@/components/ui/animated-presence"
import { UI_EASING, UI_MOTION } from "@/components/ui/motion"
import { cn } from "@/lib/utils"

interface FloatingMenuProps {
  open: boolean
  children: ReactNode
  className?: string
  positionClassName?: string
  originClassName?: string
  enterFromClassName?: string
  enterClassName?: string
  exitClassName?: string
  durationMs?: number
  enterDurationMs?: number
  exitDurationMs?: number
  onEscapeKeyDown?: () => void
  onClick?: MouseEventHandler<HTMLDivElement>
  role?: AriaRole
  /** Fixed positioning is used for pointer/selection anchored surfaces. */
  strategy?: "absolute" | "fixed"
  /** Dynamic coordinates such as `{ top, left }`. */
  style?: CSSProperties
  /** Fixed-coordinate surfaces should portal to avoid transformed ancestors. */
  portal?: boolean
  elementRef?: Ref<HTMLDivElement>
}

export const FLOATING_MENU_SURFACE_CLASS =
  "rounded-floating border border-border-soft bg-surface-floating/95 text-popover-foreground shadow-floating backdrop-blur-xl"

export const FLOATING_MENU_RADIX_MOTION_CLASS = "ha-radix-menu-motion"

export const FLOATING_MENU_ITEM_CLASS =
  "ha-focus-item flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-colors duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground"

export function FloatingMenu({
  open,
  children,
  className,
  positionClassName = "bottom-full left-0 mb-2",
  originClassName = "origin-bottom-left",
  enterFromClassName = "opacity-0 pointer-events-none",
  enterClassName = "ha-menu-popover-enter",
  exitClassName = "ha-menu-popover-exit pointer-events-none",
  durationMs,
  enterDurationMs = UI_MOTION.popoverEnter,
  exitDurationMs = UI_MOTION.popoverExit,
  onEscapeKeyDown,
  onClick,
  role,
  strategy = "absolute",
  style,
  portal = false,
  elementRef,
}: FloatingMenuProps) {
  useEffect(() => {
    if (!open || !onEscapeKeyDown) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return
      event.stopPropagation()
      onEscapeKeyDown()
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [open, onEscapeKeyDown])

  const content = (
    <AnimatedPresenceBox
      open={open}
      durationMs={durationMs}
      enterDurationMs={durationMs ?? enterDurationMs}
      exitDurationMs={durationMs ?? exitDurationMs}
      enterEasing={UI_EASING.menuEnter}
      exitEasing={UI_EASING.menuExit}
      className={cn(
        strategy === "fixed" ? "fixed" : "absolute",
        "z-50 transform-gpu will-change-[opacity,transform]",
        FLOATING_MENU_SURFACE_CLASS,
        originClassName,
        positionClassName,
        className,
      )}
      enterFromClassName={enterFromClassName}
      enterClassName={enterClassName}
      exitClassName={exitClassName}
      onClick={onClick}
      role={role}
      style={style}
      elementRef={elementRef}
    >
      {children}
    </AnimatedPresenceBox>
  )

  return portal && typeof document !== "undefined" ? createPortal(content, document.body) : content
}
