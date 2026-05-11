// Parses `@plan:<short_id>[:v<version>]` tokens out of chat input text.
//
// `short_id` is the first 8 hex chars of a session id (per backend
// `find_sessions_by_id_prefix`). `version` defaults to 0 (current plan)
// when omitted. The regex tolerates 4–16 hex chars so users can paste
// longer prefixes for ambiguity-tolerant matching.

const PLAN_MENTION_RE = /@plan:([0-9a-f]{4,16})(?::v(\d+))?/gi

export interface PlanMentionToken {
  shortId: string
  version: number
  /// Original matched substring, useful for de-duplication / replacement.
  raw: string
}

export function parsePlanMentions(input: string): PlanMentionToken[] {
  if (!input) return []
  const out: PlanMentionToken[] = []
  const seen = new Set<string>()
  let match: RegExpExecArray | null
  PLAN_MENTION_RE.lastIndex = 0
  while ((match = PLAN_MENTION_RE.exec(input)) !== null) {
    const shortId = match[1].toLowerCase()
    const version = match[2] ? Number.parseInt(match[2], 10) : 0
    const key = `${shortId}@${version}`
    if (seen.has(key)) continue
    seen.add(key)
    out.push({ shortId, version, raw: match[0] })
  }
  return out
}
