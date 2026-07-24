import type { ReactNode } from "react"

import { cn } from "@/lib/utils"
import { selectionPillStateClass } from "./selection-pill-styles"

interface TogglePillOption<V extends string | number> {
  value: V
  label: ReactNode
  icon?: ReactNode
  disabled?: boolean
}

interface TogglePillsProps<V extends string | number> {
  values: ReadonlySet<V>
  options: ReadonlyArray<TogglePillOption<V>>
  onToggle: (value: V) => void
  ariaLabel: string
  className?: string
  itemClassName?: string
}

/**
 * Flat multi-select pills. Selected items use the same high-contrast reverse
 * surface as strong radio pills; borders, shadows and extra check marks never
 * carry selection state.
 */
export function TogglePills<V extends string | number>({
  values,
  options,
  onToggle,
  ariaLabel,
  className,
  itemClassName,
}: TogglePillsProps<V>) {
  return (
    <div className={cn("flex flex-wrap gap-1.5", className)} role="group" aria-label={ariaLabel}>
      {options.map((option) => {
        const pressed = values.has(option.value)
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={pressed}
            disabled={option.disabled}
            onClick={() => {
              if (!option.disabled) onToggle(option.value)
            }}
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-normal transition-colors",
              selectionPillStateClass(pressed, !option.disabled),
              option.disabled && "cursor-not-allowed opacity-45",
              itemClassName,
            )}
          >
            {option.icon}
            {option.label}
          </button>
        )
      })}
    </div>
  )
}
