export interface ParsedPlanStep {
  index: number
  phase: string
  title: string
  status: "pending" | "in_progress" | "completed" | "skipped" | "failed"
  durationMs?: number
}

/**
 * Detect if markdown content contains a Plan checklist format.
 * Expected format:
 *   ### Phase N: title
 *   - [ ] Step description
 *   - [x] Completed step
 */
export function detectPlanContent(content: string): {
  isPlan: boolean
  steps: ParsedPlanStep[]
  title?: string
} {
  const lines = content.split("\n")
  const steps: ParsedPlanStep[] = []
  let currentPhase = ""
  let index = 0
  let title: string | undefined
  let hasPhase = false
  let hasChecklist = false

  for (const line of lines) {
    const trimmed = line.trim()

    // Match phase headers: "### Phase N: title" or "### Phase N：title"
    const phaseMatch = trimmed.match(/^###\s+(.+)/)
    if (phaseMatch) {
      currentPhase = phaseMatch[1]
      hasPhase = true
      if (!title) {
        // Use the overall heading before phases if present, otherwise first phase
        title = currentPhase
      }
      continue
    }

    // Match checklist items: "- [ ] text" or "- [x] text"
    const checkMatch = trimmed.match(/^-\s+\[([ xX])\]\s+(.+)/)
    if (checkMatch) {
      hasChecklist = true
      const checked = checkMatch[1].toLowerCase() === "x"
      steps.push({
        index,
        phase: currentPhase,
        title: checkMatch[2],
        status: checked ? "completed" : "pending",
      })
      index++
    }
  }

  return {
    isPlan: hasPhase && hasChecklist && steps.length >= 2,
    steps,
    title,
  }
}

/**
 * Group steps by phase name.
 */
export function groupStepsByPhase(
  steps: ParsedPlanStep[]
): { name: string; steps: ParsedPlanStep[] }[] {
  const groups: { name: string; steps: ParsedPlanStep[] }[] = []
  let currentGroup: { name: string; steps: ParsedPlanStep[] } | null = null

  for (const step of steps) {
    if (!currentGroup || currentGroup.name !== step.phase) {
      currentGroup = { name: step.phase || "Steps", steps: [] }
      groups.push(currentGroup)
    }
    currentGroup.steps.push(step)
  }

  return groups
}

/**
 * Format duration in milliseconds to human-readable string.
 */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = Math.round(ms / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const remainSec = seconds % 60
  return remainSec > 0 ? `${minutes}m${remainSec}s` : `${minutes}m`
}
