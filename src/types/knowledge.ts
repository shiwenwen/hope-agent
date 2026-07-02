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
  /** Agent baked into this thread — restored on history-picker switch so
   *  follow-ups run with the thread's own agent + model. */
  agentId: string
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

export type KnowledgeSourceKind =
  | "markdown"
  | "text"
  | "pdf"
  | "docx"
  | "audio_transcript"
  | "video_transcript"
  | "image_ocr"
  | "browser_snapshot"
  | "url_snapshot"
export type KnowledgeSourceStatus = "ready" | "failed"
export type KnowledgeBrowserCaptureMode = "auto" | "selection" | "page"

export interface KnowledgeSourceImportInput {
  kind?: KnowledgeSourceKind | null
  title?: string | null
  fileName?: string | null
  mimeType?: string | null
  content?: string | null
  dataBase64?: string | null
  url?: string | null
}

export interface KnowledgeSourceImportBatchItemInput {
  clientId?: string | null
  label?: string | null
  input: KnowledgeSourceImportInput
}

export interface KnowledgeSourceImportBatchInput {
  items: KnowledgeSourceImportBatchItemInput[]
}

export interface KnowledgeBrowserSourceImportInput {
  mode?: KnowledgeBrowserCaptureMode
  title?: string | null
}

export interface KnowledgeSource {
  id: string
  kbId: string
  kind: KnowledgeSourceKind
  title: string
  originUri?: string | null
  storedPath: string
  contentHash: string
  extractedTextHash?: string | null
  status: KnowledgeSourceStatus
  compiledAt?: number | null
  createdAt: number
  updatedAt: number
  size: number
  chunkCount: number
}

export interface KnowledgeSourceReadResult extends KnowledgeSource {
  content: string
}

export type KnowledgeSourceImportRunStatus =
  | "running"
  | "completed"
  | "completed_with_errors"
  | "failed"

export type KnowledgeSourceImportItemStatus =
  | "pending"
  | "running"
  | "imported"
  | "duplicate"
  | "failed"

export interface KnowledgeSourceImportItem {
  id: number
  runId: string
  kbId: string
  position: number
  clientId?: string | null
  label?: string | null
  kind?: KnowledgeSourceKind | null
  status: KnowledgeSourceImportItemStatus
  sourceId?: string | null
  duplicateOfSourceId?: string | null
  error?: string | null
  createdAt: number
  startedAt?: number | null
  finishedAt?: number | null
  updatedAt: number
}

export interface KnowledgeSourceImportRun {
  id: string
  kbId: string
  status: KnowledgeSourceImportRunStatus
  totalCount: number
  importedCount: number
  duplicateCount: number
  failedCount: number
  createdAt: number
  startedAt?: number | null
  finishedAt?: number | null
  updatedAt: number
}

export interface KnowledgeSourceImportRunDetail extends KnowledgeSourceImportRun {
  items: KnowledgeSourceImportItem[]
}

export type KnowledgeSourceSimilarityGroupKind = "exact_duplicate" | "similar"

export interface KnowledgeSourceSimilarityGroup {
  id: string
  kind: KnowledgeSourceSimilarityGroupKind
  similarity: number
  fingerprint: string
  sources: KnowledgeSource[]
}

export interface SchemaPageTypeSpec {
  key: string
  label: string
  requiredSections: string[]
  requiredFrontmatter: string[]
}

export interface SchemaProfile {
  kbId: string
  pageTypes: SchemaPageTypeSpec[]
  defaultPageType: string
  requiredSections: string[]
  updatedAt: number
}

export type SchemaIssueKind =
  | "missing_evidence"
  | "stale_source"
  | "schema_violation"
  | "conflicting_claim"
  | "unfiled_open_question"

export interface SchemaIssue {
  kbId: string
  relPath: string
  title: string
  kind: SchemaIssueKind
  detail: string
  sourceIds?: string[]
}

export interface NoteSourceRef {
  sourceId: string
  title?: string | null
  originUri?: string | null
  missing: boolean
  stale: boolean
  sourceUpdatedAt?: number | null
  noteLastCompiledAt?: number | null
  citedIn?: string[]
}

export type KnowledgeAgentItemKind = "note" | "compiled_note" | "source"

export interface KnowledgeAgentSearchInput {
  query: string
  kbId?: string | null
  limit?: number | null
  includeSources?: boolean
}

export interface KnowledgeAgentNoteHit {
  kind: KnowledgeAgentItemKind
  kbId: string
  kbName?: string
  kbEmoji?: string | null
  noteId: number
  relPath: string
  title: string
  score: number
  snippet: string
  headingPath?: string | null
  startLine: number
}

export interface KnowledgeAgentSourceItem {
  kind: "source"
  kbId: string
  sourceId: string
  sourceKind: KnowledgeSourceKind
  status: KnowledgeSourceStatus
  title: string
  originUri?: string | null
  contentHash: string
  compiledAt?: number | null
  stale: boolean
  createdAt: number
  updatedAt: number
  size: number
  chunkCount: number
  snippet?: string | null
  content?: string | null
}

export interface KnowledgeAgentSearchResult {
  notes: KnowledgeAgentNoteHit[]
  sources?: KnowledgeAgentSourceItem[]
  truncated: boolean
}

export interface KnowledgeAgentReadInput {
  kbId: string
  path?: string | null
  reference?: string | null
  includeSourceRefs?: boolean | null
}

export interface KnowledgeAgentReadResult {
  kind: KnowledgeAgentItemKind
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
  sourceRefs?: NoteSourceRef[]
}

export interface KnowledgeAgentExpandInput {
  kbId: string
  path: string
  limit?: number | null
}

export interface KnowledgeAgentExpandResult {
  note: KnowledgeAgentReadResult
  relatedNotes: KnowledgeAgentNoteHit[]
}

export interface KnowledgeAgentSourcesInput {
  kbId: string
  sourceId?: string | null
  query?: string | null
  limit?: number | null
  includeContent?: boolean
}

export interface KnowledgeAgentSourcesResult {
  sources: KnowledgeAgentSourceItem[]
  truncated: boolean
}

export interface KnowledgeAgentCompileProposeInput {
  kbId: string
  sourceIds: string[]
  strategy?: string | null
}

export type CompileRunStatus = "running" | "completed" | "failed" | "cancelled"
export type CompileProposalStatus = "draft" | "applied" | "rejected" | "failed"
export type CompileProposalKind =
  | "create_note"
  | "patch_note"
  | "set_frontmatter"
  | "append_link"
  | "create_moc"

export interface CompileStartInput {
  sourceIds: string[]
  strategy?: string | null
}

export type QueryFileMode =
  | "create_note"
  | "update_current_note"
  | "append_to_moc"
  | "append_open_questions"

export interface QueryFileInput {
  sessionId: string
  messageId: number
  mode?: QueryFileMode | null
  currentNotePath?: string | null
  targetPath?: string | null
  title?: string | null
  confirmConversationSource?: boolean
}

export interface CompileRun {
  id: string
  kbId: string
  status: CompileRunStatus
  sourceIds: string[]
  strategy: string
  modelLabel?: string | null
  fingerprint: string
  error?: string | null
  summary?: string | null
  proposalCount: number
  createdAt: number
  startedAt?: number | null
  finishedAt?: number | null
  updatedAt: number
}

export type CompileProposalAction =
  | { op: "create_note"; path: string; content: string; overwrite?: boolean }
  | {
      op: "patch_note"
      path: string
      old: string
      new: string
      expected_file_hash?: string | null
    }
  | { op: "set_frontmatter"; path: string; props: Record<string, unknown> }
  | { op: "append_link"; from_path: string; to_ref: string }
  | { op: "create_moc"; path: string; content: string; overwrite?: boolean }

export interface CompileProposal {
  id: number
  runId: string
  kbId: string
  kind: CompileProposalKind
  status: CompileProposalStatus
  title: string
  detail: string
  action: CompileProposalAction
  fingerprint: string
  sourceIds: string[]
  createdAt: number
  decidedAt?: number | null
  error?: string | null
  beforeText?: string | null
  afterText?: string | null
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

// ── Sprite / inspiration mode (Phase 2) ──────────────────────────

export interface SpriteSenses {
  doc: boolean
  edit: boolean
  conversation: boolean
  memory: boolean
  awareness: boolean
}

/** When the sprite may fire (orthogonal to `senses` = what it reads). */
export interface SpriteTriggers {
  editIdle: boolean
  noteOpen: boolean
  conversation: boolean
  periodic: boolean
  paste: boolean
}

export interface SpriteConfig {
  enabled: boolean
  idleEditSecs: number
  minChangeChars: number
  cooldownSecs: number
  maxPerSessionPerHour: number
  periodicSecs: number
  pasteMinChars: number
  proactive: boolean
  triggers: SpriteTriggers
  senses: SpriteSenses
  maxTokens: number
  timeoutSecs: number
}

/** Hybrid `note_search` ranking parameters (`AppConfig.knowledge_search`). */
export interface KnowledgeSearchConfig {
  textWeight: number
  vectorWeight: number
  rrfK: number
  mmrLambda: number
  candidateMultiplier: number
}

/** Best-practice defaults — the reset-to values (must mirror the Rust defaults). */
export const KNOWLEDGE_SEARCH_DEFAULTS: KnowledgeSearchConfig = {
  textWeight: 0.4,
  vectorWeight: 0.6,
  rrfK: 60,
  mmrLambda: 0.7,
  candidateMultiplier: 3,
}

export type SpriteCategory = "writing" | "review" | "encourage" | "remind" | "connect"

/** A transient sprite suggestion, delivered via the `sprite:suggestion` event. */
export interface SpriteSuggestion {
  sessionId?: string | null
  kbId: string
  notePath: string
  category: SpriteCategory
  text: string
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
  | "source_compile"
  | "source_conflict"
  | "open_questions_moc"
  | "for_agent_summary"

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
  sourceCompile: boolean
  sourceConflict: boolean
  openQuestionsMoc: boolean
  forAgentSummary: boolean
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
