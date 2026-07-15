// Knowledge Base ("Knowledge Space") frontend types. Mirror the ha-core serde
// (camelCase) — see crates/ha-core/src/knowledge/types.rs.

export type KbAccess = "read" | "write"
export type KnowledgeExternalRawSyncMode = "disabled" | "raw" | "sources"

export interface KnowledgeBase {
  id: string
  name: string
  emoji?: string | null
  /** External bound root when set; null = internal. Read-only unless
   *  `allowExternalWrites` is enabled (WS7). */
  rootDir?: string | null
  /** Opt-in to editing an external (bound) root (WS7). Ignored for internal KBs. */
  allowExternalWrites: boolean
  /** Optional mirror of source text snapshots into an external vault folder. */
  externalRawSync: KnowledgeExternalRawSyncMode
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
export type KnowledgeSourceStatus = "ready" | "failed" | "partially_extracted"
export type KnowledgeBrowserCaptureMode = "auto" | "selection" | "page"
export type KnowledgeSourceAssetKind = "original" | "thumbnail"

export interface KnowledgeSourceAsset {
  kind: KnowledgeSourceAssetKind
  fileName: string
  mimeType: string
  size: number
  width?: number | null
  height?: number | null
  storedPath: string
  localPath?: string | null
  createdAt: number
}

export interface KnowledgeSourceAssets {
  original?: KnowledgeSourceAsset | null
  thumbnail?: KnowledgeSourceAsset | null
}

export interface KnowledgeSourceAssetLink {
  kbId: string
  sourceId: string
  kind: KnowledgeSourceAssetKind
  fileName: string
  mimeType: string
  size: number
  width?: number | null
  height?: number | null
  localPath?: string | null
}

export interface KnowledgeMediaRetentionConfig {
  enabled: boolean
  maxTotalBytes: number
  maxSourceBytes: number
  thumbnailMaxEdgePx: number
  pruneWhenOverQuota: boolean
}

export interface KnowledgeSourceLimitsConfig {
  maxTextSourceMb: number
  maxBinarySourceMb: number
  maxUrlResponseMb: number
}

export interface KnowledgeSourceImportInput {
  kind?: KnowledgeSourceKind | null
  title?: string | null
  fileName?: string | null
  mimeType?: string | null
  content?: string | null
  dataBase64?: string | null
  uploadId?: string | null
  url?: string | null
}

export interface KnowledgeSourceImportSessionAttachmentInput {
  sessionId: string
  path: string
  kind?: KnowledgeSourceKind | null
  title?: string | null
  fileName?: string | null
  mimeType?: string | null
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
  externalRawPath?: string | null
  contentHash: string
  extractedTextHash?: string | null
  status: KnowledgeSourceStatus
  compiledAt?: number | null
  createdAt: number
  updatedAt: number
  size: number
  chunkCount: number
  versionOfSourceId?: string | null
  versionIndex: number
  supersededBySourceId?: string | null
  supersededAt?: number | null
  assets?: KnowledgeSourceAssets | null
}

export interface KnowledgeSourceReadResult extends KnowledgeSource {
  content: string
}

export interface KnowledgeSourceRefreshInput {
  title?: string | null
  browserMode?: KnowledgeBrowserCaptureMode
  requireSameUrl?: boolean
}

export type KnowledgeSourceDiffLineKind = "context" | "added" | "removed"

export interface KnowledgeSourceDiffLine {
  kind: KnowledgeSourceDiffLineKind
  oldLine?: number | null
  newLine?: number | null
  text: string
}

export interface KnowledgeSourceDiff {
  fromSourceId: string
  toSourceId: string
  fromTitle: string
  toTitle: string
  fromContentHash: string
  toContentHash: string
  addedLines: number
  removedLines: number
  contextLines: number
  truncated: boolean
  lines: KnowledgeSourceDiffLine[]
}

export interface KnowledgeSourceRefreshResult {
  source: KnowledgeSource
  previousSource: KnowledgeSource
  changed: boolean
  diff?: KnowledgeSourceDiff | null
}

export interface KnowledgeSourceExternalRawSyncResult {
  syncedCount: number
  skippedCount: number
  failedCount: number
  errors: string[]
}

export interface KnowledgeSourceVersionHistory {
  rootSourceId: string
  currentSourceId: string
  versions: KnowledgeSource[]
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

export type KnowledgeSourceOcrPageStatus = "pending" | "running" | "succeeded" | "failed"
export type KnowledgeSourceOcrPageStage = "render" | "vision" | "timeout"

export interface KnowledgeSourceOcrPage {
  id: number
  sourceId: string
  kbId: string
  pageNumber: number
  status: KnowledgeSourceOcrPageStatus
  stage?: KnowledgeSourceOcrPageStage | null
  error?: string | null
  modelLabel?: string | null
  attemptCount: number
  createdAt: number
  updatedAt: number
}

export interface KnowledgeSourceImportRun {
  id: string
  kbId: string
  status: KnowledgeSourceImportRunStatus
  backgroundJobId?: string | null
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
export type KnowledgeSourceSimilarityGroupScope = "same_kb" | "cross_kb"

export interface KnowledgeSourceSimilarityGroup {
  id: string
  kind: KnowledgeSourceSimilarityGroupKind
  scope: KnowledgeSourceSimilarityGroupScope
  similarity: number
  fingerprint: string
  sources: KnowledgeSource[]
}

export interface KnowledgeSourceSimilarityDismissInput {
  fingerprint: string
  reason?: string | null
}

export interface KnowledgeSourceSimilarityResolveInput {
  fingerprint: string
  keepSourceId: string
  deleteSourceIds: string[]
}

export interface KnowledgeSourceSimilarityResolveResult {
  keptSourceId: string
  deletedSourceIds: string[]
  dismissed: boolean
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
  superseded: boolean
  latestSourceId?: string | null
  sourceUpdatedAt?: number | null
  noteLastCompiledAt?: number | null
  citedIn?: string[]
}

export interface KnowledgeEvidenceClaim {
  kbId: string
  relPath: string
  noteTitle: string
  sourceId: string
  sourceTitle?: string | null
  originUri?: string | null
  claimIndex: number
  section: string
  claimText: string
  missing: boolean
  stale: boolean
  superseded: boolean
  latestSourceId?: string | null
  sourceUpdatedAt?: number | null
  noteLastCompiledAt?: number | null
}

export interface KnowledgeEvidenceCoverage {
  kbId: string
  compiledNoteCount: number
  notesWithEvidence: number
  notesMissingEvidence: number
  sourceRefCount: number
  staleRefCount: number
  missingRefCount: number
  claimCount: number
  claimsWithEvidence: number
  coverageScore: number
  updatedAt: number
}

export interface KnowledgeEvidenceRebuildResult {
  kbId: string
  scannedCount: number
  indexedRefCount: number
  indexedClaimCount: number
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
  versionOfSourceId?: string | null
  versionIndex: number
  supersededBySourceId?: string | null
  supersededAt?: number | null
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

export interface KnowledgeCompileConfig {
  /** Deprecated — superseded by `modelOverride`. Read-only display concern. */
  agentId?: string | null
  modelOverride?: { primary: { providerId: string; modelId: string }; fallbacks: { providerId: string; modelId: string }[] } | null
}

export interface KnowledgeVisionConfig {
  modelOverride?: { primary: { providerId: string; modelId: string }; fallbacks: { providerId: string; modelId: string }[] } | null
  timeoutSecs: number
  maxTokens: number
  /** Bounded concurrency for the scanned-PDF OCR fallback's per-page calls. */
  ocrConcurrency: number
  /** Page cap for the scanned-PDF OCR fallback. */
  maxOcrPages: number
}

export interface NoteToolsConfig {
  modelOverride?: { primary: { providerId: string; modelId: string }; fallbacks: { providerId: string; modelId: string }[] } | null
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
  /** Copy source text snapshots into an external vault folder (`raw/` or `sources/`). */
  externalRawSync?: KnowledgeExternalRawSyncMode | null
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
  modelOverride?: { primary: { providerId: string; modelId: string }; fallbacks: { providerId: string; modelId: string }[] } | null
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
  modelOverride?: { primary: { providerId: string; modelId: string }; fallbacks: { providerId: string; modelId: string }[] } | null
}
