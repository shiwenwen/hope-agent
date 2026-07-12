export type DraftGoalCriterionKind = "required" | "optional" | "follow_up"

export interface DraftGoalCriterionItem {
  id: string
  text: string
  kind: DraftGoalCriterionKind
}

// Keep this preview parser aligned with ha-core goal::parse_goal_criteria_items.
// The Rust parser remains the durable source of truth; this only explains drafts before save.
export function parseGoalCriteriaDraft(raw: string): DraftGoalCriterionItem[] {
  const items: DraftGoalCriterionItem[] = []
  let sectionKind: DraftGoalCriterionKind = "required"
  for (const line of raw.split(/\r?\n/)) {
    for (const part of line.split(";")) {
      let text = cleanGoalCriterionText(part)
      if (!text) continue
      const parsed = parseGoalCriterionKindPrefix(text)
      let kind = sectionKind
      if (parsed) {
        const rest = cleanGoalCriterionText(parsed.rest)
        if (!rest) {
          sectionKind = parsed.kind
          continue
        }
        text = rest
        kind = parsed.kind
      }
      items.push({ id: `criterion-${items.length + 1}`, text, kind })
    }
  }
  return items
}

function cleanGoalCriterionText(raw: string): string {
  let text = raw.trim()
  let next = text.replace(/^[-*\u2022]+/, "").trim()
  while (next !== text) {
    text = next
    next = text.replace(/^[-*\u2022]+/, "").trim()
  }
  for (const checkbox of ["[ ]", "[x]", "[X]", "\u2610", "\u2611"]) {
    if (text.startsWith(checkbox)) {
      text = text.slice(checkbox.length).trim()
      break
    }
  }
  const numbered = text.match(/^(\d+)([.)\u3001])\s*(.*)$/)
  if (numbered) return numbered[3]?.trim() ?? ""
  return text
}

function parseGoalCriterionKindPrefix(
  text: string,
): { kind: DraftGoalCriterionKind; rest: string } | null {
  const trimmed = text.trim()
  if (trimmed.startsWith("[")) {
    const end = trimmed.indexOf("]")
    if (end > 0) {
      const kind = goalKindFromLabel(normalizeGoalKindLabel(trimmed.slice(1, end)))
      if (kind) return { kind, rest: trimmed.slice(end + 1) }
    }
  }
  const colonIndex = findKindSeparator(trimmed)
  if (colonIndex >= 0) {
    const kind = goalKindFromLabel(normalizeGoalKindLabel(trimmed.slice(0, colonIndex)))
    if (kind) return { kind, rest: trimmed.slice(colonIndex + 1) }
  }
  return null
}

function findKindSeparator(text: string): number {
  const ascii = text.indexOf(":")
  const fullWidth = text.indexOf("\uff1a")
  if (ascii < 0) return fullWidth
  if (fullWidth < 0) return ascii
  return Math.min(ascii, fullWidth)
}

function normalizeGoalKindLabel(label: string): string {
  return label.trim().toLowerCase().replace(/[ -]/g, "_")
}

function goalKindFromLabel(label: string): DraftGoalCriterionKind | null {
  switch (label) {
    case "required":
    case "require":
    case "must":
    case "must_have":
    case "\u5fc5\u987b":
    case "\u5fc5\u9700":
    case "\u5fc5\u8981":
      return "required"
    case "optional":
    case "nice_to_have":
    case "\u53ef\u9009":
    case "\u53ef\u6709":
      return "optional"
    case "follow_up":
    case "followup":
    case "later":
    case "backlog":
    case "\u540e\u7eed":
    case "\u540e\u7eed\u9879":
    case "\u589e\u5f3a":
      return "follow_up"
    default:
      return null
  }
}
