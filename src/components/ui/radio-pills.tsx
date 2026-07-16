import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

type RadioPillsVariant = "subtle" | "strong"

interface RadioPillsProps<V extends string | number> {
  value: V | null
  options: ReadonlyArray<{ value: V; label: ReactNode; icon?: ReactNode }>
  onChange: (next: V) => void
  /** Tailwind grid columns class. Default `grid-cols-3`. */
  cols?: string
  /** Grid for fixed-width settings choices; wrap for tag-like choices. */
  layout?: "grid" | "wrap"
  /** Strong is reserved for mutually exclusive category tags. */
  variant?: RadioPillsVariant
  className?: string
  itemClassName?: string
  ariaLabel?: string
}

/**
 * Inline pill-style radio button group used by settings panels (Smart mode
 * strategy / fallback selectors, approval-timeout action). One active pill,
 * keyboard accessible via the underlying `<button>` elements.
 */
export function RadioPills<V extends string | number>({
  value,
  options,
  onChange,
  cols = "grid-cols-3",
  layout = "grid",
  variant = "subtle",
  className,
  itemClassName,
  ariaLabel,
}: RadioPillsProps<V>) {
  return (
    <div
      className={cn(
        layout === "grid" ? ["grid gap-1.5", cols] : "flex flex-wrap gap-1.5",
        className,
      )}
      role="radiogroup"
      aria-label={ariaLabel}
    >
      {options.map((opt) => {
        const isActive = value === opt.value
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={isActive}
            onClick={() => {
              if (!isActive) onChange(opt.value)
            }}
            className={cn(
              "inline-flex items-center justify-center gap-1.5 px-2 py-1.5 text-xs transition-colors",
              variant === "strong" ? "rounded-full" : "rounded-md",
              variant === "strong"
                ? isActive
                  ? "bg-primary font-medium text-primary-foreground"
                  : "text-muted-foreground hover:bg-secondary/40 hover:text-foreground"
                : isActive
                  ? "bg-secondary/70 text-foreground"
                  : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60 hover:text-foreground",
              itemClassName,
            )}
          >
            {opt.icon}
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
