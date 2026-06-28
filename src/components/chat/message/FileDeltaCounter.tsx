import { useMemo } from "react"

import { cn } from "@/lib/utils"

const DIGITS = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]

function cleanCount(value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.max(0, Math.trunc(value))
}

function RollingInteger({ value, className }: { value: number; className?: string }) {
  const text = cleanCount(value).toString()

  return (
    <span className={cn("tool-delta-number", className)} aria-hidden="true">
      {Array.from(text).map((char, idx) => {
        const digit = Number(char)
        return (
          <span
            // Keep columns anchored from the right so 9 -> 10 adds a new leading
            // slot instead of remounting every existing digit.
            key={`${text.length - idx}`}
            className="tool-delta-digit"
          >
            <span
              className="tool-delta-digit-strip"
              style={{ transform: `translate3d(0, -${digit}em, 0)` }}
            >
              {DIGITS.map((d) => (
                <span key={d}>{d}</span>
              ))}
            </span>
          </span>
        )
      })}
    </span>
  )
}

function DeltaStat({
  sign,
  value,
  tone,
}: {
  sign: "+" | "-"
  value: number
  tone: "added" | "removed"
}) {
  return (
    <span
      className={cn(
        "tool-delta-stat",
        tone === "added"
          ? "text-emerald-600 dark:text-emerald-400"
          : "text-rose-600 dark:text-rose-400",
      )}
    >
      <span className="tool-delta-sign" aria-hidden="true">
        {sign}
      </span>
      <RollingInteger value={value} />
    </span>
  )
}

export function FileDeltaCounter({
  linesAdded,
  linesRemoved,
  estimated = false,
  className,
}: {
  linesAdded: number
  linesRemoved: number
  estimated?: boolean
  className?: string
}) {
  const added = cleanCount(linesAdded)
  const removed = cleanCount(linesRemoved)
  const label = useMemo(
    () => `${estimated ? "estimated " : ""}+${added} -${removed}`,
    [added, estimated, removed],
  )

  return (
    <span
      className={cn(
        "tool-delta-counter inline-flex shrink-0 items-center gap-1.5 tabular-nums",
        estimated && "opacity-85",
        className,
      )}
      aria-label={label}
      title={estimated ? label : undefined}
      data-estimated={estimated ? "true" : undefined}
    >
      {estimated && (
        <span className="tool-delta-estimate-mark text-muted-foreground/50" aria-hidden="true">
          ≈
        </span>
      )}
      <DeltaStat sign="+" value={added} tone="added" />
      <DeltaStat sign="-" value={removed} tone="removed" />
    </span>
  )
}
