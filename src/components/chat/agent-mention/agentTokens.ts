import type { AgentSummaryForSidebar } from "@/types/chat"

/**
 * Composer token for a user-requested sub-agent delegation.
 *
 * The visible label is friendly/localized user text, while the href carries a
 * stable agent id. The fragment form mirrors `@skill` and survives the markdown
 * sanitizer in message history.
 */
export const AGENT_HREF_PREFIX = "#agent:"

export interface ParsedAgentMention {
  start: number
  end: number
  raw: string
  agentId: string
  label: string
}

const AGENT_MENTION_RE_SOURCE = /\[@([^\]\n]+)\]\(#agent:([A-Za-z0-9._-]+)\)/

export function parseAgentMentions(input: string): ParsedAgentMention[] {
  const out: ParsedAgentMention[] = []
  const re = new RegExp(AGENT_MENTION_RE_SOURCE.source, "g")
  for (const m of input.matchAll(re)) {
    const start = m.index ?? 0
    const end = start + m[0].length
    out.push({
      start,
      end,
      raw: m[0],
      label: m[1] ?? "",
      agentId: m[2] ?? "",
    })
  }
  return out
}

export function formatAgentInsertion(agentId: string, label: string): string {
  return `[@${safeAgentMentionLabel(label, agentId)}](${AGENT_HREF_PREFIX}${agentId})`
}

function safeAgentMentionLabel(label: string, fallback: string): string {
  const safe = label
    .replaceAll("[", " ")
    .replaceAll("]", " ")
    .replace(/[\r\n]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
  return safe || fallback
}

export function agentQueryFromToken(token: string): string {
  const t = token.trim().toLowerCase()
  if (t.startsWith("agent:")) return t.slice("agent:".length)
  if (t.startsWith("subagent:")) return t.slice("subagent:".length)
  return t
}

export function agentMatchesQuery(agent: AgentSummaryForSidebar, query: string): boolean {
  if (!query) return true
  const q = query.toLowerCase()
  return (
    agent.id.toLowerCase().includes(q) ||
    agent.name.toLowerCase().includes(q) ||
    (agent.description ?? "").toLowerCase().includes(q)
  )
}
