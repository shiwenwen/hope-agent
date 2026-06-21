import { useState, type ComponentProps } from "react"
import { Input } from "@/components/ui/input"

type InputProps = ComponentProps<typeof Input>

interface DeferredNumberInputProps
  extends Omit<InputProps, "type" | "value" | "onChange"> {
  value: number | null | undefined
  min?: number
  max?: number
  integer?: boolean
  onValueCommit: (value: number) => void
  onEmptyCommit?: () => void
}

export function DeferredNumberInput({
  value,
  min,
  max,
  integer = true,
  onValueCommit,
  onEmptyCommit,
  onFocus,
  onBlur,
  onKeyDown,
  ...props
}: DeferredNumberInputProps) {
  const formatValue = (nextValue: number | null | undefined) =>
    nextValue == null ? "" : String(nextValue)
  const [draft, setDraft] = useState("")
  const [editing, setEditing] = useState(false)
  const displayValue = editing ? draft : formatValue(value)

  const commitDraft = () => {
    const raw = draft.trim()
    const parsed = Number(raw)

    setEditing(false)
    if (raw === "") {
      if (!onEmptyCommit) {
        setDraft(formatValue(value))
        return
      }
      setDraft("")
      if (value != null) {
        onEmptyCommit()
      }
      return
    }

    if (!Number.isFinite(parsed)) {
      setDraft(formatValue(value))
      return
    }

    const rounded = integer ? Math.round(parsed) : parsed
    const lowerBounded = min == null ? rounded : Math.max(min, rounded)
    const next = max == null ? lowerBounded : Math.min(max, lowerBounded)
    setDraft(String(next))
    if (next !== value) {
      onValueCommit(next)
    }
  }

  return (
    <Input
      {...props}
      type="number"
      min={min}
      max={max}
      value={displayValue}
      onFocus={(event) => {
        setDraft(formatValue(value))
        setEditing(true)
        onFocus?.(event)
      }}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={(event) => {
        commitDraft()
        onBlur?.(event)
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.currentTarget.blur()
        }
        onKeyDown?.(event)
      }}
    />
  )
}
