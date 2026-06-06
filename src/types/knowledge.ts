// Knowledge Base ("Knowledge Space") frontend types. Mirror the ha-core serde
// (camelCase) — see crates/ha-core/src/knowledge/types.rs.

export type KbAccess = "read" | "write"

export interface KnowledgeBase {
  id: string
  name: string
  emoji?: string | null
  /** External bound root (read-only in Phase 1) when set; null = internal. */
  rootDir?: string | null
  archived: boolean
  createdAt: number
  updatedAt: number
}

export interface KnowledgeBaseMeta extends KnowledgeBase {
  noteCount: number
  external: boolean
}

export interface KbAttachment extends KnowledgeBase {
  access: KbAccess
  /** "session" | "project" */
  via: string
}

/** A KB attach staged in the composer before a session exists; replayed as a
 *  real attach once the first message creates the session. */
export interface KbDraftAttachment {
  kbId: string
  access: KbAccess
}

/** A note the chat composer can reference via `[[ ]]`, flattened across the KBs
 *  reachable from the current chat. Mirrors ha-core `ReferenceableNote`. */
export interface ReferenceableNote {
  kbId: string
  kbName: string
  kbEmoji?: string | null
  relPath: string
  title: string
}

export interface Note {
  id: number
  kbId: string
  relPath: string
  title: string
  frontmatterJson?: string | null
  mtime: number
  contentHash: string
  size: number
}

export type LinkType = "wiki" | "embed" | "md"

export interface NoteLink {
  srcNoteId: number
  targetRef: string
  targetNoteId?: number | null
  linkType: LinkType
  anchor?: string | null
  alias?: string | null
  rawText: string
  srcStartLine: number
  srcStartCol: number
  srcEndLine: number
  srcEndCol: number
  srcHeadingPath?: string | null
}

export interface Backlink {
  srcNoteId: number
  srcRelPath: string
  srcTitle: string
  rawText: string
  srcStartLine: number
  srcStartCol: number
  srcHeadingPath?: string | null
}

/** A broken (dangling) link — feeds the maintenance panel (Phase 2). */
export interface BrokenLink {
  srcNoteId: number
  srcRelPath: string
  srcTitle: string
  /** Unresolved target inside `[[ ]]` (a candidate note to create). */
  targetRef: string
  rawText: string
  srcStartLine: number
  srcStartCol: number
  srcHeadingPath?: string | null
}

/**
 * Result of a note/folder rename or move that also rewrites inbound `[[ ]]`
 * links (#9). Wire shape of `kb_note_rename_cmd` / `kb_rename_dir_cmd`.
 */
export interface RenameOutcome {
  newRel: string
  filesChanged: number
  linksRewritten: number
}

export interface NoteReadResult {
  kbId: string
  noteId: number
  relPath: string
  title: string
  content: string
  contentHash: string
  frontmatterJson?: string | null
  outgoingLinks: NoteLink[]
  backlinks: Backlink[]
  tags: string[]
}

/** One node in the KB link graph (WS1). Degrees count resolved links only; a
 *  node with both at 0 is an orphan. Mirrors ha-core `GraphNode`. */
export interface GraphNode {
  id: number
  relPath: string
  title: string
  inDegree: number
  outDegree: number
}

/** A directed resolved-link edge `source → target` (note ids). */
export interface GraphEdge {
  source: number
  target: number
}

/** The KB link graph. `truncated` = a node cap clipped a huge vault. Wire shape
 *  of `kb_graph_cmd`. */
export interface KnowledgeGraph {
  nodes: GraphNode[]
  edges: GraphEdge[]
  truncated: boolean
}

export interface NoteSearchHit {
  kbId: string
  noteId: number
  relPath: string
  title: string
  score: number
  snippet: string
  headingPath?: string | null
  startLine: number
}

export interface CreateKnowledgeBaseInput {
  name: string
  emoji?: string | null
  rootDir?: string | null
}

export interface UpdateKnowledgeBaseInput {
  name?: string | null
  emoji?: string | null
  archived?: boolean | null
}

/** Note editor view modes (design D13). */
export type NoteEditorMode = "source" | "preview" | "split"

/**
 * Advanced chunking parameters (D12). Wire shape of `knowledge_chunk_get_cmd` /
 * `knowledge_chunk_set_cmd`. Values returned are already clamped server-side
 * (maxChars 200–8000; overlapChars 0–maxChars/2).
 */
export interface ChunkConfig {
  maxChars: number
  overlapChars: number
}
