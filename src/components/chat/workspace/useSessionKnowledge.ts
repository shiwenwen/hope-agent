import { useMemo } from "react"

import type { Message } from "@/types/chat"
import type { KbAttachment } from "@/types/knowledge"
import { iterateMessageToolCalls } from "./useSessionFileChanges"
import { useSessionAttachments } from "./useSessionAttachments"

export type KnowledgeActivityKind = "write" | "read"

export interface KnowledgeActivityEntry {
  /** Dedup/display key (kb-scoped note ref). */
  key: string
  kbId?: string
  /** Note path / title as given in the tool call. */
  ref: string
  kind: KnowledgeActivityKind
}

export interface SessionKnowledgeActivity {
  entries: KnowledgeActivityEntry[]
  writeCount: number
  readCount: number
  searchCount: number
}

// note_* tools that mutate a note → "write" activity. (note_link/distill/moc
// don't expose a single flat path arg, so they're counted but not listed.)
const NOTE_WRITE_TOOLS = new Set([
  "note_create",
  "note_update",
  "note_patch",
  "note_append",
  "note_delete",
  "note_move",
  "note_rename",
  "note_set_frontmatter",
  "note_assign_block",
  "note_link",
  "note_distill",
  "note_moc",
  "session_to_note",
])
const NOTE_READ_TOOLS = new Set(["note_read"])
// Search / recall tools — counted, not listed (they don't target one note).
const NOTE_SEARCH_TOOLS = new Set([
  "note_search",
  "knowledge_recall",
  "note_similar",
  "note_related",
  "note_by_tag",
  "note_backlinks",
  "note_tags",
  "note_orphans",
  "note_broken_links",
  "note_graph",
  "note_suggest_links",
])

function parseArgs(raw: string | undefined): Record<string, unknown> {
  if (!raw) return {}
  try {
    const o = JSON.parse(raw)
    return o && typeof o === "object" ? (o as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

function strField(o: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const k of keys) {
    const v = o[k]
    if (typeof v === "string" && v.trim()) return v.trim()
  }
  return undefined
}

/**
 * Aggregate this session's note activity from the loaded message window. note_*
 * tools emit NO tool_metadata, so (unlike files/URLs) there's no backend
 * aggregate — this is live-tail only, covering the loaded window. Writes beat
 * reads on the same note; most-recent-first. Search/recall calls are counted but
 * not listed (they don't target a single note). Pure function.
 */
export function aggregateKnowledgeActivity(messages: Message[]): SessionKnowledgeActivity {
  const map = new Map<string, KnowledgeActivityEntry>()
  const touch = (e: KnowledgeActivityEntry) => {
    map.delete(e.key)
    map.set(e.key, e)
  }
  let writeCount = 0
  let readCount = 0
  let searchCount = 0

  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      const name = tool.name
      if (NOTE_SEARCH_TOOLS.has(name)) {
        searchCount++
        continue
      }
      const isWrite = NOTE_WRITE_TOOLS.has(name)
      const isRead = NOTE_READ_TOOLS.has(name)
      if (!isWrite && !isRead) continue
      if (isWrite) writeCount++
      else readCount++

      const args = parseArgs(tool.arguments)
      const kbId = strField(args, "kb")
      // `to` covers note_move's destination; `note`/`path`/`title` the rest.
      const ref = strField(args, "note", "path", "title", "to", "from")
      if (!ref) continue // counted above, but no listable ref

      const key = `${kbId ?? ""}:${ref}`
      const existing = map.get(key)
      // A note already written stays "write" even if later read; refresh recency.
      if (existing?.kind === "write" && !isWrite) {
        touch(existing)
        continue
      }
      touch({ key, kbId, ref, kind: isWrite ? "write" : "read" })
    }
  }

  return { entries: [...map.values()].reverse(), writeCount, readCount, searchCount }
}

/** Cheap existence check: did this session touch any knowledge tool? */
export function messagesHaveKnowledgeActivity(messages: Message[]): boolean {
  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      if (
        NOTE_WRITE_TOOLS.has(tool.name) ||
        NOTE_READ_TOOLS.has(tool.name) ||
        NOTE_SEARCH_TOOLS.has(tool.name)
      ) {
        return true
      }
    }
  }
  return false
}

/**
 * Knowledge info for the Workspace panel: the KBs attached to this session
 * (owner-plane `list_session_kbs_cmd`, refreshed on `knowledge:changed`) plus
 * the live-tail note activity. Incognito returns no attachments (D10
 * close-on-burn) — the owner-plane list is never called.
 */
export function useSessionKnowledge(
  sessionId: string | null | undefined,
  projectId: string | null | undefined,
  opts: { incognito?: boolean; messages: Message[] },
): { attachments: KbAttachment[]; activity: SessionKnowledgeActivity } {
  const { incognito = false, messages } = opts
  const { attachments } = useSessionAttachments(sessionId, projectId, { incognito })
  const activity = useMemo(() => aggregateKnowledgeActivity(messages), [messages])
  return { attachments, activity }
}
