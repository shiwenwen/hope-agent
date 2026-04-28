export interface ParsedPlanStep {
  index: number
  phase: string
  title: string
  status: "pending" | "in_progress" | "completed" | "skipped" | "failed"
  durationMs?: number
}

type ListKind = "ordered" | "unordered"

/**
 * Detect if markdown content contains a Plan format.
 *
 * Preferred format:
 *   ## Steps
 *   1. Major step
 *      - Detail, not a progress row
 *
 * `- [ ]` checkbox items are parsed only as a legacy fallback.
 */
export function detectPlanContent(content: string): {
  isPlan: boolean
  steps: ParsedPlanStep[]
  title?: string
} {
  const title = detectTitle(content)
  const headingSteps = parseHeadingSteps(content)
  if (headingSteps.length > 0) {
    return { isPlan: true, steps: headingSteps, title: title ?? headingSteps[0]?.title }
  }

  const orderedSteps = parseListSteps(content, "ordered")
  if (orderedSteps.length > 0) {
    return { isPlan: true, steps: orderedSteps, title: title ?? orderedSteps[0]?.phase }
  }

  const unorderedSteps = parseListSteps(content, "unordered")
  if (unorderedSteps.length > 0) {
    return { isPlan: true, steps: unorderedSteps, title: title ?? unorderedSteps[0]?.phase }
  }

  const legacySteps = parseLegacyChecklistSteps(content)
  return {
    isPlan: legacySteps.length > 0,
    steps: legacySteps,
    title: title ?? legacySteps[0]?.phase,
  }
}

function detectTitle(content: string): string | undefined {
  for (const line of content.split("\n")) {
    const match = line.trim().match(/^#\s+(.+)/)
    if (match) return match[1].replace(/#+$/, "").trim()
  }
  return undefined
}

function parseHeadingSteps(content: string): ParsedPlanStep[] {
  const steps: ParsedPlanStep[] = []
  let currentSection = ""
  let inCodeFence = false

  for (const line of content.split("\n")) {
    const trimmed = line.trim()
    if (toggleCodeFence(trimmed, () => { inCodeFence = !inCodeFence }) || inCodeFence) {
      continue
    }

    const heading = parseHeading(trimmed)
    if (!heading) continue

    if (heading.level === 2) {
      if (isVerificationHeading(heading.title) && steps.length > 0) {
        steps.push(makeStep(steps.length, heading.title, heading.title))
      }
      currentSection = heading.title
      continue
    }

    if (heading.level === 3 && isExecutableHeading(heading.title, currentSection)) {
      steps.push(makeStep(steps.length, sectionName(currentSection), heading.title))
    }
  }

  return steps
}

function parseListSteps(content: string, kind: ListKind): ParsedPlanStep[] {
  const steps: ParsedPlanStep[] = []
  let currentSection = ""
  let inCodeFence = false

  for (const line of content.split("\n")) {
    const trimmed = line.trim()
    if (toggleCodeFence(trimmed, () => { inCodeFence = !inCodeFence }) || inCodeFence) {
      continue
    }

    const heading = parseHeading(trimmed)
    if (heading) {
      if (heading.level <= 2) currentSection = heading.title
      continue
    }

    if (!isStepListSection(currentSection) || leadingWhitespace(line) > 2) {
      continue
    }

    const title = kind === "ordered"
      ? stripOrderedMarker(trimmed)
      : stripUnorderedMarker(trimmed)

    if (title) {
      steps.push(makeStep(steps.length, sectionName(currentSection), title))
    }
  }

  return steps
}

function parseLegacyChecklistSteps(content: string): ParsedPlanStep[] {
  const steps: ParsedPlanStep[] = []
  let currentPhase = ""
  let inCodeFence = false

  for (const line of content.split("\n")) {
    const trimmed = line.trim()
    if (toggleCodeFence(trimmed, () => { inCodeFence = !inCodeFence }) || inCodeFence) {
      continue
    }

    const heading = parseHeading(trimmed)
    if (heading && heading.level <= 3) {
      currentPhase = heading.title
      continue
    }

    const checkMatch = trimmed.match(/^-\s+\[([ xX])\]\s+(.+)/)
    if (checkMatch) {
      const checked = checkMatch[1].toLowerCase() === "x"
      steps.push({
        index: steps.length,
        phase: currentPhase,
        title: checkMatch[2],
        status: checked ? "completed" : "pending",
      })
    }
  }

  return steps
}

function makeStep(index: number, phase: string, title: string): ParsedPlanStep {
  return {
    index,
    phase,
    title,
    status: "pending",
  }
}

function parseHeading(trimmed: string): { level: number; title: string } | null {
  const match = trimmed.match(/^(#{1,6})\s+(.+)$/)
  if (!match) return null
  const title = match[2].replace(/#+$/, "").trim()
  return title ? { level: match[1].length, title } : null
}

function toggleCodeFence(trimmed: string, toggle: () => void): boolean {
  if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) {
    toggle()
    return true
  }
  return false
}

function normalizeHeading(value: string): string {
  return value.toLowerCase()
}

function isContextHeading(title: string): boolean {
  const normalized = normalizeHeading(title)
  return normalized.includes("context")
    || normalized.includes("background")
    || normalized.includes("overview")
    || normalized.includes("背景")
    || normalized.includes("上下文")
    || normalized.includes("概览")
}

function isStepListSection(section: string): boolean {
  if (!section) return true
  if (isContextHeading(section)) return false

  const normalized = normalizeHeading(section)
  return normalized.includes("step")
    || normalized.includes("plan")
    || normalized.includes("implementation")
    || normalized.includes("execution")
    || normalized.includes("verify")
    || normalized.includes("verification")
    || normalized.includes("步骤")
    || normalized.includes("计划")
    || normalized.includes("方案")
    || normalized.includes("实施")
    || normalized.includes("执行")
    || normalized.includes("验证")
    || normalized.includes("验收")
}

function isVerificationHeading(title: string): boolean {
  const normalized = normalizeHeading(title)
  return normalized.includes("verify")
    || normalized.includes("verification")
    || normalized.includes("验证")
    || normalized.includes("验收")
}

function isExecutableHeading(title: string, section: string): boolean {
  if (isContextHeading(section) || isContextHeading(title)) return false

  const normalized = normalizeHeading(title)
  return normalized.startsWith("step ")
    || normalized.startsWith("phase ")
    || normalized.startsWith("verification")
    || normalized.startsWith("verify")
    || normalized.startsWith("步骤")
    || normalized.startsWith("阶段")
    || normalized.startsWith("验证")
    || normalized.startsWith("验收")
    || isStepListSection(section)
}

function sectionName(section: string): string {
  return section.trim() || "Steps"
}

function leadingWhitespace(line: string): number {
  return line.length - line.trimStart().length
}

function stripOrderedMarker(trimmed: string): string | null {
  const match = trimmed.match(/^\d+[\.)]\s+(.+)/)
  return match?.[1]?.trim() || null
}

function stripUnorderedMarker(trimmed: string): string | null {
  if (/^-\s+\[/.test(trimmed)) return null
  const match = trimmed.match(/^[-*+]\s+(.+)/)
  return match?.[1]?.trim() || null
}

/**
 * Group steps by phase name.
 */
export function groupStepsByPhase<T extends ParsedPlanStep>(
  steps: T[]
): { name: string; steps: T[] }[] {
  const groups: { name: string; steps: T[] }[] = []
  let currentGroup: { name: string; steps: T[] } | null = null

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
