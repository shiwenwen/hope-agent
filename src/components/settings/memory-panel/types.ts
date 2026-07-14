import {
  User,
  MessageSquareHeart,
  FolderKanban,
  BookOpen,
} from "lucide-react"

// ── Types ─────────────────────────────────────────────────────────

export interface MemoryEntry {
  id: number
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string } | { kind: "project"; id: string }
  content: string
  tags: string[]
  source: string
  sourceSessionId?: string | null
  createdAt: string
  updatedAt: string
  relevanceScore?: number | null
  pinned?: boolean
}

export type MemoryHistoryAction = "add" | "update" | "delete" | "pin" | "unpin" | "import"

export interface MemoryHistoryRecord {
  id: string
  memoryId: number
  action: MemoryHistoryAction
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string } | { kind: "project"; id: string }
  source: string
  sourceSessionId?: string | null
  contentPreview: string
  pinned: boolean
  createdAt: string
}

export interface MemoryHistoryQuery {
  query?: string | null
  actions?: MemoryHistoryAction[] | null
  memoryTypes?: MemoryEntry["memoryType"][] | null
  sources?: string[] | null
  limit?: number | null
  offset?: number | null
}

export interface MemoryHistoryListResponse {
  items: MemoryHistoryRecord[]
  total: number
  totalTruncated?: boolean
}

export type MemoryScope =
  | { kind: "global" }
  | { kind: "agent"; id: string }
  | { kind: "project"; id: string }

export type MemoryLearningMode = "smart" | "review_first" | "manual"
export type MemoryRecallMode = "fast" | "deep"

export interface MemoryRuntimeConfig {
  configVersion: number
  enabled: boolean
  core: {
    enabled: boolean
    totalTokens: number
    hardMaxTokens: number
    globalTokens: number
    agentTokens: number
    projectTokens: number
    protocolTokens: number
    topicReadMaxTokens: number
  }
  recall: {
    enabled: boolean
    userConfigured: boolean
    mode: MemoryRecallMode
    maxTokens: number
    maxSelected: number
    candidateLimit: number
    timeoutMs: number
    includeClaims: boolean
    includeProfile: boolean
    includeProcedures: boolean
    includeGraph: boolean
  }
  deepRecall: {
    enabled: boolean
    timeoutMs: number
    cacheTtlSecs: number
    maxChars: number
    budgetTokens: number
  }
  learning: {
    mode: MemoryLearningMode
    promoteCoreAutomatically: boolean
  }
  rollout: {
    enabled: boolean
    dynamicRecall: boolean
    coreRepository: boolean
    shadowPlan: boolean
  }
  compatibility: {
    legacyStaticMemory: boolean
  }
}

export interface CoreMemoryBudgetStatus {
  configuredTokens: number
  effectiveTokens: number
  contextWindowTokens?: number | null
  modelSafetyLimitTokens?: number | null
  emergencyLimitTokens: number
  limitedBy?: "context_window" | "emergency_guard" | null
}

export interface PendingMemoryCandidate {
  id: string
  content: string
  memoryType: MemoryEntry["memoryType"]
  tags: string[]
  sourceSessionId: string
  agentId: string
  reason:
    | "review_first"
    | "project_scope_missing"
    | "scope_uncertain"
    | "sensitive"
    | "conflict"
    | "core_promotion"
  suggestedScope?: MemoryScope | null
  candidateKind: "memory" | "claim"
  payloadJson?: string | null
  status: "pending" | "approved" | "rejected"
  createdAt: string
  updatedAt: string
}

export interface PendingMemoryCandidatePage {
  items: PendingMemoryCandidate[]
  total: number
  offset: number
  limit: number
  reasonCounts?: Record<string, number>
}

export interface MemoryEpisodeRecord {
  id: string
  scope: MemoryScope
  title: string
  situation: string
  actions: string[]
  outcome: string
  lesson: string
  sourceSessionId?: string | null
  sourceMessageIds: string[]
  successScore: number
  tags: string[]
  status: string
  createdAt: string
  updatedAt: string
}

export interface NewMemoryEpisode {
  scope: MemoryScope
  title: string
  situation: string
  actions?: string[]
  outcome?: string
  lesson?: string
  sourceSessionId?: string | null
  sourceMessageIds?: string[]
  successScore?: number | null
  tags?: string[]
}

export interface MemoryEpisodePatch {
  scope?: MemoryScope | null
  title?: string | null
  situation?: string | null
  actions?: string[] | null
  outcome?: string | null
  lesson?: string | null
  successScore?: number | null
  tags?: string[] | null
}

export interface NewMemoryProcedure {
  scope: MemoryScope
  title: string
  trigger: string
  stepsMarkdown: string
  constraintsMarkdown?: string
  confidence?: number | null
  sourceEpisodeIds?: string[]
  tags?: string[]
}

export interface MemoryProcedurePatch {
  scope?: MemoryScope | null
  title?: string | null
  trigger?: string | null
  stepsMarkdown?: string | null
  constraintsMarkdown?: string | null
  confidence?: number | null
  sourceEpisodeIds?: string[] | null
  tags?: string[] | null
}

export interface MemoryEpisodeQuery {
  scope?: MemoryScope | null
  status?: string | null
  query?: string | null
  sort?: string | null
  limit?: number | null
  offset?: number | null
}

export interface MemoryEpisodeListPage {
  items: MemoryEpisodeRecord[]
  total: number
  totalTruncated?: boolean
}

export interface MemoryProcedureRecord {
  id: string
  scope: MemoryScope
  title: string
  trigger: string
  stepsMarkdown: string
  constraintsMarkdown: string
  confidence: number
  status: string
  sourceEpisodeIds: string[]
  tags: string[]
  createdAt: string
  updatedAt: string
}

export interface MemoryProcedureQuery {
  scope?: MemoryScope | null
  status?: string | null
  query?: string | null
  sort?: string | null
  limit?: number | null
  offset?: number | null
}

export interface MemoryProcedureListPage {
  items: MemoryProcedureRecord[]
  total: number
  totalTruncated?: boolean
}

export interface MemoryExperienceHistoryQuery {
  targetKind?: "episode" | "procedure" | null
  targetId?: string | null
  actions?: MemoryExperienceHistoryRecord["action"][] | null
  scope?: MemoryScope | null
  query?: string | null
  limit?: number | null
  offset?: number | null
}

export interface MemoryExperienceHistoryRecord {
  id: string
  targetKind: "episode" | "procedure"
  targetId: string
  action: "add" | "promote" | "update" | "archive" | "restore" | "restore_import"
  scope: MemoryScope
  titlePreview: string
  contentPreview: string
  createdAt: string
}

export interface MemoryExperienceHistoryListPage {
  items: MemoryExperienceHistoryRecord[]
  total: number
  totalTruncated?: boolean
}

export interface MemorySearchQuery {
  query: string
  types?: string[] | null
  sources?: string[] | null
  scope?: MemoryScope | null
  agentId?: string | null
  limit?: number | null
}

export interface NewMemory {
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string } | { kind: "project"; id: string }
  content: string
  tags: string[]
  source: string
}

export interface MemoryImportPreviewIssue {
  code: string
  message: string
}

export interface MemoryImportPreviewSample {
  memoryType: MemoryEntry["memoryType"]
  scope: MemoryScope
  contentPreview: string
  tags: string[]
  dedupStatus?: "new" | "merge" | "duplicate" | string | null
  dedupExistingId?: number | null
  dedupExistingPreview?: string | null
  dedupScore?: number | null
}

export interface MemoryImportPreview {
  valid: boolean
  format: string
  candidateCount: number
  dedupChecked?: boolean
  likelyNewCount?: number
  likelyMergeCount?: number
  likelyDuplicateCount?: number
  byType: Record<string, number>
  byScope: Record<string, number>
  samples: MemoryImportPreviewSample[]
  issues: MemoryImportPreviewIssue[]
}

export type {
  EmbeddingConfig,
  EmbeddingModelConfig,
  EmbeddingModelTemplate,
  EmbeddingPreset,
  MemoryEmbeddingSelection,
  MemoryEmbeddingSetDefaultResult,
  MemoryEmbeddingState,
} from "@/types/embedding-models"

export interface LocalEmbeddingModel {
  id: string
  name: string
  dimensions: number
  sizeMb: number
  minRamGb: number
  languages: string[]
  downloaded: boolean
}

export interface OllamaEmbeddingModel {
  id: string
  displayName: string
  dimensions: number
  sizeMb: number
  contextWindow: number
  languages: string[]
  minOllamaVersion?: string | null
  installed: boolean
  recommended: boolean
}

export type { AgentInfo } from "@/types/chat"

export interface MemoryStats {
  total: number
  byType: Record<string, number>
  bySource: Record<string, number>
  withEmbedding: number
}

export type MemoryHealthStatus = "ok" | "warning" | "error"
export type MemoryHealthSeverity = "info" | "warning" | "error"

export interface MemoryHealthIssue {
  code: string
  severity: MemoryHealthSeverity
  message: string
  action?: string | null
}

export interface MemoryHealth {
  backendKind: string
  status: MemoryHealthStatus
  checkedAt: string
  quickCheck: string
  totalMemories: number
  memoriesWithActiveEmbedding: number
  memoriesPendingEmbedding: number
  activeEmbeddingSignature?: string | null
  embeddingProviderConfigured: boolean
  embeddingProviderLoaded: boolean
  embeddingProviderDimensions?: number | null
  embeddingProviderMultimodal: boolean
  embeddingProviderBatch: boolean
  vectorRows?: number | null
  ftsRows: number
  ftsMissingRows: number
  claimsTotal: number
  claimsNeedsReview: number
  claimsWithoutEvidence: number
  claimFtsRows: number
  claimFtsMissingRows: number
  evidenceFtsRows?: number
  evidenceFtsMissingRows?: number
  orphanEvidenceRows: number
  orphanClaimLinks: number
  episodesTotal: number
  proceduresTotal: number
  orphanProcedureEpisodeRefs: number
  dreamingRunningRuns: number
  dreamingStaleRuns: number
  dreamingLocks: number
  dreamingStaleLocks: number
  deepResolverActiveClaims?: number
  deepResolverExpiredCandidates?: number
  deepResolverConflictGroups?: number
  deepResolverGroupsToAnalyze?: number
  deepResolverGroupCap?: number
  deepResolverTruncated?: boolean
  deepResolverWouldCallLlm?: boolean
  deepResolverBlockingReasons?: string[]
  externalProvidersEnabled: boolean
  externalProviderCount: number
  externalProviderActiveCount: number
  externalProviders: ExternalMemoryProviderHealth[]
  latestDbSnapshot?: MemoryDbSnapshotArtifact | null
  issues: MemoryHealthIssue[]
}

export type ExternalMemoryProviderKind =
  | "mem0"
  | "zep"
  | "supermemory"
  | "honcho"
  | "hindsight"
  | "open_viking"
  | "custom"

export type ExternalMemorySyncPolicy =
  | "off"
  | "manual"
  | "pull_only"
  | "push_only"
  | "bidirectional"

export type ExternalMemoryProviderDataFlow =
  | "none"
  | "manual"
  | "pull_only"
  | "push_only"
  | "bidirectional"

export type ExternalMemoryProviderSyncBlockReason =
  | "global_disabled"
  | "provider_disabled"
  | "policy_off"
  | "endpoint_missing"
  | "policy_unsupported"
  | "adapter_unavailable"
  | "last_error"

export type ExternalMemoryProviderPreflightAction = "off" | "blocked" | "would_sync"

export type ExternalMemoryProviderSyncStatus =
  | "off"
  | "blocked"
  | "no_runtime_adapter"
  | "succeeded"
  | "failed"

export interface ExternalMemoryProviderHealth {
  id: string
  kind: ExternalMemoryProviderKind
  displayName: string
  enabled: boolean
  syncPolicy: ExternalMemorySyncPolicy
  status: MemoryHealthStatus
  capabilities?: ExternalMemoryProviderCapabilities
  policySupported?: boolean
  policyDataFlow?: ExternalMemoryProviderDataFlow
  runtimeDataFlow?: ExternalMemoryProviderDataFlow
  runtimeSyncEnabled?: boolean
  syncBlocked?: boolean
  syncBlockReasons?: ExternalMemoryProviderSyncBlockReason[]
  sendsQueryContext?: boolean
  sendsLocalMemory?: boolean
  importsExternalMemory?: boolean
  requiresExplicitAction?: boolean
  automaticSync?: boolean
  endpointConfigured: boolean
  lastSyncAt?: string | null
  lastError?: string | null
}

export interface ExternalMemoryProviderCapabilities {
  adapterAvailable: boolean
  requiresEndpoint: boolean
  supportsManual: boolean
  supportsPull: boolean
  supportsPush: boolean
  supportsBidirectional: boolean
}

export interface ExternalMemoryProviderConfig {
  id: string
  kind: ExternalMemoryProviderKind
  displayName: string
  enabled: boolean
  syncPolicy: ExternalMemorySyncPolicy
  endpointConfigured: boolean
  lastSyncAt?: string | null
  lastError?: string | null
}

export interface ExternalMemoryProvidersConfig {
  enabled: boolean
  providers: ExternalMemoryProviderConfig[]
}

export interface ExternalMemoryProviderCredentialInput {
  providerId: string
  endpoint: string
  /** Omit to preserve the stored key; an empty string explicitly clears it. */
  apiKey?: string | null
  subjectId: string
  protocol?: string | null
}

export interface ExternalMemoryProviderCredentialStatus {
  providerId: string
  configured: boolean
  endpointConfigured: boolean
  apiKeyConfigured: boolean
  endpointOrigin?: string | null
  subjectId?: string | null
  protocol?: string | null
  source?: string | null
}

export interface ExternalMemoryProviderPreflight {
  id: string
  kind: ExternalMemoryProviderKind
  displayName: string
  action: ExternalMemoryProviderPreflightAction
  dryRunOnly: boolean
  health: ExternalMemoryProviderHealth
  plannedDataFlow: ExternalMemoryProviderDataFlow
  runtimeDataFlow: ExternalMemoryProviderDataFlow
  plannedSendsQueryContext: boolean
  plannedSendsLocalMemory: boolean
  plannedImportsExternalMemory: boolean
  runtimeSendsQueryContext: boolean
  runtimeSendsLocalMemory: boolean
  runtimeImportsExternalMemory: boolean
  localMemoryCandidateCount: number
}

export interface ExternalMemoryProviderPreflightReport {
  generatedAt: string
  globalEnabled: boolean
  dryRunOnly: boolean
  localMemoryTotal: number
  localMemoryWithEmbedding: number
  statsUnavailable?: boolean
  statsError?: string | null
  runnableProviderCount: number
  blockedProviderCount: number
  providers: ExternalMemoryProviderPreflight[]
}

export interface ExternalMemoryProviderSyncResult {
  id: string
  kind: ExternalMemoryProviderKind
  displayName: string
  status: ExternalMemoryProviderSyncStatus
  externalIoPerformed: boolean
  preflight: ExternalMemoryProviderPreflight
  importedMemoryCount: number
  exportedMemoryCount: number
  updatedMemoryCount: number
  skippedMemoryCount: number
  error?: string | null
}

export interface ExternalMemoryProviderSyncReport {
  generatedAt: string
  globalEnabled: boolean
  externalIoPerformed: boolean
  localMemoryTotal: number
  localMemoryWithEmbedding: number
  statsUnavailable?: boolean
  statsError?: string | null
  runnableProviderCount: number
  blockedProviderCount: number
  executedProviderCount: number
  succeededProviderCount: number
  failedProviderCount: number
  providers: ExternalMemoryProviderSyncResult[]
}

export type MemoryRepairAction =
  | "rebuild_fts"
  | "rebuild_claim_fts"
  | "repair_claim_graph"
  | "repair_experience_graph"
  | "recover_dreaming_state"
  | "create_db_snapshot"

export interface MemoryRepairArtifactFile {
  name: string
  sizeBytes: number
  sha256: string
}

export type MemoryDbSnapshotStatus = "ok" | "no_metadata" | "missing_files" | "size_mismatch"

export type MemoryDbSnapshotRestoreStatus =
  | "ready"
  | "no_metadata"
  | "missing_files"
  | "size_mismatch"
  | "sha256_mismatch"
  | "quick_check_failed"

export type MemoryDbSnapshotFileStatus =
  | "ok"
  | "unverified"
  | "missing"
  | "size_mismatch"
  | "sha256_mismatch"

export interface MemoryDbSnapshotArtifact {
  path: string
  createdAt?: string | null
  status?: MemoryDbSnapshotStatus
  issues?: string[]
  files: MemoryRepairArtifactFile[]
}

export interface MemoryDbSnapshotRestoreFileCheck {
  name: string
  snapshotPath: string
  targetPath: string
  status: MemoryDbSnapshotFileStatus
  expectedSizeBytes: number
  actualSizeBytes?: number | null
  expectedSha256: string
  actualSha256?: string | null
}

export interface MemoryDbSnapshotRestorePreview {
  snapshotPath: string
  currentDbPath: string
  createdAt?: string | null
  status: MemoryDbSnapshotRestoreStatus
  canRestore: boolean
  quickCheck: string
  issues: string[]
  files: MemoryDbSnapshotRestoreFileCheck[]
}

export interface MemoryDbSnapshotRestoreReport {
  restored: boolean
  snapshotPath: string
  rollbackSnapshotPath: string
  rollbackSnapshotFiles: MemoryRepairArtifactFile[]
  preflight: MemoryDbSnapshotRestorePreview
  before: MemoryHealth
  after: MemoryHealth
}

export interface MemoryRepairReport {
  action: MemoryRepairAction
  changed: boolean
  artifactPath?: string | null
  artifactFiles?: MemoryRepairArtifactFile[]
  before: MemoryHealth
  after: MemoryHealth
}

export interface MemoryBackupManifest {
  complete: boolean
  legacyMemoryCount: number
  legacyHistoryCount?: number
  attachmentRefCount: number
  attachmentPayloadCount?: number
  attachmentChunkCount?: number
  attachmentChunkedRefCount?: number
  attachmentExternalRefCount?: number
  attachmentPayloadBytes?: number
  attachmentMissingCount?: number
  claimCount: number
  evidenceCount: number
  claimLinkCount: number
  profileSnapshotCount: number
  episodeCount?: number
  procedureCount?: number
  experienceHistoryCount?: number
  unsupportedSections: string[]
  warnings: string[]
}

export interface MemoryBackupBundle {
  schemaVersion: string
  exportedAt: string
  appVersion: string
  manifest: MemoryBackupManifest
  stats: MemoryStats
  health?: MemoryHealth | null
  configManifest: unknown
  legacyMemories: MemoryEntry[]
  legacyHistory?: MemoryHistoryRecord[]
  attachmentPayloads?: unknown[]
  attachmentPayloadChunks?: unknown[]
  attachmentExternalPayloads?: unknown[]
  legacyMarkdown: string
  claims: unknown[]
  profileSnapshots: unknown[]
  episodes?: MemoryEpisodeRecord[]
  procedures?: MemoryProcedureRecord[]
  experienceHistory?: unknown[]
}

export interface MemoryEncryptedBackupBundle {
  schemaVersion: string
  exportedAt: string
  appVersion: string
  plaintextSchemaVersion: string
  kdf: {
    name: string
    iterations: number
    saltBase64: string
  }
  cipher: {
    name: string
    nonceBase64: string
    ciphertextBase64: string
    macBase64: string
  }
}

export interface MemoryBackupPreviewIssue {
  severity: MemoryHealthSeverity
  code: string
  message: string
}

export interface MemoryBackupClaimConflictExample {
  incomingClaimId: string
  existingClaimId: string
  scope: string
  claimType: string
  subject: string
  predicate: string
  incomingObject: string
  existingObject: string
  incomingContent: string
  existingContent: string
}

export interface MemoryBackupClaimRestorePlan {
  total: number
  existingById: number
  exactMatches: number
  importCandidates: number
  conflictingCandidates: number
  needsReviewCandidates: number
  archivedCandidates: number
  supersededCandidates: number
  expiredCandidates: number
  manualEvidenceRows: number
  byType: Record<string, number>
  byStatus: Record<string, number>
  conflictExamples: MemoryBackupClaimConflictExample[]
  previewOnly: boolean
}

export interface MemoryBackupProfileRestorePlan {
  total: number
  matchingScopes: number
  exactMatches: number
  importCandidates: number
  conflictingScopeCandidates: number
  byScopeType: Record<string, number>
  previewOnly: boolean
}

export interface MemoryBackupImportPreview {
  valid: boolean
  schemaVersion?: string | null
  exportedAt?: string | null
  appVersion?: string | null
  sourceManifest?: MemoryBackupManifest | null
  currentStats: MemoryStats
  legacyMemoryCount: number
  legacyExactMatches: number
  legacyImportCandidates: number
  legacyDuplicateInBundle: number
  legacyHistoryCount?: number
  legacyHistoryRestorable?: number
  legacyHistorySkippedUnmapped?: number
  attachmentRefCount: number
  attachmentPayloadCount: number
  attachmentChunkCount: number
  attachmentChunkedRefCount: number
  attachmentExternalRefCount: number
  attachmentExternalAvailableCount: number
  attachmentPayloadBytes: number
  attachmentMissingCount: number
  claimCount: number
  claimIdMatches: number
  claimRestorePlan: MemoryBackupClaimRestorePlan
  evidenceCount: number
  claimLinkCount: number
  profileSnapshotCount: number
  profileRestorePlan: MemoryBackupProfileRestorePlan
  episodeCount?: number
  episodeIdMatches?: number
  episodeExactMatches?: number
  episodeImportCandidates?: number
  procedureCount?: number
  procedureIdMatches?: number
  procedureExactMatches?: number
  procedureImportCandidates?: number
  experienceHistoryCount?: number
  experienceHistoryRestorable?: number
  experienceHistorySkippedUnmapped?: number
  unsupportedSections: string[]
  issues: MemoryBackupPreviewIssue[]
  nextSteps: string[]
}

export interface MemoryBackupRestoreResult {
  preview: MemoryBackupImportPreview
  importResult: {
    created: number
    skippedDuplicate: number
    failed: number
    errors: string[]
  }
  attemptedLegacyMemories: number
  skippedExactMatches: number
  skippedDuplicateInBundle: number
  skippedAttachmentRefs: number
  restoredAttachments: number
  restoredLegacyHistory?: number
  skippedLegacyHistoryUnmapped?: number
  previewOnlyClaims: number
  previewOnlyProfileSnapshots: number
}

export interface MemoryBackupStructuredRestoreResult {
  preview: MemoryBackupImportPreview
  restoredClaims: number
  restoredClaimsNeedingReview: number
  skippedClaimIdMatches: number
  skippedClaimExactMatches: number
  restoredEvidenceRows: number
  restoredClaimLinks: number
  skippedClaimLinks: number
  failedClaims: number
  restoredProfileSnapshots: number
  skippedProfileExactMatches: number
  skippedProfileScopeConflicts: number
  failedProfileSnapshots: number
  restoredEpisodes?: number
  skippedEpisodeIdMatches?: number
  skippedEpisodeExactMatches?: number
  failedEpisodes?: number
  restoredProcedures?: number
  skippedProcedureIdMatches?: number
  skippedProcedureExactMatches?: number
  failedProcedures?: number
  restoredExperienceHistory?: number
  skippedExperienceHistoryUnmapped?: number
  errors: string[]
}

export interface MemoryBackupStructuredRestoreOptions {
  restoreClaims: boolean
  restoreProfileSnapshots: boolean
  restoreEpisodes: boolean
  restoreProcedures: boolean
  restoreExperienceHistory: boolean
  allowProfileScopeConflicts: boolean
}

export function buildMemoryBackupStructuredRestoreOptions(
  allowProfileScopeConflicts: boolean,
): MemoryBackupStructuredRestoreOptions {
  return {
    restoreClaims: true,
    restoreProfileSnapshots: true,
    restoreEpisodes: true,
    restoreProcedures: true,
    restoreExperienceHistory: true,
    allowProfileScopeConflicts,
  }
}

export type MemoryView = "list" | "add" | "edit" | "embedding"

// ── Constants ─────────────────────────────────────────────────────

export const MEMORY_TYPES = ["user", "feedback", "project", "reference"] as const

export const MEMORY_SOURCE_FILTERS = [
  "user",
  "auto",
  "auto-reflect",
  "auto-claim",
  "import",
] as const

export type MemorySourceFilter = (typeof MEMORY_SOURCE_FILTERS)[number]

export const MEMORY_SOURCE_FILTER_SOURCES: Record<MemorySourceFilter, string[]> = {
  user: ["user"],
  auto: ["auto", "flush"],
  "auto-reflect": ["auto-reflect"],
  "auto-claim": ["auto-claim"],
  import: ["import"],
}

export interface MemoryBackupPassphrasePolicy {
  score: number
  accepted: boolean
  reasonKey?: string
  reasonDefault?: string
}

export function evaluateMemoryBackupPassphrase(passphrase: string): MemoryBackupPassphrasePolicy {
  const charCount = Array.from(passphrase).length
  const compact = passphrase.toLowerCase().replace(/[^a-z0-9]/g, "")
  const weakPatterns = ["password", "passphrase", "qwerty", "123456", "hopeagent", "memorybackup"]
  const uniqueChars = new Set(Array.from(passphrase)).size
  const hasLower = /[a-z]/.test(passphrase)
  const hasUpper = /[A-Z]/.test(passphrase)
  const hasDigit = /\d/.test(passphrase)
  const hasSymbol = /[^A-Za-z0-9\s]/.test(passphrase)
  const hasSpace = /\s/.test(passphrase)
  const characterClasses = [hasLower, hasUpper, hasDigit, hasSymbol || hasSpace].filter(Boolean)
    .length
  const wordCount = passphrase.split(/\s+/).filter((word) => Array.from(word).length >= 3).length
  const longPhrase = charCount >= 24 && wordCount >= 4
  const longMixed = charCount >= 16 && characterClasses >= 2 && uniqueChars >= 8
  const mixed = charCount >= 12 && characterClasses >= 3 && uniqueChars >= 8
  let score = 0
  if (charCount >= 12) score += 1
  if (charCount >= 16) score += 1
  if (charCount >= 24) score += 1
  if (characterClasses >= 2) score += 1
  if (characterClasses >= 3) score += 1
  if (wordCount >= 4) score += 1
  score = Math.min(score, 5)

  if (charCount === 0) {
    return {
      score: 0,
      accepted: false,
      reasonKey: "settings.memoryBackupEncryptedPassphraseRequired",
      reasonDefault: "Enter a backup passphrase",
    }
  }
  if (charCount < 12) {
    return {
      score,
      accepted: false,
      reasonKey: "settings.memoryBackupEncryptedPassphraseTooShort",
      reasonDefault: "Use at least 12 characters for the backup passphrase",
    }
  }
  if (weakPatterns.some((pattern) => compact.includes(pattern))) {
    return {
      score: Math.min(score, 2),
      accepted: false,
      reasonKey: "settings.memoryBackupEncryptedPassphraseCommon",
      reasonDefault: "Avoid common words or patterns in the backup passphrase",
    }
  }
  if (uniqueChars < 6) {
    return {
      score: Math.min(score, 2),
      accepted: false,
      reasonKey: "settings.memoryBackupEncryptedPassphraseRepetitive",
      reasonDefault: "Use a less repetitive backup passphrase",
    }
  }
  if (!longPhrase && !longMixed && !mixed) {
    return {
      score: Math.min(score, 3),
      accepted: false,
      reasonKey: "settings.memoryBackupEncryptedPassphraseVariety",
      reasonDefault: "Use more variety, or a longer four-word phrase",
    }
  }
  return { score, accepted: true }
}

export const MEMORY_TYPE_ICONS: Record<string, typeof User> = {
  user: User,
  feedback: MessageSquareHeart,
  project: FolderKanban,
  reference: BookOpen,
}
