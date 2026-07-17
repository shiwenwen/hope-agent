import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { BookText, Brain, FileText } from "lucide-react"

import { cn } from "@/lib/utils"
import { kbLabel } from "@/types/knowledge"
import { memoryScopeLabel } from "./memoryTraceFormat"

interface NoteHit {
  kbId: string
  kbName?: string
  kbEmoji?: string | null
  relPath: string
  title: string
  snippet?: string
  startLine?: number
}

interface MemoryHit {
  id?: string
  type?: string
  scope?: string
  content?: string
}

interface KbGroup {
  kbId: string
  label: string
  hits: NoteHit[]
}

function groupByKb(hits: NoteHit[]): KbGroup[] {
  const map = new Map<string, KbGroup>()
  for (const h of hits) {
    const name = h.kbName?.trim() || h.kbId
    const label = kbLabel(h.kbEmoji, name)
    const g = map.get(h.kbId)
    if (g) g.hits.push(h)
    else map.set(h.kbId, { kbId: h.kbId, label, hits: [h] })
  }
  return [...map.values()]
}

function asNoteHits(value: unknown): NoteHit[] {
  if (!Array.isArray(value)) return []
  return value.filter((h): h is NoteHit => !!h && typeof h === "object" && "kbId" in h)
}

function NoteHitRow({ hit }: { hit: NoteHit }) {
  const title = hit.title?.trim() || hit.relPath
  return (
    <div className="flex items-start gap-1.5 py-1">
      <FileText className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground/50" />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-1.5">
          <span className="truncate font-medium text-foreground/90">{title}</span>
          <span className="shrink-0 truncate text-[10px] text-muted-foreground/50">{hit.relPath}</span>
        </div>
        {hit.snippet?.trim() ? (
          <p className="mt-0.5 line-clamp-2 text-muted-foreground/70">{hit.snippet.trim()}</p>
        ) : null}
      </div>
    </div>
  )
}

function NotesSection({ hits }: { hits: NoteHit[] }) {
  const groups = useMemo(() => groupByKb(hits), [hits])
  if (groups.length === 0) return null
  return (
    <div className="space-y-2">
      {groups.map((g) => (
        <div key={g.kbId}>
          {/* The KB header IS the source attribution — which knowledge space each hit came from. */}
          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground/60">
            <BookText className="h-3 w-3 shrink-0" />
            <span className="truncate">{g.label}</span>
            <span className="text-muted-foreground/40">· {g.hits.length}</span>
          </div>
          <div className="mt-0.5 border-l border-border/40 pl-2">
            {g.hits.map((h, i) => (
              <NoteHitRow key={`${h.kbId}:${h.relPath}:${i}`} hit={h} />
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

function MemoriesSection({ hits }: { hits: MemoryHit[] }) {
  const { t } = useTranslation()
  if (hits.length === 0) return null
  return (
    <div>
      <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground/60">
        <Brain className="h-3 w-3 shrink-0" />
        <span className="truncate">{t("knowledge.result.memory")}</span>
        <span className="text-muted-foreground/40">· {hits.length}</span>
      </div>
      <div className="mt-0.5 space-y-1 border-l border-border/40 pl-2">
        {hits.map((m, i) => (
          <div key={m.id ?? i} className="flex items-start gap-1.5 py-0.5">
            <div className="min-w-0 flex-1">
              {m.content?.trim() ? (
                <p className="line-clamp-2 text-foreground/85">{m.content.trim()}</p>
              ) : null}
              {m.scope ? (
                <span className="text-[10px] text-muted-foreground/45">
                  {memoryScopeLabel(m.scope, t)}
                </span>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

/**
 * Render `note_search` / `note_similar` / `knowledge_recall` results grouped by
 * source knowledge space (emoji + name header = source attribution, the point in
 * multi-KB sessions). `knowledge_recall` keeps notes and memory as two separate
 * sections (D7 — never mixed). Falls back to the raw JSON on a parse failure.
 */
export function KnowledgeResultCard({
  result,
  className,
}: {
  toolName: string
  result: string
  className?: string
}) {
  const parsed = useMemo<{ notes: NoteHit[]; memories: MemoryHit[] } | null>(() => {
    try {
      const obj = JSON.parse(result) as Record<string, unknown>
      // note_search / note_similar → { hits }; knowledge_recall → { notes:{hits}, memories:{hits} }
      const notesContainer =
        (obj.notes as { hits?: unknown } | undefined)?.hits ?? obj.hits
      const notes = asNoteHits(notesContainer)
      const memRaw = (obj.memories as { hits?: unknown } | undefined)?.hits
      const memories = Array.isArray(memRaw) ? (memRaw as MemoryHit[]) : []
      if (notes.length === 0 && memories.length === 0) return null
      return { notes, memories }
    } catch {
      return null
    }
  }, [result])

  if (!parsed) {
    return (
      <pre className="whitespace-pre-wrap rounded-md border border-border/50 bg-secondary/40 p-2.5 text-[11px] leading-relaxed text-muted-foreground/80 max-h-64 overflow-y-auto">
        {result}
      </pre>
    )
  }

  return (
    <div
      className={cn(
        "space-y-3 rounded-md border border-border/50 bg-secondary/40 p-2.5 text-[11px] leading-relaxed max-h-72 overflow-y-auto",
        className,
      )}
    >
      <NotesSection hits={parsed.notes} />
      <MemoriesSection hits={parsed.memories} />
    </div>
  )
}
