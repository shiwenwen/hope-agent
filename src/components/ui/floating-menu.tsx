import { useEffect, type AriaRole, type MouseEventHandler, type ReactNode } from "react"

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
}

export const FLOATING_MENU_SURFACE_CLASS =
  "rounded-floating border border-border-soft bg-surface-floating/95 text-popover-foreground shadow-floating backdrop-blur-xl"

export const FLOATING_MENU_ITEM_CLASS =
  "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-colors duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground"

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

  return (
    <AnimatedPresenceBox
      open={open}
      durationMs={durationMs}
      enterDurationMs={durationMs ?? enterDurationMs}
      exitDurationMs={durationMs ?? exitDurationMs}
      enterEasing={UI_EASING.menuEnter}
      exitEasing={UI_EASING.menuExit}
      className={cn(
        "absolute z-50 transform-gpu will-change-[opacity,transform]",
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
    >
      {children}
    </AnimatedPresenceBox>
  )
}
