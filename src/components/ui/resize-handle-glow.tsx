import { cn } from "@/lib/utils"

interface ResizeHandleGlowProps {
  active?: boolean
  className?: string
}

/** Invisible resize affordance that shows a 1px blue glow on hover, focus, or drag. */
export function ResizeHandleGlow({ active = false, className }: ResizeHandleGlowProps) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute rounded-full bg-transparent opacity-0 shadow-none transition-[background-color,box-shadow,opacity] duration-200",
        "group-hover:bg-[#339cff]/80 group-hover:opacity-100 group-hover:shadow-[0_0_8px_rgb(51_156_255/0.5)] group-focus-visible:bg-[#339cff]/80 group-focus-visible:opacity-100 group-focus-visible:shadow-[0_0_8px_rgb(51_156_255/0.5)]",
        active &&
          "bg-[#339cff]/90 opacity-100 shadow-[0_0_8px_rgb(51_156_255/0.6)] group-hover:bg-[#339cff]/90 group-hover:shadow-[0_0_8px_rgb(51_156_255/0.6)]",
        className,
      )}
    />
  )
}
