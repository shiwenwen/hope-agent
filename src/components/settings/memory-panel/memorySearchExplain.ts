import type { MemoryEntry } from "./types"

export type MemorySearchMatchKind = "content" | "tag" | "source" | "session" | "ranked"

export interface MemorySearchMatch {
  kind: MemorySearchMatchKind
}

function normalizeSearchText(value: string | null | undefined): string {
  return (value ?? "").trim().toLocaleLowerCase()
}

function includesNeedle(value: string | null | undefined, needle: string): boolean {
  if (!needle) return false
  return normalizeSearchText(value).includes(needle)
}

export function explainMemorySearchMatch(
  memory: Pick<MemoryEntry, "content" | "tags" | "source" | "sourceSessionId" | "relevanceScore">,
  query: string,
): MemorySearchMatch[] {
  const needle = normalizeSearchText(query)
  if (!needle) return []

  const matches: MemorySearchMatch[] = []
  if (includesNeedle(memory.content, needle)) matches.push({ kind: "content" })
  if (memory.tags.some((tag) => includesNeedle(tag, needle))) matches.push({ kind: "tag" })
  if (includesNeedle(memory.source, needle)) matches.push({ kind: "source" })
  if (includesNeedle(memory.sourceSessionId, needle)) matches.push({ kind: "session" })

  if (matches.length === 0 && memory.relevanceScore != null) {
    matches.push({ kind: "ranked" })
  }

  return matches
}
