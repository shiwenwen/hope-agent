/**
 * Pure context-usage color helpers — the single source of truth for the
 * green → yellow → red ramp shared by the status popover, the workspace session
 * card, and the input-dock bottom bar so all three agree at the same fullness.
 *
 * Kept dependency-free (no transport / i18n / executionStatus imports) so leaf
 * consumers like the input dock can import it without pulling chatUtils' heavier
 * runtime chain.
 */

export type ContextUsageLevel = "low" | "mid" | "high"

/** Shared thresholds for the green → yellow → red context-usage color ramp. */
export function contextUsageLevel(pct: number): ContextUsageLevel {
  return pct < 50 ? "low" : pct < 80 ? "mid" : "high"
}

/** Tailwind fill class for the context-usage bar at the given fullness. */
export function contextUsageBarClass(pct: number): string {
  const level = contextUsageLevel(pct)
  return level === "low"
    ? "bg-green-500/70"
    : level === "mid"
      ? "bg-yellow-500/70"
      : "bg-red-500/70"
}
