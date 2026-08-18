import { cn } from "@/lib/utils"

interface ResizeHandleGlowProps {
  active?: boolean
  className?: string
}

/**
 * Invisible resize affordance that shows a 1px accent glow on hover, focus, or
 * drag — a registered exception to "hover only deepens the background" (see
 * `ui-interaction-system.md` → 登记的例外). It draws the drag feedback itself
 * and never replaces the 1px structural border underneath it. The colour comes
 * from the shared accent token, not a literal.
 */
export function ResizeHandleGlow({ active = false, className }: ResizeHandleGlowProps) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute rounded-full bg-transparent opacity-0 shadow-none transition-[background-color,box-shadow,opacity] duration-200",
        "group-hover:bg-(--ha-markdown-link)/80 group-hover:opacity-100 group-hover:shadow-[0_0_8px_var(--ha-resize-glow)]",
        "group-focus-visible:bg-(--ha-markdown-link)/80 group-focus-visible:opacity-100 group-focus-visible:shadow-[0_0_8px_var(--ha-resize-glow)]",
        active &&
          "bg-(--ha-markdown-link)/90 opacity-100 shadow-[0_0_8px_var(--ha-resize-glow)] group-hover:bg-(--ha-markdown-link)/90",
        className,
      )}
    />
  )
}
