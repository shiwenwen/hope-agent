interface ProgressProps {
  value?: number | null
  className?: string
  indeterminate?: boolean
}

export function Progress({ value, className = "", indeterminate = false }: ProgressProps) {
  const clamped =
    value == null ? 0 : Math.max(0, Math.min(100, value))
  return (
    <div
      className={`relative h-1.5 w-full overflow-hidden rounded-full bg-secondary ${className}`}
      role="progressbar"
      aria-valuenow={indeterminate ? undefined : clamped}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      {indeterminate ? (
        <div className="absolute inset-y-0 left-0 w-1/3 bg-primary/70 animate-[indeterminate_1.4s_ease-in-out_infinite]" />
      ) : (
        <div
          className="h-full bg-primary transition-[width] duration-200"
          style={{ width: `${clamped}%` }}
        />
      )}
    </div>
  )
}
