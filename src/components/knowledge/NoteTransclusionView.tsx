// Knowledge-base note preview with `![[ ]]` transclusion (WS2, Phase 2).
//
// Renders a note's markdown, inlining block-level `![[ref]]` embeds (a `![[ ]]`
// alone on its line) by recursively fetching the target through the owner-plane
// resolver (single source of truth for `[[ ]]` resolution, design #8). Recursion
// is bounded (depth cap + cycle detection over resolved rel-paths); broken refs
// show a placeholder. Inline `![[ ]]` (mid-paragraph) is left as raw text.

import { FileText } from "lucide-react"
import { memo, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import type { NoteReadResult } from "@/types/knowledge"

import { fetchNoteRef } from "./noteRefFetch"
import { cleanEmbedRef, parseEmbedSegments, stripFrontmatter } from "./transclusionParse"

/** Max embed nesting before we stop recursing — the root note is depth 0, so up
 *  to 4 levels of embedded notes are inlined before the limit frame appears. */
const MAX_EMBED_DEPTH = 4

const EMPTY_SEEN: ReadonlySet<string> = new Set()

interface NoteTransclusionViewProps {
  kbId: string
  content: string
  /** Bumped on knowledge:changed to invalidate the embed cache. */
  cacheBustKey: number
  /** Open a note (clicking an embed header). Optional in nested previews. */
  onOpenNote?: (relPath: string) => void
  /** Recursion depth (0 = the note itself). */
  depth?: number
  /** Resolved rel-paths already in the embed chain (cycle guard). */
  seen?: ReadonlySet<string>
}

/** Recursive note renderer: markdown runs verbatim, `![[ ]]` runs as embeds. */
function NoteTransclusionView({
  kbId,
  content,
  cacheBustKey,
  onOpenNote,
  depth = 0,
  seen,
}: NoteTransclusionViewProps) {
  const segments = useMemo(() => parseEmbedSegments(content), [content])
  return (
    <>
      {segments.map((seg, i) =>
        seg.type === "md" ? (
          seg.text.trim() ? <MarkdownRenderer key={`m:${i}`} content={seg.text} /> : null
        ) : (
          <EmbedBlock
            key={`e:${i}:${seg.ref}`}
            kbId={kbId}
            reference={seg.ref}
            cacheBustKey={cacheBustKey}
            onOpenNote={onOpenNote}
            depth={depth}
            seen={seen ?? EMPTY_SEEN}
          />
        ),
      )}
    </>
  )
}

interface EmbedBlockProps {
  kbId: string
  reference: string
  cacheBustKey: number
  onOpenNote?: (relPath: string) => void
  depth: number
  seen: ReadonlySet<string>
}

function EmbedBlock({ kbId, reference, cacheBustKey, onOpenNote, depth, seen }: EmbedBlockProps) {
  const { t } = useTranslation()
  // Result tagged with the ref it resolved, so a ref/bust change reads as
  // "loading" without a synchronous setState in the effect.
  const [entry, setEntry] = useState<{
    ref: string
    bust: number
    note: NoteReadResult | null
  } | null>(null)

  const overDepth = depth >= MAX_EMBED_DEPTH
  // Resolve by the cleaned target (drop `#anchor` / `|alias`), mirroring the
  // canonical link pipeline so embeds resolve the same notes the graph does.
  const target = cleanEmbedRef(reference)

  useEffect(() => {
    if (overDepth) return
    let alive = true
    void fetchNoteRef(kbId, target, cacheBustKey).then((note) => {
      if (alive) setEntry({ ref: reference, bust: cacheBustKey, note })
    })
    return () => {
      alive = false
    }
  }, [kbId, reference, target, cacheBustKey, overDepth])

  if (overDepth) {
    return (
      <EmbedFrame reference={reference}>
        <span className="text-xs italic text-muted-foreground">
          {t("knowledge.embed.depth", "Embed depth limit reached")}
        </span>
      </EmbedFrame>
    )
  }

  const ready = entry && entry.ref === reference && entry.bust === cacheBustKey
  if (!ready) {
    return (
      <EmbedFrame reference={reference}>
        <span className="animate-pulse text-xs text-muted-foreground">
          {t("knowledge.embed.loading", "Loading embed…")}
        </span>
      </EmbedFrame>
    )
  }

  const note = entry.note
  if (!note) {
    return (
      <div className="my-2 rounded-md border border-dashed border-destructive/50 bg-destructive/5 px-3 py-1.5 text-xs text-destructive">
        {t('knowledge.embed.broken', 'No note matches "{{ref}}"', { ref: reference })}
      </div>
    )
  }

  if (seen.has(note.relPath)) {
    return (
      <EmbedFrame reference={reference} title={note.title}>
        <span className="text-xs italic text-muted-foreground">
          {t("knowledge.embed.cycle", "Circular embed skipped")}
        </span>
      </EmbedFrame>
    )
  }

  const nextSeen = new Set(seen)
  nextSeen.add(note.relPath)
  const body = stripFrontmatter(note.content)

  return (
    <div className="my-2 overflow-hidden rounded-md border border-border-soft/60 bg-muted/20">
      <button
        type="button"
        onClick={() => onOpenNote?.(note.relPath)}
        className="flex w-full items-center gap-1.5 border-b border-border-soft/50 bg-muted/30 px-3 py-1 text-left text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        <FileText className="h-3 w-3 shrink-0" />
        <span className="truncate">{note.title}</span>
      </button>
      <div className="px-3 py-2">
        <NoteTransclusionView
          kbId={kbId}
          content={body}
          cacheBustKey={cacheBustKey}
          onOpenNote={onOpenNote}
          depth={depth + 1}
          seen={nextSeen}
        />
      </div>
    </div>
  )
}

/** Bordered shell for non-content embed states (loading / depth / cycle). */
function EmbedFrame({
  reference,
  title,
  children,
}: {
  reference: string
  title?: string
  children: React.ReactNode
}) {
  return (
    <div className="my-2 overflow-hidden rounded-md border border-border-soft/60 bg-muted/20">
      <div className="flex items-center gap-1.5 border-b border-border-soft/50 bg-muted/30 px-3 py-1 text-xs font-medium text-muted-foreground">
        <FileText className="h-3 w-3 shrink-0" />
        <span className="truncate">{title ?? reference}</span>
      </div>
      <div className="px-3 py-2">{children}</div>
    </div>
  )
}

export default memo(NoteTransclusionView)
