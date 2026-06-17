// Shared owner-plane resolver fetch for `[[ ]]` / `![[ ]]` references, with a
// module-level cache. Both the transclusion preview (WS2) and the wikilink hover
// card (WS9) read through this so a ref is fetched at most once per cache epoch.
//
// Keyed by `kbId::normalizedRef`; the whole map clears when `bust` advances (a
// knowledge:changed tick) so edits to referenced notes show through.

import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { NoteReadResult } from "@/types/knowledge"

let cacheBust = -1
const refCache = new Map<string, Promise<NoteReadResult | null>>()

export function fetchNoteRef(
  kbId: string,
  reference: string,
  bust: number,
): Promise<NoteReadResult | null> {
  if (bust !== cacheBust) {
    refCache.clear()
    cacheBust = bust
  }
  const key = `${kbId}::${reference.trim().toLowerCase()}`
  let p = refCache.get(key)
  if (!p) {
    const tx = getTransport()
    p = tx
      .call<NoteReadResult | null>("kb_note_read_ref_cmd", { kbId, reference })
      .catch((e) => {
        logger.error("knowledge", "noteRefFetch::fetchNoteRef", "kb_note_read_ref failed", e)
        // Don't pin a transient transport failure as a permanent broken ref —
        // drop the entry so the next access retries. A successful null (genuinely
        // unresolved ref) stays cached.
        refCache.delete(key)
        return null
      })
    refCache.set(key, p)
  }
  return p
}
