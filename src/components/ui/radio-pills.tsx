import { cn } from "@/lib/utils"

interface RadioPillsProps<V extends string> {
  value: V
  options: ReadonlyArray<{ value: V; label: string }>
  onChange: (next: V) => void
  /** Tailwind grid columns class. Default `grid-cols-3`. */
  cols?: string
  className?: string
}

/**
 * Inline pill-style radio button group used by settings panels (Smart mode
 * strategy / fallback selectors, approval-timeout action). One active pill,
 * keyboard accessible via the underlying `<button>` elements.
 */
export function RadioPills<V extends string>({
  value,
  options,
  onChange,
  cols = "grid-cols-3",
  className,
}: RadioPillsProps<V>) {
  return (
    <div className={cn("grid gap-1.5", cols, className)} role="radiogroup">
      {options.map((opt) => {
        const isActive = value === opt.value
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={isActive}
            onClick={() => onChange(opt.value)}
            className={cn(
              "text-xs rounded-md px-2 py-1.5 border transition-colors",
              isActive
                ? "bg-primary/10 border-primary/40 text-primary"
                : "bg-secondary/40 border-border/40 text-muted-foreground hover:border-border",
            )}
          >
            {opt.label}
          </button>
        )
      })}
    </div>
  )
}
