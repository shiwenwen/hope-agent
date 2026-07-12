// Knowledge-base note preview with `![[ ]]` transclusion (WS2, Phase 2).
//
// Renders a note's markdown, inlining block-level `![[ref]]` embeds (a `![[ ]]`
// alone on its line) by recursively fetching the target through the owner-plane
// resolver (single source of truth for `[[ ]]` resolution, design #8). Recursion
// is bounded (depth cap + cycle detection over resolved rel-paths); broken refs
// show a placeholder. Inline `![[ ]]` (mid-paragraph) is left as raw text.

import { FileText } from "lucide-react"
import { memo, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"

import { knowledgeEmbedErrorDescription } from "./noteEmbedFeedback"
import { fetchNoteRef } from "./noteRefFetch"
import type { NoteRefFetchResult } from "./noteRefFetch"
import { splitMarkdownForPreviewHighlight } from "./previewHighlight"
import { embedAnchor, parseEmbedSegments, stripFrontmatter } from "./transclusionParse"

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
  /** Highlight + scroll to a source block in the root note preview. */
  highlightLine?: number | null
  /** Identity token for repeated reveal of the same line. */
  highlightToken?: unknown
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
  highlightLine,
  highlightToken,
  depth = 0,
  seen,
}: NoteTransclusionViewProps) {
  const split = useMemo(
    () =>
      depth === 0 ? splitMarkdownForPreviewHighlight(content, highlightLine) : null,
    [content, depth, highlightLine],
  )

  if (split) {
    return (
      <>
        <RenderedSegments
          kbId={kbId}
          content={split.before}
          cacheBustKey={cacheBustKey}
          onOpenNote={onOpenNote}
          depth={depth}
          seen={seen}
        />
        <HighlightedPreviewBlock token={highlightToken}>
          <RenderedSegments
            kbId={kbId}
            content={split.highlighted}
            cacheBustKey={cacheBustKey}
            onOpenNote={onOpenNote}
            depth={depth}
            seen={seen}
          />
        </HighlightedPreviewBlock>
        <RenderedSegments
          kbId={kbId}
          content={split.after}
          cacheBustKey={cacheBustKey}
          onOpenNote={onOpenNote}
          depth={depth}
          seen={seen}
        />
      </>
    )
  }

  return (
    <RenderedSegments
      kbId={kbId}
      content={content}
      cacheBustKey={cacheBustKey}
      onOpenNote={onOpenNote}
      depth={depth}
      seen={seen}
    />
  )
}

function RenderedSegments({
  kbId,
  content,
  cacheBustKey,
  onOpenNote,
  depth,
  seen,
}: {
  kbId: string
  content: string
  cacheBustKey: number
  onOpenNote?: (relPath: string) => void
  depth: number
  seen?: ReadonlySet<string>
}) {
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

function HighlightedPreviewBlock({
  token,
  children,
}: {
  token: unknown
  children: React.ReactNode
}) {
  const ref = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    ref.current?.scrollIntoView({ block: "center" })
  }, [token])
  return (
    <div
      ref={ref}
      className="my-2 rounded-md border border-primary/30 bg-primary/5 px-2 py-1 ring-1 ring-primary/15"
    >
      {children}
    </div>
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
    result: NoteRefFetchResult
  } | null>(null)

  const overDepth = depth >= MAX_EMBED_DEPTH

  useEffect(() => {
    if (overDepth) return
    let alive = true
    // Pass the full reference (anchor included) so the resolver slices a
    // `#Heading` section / `#^block` server-side (whole note when no anchor).
    void fetchNoteRef(kbId, reference, cacheBustKey).then((result) => {
      if (alive) setEntry({ ref: reference, bust: cacheBustKey, result })
    })
    return () => {
      alive = false
    }
  }, [kbId, reference, cacheBustKey, overDepth])

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

  const result = entry.result
  if (result.status === "failed") {
    const description = knowledgeEmbedErrorDescription(t, result.detail)
    return (
      <EmbedFrame reference={reference}>
        <div className="space-y-1 text-xs text-destructive">
          <div>
            {t("knowledge.embed.loadFailed", {
              defaultValue: "Couldn't load embedded note",
            })}
          </div>
          {description ? (
            <div className="break-words text-[11px] leading-relaxed text-muted-foreground">
              {description}
            </div>
          ) : null}
        </div>
      </EmbedFrame>
    )
  }

  if (result.status === "missing") {
    return (
      <div className="my-2 rounded-md border border-dashed border-destructive/50 bg-destructive/5 px-3 py-1.5 text-xs text-destructive">
        {t('knowledge.embed.broken', 'No note matches "{{ref}}"', { ref: reference })}
      </div>
    )
  }

  const note = result.note

  // Scope the cycle guard by target + anchor: an anchored embed (`A#^p1`) is a
  // slice of a distinct block, so it must not collide with the whole-note key
  // (`A`) seeded for the root note. True recursion (the same anchored ref nested
  // inside its own slice) still re-collides and is caught.
  const anchor = embedAnchor(reference)
  const seenKey = anchor ? `${note.relPath}#${anchor}` : note.relPath
  if (seen.has(seenKey)) {
    return (
      <EmbedFrame reference={reference} title={note.title}>
        <span className="text-xs italic text-muted-foreground">
          {t("knowledge.embed.cycle", "Circular embed skipped")}
        </span>
      </EmbedFrame>
    )
  }

  const nextSeen = new Set(seen)
  nextSeen.add(seenKey)
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
