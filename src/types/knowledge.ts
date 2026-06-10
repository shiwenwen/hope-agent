// Knowledge Base ("Knowledge Space") frontend types. Mirror the ha-core serde
// (camelCase) — see crates/ha-core/src/knowledge/types.rs.

export type KbAccess = "read" | "write"

export interface KnowledgeBase {
  id: string
  name: string
  emoji?: string | null
  /** External bound root when set; null = internal. Read-only unless
   *  `allowExternalWrites` is enabled (WS7). */
  rootDir?: string | null
  /** Opt-in to editing an external (bound) root (WS7). Ignored for internal KBs. */
  allowExternalWrites: boolean
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

/** Mirror ha-core `KnowledgeBase::display_label()`: emoji + space + name, or
 *  name only when there's no emoji. For inline single-string labels (no separate
 *  emoji chip). */
export function kbLabel(emoji: string | null | undefined, name: string): string {
  const e = emoji?.trim()
  return e ? `${e} ${name}` : name
}

/** A KB attach staged in the composer before a session exists; replayed as a
 *  real attach once the first message creates the session. */
export interface KbDraftAttachment {
  kbId: string
  access: KbAccess
}

/** A knowledge-space sidebar conversation thread. Mirrors ha-core
 *  `KbChatThread` — one row per `kind='knowledge'` session, used by the panel's
 *  history picker + default-load. */
export interface KbChatThread {
  sessionId: string
  kbId: string
  anchorNotePath?: string | null
  title?: string | null
  /** Thread creation time (epoch ms). */
  createdAt: number
  /** Session `updated_at` (rfc3339) — recency sort key. */
  updatedAt: string
  messageCount: number
  lastSnippet?: string | null
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

/** A user-pinned graph node position (Batch J), keyed by `relPath` (stable across
 *  index rebuilds). Wire shape of `kb_graph_layout_{get,save}_cmd`. */
export interface GraphNodePosition {
  relPath: string
  x: number
  y: number
}

export interface NoteSearchHit {
  kbId: string
  /** Human-readable source KB name (registry truth source; falls back to kbId). */
  kbName?: string
  /** Source KB emoji, if any. */
  kbEmoji?: string | null
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
  /** Unlock / re-lock writes to an external (bound) root (WS7). */
  allowExternalWrites?: boolean | null
}

/** Note editor view modes (design D13). `live` = source pane with syntax markers
 *  hidden in place (Obsidian-style live preview); see livePreviewExtensions.ts.
 *  `outline` = collapsible read-only heading-tree render (Phase 3 G, D8 optional
 *  layer; never replaces the CM6 base). */
export type NoteEditorMode = "source" | "preview" | "split" | "live" | "outline"

/**
 * Advanced chunking parameters (D12). Wire shape of `knowledge_chunk_get_cmd` /
 * `knowledge_chunk_set_cmd`. Values returned are already clamped server-side
 * (maxChars 200–8000; overlapChars 0–maxChars/2).
 */
export interface ChunkConfig {
  maxChars: number
  overlapChars: number
}

/**
 * Read bridge ③ — passive related-notes config (`AppConfig.knowledge_passive_recall`).
 * Wire shape of `kb_passive_recall_config_get_cmd` / `_set_cmd`. Values are
 * clamped server-side (topN 1–20; maxChars 100–4000; cacheTtlSecs ≥ 1).
 */
export interface PassiveRecallConfig {
  enabled: boolean
  topN: number
  maxChars: number
  cacheTtlSecs: number
  showSnippet: boolean
}

// ── Layer-2 autonomous maintenance (WS6) ─────────────────────────

export type ProposalKind =
  | "auto_link"
  | "orphan_rescue"
  | "frontmatter_fill"
  | "dedup_merge"
  | "knowledge_gap"
  | "auto_tag"
  | "moc_upkeep"
  | "memory_to_note"

export type ProposalStatus = "draft" | "applied" | "rejected" | "failed"

/** A queued maintenance proposal awaiting owner review. */
export interface MaintenanceProposal {
  id: number
  kbId: string
  kind: ProposalKind
  status: ProposalStatus
  title: string
  detail: string
  /** Tagged file action (`{ op: "append_link" | ... }`) — opaque to the UI. */
  action: Record<string, unknown>
  fingerprint: string
  createdAt: number
  decidedAt?: number
  error?: string
}

export interface MaintenanceReport {
  generated: number
  byKind: Record<string, number>
  skippedExisting: number
  autoApplied: number
  note?: string
  durationMs: number
}

export interface MaintenanceStatus {
  running: boolean
  lastReport?: MaintenanceReport
}

export interface MaintenanceTasks {
  autoLink: boolean
  orphanRescue: boolean
  frontmatterFill: boolean
  dedupMerge: boolean
  knowledgeGap: boolean
  autoTag: boolean
  mocUpkeep: boolean
  memoryToNote: boolean
}

/** Wire shape of the `knowledge_maintenance` settings category. */
export interface MaintenanceConfig {
  enabled: boolean
  idleTrigger: { enabled: boolean; idleMinutes: number }
  cronTrigger: { enabled: boolean; cronExpr: string }
  manualEnabled: boolean
  tasks: MaintenanceTasks
  autoApprove: boolean
  maxProposalsPerCycle: number
  dedupSimilarity: number
  llmTimeoutSecs: number
  llmMaxTokens: number
}
