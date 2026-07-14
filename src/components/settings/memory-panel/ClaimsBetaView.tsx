import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { openExternalUrl } from "@/lib/openExternalUrl"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Button } from "@/components/ui/button"
import { SearchInput } from "@/components/ui/search-input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  AlertTriangle,
  Archive,
  BookmarkPlus,
  CheckCircle2,
  Copy,
  DatabaseZap,
  ExternalLink,
  History,
  Lightbulb,
  Link2,
  Loader2,
  Search,
  X,
} from "lucide-react"
import ClaimReviewActions from "@/components/dashboard/dreaming/ClaimReviewActions"
import { requestChatFocus } from "@/components/chat/chatFocus"
import type { AgentInfo } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import {
  DEFAULT_CLAIM_SCHEMA,
  claimTypeFilterValues,
  confidenceSourceFilterValues,
  evidenceClassFilterValues,
  evidenceSourceFilterValues,
  normalizeClaimSchema,
  normalizeClaimTypeFilter,
  type ClaimSchemaMetadata,
  type ClaimRecord,
  type ClaimTypeFilter,
} from "./claimTypes"
import {
  claimListBackendSortArg,
  claimListSortRuntimeMode,
  claimSearchDiagnostics,
  type ClaimSearchRankSignal,
} from "./claimSearchExplain"
import {
  memoryFocusUrl,
  setMemoryFocusUrl,
  type ClaimFocusFilters,
  type MemoryFocusTarget,
} from "./memoryFocus"
import { requestMemoryScopeFocus } from "./scopeFocus"
import { conflictResolutionNote } from "./claimConflictAudit"
import { claimClipboardErrorToast } from "./claimClipboardFeedback"
import {
  claimOwnerOperationErrorDetail,
  claimOwnerOperationErrorToast,
  type ClaimOwnerOperationErrorToast,
} from "./claimOwnerOperationFeedback"

interface EvidenceRecord {
  id: string
  sourceType: string
  evidenceClass: string
  sourceId: string
  sessionId?: string | null
  messageId?: string | null
  filePath?: string | null
  url?: string | null
  quote?: string | null
  createdAt: string
}

interface ClaimLink {
  claimId: string
  memoryId: number
  syncMode: string
}

interface ClaimDetail {
  claim: ClaimRecord
  evidence: EvidenceRecord[]
  links: ClaimLink[]
}

interface ClaimListPage {
  items: ClaimRecord[]
  total: number
  totalTruncated?: boolean
}

interface ClaimGraphProjection {
  centerClaimId: string
  nodes: ClaimGraphNode[]
  edges: ClaimGraphEdge[]
  truncated?: boolean
}

interface ClaimGraphNode {
  id: string
  label: string
  entityType: string
  scopeType: string
  scopeId?: string | null
  claimCount: number
}

interface ClaimGraphEdge {
  id: string
  source: string
  target: string
  predicate: string
  claimId: string
  content: string
  status: string
  confidence: number
  salience: number
  validFrom?: string | null
  validUntil?: string | null
}

interface ClaimFocus extends ClaimFocusFilters {
  nonce: number
  selectedId?: string | null
}

interface ClaimsBetaViewProps {
  focus?: ClaimFocus | null
}

// Mirrors ha-core backfill types (camelCase).
interface BackfillSummary {
  totalMemories: number
  alreadyLinked: number
  candidates: number
  autoActive: number
  needsReview: number
}
interface BackfillCandidatePreview {
  memoryId: number
  scopeType: string
  scopeId?: string | null
  claimType: string
  content: string
  confidence: number
  salience: number
  pinned: boolean
  proposedStatus: string
}
interface BackfillPlan {
  summary: BackfillSummary
  candidates: BackfillCandidatePreview[]
  previewTruncated: boolean
}
interface BackfillApplyResult {
  created: number
  autoActive: number
  needsReview: number
  skipped: number
  failed: number
}

interface DreamingRunRecord {
  id: string
  trigger: string
  phase: string
  status: string
  startedAt: string
  finishedAt?: string | null
  durationMs: number
  candidatesScanned: number
  candidatesNominated: number
  promotedCount: number
  decisionCount: number
  diaryPath?: string | null
  note?: string | null
}

interface DreamingDecisionRecord {
  id: string
  decisionType: string
  targetType: string
  targetId?: string | null
  score?: number | null
  rationale: string
  beforeJson?: string | null
  afterJson?: string | null
  createdAt: string
}

interface DreamingRunDetail {
  run: DreamingRunRecord
  decisions: DreamingDecisionRecord[]
}

interface DreamingDecisionListItem extends DreamingDecisionRecord {
  runId: string
  runTrigger: string
  runPhase: string
  runStatus: string
  content?: string | null
  scopeType?: string | null
  scopeId?: string | null
}

interface DreamingDecisionListResponse {
  items: DreamingDecisionListItem[]
  total: number
  totalTruncated?: boolean
}

interface ReviewHistoryItem {
  id: string
  decisionType: string
  targetType: string
  targetId?: string | null
  scopeType?: string | null
  scopeId?: string | null
  trigger: string
  phase: string
  status: string
  rationale: string
  content?: string | null
  createdAt: string
}

type ReviewHistoryTimeRange = "all" | "7d" | "30d"
type ClaimListSort =
  | "relevance"
  | "updated_desc"
  | "created_desc"
  | "created_asc"
  | "confidence_desc"
  | "confidence_asc"
  | "salience_desc"
  | "salience_asc"

interface ReviewHistoryFilterPreset {
  id: string
  decisionType: string
  timeRange: ReviewHistoryTimeRange
  scopeFilter: string
  query: string
  updatedAt: number
}

interface ClaimListFilterPreset {
  id: string
  statusFilter: string
  claimTypeFilter: string
  scopeFilter: string
  confidenceSourceFilter: string
  evidenceClassFilter: string
  evidenceSourceFilter: string
  sort: ClaimListSort
  query: string
  updatedAt: number
}

const REVIEW_HISTORY_DECISION_TYPES = [
  "approve",
  "edit",
  "reject",
  "expire",
  "move_scope",
  "flag",
  "forget",
  "forget_permanent",
  "merge",
  "needs_review",
  "pin",
  "unpin",
] as const

const REVIEW_HISTORY_TIME_RANGES: ReviewHistoryTimeRange[] = ["all", "7d", "30d"]
const CLAIM_LIST_SORT_VALUES: ClaimListSort[] = [
  "relevance",
  "updated_desc",
  "created_desc",
  "created_asc",
  "confidence_desc",
  "confidence_asc",
  "salience_desc",
  "salience_asc",
]
const CLAIM_LIST_DEFAULT_PAGE_SIZE = 200
const CLAIM_LIST_SEARCH_PAGE_SIZE = 500
const CLAIM_LIST_MAX_DEEPLINK_LOAD = 2000
const REVIEW_HISTORY_PAGE_SIZE = 50
const REVIEW_HISTORY_EXPORT_PAGE_SIZE = 200
const REVIEW_HISTORY_PRESET_LIMIT = 6
const REVIEW_HISTORY_PRESET_STORAGE_KEY = "hope.memory.reviewHistoryFilterPresets.v1"
const CLAIM_LIST_PRESET_LIMIT = 6
const CLAIM_LIST_PRESET_STORAGE_KEY = "hope.memory.claimListFilterPresets.v1"

const uniqueClaimRecords = (records: ClaimRecord[]): ClaimRecord[] => {
  const seen = new Set<string>()
  const result: ClaimRecord[] = []
  for (const record of records) {
    if (seen.has(record.id)) continue
    seen.add(record.id)
    result.push(record)
  }
  return result
}

const appendClaimRecords = (previous: ClaimRecord[], next: ClaimRecord[]): ClaimRecord[] => {
  const seen = new Set(previous.map((record) => record.id))
  const merged = [...previous]
  for (const record of next) {
    if (seen.has(record.id)) continue
    seen.add(record.id)
    merged.push(record)
  }
  return merged
}

const STATUS_DOT: Record<string, string> = {
  active: "bg-emerald-500",
  superseded: "bg-amber-500",
  expired: "bg-muted-foreground/50",
  archived: "bg-muted-foreground/50",
  needs_review: "bg-sky-500",
}

const LOW_CONFIDENCE_THRESHOLD = 0.6
const HIGH_SALIENCE_THRESHOLD = 0.7
type EvidenceClassFilter = string
type EvidenceSourceFilter = string
type ConfidenceSourceFilter = string
const REVIEW_BUCKET_ORDER = [
  "conflict",
  "lowConfidence",
  "highImpact",
  "personal",
  "other",
] as const
type ReviewBucketKey = (typeof REVIEW_BUCKET_ORDER)[number]
type ClaimConflictIndex = Map<string, Set<string>>
type ConflictMatchKind = "active" | "review"
type ConflictSuggestionKey = "useCurrent" | "keepExisting" | "compare"
type ClaimTrustKey = "userCorrected" | "userConfirmed" | "sourceBacked" | "inferred" | "weak"
const REVIEW_RISK_KEYS = [
  "conflict",
  "lowConfidence",
  "inferred",
  "highImpact",
  "personal",
  "broadScope",
  "projectScoped",
  "timeBound",
  "pendingConfirmation",
] as const
type ReviewRiskKey = (typeof REVIEW_RISK_KEYS)[number]
const REVIEW_BUCKET_KEY_SET = new Set<string>(REVIEW_BUCKET_ORDER)
const REVIEW_RISK_KEY_SET = new Set<string>(REVIEW_RISK_KEYS)

interface ReviewProjection {
  primary: ReviewBucketKey
  risks: ReviewRiskKey[]
}

interface ClaimConflictMatch {
  claim: ClaimRecord
  kind: ConflictMatchKind
}

interface ClaimConflictInsight {
  suggestion: ConflictSuggestionKey
  otherObjects: string[]
  activeCount: number
  strongest: ClaimRecord | null
}

interface ClaimConflictSummary {
  claimId: string
  conflictCount: number
  activeCount: number
  needsReviewCount: number
  examples?: ClaimConflictExample[]
}

interface ClaimConflictExample {
  claimId: string
  status: string
  object: string
  content: string
  confidence: number
  salience: number
}

interface ClaimEvidenceSummary {
  claimId: string
  evidenceCount: number
  confirmedCount: number
  sourceBackedCount: number
  inferredCount: number
  trust: ClaimTrustKey
}

interface ClaimReviewSummary {
  claimId: string
  primary: ReviewBucketKey
  risks: ReviewRiskKey[]
  conflictCount: number
}

type ClaimListSummaryErrorSource = "activeConflict" | "conflict" | "review" | "evidence"

interface EvidenceTrustStats {
  confirmed: number
  inferred: number
  sourceBacked: number
}

const REVIEW_BUCKET_TONE: Record<ReviewBucketKey, string> = {
  conflict: "bg-red-500",
  lowConfidence: "bg-amber-500",
  highImpact: "bg-rose-500",
  personal: "bg-sky-500",
  other: "bg-muted-foreground/50",
}

const REVIEW_RISK_TONE: Record<ReviewRiskKey, string> = {
  conflict: "border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-300",
  lowConfidence: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  inferred: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  highImpact: "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300",
  personal: "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300",
  broadScope: "border-violet-500/30 bg-violet-500/10 text-violet-700 dark:text-violet-300",
  projectScoped: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  timeBound: "border-cyan-500/30 bg-cyan-500/10 text-cyan-700 dark:text-cyan-300",
  pendingConfirmation: "border-muted-foreground/20 bg-secondary/70 text-muted-foreground",
}

const EVIDENCE_CLASS_LABEL_KEYS: Record<string, string> = {
  manual_correction: "manualCorrection",
  user_confirmed: "userConfirmed",
  explicit_user_statement: "explicitUserStatement",
  project_artifact_fact: "projectArtifactFact",
  assistant_inferred: "assistantInferred",
  behavioral_pattern: "behavioralPattern",
}

function normalizeFilterValue(value: string | null | undefined, allowed: string[]): string {
  return allowed.includes(value ?? "") ? (value as string) : "all"
}

function normalizeEvidenceClassFilter(
  value: string | null | undefined,
  schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA,
): EvidenceClassFilter {
  return normalizeFilterValue(value, evidenceClassFilterValues(schema))
}

function normalizeEvidenceSourceFilter(
  value: string | null | undefined,
  schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA,
): EvidenceSourceFilter {
  return normalizeFilterValue(value, evidenceSourceFilterValues(schema))
}

function normalizeConfidenceSourceFilter(
  value: string | null | undefined,
  schema: ClaimSchemaMetadata = DEFAULT_CLAIM_SCHEMA,
): ConfidenceSourceFilter {
  return normalizeFilterValue(value, confidenceSourceFilterValues(schema))
}

function normalizeReviewHistoryDecisionFilter(value: string | null | undefined): string {
  return value && value.trim() ? value.trim() : "all"
}

function normalizeReviewHistoryTimeRange(value: string | null | undefined): ReviewHistoryTimeRange {
  return REVIEW_HISTORY_TIME_RANGES.includes(value as ReviewHistoryTimeRange)
    ? (value as ReviewHistoryTimeRange)
    : "all"
}

function reviewHistoryPresetId(
  decisionType: string,
  timeRange: ReviewHistoryTimeRange,
  scopeFilter: string,
  query: string,
): string {
  return [decisionType, timeRange, scopeFilter, normalizeClaimSearch(query)].join("|")
}

function normalizeReviewHistoryPreset(raw: unknown): ReviewHistoryFilterPreset | null {
  if (!raw || typeof raw !== "object") return null
  const value = raw as Record<string, unknown>
  const decisionType = normalizeReviewHistoryDecisionFilter(
    typeof value.decisionType === "string" ? value.decisionType : null,
  )
  const timeRange = normalizeReviewHistoryTimeRange(
    typeof value.timeRange === "string" ? value.timeRange : null,
  )
  const scopeFilter =
    typeof value.scopeFilter === "string" && value.scopeFilter.trim()
      ? value.scopeFilter.trim()
      : "all"
  const query = typeof value.query === "string" ? value.query.trim().slice(0, 200) : ""
  const updatedAt = Number(value.updatedAt)
  return {
    id: reviewHistoryPresetId(decisionType, timeRange, scopeFilter, query),
    decisionType,
    timeRange,
    scopeFilter,
    query,
    updatedAt: Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : Date.now(),
  }
}

function loadReviewHistoryFilterPresets(): ReviewHistoryFilterPreset[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(REVIEW_HISTORY_PRESET_STORAGE_KEY)
    const parsed = raw ? JSON.parse(raw) : []
    if (!Array.isArray(parsed)) return []
    const deduped = new Map<string, ReviewHistoryFilterPreset>()
    for (const item of parsed) {
      const preset = normalizeReviewHistoryPreset(item)
      if (!preset) continue
      const existing = deduped.get(preset.id)
      if (!existing || existing.updatedAt < preset.updatedAt) {
        deduped.set(preset.id, preset)
      }
    }
    return [...deduped.values()]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, REVIEW_HISTORY_PRESET_LIMIT)
  } catch {
    return []
  }
}

function persistReviewHistoryFilterPresets(presets: ReviewHistoryFilterPreset[]) {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      REVIEW_HISTORY_PRESET_STORAGE_KEY,
      JSON.stringify(presets.slice(0, REVIEW_HISTORY_PRESET_LIMIT)),
    )
  } catch {
    // localStorage may be unavailable in private / restricted contexts.
  }
}

function claimListPresetId(input: {
  statusFilter: string
  claimTypeFilter: string
  scopeFilter: string
  confidenceSourceFilter: string
  evidenceClassFilter: string
  evidenceSourceFilter: string
  sort: string
  query: string
}): string {
  return [
    input.statusFilter || "all",
    input.claimTypeFilter || "all",
    input.scopeFilter || "all",
    input.confidenceSourceFilter || "all",
    input.evidenceClassFilter || "all",
    input.evidenceSourceFilter || "all",
    normalizeClaimListSort(input.sort),
    normalizeClaimSearch(input.query),
  ].join("|")
}

function boundedPresetString(value: unknown, fallback = "all"): string {
  if (typeof value !== "string") return fallback
  const trimmed = value.trim().slice(0, 200)
  return trimmed || fallback
}

function normalizeClaimListPreset(raw: unknown): ClaimListFilterPreset | null {
  if (!raw || typeof raw !== "object") return null
  const value = raw as Record<string, unknown>
  const preset = {
    statusFilter: boundedPresetString(value.statusFilter),
    claimTypeFilter: boundedPresetString(value.claimTypeFilter),
    scopeFilter: boundedPresetString(value.scopeFilter),
    confidenceSourceFilter: boundedPresetString(value.confidenceSourceFilter),
    evidenceClassFilter: boundedPresetString(value.evidenceClassFilter),
    evidenceSourceFilter: boundedPresetString(value.evidenceSourceFilter),
    sort: normalizeClaimListSort(typeof value.sort === "string" ? value.sort : "relevance"),
    query: typeof value.query === "string" ? value.query.trim().slice(0, 200) : "",
  }
  const updatedAt = Number(value.updatedAt)
  return {
    id: claimListPresetId(preset),
    ...preset,
    updatedAt: Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : Date.now(),
  }
}

function loadClaimListFilterPresets(): ClaimListFilterPreset[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(CLAIM_LIST_PRESET_STORAGE_KEY)
    const parsed = raw ? JSON.parse(raw) : []
    if (!Array.isArray(parsed)) return []
    const deduped = new Map<string, ClaimListFilterPreset>()
    for (const item of parsed) {
      const preset = normalizeClaimListPreset(item)
      if (!preset) continue
      const existing = deduped.get(preset.id)
      if (!existing || existing.updatedAt < preset.updatedAt) {
        deduped.set(preset.id, preset)
      }
    }
    return [...deduped.values()]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, CLAIM_LIST_PRESET_LIMIT)
  } catch {
    return []
  }
}

function persistClaimListFilterPresets(presets: ClaimListFilterPreset[]) {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      CLAIM_LIST_PRESET_STORAGE_KEY,
      JSON.stringify(presets.slice(0, CLAIM_LIST_PRESET_LIMIT)),
    )
  } catch {
    // localStorage may be unavailable in private / restricted contexts.
  }
}

function scopeFilterValue(
  scopeType: string | null | undefined,
  scopeId: string | null | undefined,
): string {
  if (scopeType === "global") return "global"
  if ((scopeType === "agent" || scopeType === "project") && scopeId) {
    return `${scopeType}:${scopeId}`
  }
  return "all"
}

function scopeFilterArgs(value: string): Record<string, string> {
  if (value === "global") return { scopeType: "global" }
  const separator = value.indexOf(":")
  if (separator <= 0) return {}
  const scopeType = value.slice(0, separator)
  const scopeId = value.slice(separator + 1)
  if ((scopeType === "agent" || scopeType === "project") && scopeId) {
    return { scopeType, scopeId }
  }
  return {}
}

function scopeFilterFocus(value: string): Pick<ClaimFocusFilters, "scopeType" | "scopeId"> {
  if (value === "global") return { scopeType: "global", scopeId: null }
  const separator = value.indexOf(":")
  if (separator <= 0) return { scopeType: null, scopeId: null }
  const scopeType = value.slice(0, separator)
  const scopeId = value.slice(separator + 1)
  if ((scopeType === "agent" || scopeType === "project") && scopeId) {
    return { scopeType, scopeId }
  }
  return { scopeType: null, scopeId: null }
}

function reviewHistoryScopeFilterFocus(
  value: string,
): Pick<ClaimFocusFilters, "reviewHistoryScopeType" | "reviewHistoryScopeId"> {
  const scope = scopeFilterFocus(value)
  return {
    reviewHistoryScopeType: scope.scopeType,
    reviewHistoryScopeId: scope.scopeId,
  }
}

function timeValue(value: string): number {
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

function formatActivityTime(value: string): string {
  const parsed = timeValue(value)
  if (!parsed) return value
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(parsed))
}

function sortClaimsNewestFirst(a: ClaimRecord, b: ClaimRecord): number {
  const updated = timeValue(b.updatedAt) - timeValue(a.updatedAt)
  return updated !== 0 ? updated : a.id.localeCompare(b.id)
}

function normalizeClaimListSort(value: string | null | undefined): ClaimListSort {
  return CLAIM_LIST_SORT_VALUES.includes(value as ClaimListSort)
    ? (value as ClaimListSort)
    : "relevance"
}

function normalizeClaimLoadedTarget(value: number | null | undefined): number | null {
  if (!Number.isFinite(value ?? NaN) || !value || value <= 0) return null
  return Math.min(Math.floor(value), CLAIM_LIST_MAX_DEEPLINK_LOAD)
}

function compareClaimsForSort(sort: ClaimListSort, a: ClaimRecord, b: ClaimRecord): number {
  const fallback = sortClaimsNewestFirst(a, b)
  if (sort === "created_desc") {
    const created = timeValue(b.createdAt) - timeValue(a.createdAt)
    return created !== 0 ? created : fallback
  }
  if (sort === "created_asc") {
    const created = timeValue(a.createdAt) - timeValue(b.createdAt)
    return created !== 0 ? created : fallback
  }
  if (sort === "confidence_desc") {
    const confidence = b.confidence - a.confidence
    return confidence !== 0 ? confidence : fallback
  }
  if (sort === "confidence_asc") {
    const confidence = a.confidence - b.confidence
    return confidence !== 0 ? confidence : fallback
  }
  if (sort === "salience_desc") {
    const salience = b.salience - a.salience
    return salience !== 0 ? salience : fallback
  }
  if (sort === "salience_asc") {
    const salience = a.salience - b.salience
    return salience !== 0 ? salience : fallback
  }
  return fallback
}

function normalizeClaimSearch(value: string): string {
  return value.trim().toLocaleLowerCase()
}

function claimMatchesSearch(claim: ClaimRecord, query: string, scopeLabel: string): boolean {
  const terms = normalizeClaimSearch(query).split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const haystack = normalizeClaimSearch(
    [
      claim.content,
      claim.claimType,
      claim.status,
      claim.scopeType,
      claim.scopeId ?? "",
      scopeLabel,
      claim.subject,
      claim.predicate,
      claim.object,
      claim.confidenceSource,
      ...(claim.tags ?? []),
    ].join(" "),
  )
  return terms.every((term) => haystack.includes(term))
}

function claimConflictKey(claim: ClaimRecord): string {
  return [
    claim.scopeType,
    claim.scopeId ?? "",
    claim.claimType,
    claim.subject.trim().toLowerCase(),
    claim.predicate.trim().toLowerCase(),
  ].join("\u001f")
}

function claimObjectKey(claim: ClaimRecord): string {
  return claim.object.trim().toLowerCase()
}

function buildClaimConflictIndex(claims: ClaimRecord[]): ClaimConflictIndex {
  const index: ClaimConflictIndex = new Map()
  for (const claim of claims) {
    const key = claimConflictKey(claim)
    const objects = index.get(key) ?? new Set<string>()
    objects.add(claimObjectKey(claim))
    index.set(key, objects)
  }
  return index
}

function visibleConflictKeys(claims: ClaimRecord[]): Set<string> {
  const index = buildClaimConflictIndex(claims)
  return new Set([...index.entries()].filter(([, objects]) => objects.size > 1).map(([key]) => key))
}

function conflictsWithIndex(claim: ClaimRecord, index: ClaimConflictIndex): boolean {
  const objects = index.get(claimConflictKey(claim))
  return !!objects && objects.size > 0 && !objects.has(claimObjectKey(claim))
}

function isReviewBucketKey(value: string): value is ReviewBucketKey {
  return REVIEW_BUCKET_KEY_SET.has(value)
}

function isReviewRiskKey(value: string): value is ReviewRiskKey {
  return REVIEW_RISK_KEY_SET.has(value)
}

function reviewProjectionFromSummary(summary: ClaimReviewSummary): ReviewProjection {
  const primary = isReviewBucketKey(summary.primary) ? summary.primary : "other"
  const risks = summary.risks.filter(isReviewRiskKey)
  return {
    primary,
    risks: risks.length > 0 ? risks : ["pendingConfirmation"],
  }
}

function reviewBucketForClaim(
  claim: ClaimRecord,
  conflictKeys: Set<string> = new Set(),
  activeConflictIndex: ClaimConflictIndex = new Map(),
  conflictSummaries: ReadonlyMap<string, ClaimConflictSummary> = new Map(),
  personalClaimTypes: ReadonlySet<string> = new Set(DEFAULT_CLAIM_SCHEMA.profileClaimTypes),
  reviewSummaries: ReadonlyMap<string, ClaimReviewSummary> = new Map(),
): ReviewBucketKey {
  const reviewSummary = reviewSummaries.get(claim.id)
  if (reviewSummary) return reviewProjectionFromSummary(reviewSummary).primary
  const summary = conflictSummaries.get(claim.id)
  if (
    (summary?.conflictCount ?? 0) > 0 ||
    conflictKeys.has(claimConflictKey(claim)) ||
    conflictsWithIndex(claim, activeConflictIndex)
  ) {
    return "conflict"
  }
  if (claim.confidence < LOW_CONFIDENCE_THRESHOLD) return "lowConfidence"
  if (claim.salience >= HIGH_SALIENCE_THRESHOLD) return "highImpact"
  if (personalClaimTypes.has(claim.claimType)) return "personal"
  return "other"
}

function reviewProjectionForClaim(
  claim: ClaimRecord,
  conflictKeys: Set<string> = new Set(),
  activeConflictIndex: ClaimConflictIndex = new Map(),
  conflictSummaries: ReadonlyMap<string, ClaimConflictSummary> = new Map(),
  personalClaimTypes: ReadonlySet<string> = new Set(DEFAULT_CLAIM_SCHEMA.profileClaimTypes),
  reviewSummaries: ReadonlyMap<string, ClaimReviewSummary> = new Map(),
): ReviewProjection {
  const reviewSummary = reviewSummaries.get(claim.id)
  if (reviewSummary) return reviewProjectionFromSummary(reviewSummary)
  const primary = reviewBucketForClaim(
    claim,
    conflictKeys,
    activeConflictIndex,
    conflictSummaries,
    personalClaimTypes,
    reviewSummaries,
  )
  const risks: ReviewRiskKey[] = []
  const addRisk = (key: ReviewRiskKey) => {
    if (!risks.includes(key)) risks.push(key)
  }

  if (primary === "conflict") addRisk("conflict")
  if (claim.confidence < LOW_CONFIDENCE_THRESHOLD) addRisk("lowConfidence")
  if (claim.confidenceSource === "derived" && claim.confidence < LOW_CONFIDENCE_THRESHOLD) {
    addRisk("inferred")
  }
  if (claim.salience >= HIGH_SALIENCE_THRESHOLD) addRisk("highImpact")
  if (personalClaimTypes.has(claim.claimType)) addRisk("personal")
  if (claim.scopeType === "global") addRisk("broadScope")
  if (claim.scopeType === "project") addRisk("projectScoped")
  if (claim.validUntil) addRisk("timeBound")
  if (risks.length === 0) addRisk("pendingConfirmation")

  return { primary, risks }
}

function buildReviewBuckets(
  claims: ClaimRecord[],
  activeConflictIndex: ClaimConflictIndex,
  conflictKeys: Set<string> = visibleConflictKeys(claims),
  conflictSummaries: ReadonlyMap<string, ClaimConflictSummary> = new Map(),
  personalClaimTypes: ReadonlySet<string> = new Set(DEFAULT_CLAIM_SCHEMA.profileClaimTypes),
  reviewSummaries: ReadonlyMap<string, ClaimReviewSummary> = new Map(),
) {
  const bucketMap = new Map<ReviewBucketKey, ClaimRecord[]>()
  for (const key of REVIEW_BUCKET_ORDER) bucketMap.set(key, [])
  for (const claim of claims) {
    const bucketKey = reviewBucketForClaim(
      claim,
      conflictKeys,
      activeConflictIndex,
      conflictSummaries,
      personalClaimTypes,
      reviewSummaries,
    )
    bucketMap.get(bucketKey)?.push(claim)
  }
  return REVIEW_BUCKET_ORDER.map((key) => ({
    key,
    claims: bucketMap.get(key) ?? [],
  })).filter((bucket) => bucket.claims.length > 0)
}

function isReviewHistoryDecision(decision: DreamingDecisionRecord): boolean {
  return (
    decision.targetType === "claim" &&
    decision.decisionType !== "promote" &&
    decision.decisionType !== "noop" &&
    decision.decisionType !== "no_op"
  )
}

function textFieldFromJson(raw: string | null | undefined, field: string): string | null {
  if (!raw) return null
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>
    const value = parsed[field]
    return typeof value === "string" && value.trim() ? value.trim() : null
  } catch {
    return null
  }
}

function decisionClaimScope(
  decision: DreamingDecisionRecord,
): Pick<ReviewHistoryItem, "scopeType" | "scopeId"> {
  const scopeType =
    textFieldFromJson(decision.afterJson, "scopeType") ??
    textFieldFromJson(decision.beforeJson, "scopeType")
  const scopeId =
    textFieldFromJson(decision.afterJson, "scopeId") ??
    textFieldFromJson(decision.beforeJson, "scopeId")
  return { scopeType, scopeId }
}

function decisionContent(decision: DreamingDecisionRecord): string | null {
  return (
    textFieldFromJson(decision.afterJson, "content") ??
    textFieldFromJson(decision.beforeJson, "content")
  )
}

function reviewHistoryItemMatchesScope(item: ReviewHistoryItem, value: string): boolean {
  if (value === "all") return true
  if (value === "global") return item.scopeType === "global"
  const separator = value.indexOf(":")
  if (separator <= 0) return true
  const scopeType = value.slice(0, separator)
  const scopeId = value.slice(separator + 1)
  return item.scopeType === scopeType && item.scopeId === scopeId
}

function reviewHistoryItemMatchesTimeRange(
  item: ReviewHistoryItem,
  value: ReviewHistoryTimeRange,
  now: number,
): boolean {
  if (value === "all") return true
  const createdAt = timeValue(item.createdAt)
  if (!createdAt) return false
  const days = value === "7d" ? 7 : 30
  return createdAt >= now - days * 24 * 60 * 60 * 1000
}

function reviewHistoryItemMatchesQuery(
  item: ReviewHistoryItem,
  query: string,
  scopeLabel: string,
): boolean {
  const terms = query.split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const haystack = normalizeClaimSearch(
    [
      item.content ?? "",
      item.rationale,
      item.decisionType,
      item.trigger,
      item.phase,
      item.status,
      item.targetId ?? "",
      item.scopeType ?? "",
      item.scopeId ?? "",
      scopeLabel,
    ].join(" "),
  )
  return terms.every((term) => haystack.includes(term))
}

function reviewHistorySince(value: ReviewHistoryTimeRange): string | null {
  if (value === "all") return null
  const days = value === "7d" ? 7 : 30
  return new Date(Date.now() - days * 24 * 60 * 60 * 1000).toISOString()
}

function reviewHistoryBackendScopeArgs(value: string): Record<string, string> {
  const scope = scopeFilterFocus(value)
  const args: Record<string, string> = {}
  if (scope.scopeType) args.scopeType = scope.scopeType
  if (scope.scopeId) args.scopeId = scope.scopeId
  return args
}

function reviewHistoryItemFromDecisionListItem(item: DreamingDecisionListItem): ReviewHistoryItem {
  return {
    id: item.id,
    decisionType: item.decisionType,
    targetType: item.targetType,
    targetId: item.targetId ?? null,
    scopeType: item.scopeType ?? null,
    scopeId: item.scopeId ?? null,
    trigger: item.runTrigger,
    phase: item.runPhase,
    status: item.runStatus,
    rationale: item.rationale,
    content: item.content ?? decisionContent(item),
    createdAt: item.createdAt,
  }
}

function conflictMatchesForClaim(
  claim: ClaimRecord,
  reviewClaims: ClaimRecord[],
  activeClaims: ClaimRecord[],
): ClaimConflictMatch[] {
  const key = claimConflictKey(claim)
  const object = claimObjectKey(claim)
  const matches: ClaimConflictMatch[] = []
  const seen = new Set<string>()

  const pushMatch = (candidate: ClaimRecord, kind: ConflictMatchKind) => {
    if (candidate.id === claim.id) return
    if (seen.has(candidate.id)) return
    if (claimConflictKey(candidate) !== key) return
    if (claimObjectKey(candidate) === object) return
    seen.add(candidate.id)
    matches.push({ claim: candidate, kind })
  }

  for (const candidate of activeClaims) pushMatch(candidate, "active")
  for (const candidate of reviewClaims) pushMatch(candidate, "review")
  return matches
}

function matchKindForClaim(claim: ClaimRecord): ConflictMatchKind {
  return claim.status === "active" ? "active" : "review"
}

function mergeConflictMatches(
  primary: ClaimConflictMatch[],
  fallback: ClaimConflictMatch[],
): ClaimConflictMatch[] {
  const out: ClaimConflictMatch[] = []
  const seen = new Set<string>()
  for (const match of [...primary, ...fallback]) {
    if (seen.has(match.claim.id)) continue
    seen.add(match.claim.id)
    out.push(match)
  }
  return out
}

function dateScore(value: string): number {
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

function strongestConflict(matches: ClaimConflictMatch[]): ClaimRecord | null {
  let best: ClaimRecord | null = null
  let bestScore = Number.NEGATIVE_INFINITY
  for (const { claim, kind } of matches) {
    const statusBoost = kind === "active" ? 0.15 : 0
    const score = claim.confidence * 0.6 + claim.salience * 0.25 + statusBoost
    if (score > bestScore) {
      best = claim
      bestScore = score
    }
  }
  return best
}

function buildConflictInsight(
  current: ClaimRecord,
  matches: ClaimConflictMatch[],
): ClaimConflictInsight | null {
  if (matches.length === 0) return null
  const otherObjects = [...new Set(matches.map(({ claim }) => claim.object).filter(Boolean))]
  const activeCount = matches.filter((match) => match.kind === "active").length
  const strongest = strongestConflict(matches)
  let suggestion: ConflictSuggestionKey = "compare"

  if (strongest) {
    const currentNewer = dateScore(current.updatedAt) >= dateScore(strongest.updatedAt)
    const currentMuchStronger =
      current.confidence >= strongest.confidence + 0.12 ||
      current.salience >= strongest.salience + 0.18
    const existingMuchStronger =
      strongest.confidence >= current.confidence + 0.12 ||
      strongest.salience >= current.salience + 0.18

    if (activeCount > 0 && existingMuchStronger && !currentMuchStronger) {
      suggestion = "keepExisting"
    } else if (currentNewer && currentMuchStronger) {
      suggestion = "useCurrent"
    }
  }

  return { suggestion, otherObjects, activeCount, strongest }
}

function conflictSuggestionLabelKey(suggestion: ConflictSuggestionKey): string {
  if (suggestion === "useCurrent") return "settings.claims.conflictSuggestionUseCurrent"
  if (suggestion === "keepExisting") return "settings.claims.conflictSuggestionKeepExisting"
  return "settings.claims.conflictSuggestionCompare"
}

function parseEvidenceMessageId(messageId?: string | null): number | undefined {
  if (!messageId) return undefined
  const parsed = Number(messageId)
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined
}

function evidenceTrustKey(evidence: EvidenceRecord): ClaimTrustKey {
  if (evidence.evidenceClass === "manual_correction" || evidence.sourceType === "manual") {
    return "userCorrected"
  }
  if (
    evidence.evidenceClass === "user_confirmed" ||
    evidence.evidenceClass === "explicit_user_statement"
  ) {
    return "userConfirmed"
  }
  if (evidence.evidenceClass === "project_artifact_fact" || evidence.filePath || evidence.url) {
    return "sourceBacked"
  }
  if (
    evidence.evidenceClass === "assistant_inferred" ||
    evidence.evidenceClass === "behavioral_pattern"
  ) {
    return "inferred"
  }
  return "weak"
}

function claimTrustKey(claim: ClaimRecord, evidence: EvidenceRecord[]): ClaimTrustKey {
  if (evidence.some((item) => evidenceTrustKey(item) === "userCorrected")) return "userCorrected"
  if (evidence.some((item) => evidenceTrustKey(item) === "userConfirmed")) return "userConfirmed"
  if (evidence.some((item) => evidenceTrustKey(item) === "sourceBacked")) return "sourceBacked"
  if (claim.confidence < LOW_CONFIDENCE_THRESHOLD) return "weak"
  if (evidence.some((item) => evidenceTrustKey(item) === "inferred")) return "inferred"
  return "weak"
}

function evidenceTrustStats(evidence: EvidenceRecord[]): EvidenceTrustStats {
  return {
    confirmed: evidence.filter((item) =>
      ["userCorrected", "userConfirmed"].includes(evidenceTrustKey(item)),
    ).length,
    inferred: evidence.filter((item) => evidenceTrustKey(item) === "inferred").length,
    sourceBacked: evidence.filter((item) => evidenceTrustKey(item) === "sourceBacked").length,
  }
}

function evidenceRank(evidence: EvidenceRecord): number {
  const trust = evidenceTrustKey(evidence)
  if (trust === "userCorrected") return 5
  if (trust === "userConfirmed") return 4
  if (trust === "sourceBacked") return 3
  if (trust === "inferred") return 2
  return 1
}

function bestEvidence(evidence: EvidenceRecord[]): EvidenceRecord | null {
  return (
    [...evidence].sort((a, b) => {
      const rank = evidenceRank(b) - evidenceRank(a)
      if (rank !== 0) return rank
      const quote = Number(!!b.quote) - Number(!!a.quote)
      if (quote !== 0) return quote
      return timeValue(b.createdAt) - timeValue(a.createdAt)
    })[0] ?? null
  )
}

function compactRationalePreview(value: string): string {
  const normalized = value.replace(/\s+/g, " ").trim()
  if (normalized.length <= 150) return normalized
  const firstSentence = normalized.match(/^.{40,150}?[.!?](?=\s|$)/)?.[0]
  return firstSentence ?? `${normalized.slice(0, 147)}...`
}

/**
 * Read-only structured-memory view over next-gen claims. Lists
 * claims via `claim_list` and shows a selected claim's evidence + legacy-memory
 * links via `claim_get`. The "Backfill" action turns existing legacy memories
 * into claims (dry-run preview → confirm); it never changes current prompt
 * injection (links are detached) — see ha-core `claims::backfill`.
 */
export default function ClaimsBetaView({ focus }: ClaimsBetaViewProps) {
  const { t } = useTranslation()
  const [claimSchema, setClaimSchema] = useState<ClaimSchemaMetadata>(DEFAULT_CLAIM_SCHEMA)
  const [claimSchemaErrorToast, setClaimSchemaErrorToast] =
    useState<ClaimOwnerOperationErrorToast | null>(null)
  const [claims, setClaims] = useState<ClaimRecord[]>([])
  const [loading, setLoading] = useState(false)
  const [claimLoadingMore, setClaimLoadingMore] = useState(false)
  const [claimListErrorToast, setClaimListErrorToast] =
    useState<ClaimOwnerOperationErrorToast | null>(null)
  const [claimHasMore, setClaimHasMore] = useState(false)
  const [claimTotal, setClaimTotal] = useState<number | null>(null)
  const [claimTotalTruncated, setClaimTotalTruncated] = useState(false)
  const [claimLoadedTarget, setClaimLoadedTarget] = useState<number | null>(
    normalizeClaimLoadedTarget(focus?.claimLoaded),
  )
  const [claimLoadedCount, setClaimLoadedCount] = useState(0)
  const [statusFilter, setStatusFilter] = useState<string>(focus?.statusFilter ?? "all")
  const [claimTypeFilter, setClaimTypeFilter] = useState<ClaimTypeFilter>(
    normalizeClaimTypeFilter(focus?.claimType),
  )
  const [scopeFilter, setScopeFilter] = useState<string>(
    scopeFilterValue(focus?.scopeType, focus?.scopeId),
  )
  const [confidenceSourceFilter, setConfidenceSourceFilter] = useState<ConfidenceSourceFilter>(
    focus?.confidenceSource ?? "all",
  )
  const [evidenceClassFilter, setEvidenceClassFilter] = useState<EvidenceClassFilter>(
    focus?.evidenceClass ?? "all",
  )
  const [evidenceSourceFilter, setEvidenceSourceFilter] = useState<EvidenceSourceFilter>(
    focus?.evidenceSource ?? "all",
  )
  const [claimListSort, setClaimListSort] = useState<ClaimListSort>(
    normalizeClaimListSort(focus?.claimSort),
  )
  const [claimSearchQuery, setClaimSearchQuery] = useState(focus?.query ?? "")
  const [claimSearchBackendQuery, setClaimSearchBackendQuery] = useState(
    normalizeClaimSearch(focus?.query ?? ""),
  )
  const [selectedId, setSelectedId] = useState<string | null>(focus?.selectedId ?? null)
  const [detail, setDetail] = useState<ClaimDetail | null>(null)
  const [detailErrorToast, setDetailErrorToast] =
    useState<ClaimOwnerOperationErrorToast | null>(null)
  const [claimGraph, setClaimGraph] = useState<ClaimGraphProjection | null>(null)
  const [claimGraphLoading, setClaimGraphLoading] = useState(false)
  const [claimGraphErrorDetail, setClaimGraphErrorDetail] = useState<string | null>(null)
  const [batchSelectedIds, setBatchSelectedIds] = useState<Set<string>>(() => new Set())
  const [batchBusy, setBatchBusy] = useState<"approve" | "archive" | null>(null)
  const [reviewHistoryOpen, setReviewHistoryOpen] = useState(focus?.reviewHistory === true)
  const [reviewHistoryDecisionFilter, setReviewHistoryDecisionFilter] = useState<string>(
    normalizeReviewHistoryDecisionFilter(focus?.reviewHistoryDecisionType),
  )
  const [reviewHistoryTimeRange, setReviewHistoryTimeRange] = useState<ReviewHistoryTimeRange>(
    normalizeReviewHistoryTimeRange(focus?.reviewHistoryTimeRange),
  )
  const [reviewHistoryScopeFilter, setReviewHistoryScopeFilter] = useState<string>(
    scopeFilterValue(focus?.reviewHistoryScopeType, focus?.reviewHistoryScopeId),
  )
  const [reviewHistoryQuery, setReviewHistoryQuery] = useState(focus?.reviewHistoryQuery ?? "")
  const [reviewHistoryBackendQuery, setReviewHistoryBackendQuery] = useState(
    normalizeClaimSearch(focus?.reviewHistoryQuery ?? ""),
  )
  const [reviewHistoryLoading, setReviewHistoryLoading] = useState(false)
  const [reviewHistoryLoadingMore, setReviewHistoryLoadingMore] = useState(false)
  const [reviewHistoryExportingAll, setReviewHistoryExportingAll] = useState(false)
  const [reviewHistoryHasMore, setReviewHistoryHasMore] = useState(false)
  const [reviewHistoryTotal, setReviewHistoryTotal] = useState<number | null>(null)
  const [reviewHistoryTotalTruncated, setReviewHistoryTotalTruncated] = useState(false)
  const [reviewHistory, setReviewHistory] = useState<ReviewHistoryItem[]>([])
  const [reviewHistoryErrorToast, setReviewHistoryErrorToast] =
    useState<ClaimOwnerOperationErrorToast | null>(null)
  const [reviewHistoryFilterPresets, setReviewHistoryFilterPresets] = useState<
    ReviewHistoryFilterPreset[]
  >(() => loadReviewHistoryFilterPresets())
  const [claimListFilterPresets, setClaimListFilterPresets] = useState<ClaimListFilterPreset[]>(
    () => loadClaimListFilterPresets(),
  )
  const [conflictResolveOpen, setConflictResolveOpen] = useState(false)
  const [conflictResolving, setConflictResolving] = useState(false)
  const [conflictCompareDetail, setConflictCompareDetail] = useState<ClaimDetail | null>(null)
  const [conflictCompareLoading, setConflictCompareLoading] = useState(false)
  const [conflictEvidenceErrorDetail, setConflictEvidenceErrorDetail] = useState<string | null>(
    null,
  )
  const [conflictCandidatesErrorDetail, setConflictCandidatesErrorDetail] = useState<
    string | null
  >(null)
  const [conflictMatrixDetails, setConflictMatrixDetails] = useState<ClaimDetail[]>([])
  const [agentNames, setAgentNames] = useState<Map<string, string>>(() => new Map())
  const [projectNames, setProjectNames] = useState<Map<string, string>>(() => new Map())
  const [scopeNameErrorToasts, setScopeNameErrorToasts] = useState<
    ClaimOwnerOperationErrorToast[]
  >([])
  const [activeClaimsForConflict, setActiveClaimsForConflict] = useState<ClaimRecord[]>([])
  const [activeClaimConflictIndex, setActiveClaimConflictIndex] = useState<ClaimConflictIndex>(
    () => new Map(),
  )
  const [claimConflictSummaries, setClaimConflictSummaries] = useState<
    Map<string, ClaimConflictSummary>
  >(() => new Map())
  const [claimEvidenceSummaries, setClaimEvidenceSummaries] = useState<
    Map<string, ClaimEvidenceSummary>
  >(() => new Map())
  const [claimReviewSummaries, setClaimReviewSummaries] = useState<
    Map<string, ClaimReviewSummary>
  >(() => new Map())
  const [claimListSummaryErrors, setClaimListSummaryErrors] = useState<
    Partial<Record<ClaimListSummaryErrorSource, ClaimOwnerOperationErrorToast>>
  >({})
  const [detailConflictClaims, setDetailConflictClaims] = useState<ClaimRecord[]>([])
  const [detailConflictLoading, setDetailConflictLoading] = useState(false)
  const pendingSelectedIdRef = useRef<string | null>(focus?.selectedId ?? null)
  const lastFocusNonceRef = useRef<number | null>(null)
  const claimListNextOffsetRef = useRef(0)
  const reviewHistoryNextOffsetRef = useRef(0)

  // Backfill (dry-run preview + apply).
  const [backfillOpen, setBackfillOpen] = useState(false)
  const [plan, setPlan] = useState<BackfillPlan | null>(null)
  const [planLoading, setPlanLoading] = useState(false)
  const [applying, setApplying] = useState(false)

  const claimTypeFilterOptions = useMemo(() => claimTypeFilterValues(claimSchema), [claimSchema])
  const confidenceSourceFilterOptions = useMemo(
    () => confidenceSourceFilterValues(claimSchema),
    [claimSchema],
  )
  const evidenceClassFilterOptions = useMemo(
    () => evidenceClassFilterValues(claimSchema),
    [claimSchema],
  )
  const evidenceSourceFilterOptions = useMemo(
    () => evidenceSourceFilterValues(claimSchema),
    [claimSchema],
  )
  const profileClaimTypes = useMemo(
    () => new Set(claimSchema.profileClaimTypes),
    [claimSchema.profileClaimTypes],
  )
  const markClaimListSummaryError = useCallback(
    (source: ClaimListSummaryErrorSource, error: unknown) => {
      const failure = claimOwnerOperationErrorToast("loadListSummaries", t, error)
      setClaimListSummaryErrors((current) => ({ ...current, [source]: failure }))
    },
    [t],
  )
  const clearClaimListSummaryError = useCallback((source: ClaimListSummaryErrorSource) => {
    setClaimListSummaryErrors((current) => {
      if (!current[source]) return current
      const next = { ...current }
      delete next[source]
      return next
    })
  }, [])
  const claimListSummaryErrorToasts = useMemo(
    () =>
      Object.values(claimListSummaryErrors).filter(
        (toast): toast is ClaimOwnerOperationErrorToast => Boolean(toast),
      ),
    [claimListSummaryErrors],
  )

  useEffect(() => {
    let cancelled = false
    void getTransport()
      .call<ClaimSchemaMetadata>("claim_schema_metadata")
      .then((schema) => {
        if (cancelled) return
        const normalized = normalizeClaimSchema(schema)
        setClaimSchema(normalized)
        setClaimSchemaErrorToast(null)
        setClaimTypeFilter((current) => normalizeClaimTypeFilter(current, normalized))
        setConfidenceSourceFilter((current) => normalizeConfidenceSourceFilter(current, normalized))
        setEvidenceClassFilter((current) => normalizeEvidenceClassFilter(current, normalized))
        setEvidenceSourceFilter((current) => normalizeEvidenceSourceFilter(current, normalized))
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::claimSchema",
          "Failed to load claim schema metadata",
          e,
        )
        if (!cancelled) {
          setClaimSchemaErrorToast(claimOwnerOperationErrorToast("loadSchema", t, e))
        }
      })
    return () => {
      cancelled = true
    }
  }, [t])

  // Fetch the list in place, WITHOUT touching the selection — used both by the
  // filter-change reload (which resets selection separately) and by the
  // post-mutation refresh (which keeps the detail pane open).
  const fetchClaims = useCallback(
    async (options?: { append?: boolean }) => {
      const fetchPage = async (args: Record<string, unknown>): Promise<ClaimListPage> => {
        try {
          const page = await getTransport().call<ClaimListPage>("claim_list_page", args)
          if (page && Array.isArray(page.items) && typeof page.total === "number") {
            return page
          }
        } catch (e) {
          logger.warn(
            "settings",
            "ClaimsBetaView::listPage",
            "Falling back to legacy claim_list",
            e,
          )
        }
        const items = (await getTransport().call<ClaimRecord[]>("claim_list", args)) ?? []
        const offset = typeof args.offset === "number" ? args.offset : 0
        const limit = typeof args.limit === "number" ? args.limit : CLAIM_LIST_DEFAULT_PAGE_SIZE
        return {
          items,
          total: offset + items.length,
          totalTruncated: items.length >= limit,
        }
      }

      const append = options?.append === true
      setClaimListErrorToast(null)
      const pageSize = claimSearchBackendQuery
        ? CLAIM_LIST_SEARCH_PAGE_SIZE
        : CLAIM_LIST_DEFAULT_PAGE_SIZE
      const targetLoadCount = append
        ? pageSize
        : Math.max(pageSize, claimLoadedTarget ?? claimListNextOffsetRef.current)
      if (append) {
        setClaimLoadingMore(true)
      } else {
        claimListNextOffsetRef.current = 0
        setClaimLoadedCount(0)
        setClaimHasMore(false)
        setClaimLoadingMore(false)
        setClaimTotal(null)
        setClaimTotalTruncated(false)
        setLoading(true)
      }
      try {
        const args: Record<string, unknown> = {
          limit: pageSize,
          ...scopeFilterArgs(scopeFilter),
        }
        const backendSort = claimListBackendSortArg(claimListSort, claimSearchBackendQuery)
        if (backendSort) args.sort = backendSort
        if (claimSearchBackendQuery) args.query = claimSearchBackendQuery
        if (statusFilter !== "all") args.status = statusFilter
        if (confidenceSourceFilter !== "all") args.confidenceSource = confidenceSourceFilter
        if (evidenceClassFilter !== "all") args.evidenceClass = evidenceClassFilter
        if (evidenceSourceFilter !== "all") args.evidenceSourceType = evidenceSourceFilter
        if (claimTypeFilter === "profile") {
          const targetLimit = append ? claimListNextOffsetRef.current + pageSize : targetLoadCount
          const pages = await Promise.all(
            [...profileClaimTypes].map(async (claimType) => {
              const items: ClaimRecord[] = []
              let offset = 0
              let total = 0
              let totalTruncated = false
              while (items.length < targetLimit) {
                const limit = Math.min(CLAIM_LIST_SEARCH_PAGE_SIZE, targetLimit - items.length)
                const page = await fetchPage({
                  ...args,
                  claimType,
                  limit,
                  offset,
                })
                items.push(...page.items)
                offset += page.items.length
                total = page.total
                totalTruncated = totalTruncated || page.totalTruncated === true
                if (page.items.length === 0) break
                if (!page.totalTruncated && offset >= page.total) break
                if (page.items.length < limit && page.totalTruncated) break
              }
              return { items, total, totalTruncated }
            }),
          )
          const profileItems = pages.flatMap((page) => page.items)
          const profileOrdered =
            claimListSort === "relevance" && claimSearchBackendQuery
              ? profileItems
              : profileItems.sort((a, b) => compareClaimsForSort(claimListSort, a, b))
          const merged = uniqueClaimRecords(profileOrdered).slice(0, targetLimit)
          const total = pages.reduce((sum, page) => sum + page.total, 0)
          const totalTruncated = pages.some((page) => page.totalTruncated)
          claimListNextOffsetRef.current = merged.length
          setClaimLoadedCount(merged.length)
          setClaims(merged)
          setClaimTotal(total)
          setClaimTotalTruncated(totalTruncated)
          setClaimHasMore(
            totalTruncated
              ? pages.some((page) => page.items.length >= targetLimit)
              : merged.length < total,
          )
        } else {
          if (claimTypeFilter !== "all") args.claimType = claimTypeFilter
          const offset = append ? claimListNextOffsetRef.current : 0
          const next: ClaimRecord[] = []
          let nextOffset = offset
          let total = 0
          let totalTruncated = false
          while (next.length < targetLoadCount) {
            const limit = Math.min(CLAIM_LIST_SEARCH_PAGE_SIZE, targetLoadCount - next.length)
            const page = await fetchPage({ ...args, limit, offset: nextOffset })
            next.push(...page.items)
            nextOffset += page.items.length
            total = page.total
            totalTruncated = totalTruncated || page.totalTruncated === true
            if (page.items.length === 0) break
            if (!page.totalTruncated && nextOffset >= page.total) break
            if (page.items.length < limit && page.totalTruncated) break
          }
          claimListNextOffsetRef.current = nextOffset
          setClaimLoadedCount(nextOffset)
          setClaims((previous) => (append ? appendClaimRecords(previous, next) : next))
          setClaimTotal(total)
          setClaimTotalTruncated(totalTruncated)
          setClaimHasMore(
            totalTruncated
              ? next.length >= targetLoadCount
              : claimListNextOffsetRef.current < total,
          )
        }
        setClaimListErrorToast(null)
      } catch (e) {
        logger.error("settings", "ClaimsBetaView::list", "Failed to list claims", e)
        setClaimListErrorToast(claimOwnerOperationErrorToast("loadList", t, e))
        if (!append) {
          claimListNextOffsetRef.current = 0
          setClaimLoadedCount(0)
          setClaims([])
          setClaimHasMore(false)
          setClaimTotal(null)
          setClaimTotalTruncated(false)
        }
      } finally {
        if (append) {
          setClaimLoadingMore(false)
        } else {
          setLoading(false)
        }
      }
    },
    [
      claimTypeFilter,
      claimLoadedTarget,
      claimListSort,
      claimSearchBackendQuery,
      confidenceSourceFilter,
      evidenceClassFilter,
      evidenceSourceFilter,
      profileClaimTypes,
      scopeFilter,
      statusFilter,
      t,
    ],
  )

  const loadClaims = useCallback(async () => {
    // Reset the selection so the detail pane can't show a claim the new
    // filter excludes (stale-detail guard).
    const pendingSelectedId = pendingSelectedIdRef.current
    pendingSelectedIdRef.current = null
    setSelectedId(pendingSelectedId)
    setDetail(null)
    await fetchClaims()
  }, [fetchClaims])

  const loadMoreClaims = useCallback(async () => {
    if (loading || claimLoadingMore) return
    await fetchClaims({ append: true })
  }, [claimLoadingMore, fetchClaims, loading])

  const loadDetail = useCallback(async (id: string) => {
    setDetailErrorToast(null)
    try {
      const d = await getTransport().call<ClaimDetail | null>("claim_get", { id })
      setDetail(d ?? null)
      if (d) {
        setDetailErrorToast(null)
      } else {
        setDetailErrorToast({
          title: t("settings.claims.detailMissing", {
            defaultValue: "This structured memory no longer exists.",
          }),
        })
      }
    } catch (e) {
      logger.error("settings", "ClaimsBetaView::get", "Failed to load claim", e)
      setDetail(null)
      setDetailErrorToast(claimOwnerOperationErrorToast("loadDetail", t, e))
    }
  }, [t])

  const loadClaimGraph = useCallback(async (id: string) => {
    setClaimGraph(null)
    setClaimGraphErrorDetail(null)
    setClaimGraphLoading(true)
    try {
      const graph = await getTransport().call<ClaimGraphProjection>("claim_graph", {
        id,
        limit: 30,
      })
      setClaimGraph(graph ?? null)
      setClaimGraphErrorDetail(null)
    } catch (e) {
      logger.warn("settings", "ClaimsBetaView::claimGraph", "Failed to load claim graph", e)
      setClaimGraph(null)
      setClaimGraphErrorDetail(claimOwnerOperationErrorDetail(e))
    } finally {
      setClaimGraphLoading(false)
    }
  }, [])

  const reviewHistoryBackendArgs = useCallback(
    (offset: number, limit: number, query = reviewHistoryBackendQuery): Record<string, unknown> => {
      const since = reviewHistorySince(reviewHistoryTimeRange)
      const args: Record<string, unknown> = {
        limit,
        offset,
        targetType: "claim",
        ...reviewHistoryBackendScopeArgs(reviewHistoryScopeFilter),
      }
      if (reviewHistoryDecisionFilter !== "all") {
        args.decisionType = reviewHistoryDecisionFilter
      }
      if (query) args.query = query
      if (since) args.since = since
      return args
    },
    [
      reviewHistoryBackendQuery,
      reviewHistoryDecisionFilter,
      reviewHistoryScopeFilter,
      reviewHistoryTimeRange,
    ],
  )

  const loadReviewHistory = useCallback(
    async (append = false) => {
      if (append) setReviewHistoryLoadingMore(true)
      else {
        setReviewHistoryLoading(true)
        setReviewHistoryTotal(null)
        setReviewHistoryTotalTruncated(false)
      }
      try {
        const tx = getTransport()
        const offset = append ? reviewHistoryNextOffsetRef.current : 0
        const backendArgs = reviewHistoryBackendArgs(offset, REVIEW_HISTORY_PAGE_SIZE)
        try {
          let response: DreamingDecisionListResponse | null = null
          try {
            response = await tx.call<DreamingDecisionListResponse>(
              "dreaming_list_decisions_page",
              backendArgs,
            )
          } catch (pageError) {
            logger.warn(
              "settings",
              "ClaimsBetaView::reviewHistoryDecisionsPage",
              "Failed to load Review History via decision page query; falling back to item query",
              pageError,
            )
            const legacyItems =
              (await tx.call<DreamingDecisionListItem[]>("dreaming_list_decisions", backendArgs)) ??
              []
            response = {
              items: legacyItems,
              total: offset + legacyItems.length,
              totalTruncated: legacyItems.length >= REVIEW_HISTORY_PAGE_SIZE,
            }
          }
          const items = response?.items ?? []
          const mapped = items
            .filter(isReviewHistoryDecision)
            .map(reviewHistoryItemFromDecisionListItem)
          reviewHistoryNextOffsetRef.current = offset + items.length
          const total = Math.max(response?.total ?? 0, offset + items.length)
          const totalTruncated = response?.totalTruncated === true
          setReviewHistoryTotal(total)
          setReviewHistoryTotalTruncated(totalTruncated)
          setReviewHistoryHasMore(
            offset + items.length < total ||
              (totalTruncated && items.length >= REVIEW_HISTORY_PAGE_SIZE),
          )
          setReviewHistory((prev) => {
            if (!append) return mapped
            if (mapped.length === 0) return prev
            const seen = new Set(prev.map((item) => item.id))
            return [...prev, ...mapped.filter((item) => !seen.has(item.id))]
          })
          setReviewHistoryErrorToast(null)
          return
        } catch (e) {
          logger.warn(
            "settings",
            "ClaimsBetaView::reviewHistoryDecisions",
            "Failed to load Review History via decision query; falling back to run fan-out",
            e,
          )
          if (append) {
            setReviewHistoryHasMore(false)
            setReviewHistoryErrorToast(claimOwnerOperationErrorToast("loadReviewHistory", t, e))
            return
          }
        }

        const runs =
          (await tx.call<DreamingRunRecord[]>("dreaming_list_runs", {
            limit: 50,
            offset: 0,
          })) ?? []
        let firstRunDetailError: unknown = null
        const details = await Promise.all(
          runs
            .filter((run) => run.decisionCount > 0)
            .slice(0, 50)
            .map((run) =>
              tx.call<DreamingRunDetail | null>("dreaming_get_run", { id: run.id }).catch((e) => {
                logger.warn(
                  "settings",
                  "ClaimsBetaView::reviewHistoryDetail",
                  "Failed to load Dreaming run detail",
                  e,
                )
                firstRunDetailError ??= e
                return null
              }),
            ),
        )
        const fallbackHistory = details
          .flatMap((detail) =>
            detail
              ? detail.decisions.filter(isReviewHistoryDecision).map((decision) => {
                  const scope = decisionClaimScope(decision)
                  return {
                    id: decision.id,
                    decisionType: decision.decisionType,
                    targetType: decision.targetType,
                    targetId: decision.targetId ?? null,
                    scopeType: scope.scopeType,
                    scopeId: scope.scopeId,
                    trigger: detail.run.trigger,
                    phase: detail.run.phase,
                    status: detail.run.status,
                    rationale: decision.rationale,
                    content: decisionContent(decision),
                    createdAt: decision.createdAt,
                  }
                })
              : [],
          )
          .sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))
          .slice(0, 100)
        setReviewHistory(fallbackHistory)
        setReviewHistoryTotal(fallbackHistory.length)
        setReviewHistoryTotalTruncated(false)
        reviewHistoryNextOffsetRef.current = 0
        setReviewHistoryHasMore(false)
        setReviewHistoryErrorToast(
          firstRunDetailError
            ? claimOwnerOperationErrorToast("loadReviewHistory", t, firstRunDetailError)
            : null,
        )
      } catch (e) {
        logger.warn("settings", "ClaimsBetaView::reviewHistory", "Failed to load review history", e)
        setReviewHistoryErrorToast(claimOwnerOperationErrorToast("loadReviewHistory", t, e))
        if (!append) {
          reviewHistoryNextOffsetRef.current = 0
          setReviewHistoryHasMore(false)
          setReviewHistoryTotal(null)
          setReviewHistoryTotalTruncated(false)
          setReviewHistory([])
        }
      } finally {
        if (append) setReviewHistoryLoadingMore(false)
        else setReviewHistoryLoading(false)
      }
    },
    [reviewHistoryBackendArgs, t],
  )

  const refreshActiveClaimsForConflict = useCallback(async () => {
    try {
      const active =
        (await getTransport().call<ClaimRecord[]>("claim_list", {
          status: "active",
          limit: 500,
        })) ?? []
      setActiveClaimsForConflict(active)
      setActiveClaimConflictIndex(buildClaimConflictIndex(active))
      clearClaimListSummaryError("activeConflict")
    } catch (e) {
      logger.warn(
        "settings",
        "ClaimsBetaView::activeConflictIndex",
        "Failed to refresh active claims for conflict grouping",
        e,
      )
      setActiveClaimsForConflict([])
      setActiveClaimConflictIndex(new Map())
      markClaimListSummaryError("activeConflict", e)
    }
  }, [clearClaimListSummaryError, markClaimListSummaryError])

  useEffect(() => {
    if (statusFilter !== "needs_review" || claims.length === 0) {
      setClaimConflictSummaries(new Map())
      clearClaimListSummaryError("conflict")
      return
    }
    let cancelled = false
    const ids = claims.map((claim) => claim.id)
    setClaimConflictSummaries(new Map())
    void getTransport()
      .call<ClaimConflictSummary[]>("claim_conflict_summaries", { ids })
      .then((summaries) => {
        if (cancelled) return
        setClaimConflictSummaries(
          new Map((summaries ?? []).map((summary) => [summary.claimId, summary])),
        )
        clearClaimListSummaryError("conflict")
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::claimConflictSummaries",
          "Failed to load claim conflict summaries",
          e,
        )
        if (!cancelled) {
          setClaimConflictSummaries(new Map())
          markClaimListSummaryError("conflict", e)
        }
      })
    return () => {
      cancelled = true
    }
  }, [claims, clearClaimListSummaryError, markClaimListSummaryError, statusFilter])

  useEffect(() => {
    if (statusFilter !== "needs_review" || claims.length === 0) {
      setClaimReviewSummaries(new Map())
      clearClaimListSummaryError("review")
      return
    }
    let cancelled = false
    const ids = claims.map((claim) => claim.id)
    setClaimReviewSummaries(new Map())
    void getTransport()
      .call<ClaimReviewSummary[]>("claim_review_summaries", { ids })
      .then((summaries) => {
        if (cancelled) return
        setClaimReviewSummaries(
          new Map((summaries ?? []).map((summary) => [summary.claimId, summary])),
        )
        clearClaimListSummaryError("review")
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::claimReviewSummaries",
          "Failed to load claim review summaries",
          e,
        )
        if (!cancelled) {
          setClaimReviewSummaries(new Map())
          markClaimListSummaryError("review", e)
        }
      })
    return () => {
      cancelled = true
    }
  }, [claims, clearClaimListSummaryError, markClaimListSummaryError, statusFilter])

  useEffect(() => {
    if (claims.length === 0) {
      setClaimEvidenceSummaries(new Map())
      clearClaimListSummaryError("evidence")
      return
    }
    let cancelled = false
    const ids = claims.map((claim) => claim.id)
    setClaimEvidenceSummaries(new Map())
    void getTransport()
      .call<ClaimEvidenceSummary[]>("claim_evidence_summaries", { ids })
      .then((summaries) => {
        if (cancelled) return
        setClaimEvidenceSummaries(
          new Map((summaries ?? []).map((summary) => [summary.claimId, summary])),
        )
        clearClaimListSummaryError("evidence")
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::claimEvidenceSummaries",
          "Failed to load claim evidence summaries",
          e,
        )
        if (!cancelled) {
          setClaimEvidenceSummaries(new Map())
          markClaimListSummaryError("evidence", e)
        }
      })
    return () => {
      cancelled = true
    }
  }, [claims, clearClaimListSummaryError, markClaimListSummaryError])

  // After a correction, refresh the list AND the open detail in place so the
  // user keeps their context (the detail pane doesn't blink shut every edit).
  const onClaimChanged = useCallback(async () => {
    await fetchClaims()
    if (statusFilter === "needs_review") await refreshActiveClaimsForConflict()
    if (selectedId) await loadDetail(selectedId)
    if (reviewHistoryOpen) await loadReviewHistory()
  }, [
    fetchClaims,
    statusFilter,
    refreshActiveClaimsForConflict,
    selectedId,
    loadDetail,
    reviewHistoryOpen,
    loadReviewHistory,
  ])

  const restoreArchivedClaims = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return
      const tx = getTransport()
      let restored = 0
      let firstError: unknown = null
      for (const id of ids) {
        try {
          await tx.call("claim_update", {
            id,
            status: "needs_review",
            note: "Restore archived claim to review queue from Memory Inbox.",
          })
          restored += 1
        } catch (e) {
          logger.warn(
            "settings",
            "ClaimsBetaView::restoreArchived",
            "One archived claim failed to restore",
            e,
          )
          firstError ??= e
        }
      }
      await fetchClaims()
      if (statusFilter === "needs_review") await refreshActiveClaimsForConflict()
      if (restored === ids.length) {
        toast.success(t("settings.claims.restoreArchiveDone", { count: restored }))
      } else if (restored > 0) {
        const failureToast = claimOwnerOperationErrorToast("restoreArchive", t, firstError)
        toast.warning(
          t("settings.claims.restoreArchivePartial", {
            count: restored,
            failed: ids.length - restored,
          }),
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } else {
        const failureToast = claimOwnerOperationErrorToast("restoreArchive", t, firstError)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      }
    },
    [fetchClaims, refreshActiveClaimsForConflict, statusFilter, t],
  )

  const showArchiveRestoreToast = useCallback(
    (
      message: string,
      ids: string[],
      tone: "success" | "warning" = "success",
      description?: string,
    ) => {
      const show = tone === "warning" ? toast.warning : toast.success
      show(message, {
        ...(description ? { description } : {}),
        duration: 12000,
        action: {
          label: t("settings.claims.restoreArchiveAction"),
          onClick: () => void restoreArchivedClaims(ids),
        },
      })
    },
    [restoreArchivedClaims, t],
  )

  const toggleBatchSelection = useCallback((id: string) => {
    setBatchSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const runBatchAction = useCallback(
    async (action: "approve" | "archive") => {
      if (batchBusy) return
      const targets = claims.filter((claim) => batchSelectedIds.has(claim.id))
      if (targets.length === 0) return

      setBatchBusy(action)
      try {
        const tx = getTransport()
        const succeededIds = new Set<string>()
        let firstError: unknown = null
        for (const claim of targets) {
          try {
            if (action === "approve") {
              await tx.call("claim_update", {
                id: claim.id,
                status: "active",
                note: "Batch approved from Memory Inbox.",
              })
            } else {
              await tx.call("claim_forget", {
                id: claim.id,
                permanent: false,
                note: "Batch archived from Memory Inbox.",
              })
            }
            succeededIds.add(claim.id)
          } catch (e) {
            logger.warn(
              "settings",
              "ClaimsBetaView::batchAction",
              "One claim batch action failed",
              e,
            )
            firstError ??= e
          }
        }
        const succeeded = succeededIds.size
        const failed = targets.length - succeeded

        if (succeeded > 0) {
          setBatchSelectedIds((prev) => {
            const next = new Set(prev)
            for (const id of succeededIds) next.delete(id)
            return next
          })
          if (selectedId && succeededIds.has(selectedId)) {
            setSelectedId(null)
            setDetail(null)
          }
        }

        await fetchClaims()
        if (action === "approve") await refreshActiveClaimsForConflict()
        if (reviewHistoryOpen) await loadReviewHistory()

        if (failed > 0) {
          const message = t("settings.claims.batchPartial", {
            count: succeeded,
            failed,
          })
          if (succeeded === 0) {
            const failureToast = claimOwnerOperationErrorToast("batchAction", t, firstError)
            toast.error(
              failureToast.title,
              failureToast.description ? { description: failureToast.description } : undefined,
            )
          } else if (action === "archive") {
            const failureToast = claimOwnerOperationErrorToast("batchAction", t, firstError)
            showArchiveRestoreToast(message, [...succeededIds], "warning", failureToast.description)
          } else {
            const failureToast = claimOwnerOperationErrorToast("batchAction", t, firstError)
            toast.warning(
              message,
              failureToast.description ? { description: failureToast.description } : undefined,
            )
          }
        } else {
          const message = t(
            action === "approve"
              ? "settings.claims.batchApproveDone"
              : "settings.claims.batchArchiveDone",
            { count: succeeded },
          )
          if (action === "archive" && succeeded > 0) {
            showArchiveRestoreToast(message, [...succeededIds])
          } else {
            toast.success(message)
          }
        }
      } catch (e) {
        logger.error("settings", "ClaimsBetaView::batchAction", "Failed to run batch action", e)
        const failureToast = claimOwnerOperationErrorToast("batchAction", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setBatchBusy(null)
      }
    },
    [
      batchBusy,
      batchSelectedIds,
      claims,
      fetchClaims,
      loadReviewHistory,
      refreshActiveClaimsForConflict,
      reviewHistoryOpen,
      selectedId,
      showArchiveRestoreToast,
      t,
    ],
  )

  const openBackfill = useCallback(async () => {
    setBackfillOpen(true)
    setPlan(null)
    setPlanLoading(true)
    try {
      const p = await getTransport().call<BackfillPlan>("memory_backfill_plan")
      setPlan(p ?? null)
    } catch (e) {
      logger.error("settings", "ClaimsBetaView::backfillPlan", "Failed to plan backfill", e)
      const failureToast = claimOwnerOperationErrorToast("backfillPlan", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setBackfillOpen(false)
    } finally {
      setPlanLoading(false)
    }
  }, [t])

  const runApply = useCallback(async () => {
    setApplying(true)
    try {
      const r = await getTransport().call<BackfillApplyResult>("memory_backfill_apply")
      // Always refresh the claims list — created claims should show even on a
      // partial run.
      await loadClaims()
      const failed = r?.failed ?? 0
      if (failed > 0) {
        // Best-effort apply: surface the failures instead of a success toast,
        // keep the dialog open and refresh the plan so the user can retry.
        toast.warning(
          t("settings.claims.backfill.appliedPartial", {
            created: r?.created ?? 0,
            failed,
          }),
        )
        await openBackfill()
      } else {
        toast.success(
          t("settings.claims.backfill.applied", {
            created: r?.created ?? 0,
            active: r?.autoActive ?? 0,
            review: r?.needsReview ?? 0,
          }),
        )
        setBackfillOpen(false)
      }
    } catch (e) {
      logger.error("settings", "ClaimsBetaView::backfillApply", "Failed to apply backfill", e)
      const failureToast = claimOwnerOperationErrorToast("backfillApply", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setApplying(false)
    }
  }, [t, loadClaims, openBackfill])

  useEffect(() => {
    void loadClaims()
  }, [loadClaims])

  useEffect(() => {
    const handle = globalThis.setTimeout(() => {
      setClaimSearchBackendQuery(normalizeClaimSearch(claimSearchQuery))
    }, 180)
    return () => globalThis.clearTimeout(handle)
  }, [claimSearchQuery])

  useEffect(() => {
    const handle = globalThis.setTimeout(() => {
      setReviewHistoryBackendQuery(normalizeClaimSearch(reviewHistoryQuery))
    }, 180)
    return () => globalThis.clearTimeout(handle)
  }, [reviewHistoryQuery])

  useEffect(() => {
    if (!focus || lastFocusNonceRef.current === focus.nonce) return
    lastFocusNonceRef.current = focus.nonce
    pendingSelectedIdRef.current = focus.selectedId ?? null
    let filterChanged = false
    if (focus.statusFilter && focus.statusFilter !== statusFilter) {
      setStatusFilter(focus.statusFilter)
      filterChanged = true
    }
    if (focus.claimType !== undefined) {
      const nextClaimType = normalizeClaimTypeFilter(focus.claimType, claimSchema)
      if (nextClaimType !== claimTypeFilter) {
        setClaimTypeFilter(nextClaimType)
        filterChanged = true
      }
    }
    if (focus.scopeType !== undefined) {
      const nextScope = scopeFilterValue(focus.scopeType, focus.scopeId)
      if (nextScope !== scopeFilter) {
        setScopeFilter(nextScope)
        filterChanged = true
      }
    }
    if (focus.confidenceSource !== undefined) {
      const nextConfidenceSource = normalizeConfidenceSourceFilter(
        focus.confidenceSource,
        claimSchema,
      )
      if (nextConfidenceSource !== confidenceSourceFilter) {
        setConfidenceSourceFilter(nextConfidenceSource)
        filterChanged = true
      }
    }
    if (focus.evidenceClass !== undefined) {
      const nextEvidenceClass = normalizeEvidenceClassFilter(focus.evidenceClass, claimSchema)
      if (nextEvidenceClass !== evidenceClassFilter) {
        setEvidenceClassFilter(nextEvidenceClass)
        filterChanged = true
      }
    }
    if (focus.evidenceSource !== undefined) {
      const nextEvidenceSource = normalizeEvidenceSourceFilter(focus.evidenceSource, claimSchema)
      if (nextEvidenceSource !== evidenceSourceFilter) {
        setEvidenceSourceFilter(nextEvidenceSource)
        filterChanged = true
      }
    }
    if (focus.claimSort !== undefined) {
      const nextSort = normalizeClaimListSort(focus.claimSort)
      if (nextSort !== claimListSort) {
        setClaimListSort(nextSort)
        filterChanged = true
      }
    }
    const nextLoadedTarget = normalizeClaimLoadedTarget(focus.claimLoaded)
    if (nextLoadedTarget !== claimLoadedTarget) {
      claimListNextOffsetRef.current = 0
      setClaimLoadedCount(0)
      setClaimLoadedTarget(nextLoadedTarget)
      filterChanged = true
    }
    if (focus.query !== undefined && focus.query !== claimSearchQuery) {
      setClaimSearchQuery(focus.query ?? "")
      setClaimSearchBackendQuery(normalizeClaimSearch(focus.query ?? ""))
      filterChanged = true
    }
    if (focus.reviewHistory !== undefined) setReviewHistoryOpen(focus.reviewHistory === true)
    if (focus.reviewHistoryDecisionType !== undefined) {
      setReviewHistoryDecisionFilter(
        normalizeReviewHistoryDecisionFilter(focus.reviewHistoryDecisionType),
      )
    }
    if (focus.reviewHistoryTimeRange !== undefined) {
      setReviewHistoryTimeRange(normalizeReviewHistoryTimeRange(focus.reviewHistoryTimeRange))
    }
    if (focus.reviewHistoryScopeType !== undefined) {
      setReviewHistoryScopeFilter(
        scopeFilterValue(focus.reviewHistoryScopeType, focus.reviewHistoryScopeId),
      )
    }
    if (focus.reviewHistoryQuery !== undefined && focus.reviewHistoryQuery !== reviewHistoryQuery) {
      setReviewHistoryQuery(focus.reviewHistoryQuery ?? "")
      setReviewHistoryBackendQuery(normalizeClaimSearch(focus.reviewHistoryQuery ?? ""))
    }
    if (filterChanged) {
      if (nextLoadedTarget === null) {
        claimListNextOffsetRef.current = 0
        setClaimLoadedCount(0)
        setClaimLoadedTarget(null)
      }
      return
    }
    if (focus.selectedId) {
      setSelectedId(focus.selectedId)
      setDetail(null)
    }
  }, [
    claimSchema,
    claimLoadedTarget,
    claimListSort,
    claimSearchQuery,
    claimTypeFilter,
    confidenceSourceFilter,
    evidenceClassFilter,
    evidenceSourceFilter,
    focus,
    reviewHistoryQuery,
    scopeFilter,
    statusFilter,
  ])

  useEffect(() => {
    if (selectedId) void loadDetail(selectedId)
    else {
      setDetail(null)
      setDetailErrorToast(null)
    }
  }, [selectedId, loadDetail])

  useEffect(() => {
    if (selectedId) void loadClaimGraph(selectedId)
    else {
      setClaimGraph(null)
      setClaimGraphErrorDetail(null)
      setClaimGraphLoading(false)
    }
  }, [selectedId, loadClaimGraph])

  useEffect(() => {
    setConflictResolveOpen(false)
  }, [selectedId])

  useEffect(() => {
    let cancelled = false
    const tx = getTransport()
    void Promise.allSettled([
      tx.call<AgentInfo[]>("list_agents"),
      tx.call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true }),
    ]).then(([agentsResult, projectsResult]) => {
      if (cancelled) return
      const failures: ClaimOwnerOperationErrorToast[] = []
      if (agentsResult.status === "fulfilled") {
        setAgentNames(new Map((agentsResult.value ?? []).map((agent) => [agent.id, agent.name])))
      } else {
        logger.warn(
          "settings",
          "ClaimsBetaView::scopeNames",
          "Failed to load agent names",
          agentsResult.reason,
        )
        failures.push(claimOwnerOperationErrorToast("loadScopeNames", t, agentsResult.reason))
      }
      if (projectsResult.status === "fulfilled") {
        setProjectNames(
          new Map((projectsResult.value ?? []).map((project) => [project.id, project.name])),
        )
      } else {
        logger.warn(
          "settings",
          "ClaimsBetaView::scopeNames",
          "Failed to load project names",
          projectsResult.reason,
        )
        failures.push(claimOwnerOperationErrorToast("loadScopeNames", t, projectsResult.reason))
      }
      setScopeNameErrorToasts(failures)
    })
    return () => {
      cancelled = true
    }
  }, [t])

  useEffect(() => {
    if (statusFilter !== "needs_review") {
      setActiveClaimsForConflict([])
      setActiveClaimConflictIndex(new Map())
      clearClaimListSummaryError("activeConflict")
      setReviewHistoryOpen(false)
      return
    }
    let cancelled = false
    void getTransport()
      .call<ClaimRecord[]>("claim_list", { status: "active", limit: 500 })
      .then((list) => {
        if (cancelled) return
        const active = list ?? []
        setActiveClaimsForConflict(active)
        setActiveClaimConflictIndex(buildClaimConflictIndex(active))
        clearClaimListSummaryError("activeConflict")
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::activeConflictIndex",
          "Failed to load active claims for conflict grouping",
          e,
        )
        if (!cancelled) {
          setActiveClaimsForConflict([])
          setActiveClaimConflictIndex(new Map())
          markClaimListSummaryError("activeConflict", e)
        }
      })
    return () => {
      cancelled = true
    }
  }, [clearClaimListSummaryError, markClaimListSummaryError, statusFilter])

  useEffect(() => {
    if (statusFilter !== "needs_review" || !reviewHistoryOpen) return
    void loadReviewHistory()
  }, [statusFilter, reviewHistoryOpen, loadReviewHistory])

  useEffect(() => {
    setBatchSelectedIds((prev) => {
      if (prev.size === 0) return prev
      if (statusFilter !== "needs_review") return new Set()
      const visibleIds = new Set(claims.map((claim) => claim.id))
      const next = new Set([...prev].filter((id) => visibleIds.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [claims, statusFilter])

  const scopeLabel = useCallback(
    (c: { scopeType: string; scopeId?: string | null }) => {
      if (c.scopeType === "global") return t("dashboard.dreaming.review.scopeGlobal")
      const id = c.scopeId ?? "?"
      if (c.scopeType === "agent") {
        const name = c.scopeId ? agentNames.get(c.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeAgent")}: ${name}` : `agent:${id}`
      }
      if (c.scopeType === "project") {
        const name = c.scopeId ? projectNames.get(c.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeProject")}: ${name}` : `project:${id}`
      }
      return `${c.scopeType}:${id}`
    },
    [agentNames, projectNames, t],
  )
  const evidenceClassLabel = (value: string) =>
    EVIDENCE_CLASS_LABEL_KEYS[value]
      ? t(`settings.claims.evidenceClasses.${EVIDENCE_CLASS_LABEL_KEYS[value]}`)
      : value.replace(/_/g, " ")
  const openEvidenceSource = (evidence: EvidenceRecord) => {
    if (!evidence.sessionId) return
    requestChatFocus({
      sessionId: evidence.sessionId,
      targetMessageId: parseEvidenceMessageId(evidence.messageId),
    })
  }
  const openEvidenceFile = (evidence: EvidenceRecord) => {
    if (!evidence.filePath) return
    void getTransport()
      .openFilePath(evidence.filePath, { sessionId: evidence.sessionId ?? undefined })
      .catch((e) => {
        logger.warn("settings", "ClaimsBetaView::openEvidenceFile", "Failed to open file", e)
        const failureToast = claimOwnerOperationErrorToast("openEvidenceSource", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      })
  }
  const openEvidenceUrl = (evidence: EvidenceRecord) => {
    if (!evidence.url) return
    openExternalUrl(evidence.url, {
      onError: (e) => {
        logger.warn("settings", "ClaimsBetaView::openEvidenceUrl", "Failed to open URL", e)
        const failureToast = claimOwnerOperationErrorToast("openEvidenceSource", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      },
    })
  }
  const evidenceDiagnosticMarkdown = (claim: ClaimRecord, evidence: EvidenceRecord): string => {
    const trustKey = evidenceTrustKey(evidence)
    const searchDiagnostic = claimSearchDiagnosticForClaim(claim)
    const lines = [
      `# Evidence ${evidence.id}`,
      "",
      `- claim: ${claim.id}`,
      `- claim content: ${claim.content}`,
      `- scope: ${scopeLabel(claim)}`,
      `- trust: ${trustLabel(trustKey)}`,
      `- source type: ${evidenceSourceLabel(evidence.sourceType)} (${evidence.sourceType})`,
      `- evidence class: ${evidenceClassLabel(evidence.evidenceClass)} (${evidence.evidenceClass})`,
      `- source id: ${evidence.sourceId || "unknown"}`,
      evidence.sessionId ? `- session id: ${evidence.sessionId}` : null,
      evidence.messageId ? `- message id: ${evidence.messageId}` : null,
      evidence.filePath ? `- file: ${evidence.filePath}` : null,
      evidence.url ? `- url: ${evidence.url}` : null,
      `- created at: ${evidence.createdAt}`,
      "",
      "## Trust explanation",
      trustDetail(trustKey),
    ].filter((line): line is string => line !== null)

    if (searchDiagnostic) {
      const matchLabels = searchDiagnostic.matches
        .map((match) => t(`settings.claims.searchMatch_${match.kind}`))
        .join(", ")
      const rankLabels = searchDiagnostic.rankSignals.map(claimRankSignalLabel).join(", ")
      lines.push(
        ...[
          "",
          "## List search diagnostics",
          `- query: ${searchDiagnostic.query}`,
          `- sort mode: ${searchDiagnostic.runtimeMode}`,
          matchLabels ? `- matched by: ${matchLabels}` : null,
          rankLabels ? `- ranked by: ${rankLabels}` : null,
        ].filter((line): line is string => line !== null),
      )
    }

    if (evidence.quote?.trim()) {
      lines.push(
        "",
        "## Quote",
        ...evidence.quote
          .trim()
          .split(/\r?\n/)
          .map((line) => `> ${line}`),
      )
    }

    return lines.join("\n")
  }
  const copyEvidenceDetails = async (claim: ClaimRecord, evidence: EvidenceRecord) => {
    try {
      await navigator.clipboard.writeText(evidenceDiagnosticMarkdown(claim, evidence))
      toast.success(t("settings.claims.copyEvidenceDone"))
    } catch (e) {
      logger.warn(
        "settings",
        "ClaimsBetaView::copyEvidenceDetails",
        "Failed to copy evidence details",
        e,
      )
      const failureToast = claimClipboardErrorToast("copyEvidence", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }
  const copyClaimLink = async (claim: ClaimRecord) => {
    const target = currentClaimFocusTarget(claim.id)
    const url = memoryFocusUrl(target)
    try {
      await navigator.clipboard.writeText(url)
      setMemoryFocusUrl(target)
      toast.success(t("settings.claims.copyLinkDone"))
    } catch (e) {
      logger.warn("settings", "ClaimsBetaView::copyClaimLink", "Failed to copy claim link", e)
      const failureToast = claimClipboardErrorToast("copyLink", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }
  const claimTypeFilterLabel = (value: ClaimTypeFilter) => {
    if (value === "all") return t("settings.claims.typeAll")
    if (value === "profile") return t("settings.claims.typeProfile")
    return t(`settings.claimType_${value}`, value)
  }
  const evidenceClassFilterLabel = (value: EvidenceClassFilter) => {
    if (value === "all") return t("settings.claims.evidenceAll")
    return evidenceClassLabel(value)
  }
  const confidenceSourceFilterLabel = (value: ConfidenceSourceFilter) => {
    if (value === "all") return t("settings.claims.confidenceSourceAll")
    return t(`settings.claims.confidenceSources.${value}`, value)
  }
  const evidenceSourceLabel = (value: string) => {
    if (value === "all") return t("settings.claims.evidenceSourceAll")
    return t(`settings.claims.evidenceSources.${value}`, value.replace(/_/g, " "))
  }
  const evidenceSourceFilterLabel = (value: EvidenceSourceFilter) => {
    if (value === "all") return t("settings.claims.evidenceSourceAll")
    return evidenceSourceLabel(value)
  }
  const claimRankSignalLabel = (signal: ClaimSearchRankSignal) => {
    if (signal.kind === "salience") {
      return `${t("settings.claims.salience", "Salience")} ${(
        Number(signal.value) * 100
      ).toFixed(0)}%`
    }
    if (signal.kind === "confidence") {
      return `${t("settings.claims.confidence", "Confidence")} ${(
        Number(signal.value) * 100
      ).toFixed(0)}%`
    }
    if (signal.kind === "created") {
      const key =
        signal.direction === "asc"
          ? "settings.claims.sort.created_asc"
          : "settings.claims.sort.created_desc"
      return `${t(key)} ${formatActivityTime(String(signal.value))}`
    }
    return `${t("settings.claims.sort.updated_desc")} ${formatActivityTime(String(signal.value))}`
  }
  const scopeFilterLabel = useCallback(
    (value: string) => {
      if (value === "all") return t("settings.claims.scopeAll")
      if (value === "global") return t("dashboard.dreaming.review.scopeGlobal")
      const separator = value.indexOf(":")
      if (separator <= 0) return value
      const scopeType = value.slice(0, separator)
      const scopeId = value.slice(separator + 1)
      if (scopeType === "agent") {
        const name = agentNames.get(scopeId)
        return name ? `${t("dashboard.dreaming.review.scopeAgent")}: ${name}` : `agent:${scopeId}`
      }
      if (scopeType === "project") {
        const name = projectNames.get(scopeId)
        return name
          ? `${t("dashboard.dreaming.review.scopeProject")}: ${name}`
          : `project:${scopeId}`
      }
      return value
    },
    [agentNames, projectNames, t],
  )
  const scopeFilterOptions = useMemo(() => {
    const options = [
      { value: "all", label: t("settings.claims.scopeAll") },
      { value: "global", label: t("dashboard.dreaming.review.scopeGlobal") },
    ]
    for (const [id, name] of [...agentNames.entries()].sort((a, b) => a[1].localeCompare(b[1]))) {
      options.push({
        value: `agent:${id}`,
        label: `${t("dashboard.dreaming.review.scopeAgent")}: ${name}`,
      })
    }
    for (const [id, name] of [...projectNames.entries()].sort((a, b) => a[1].localeCompare(b[1]))) {
      options.push({
        value: `project:${id}`,
        label: `${t("dashboard.dreaming.review.scopeProject")}: ${name}`,
      })
    }
    if (scopeFilter !== "all" && !options.some((option) => option.value === scopeFilter)) {
      options.push({ value: scopeFilter, label: scopeFilterLabel(scopeFilter) })
    }
    return options
  }, [agentNames, projectNames, scopeFilter, scopeFilterLabel, t])
  const reviewHistoryScopeOptions = useMemo(() => {
    const options = [...scopeFilterOptions]
    if (
      reviewHistoryScopeFilter !== "all" &&
      !options.some((option) => option.value === reviewHistoryScopeFilter)
    ) {
      options.push({
        value: reviewHistoryScopeFilter,
        label: scopeFilterLabel(reviewHistoryScopeFilter),
      })
    }
    return options
  }, [reviewHistoryScopeFilter, scopeFilterLabel, scopeFilterOptions])
  const reviewHistoryDecisionOptions = useMemo(() => {
    const values = new Set<string>(["all", ...REVIEW_HISTORY_DECISION_TYPES])
    for (const item of reviewHistory) values.add(item.decisionType)
    if (reviewHistoryDecisionFilter !== "all") values.add(reviewHistoryDecisionFilter)
    return [...values]
  }, [reviewHistory, reviewHistoryDecisionFilter])

  const noCandidates = !plan || plan.summary.candidates === 0
  const isReviewQueue = statusFilter === "needs_review"
  const normalizedClaimSearchQuery = normalizeClaimSearch(claimSearchQuery)
  const normalizedReviewHistoryQuery = normalizeClaimSearch(reviewHistoryQuery)
  const hasClaimSearch = normalizedClaimSearchQuery.length > 0
  const claimSearchDiagnosticForClaim = (claim: ClaimRecord) =>
    hasClaimSearch
      ? claimSearchDiagnostics(
          claim,
          normalizedClaimSearchQuery,
          scopeLabel(claim),
          claimListSort,
        )
      : null
  const claimSearchBackendSynced = claimSearchBackendQuery === normalizedClaimSearchQuery
  const claimDefaultLoadedCount = hasClaimSearch
    ? CLAIM_LIST_SEARCH_PAGE_SIZE
    : CLAIM_LIST_DEFAULT_PAGE_SIZE
  const claimLoadedForFocus = claimLoadedCount > claimDefaultLoadedCount ? claimLoadedCount : null
  const currentClaimFocusFilters = useMemo<ClaimFocusFilters>(
    () => ({
      statusFilter,
      claimType: claimTypeFilter,
      claimSort: claimListSort,
      claimLoaded: claimLoadedForFocus,
      ...scopeFilterFocus(scopeFilter),
      confidenceSource: confidenceSourceFilter,
      evidenceClass: evidenceClassFilter,
      evidenceSource: evidenceSourceFilter,
      query: claimSearchQuery.trim() || null,
      reviewHistory: isReviewQueue && reviewHistoryOpen,
      reviewHistoryDecisionType:
        isReviewQueue && reviewHistoryOpen && reviewHistoryDecisionFilter !== "all"
          ? reviewHistoryDecisionFilter
          : null,
      reviewHistoryTimeRange:
        isReviewQueue && reviewHistoryOpen && reviewHistoryTimeRange !== "all"
          ? reviewHistoryTimeRange
          : null,
      ...(isReviewQueue && reviewHistoryOpen && reviewHistoryScopeFilter !== "all"
        ? reviewHistoryScopeFilterFocus(reviewHistoryScopeFilter)
        : { reviewHistoryScopeType: null, reviewHistoryScopeId: null }),
      reviewHistoryQuery:
        isReviewQueue && reviewHistoryOpen ? reviewHistoryQuery.trim() || null : null,
    }),
    [
      claimTypeFilter,
      claimLoadedForFocus,
      claimListSort,
      claimSearchQuery,
      confidenceSourceFilter,
      evidenceClassFilter,
      evidenceSourceFilter,
      isReviewQueue,
      reviewHistoryDecisionFilter,
      reviewHistoryOpen,
      reviewHistoryQuery,
      reviewHistoryScopeFilter,
      reviewHistoryTimeRange,
      scopeFilter,
      statusFilter,
    ],
  )
  const currentClaimFocusTarget = useCallback(
    (id?: string | null): MemoryFocusTarget =>
      id
        ? { kind: "claim", id, ...currentClaimFocusFilters }
        : { kind: "claims", ...currentClaimFocusFilters },
    [currentClaimFocusFilters],
  )
  const visibleClaims = useMemo(
    () =>
      hasClaimSearch && !claimSearchBackendSynced
        ? claims.filter((claim) =>
            claimMatchesSearch(claim, normalizedClaimSearchQuery, scopeLabel(claim)),
          )
        : claims,
    [claimSearchBackendSynced, claims, hasClaimSearch, normalizedClaimSearchQuery, scopeLabel],
  )
  const claimTotalIsCurrent = claimSearchBackendSynced
  const claimTotalForDisplay = claimTotalIsCurrent ? (claimTotal ?? claims.length) : claims.length
  const claimDisplayTotal =
    claimTotalIsCurrent && claimTotalTruncated ? `${claimTotalForDisplay}+` : claimTotalForDisplay
  const claimListCountLabel =
    hasClaimSearch && !claimSearchBackendSynced
      ? `${visibleClaims.length}/${claims.length}`
      : claimTotalForDisplay !== visibleClaims.length ||
          (claimTotalIsCurrent && claimTotalTruncated)
        ? `${visibleClaims.length}/${claimDisplayTotal}`
        : `${visibleClaims.length}`
  const filteredReviewHistory = useMemo(() => {
    const now = Date.now()
    return reviewHistory.filter(
      (item) =>
        (reviewHistoryDecisionFilter === "all" ||
          item.decisionType === reviewHistoryDecisionFilter) &&
        reviewHistoryItemMatchesTimeRange(item, reviewHistoryTimeRange, now) &&
        reviewHistoryItemMatchesScope(item, reviewHistoryScopeFilter) &&
        reviewHistoryItemMatchesQuery(
          item,
          normalizedReviewHistoryQuery,
          item.scopeType ? scopeFilterLabel(scopeFilterValue(item.scopeType, item.scopeId)) : "",
        ),
    )
  }, [
    normalizedReviewHistoryQuery,
    reviewHistory,
    reviewHistoryDecisionFilter,
    reviewHistoryScopeFilter,
    reviewHistoryTimeRange,
    scopeFilterLabel,
  ])
  const reviewHistoryTotalIsCurrent = reviewHistoryBackendQuery === normalizedReviewHistoryQuery
  const reviewHistoryTotalForDisplay = reviewHistoryTotalIsCurrent
    ? (reviewHistoryTotal ?? reviewHistory.length)
    : reviewHistory.length
  const reviewHistoryDisplayTotal =
    reviewHistoryTotalIsCurrent && reviewHistoryTotalTruncated
      ? `${reviewHistoryTotalForDisplay}+`
      : reviewHistoryTotalForDisplay
  const allBatchSelected =
    isReviewQueue &&
    visibleClaims.length > 0 &&
    visibleClaims.every((claim) => batchSelectedIds.has(claim.id))
  const batchSelectedCount = batchSelectedIds.size
  const reviewConflictKeys = isReviewQueue ? visibleConflictKeys(claims) : new Set<string>()
  const reviewBuckets = isReviewQueue
    ? buildReviewBuckets(
        visibleClaims,
        activeClaimConflictIndex,
        reviewConflictKeys,
        claimConflictSummaries,
        profileClaimTypes,
        claimReviewSummaries,
      )
    : []
  const localDetailConflictMatches =
    detail?.claim.status === "needs_review"
      ? conflictMatchesForClaim(detail.claim, claims, activeClaimsForConflict)
      : []
  const loadedDetailConflictMatches =
    detail?.claim.status === "needs_review"
      ? detailConflictClaims.map((claim) => ({ claim, kind: matchKindForClaim(claim) }))
      : []
  const detailConflictMatches = mergeConflictMatches(
    loadedDetailConflictMatches,
    localDetailConflictMatches,
  )
  const visibleConflictActionTargets = detailConflictMatches.slice(0, 5)
  const hiddenConflictActionTargetCount = Math.max(
    0,
    detailConflictMatches.length - visibleConflictActionTargets.length,
  )
  const conflictInsight =
    detail?.claim.status === "needs_review"
      ? buildConflictInsight(detail.claim, detailConflictMatches)
      : null
  const detailTrustKey = detail ? claimTrustKey(detail.claim, detail.evidence) : null
  const detailEvidenceStats = detail ? evidenceTrustStats(detail.evidence) : null
  const conflictCompareTrustKey = conflictCompareDetail
    ? claimTrustKey(conflictCompareDetail.claim, conflictCompareDetail.evidence)
    : null
  const conflictCompareEvidenceStats = conflictCompareDetail
    ? evidenceTrustStats(conflictCompareDetail.evidence)
    : null
  const strongestCurrentEvidence = detail ? bestEvidence(detail.evidence) : null
  const strongestConflictEvidence = conflictCompareDetail
    ? bestEvidence(conflictCompareDetail.evidence)
    : null
  const reviewProjectionForVisibleClaim = (claim: ClaimRecord): ReviewProjection =>
    reviewProjectionForClaim(
      claim,
      reviewConflictKeys,
      activeClaimConflictIndex,
      claimConflictSummaries,
      profileClaimTypes,
      claimReviewSummaries,
    )
  const reviewReasonLabel = (key: ReviewBucketKey): string =>
    t(`settings.claims.reviewBuckets.${key}`)
  const reviewReasonDetail = (key: ReviewBucketKey): string =>
    t(`settings.claims.reviewReasonDetails.${key}`)
  const reviewRiskLabel = (key: ReviewRiskKey): string => t(`settings.claims.reviewRisks.${key}`)
  const detailReviewProjection =
    detail?.claim.status === "needs_review" ? reviewProjectionForVisibleClaim(detail.claim) : null
  const claimGraphNodeLabels = useMemo(
    () => new Map((claimGraph?.nodes ?? []).map((node) => [node.id, node.label])),
    [claimGraph],
  )
  const claimGraphEdges = claimGraph?.edges.slice(0, 5) ?? []
  const trustLabel = (key: ClaimTrustKey): string => t(`settings.claims.trust.${key}.label`)
  const trustDetail = (key: ClaimTrustKey): string => t(`settings.claims.trust.${key}.detail`)
  const decisionTypeLabel = useCallback(
    (decisionType: string) =>
      t(`settings.memoryDecisionTypes.${decisionType}`, decisionType.replace(/_/g, " ")),
    [t],
  )
  const reviewHistoryDecisionFilterLabel = (value: string) =>
    value === "all" ? t("settings.claims.reviewHistoryDecisionAll") : decisionTypeLabel(value)
  const reviewHistoryTimeRangeLabel = (value: ReviewHistoryTimeRange) =>
    t(`settings.claims.reviewHistoryTimeRanges.${value}`)
  const resetClaimListPosition = () => {
    pendingSelectedIdRef.current = null
    claimListNextOffsetRef.current = 0
    setClaimLoadedTarget(null)
    setClaimLoadedCount(0)
  }
  const claimStatusFilterLabel = (value: string) =>
    value === "all"
      ? t("settings.claims.statusAll")
      : t(`settings.claims.status.${value}`, value.replace(/_/g, " "))
  const claimListSortLabel = (value: ClaimListSort) =>
    t(`settings.claims.sort.${value}`, value.replace(/_/g, " "))
  const claimListSortRuntime = claimListSortRuntimeMode(claimListSort, claimSearchBackendQuery)
  const claimListSortRuntimeText =
    claimListSortRuntime === "best_match"
      ? t(
          "settings.claims.sortRuntimeBestMatch",
          "Best match uses claim and evidence hits.",
        )
      : claimListSortRuntime === "recent_fallback"
        ? t(
            "settings.claims.sortRuntimeRecentFallback",
            "No search query, showing recently updated.",
          )
        : t("settings.claims.sortRuntimeExplicit", "Using selected sort order.")
  const currentClaimListPresetId = claimListPresetId({
    statusFilter,
    claimTypeFilter,
    scopeFilter,
    confidenceSourceFilter,
    evidenceClassFilter,
    evidenceSourceFilter,
    sort: claimListSort,
    query: claimSearchQuery,
  })
  const claimListPresetLabel = (preset: ClaimListFilterPreset) => {
    const parts = [
      claimStatusFilterLabel(preset.statusFilter),
      claimTypeFilterLabel(preset.claimTypeFilter),
      scopeFilterLabel(preset.scopeFilter),
      confidenceSourceFilterLabel(preset.confidenceSourceFilter),
      evidenceClassFilterLabel(preset.evidenceClassFilter),
      evidenceSourceFilterLabel(preset.evidenceSourceFilter),
      claimListSortLabel(preset.sort),
    ]
    if (preset.query) parts.push(`"${preset.query}"`)
    return parts.join(" / ")
  }
  const saveClaimListFilterPreset = () => {
    const preset: ClaimListFilterPreset = {
      id: currentClaimListPresetId,
      statusFilter,
      claimTypeFilter,
      scopeFilter,
      confidenceSourceFilter,
      evidenceClassFilter,
      evidenceSourceFilter,
      sort: claimListSort,
      query: claimSearchQuery.trim(),
      updatedAt: Date.now(),
    }
    setClaimListFilterPresets((prev) => {
      const next = [preset, ...prev.filter((item) => item.id !== preset.id)].slice(
        0,
        CLAIM_LIST_PRESET_LIMIT,
      )
      persistClaimListFilterPresets(next)
      return next
    })
    toast.success(t("settings.claims.filterPresetSaved"))
  }
  const applyClaimListFilterPreset = (preset: ClaimListFilterPreset) => {
    resetClaimListPosition()
    setStatusFilter(
      preset.statusFilter === "all" || claimSchema.statuses.includes(preset.statusFilter)
        ? preset.statusFilter
        : "all",
    )
    setClaimTypeFilter(normalizeClaimTypeFilter(preset.claimTypeFilter, claimSchema))
    setScopeFilter(preset.scopeFilter || "all")
    setConfidenceSourceFilter(
      normalizeConfidenceSourceFilter(preset.confidenceSourceFilter, claimSchema),
    )
    setEvidenceClassFilter(normalizeEvidenceClassFilter(preset.evidenceClassFilter, claimSchema))
    setEvidenceSourceFilter(normalizeEvidenceSourceFilter(preset.evidenceSourceFilter, claimSchema))
    setClaimListSort(normalizeClaimListSort(preset.sort))
    setClaimSearchQuery(preset.query)
    setClaimSearchBackendQuery(normalizeClaimSearch(preset.query))
  }
  const removeClaimListFilterPreset = (id: string) => {
    setClaimListFilterPresets((prev) => {
      const next = prev.filter((item) => item.id !== id)
      persistClaimListFilterPresets(next)
      return next
    })
  }
  const currentReviewHistoryPresetId = reviewHistoryPresetId(
    reviewHistoryDecisionFilter,
    reviewHistoryTimeRange,
    reviewHistoryScopeFilter,
    reviewHistoryQuery,
  )
  const reviewHistoryPresetLabel = (preset: ReviewHistoryFilterPreset) => {
    const parts = [
      reviewHistoryDecisionFilterLabel(preset.decisionType),
      reviewHistoryTimeRangeLabel(preset.timeRange),
      scopeFilterLabel(preset.scopeFilter),
    ]
    if (preset.query) parts.push(`"${preset.query}"`)
    return parts.join(" / ")
  }
  const saveReviewHistoryFilterPreset = () => {
    const preset: ReviewHistoryFilterPreset = {
      id: currentReviewHistoryPresetId,
      decisionType: reviewHistoryDecisionFilter,
      timeRange: reviewHistoryTimeRange,
      scopeFilter: reviewHistoryScopeFilter,
      query: reviewHistoryQuery.trim(),
      updatedAt: Date.now(),
    }
    setReviewHistoryFilterPresets((prev) => {
      const next = [preset, ...prev.filter((item) => item.id !== preset.id)].slice(
        0,
        REVIEW_HISTORY_PRESET_LIMIT,
      )
      persistReviewHistoryFilterPresets(next)
      return next
    })
    toast.success(t("settings.claims.reviewHistoryPresetSaved"))
  }
  const applyReviewHistoryFilterPreset = (preset: ReviewHistoryFilterPreset) => {
    setReviewHistoryDecisionFilter(preset.decisionType)
    setReviewHistoryTimeRange(preset.timeRange)
    setReviewHistoryScopeFilter(preset.scopeFilter)
    setReviewHistoryQuery(preset.query)
    setReviewHistoryBackendQuery(normalizeClaimSearch(preset.query))
  }
  const removeReviewHistoryFilterPreset = (id: string) => {
    setReviewHistoryFilterPresets((prev) => {
      const next = prev.filter((item) => item.id !== id)
      persistReviewHistoryFilterPresets(next)
      return next
    })
  }
  const normalizeReviewHistoryExportLine = (value: string | null | undefined) =>
    value?.replace(/\s+/g, " ").trim() ?? ""
  const reviewHistoryExportFilterParts = (query: string) =>
    [
      reviewHistoryDecisionFilterLabel(reviewHistoryDecisionFilter),
      reviewHistoryTimeRangeLabel(reviewHistoryTimeRange),
      scopeFilterLabel(reviewHistoryScopeFilter),
      query.trim() ? `query: ${normalizeReviewHistoryExportLine(query)}` : null,
    ].filter(Boolean)
  const reviewHistoryItemMarkdown = (item: ReviewHistoryItem, headingLevel = 2): string => {
    const heading = normalizeReviewHistoryExportLine(item.content) || item.targetId || item.id
    const scope = item.scopeType
      ? scopeFilterLabel(scopeFilterValue(item.scopeType, item.scopeId))
      : null
    const lines = [
      `${"#".repeat(Math.max(1, headingLevel))} ${heading.replace(/^#+\s*/, "")}`,
      "",
      `- decision: ${decisionTypeLabel(item.decisionType)} (${item.decisionType})`,
      `- time: ${formatActivityTime(item.createdAt)} (${item.createdAt})`,
      `- trigger: ${t(`dashboard.dreaming.trigger.${item.trigger}`, item.trigger)} (${item.trigger})`,
      item.phase ? `- phase: ${item.phase}` : null,
      item.status ? `- status: ${item.status}` : null,
      scope ? `- scope: ${scope}` : null,
      item.targetId ? `- claim: ${item.targetId}` : null,
      `- decision id: ${item.id}`,
    ].filter((line): line is string => line !== null)

    if (item.rationale) {
      lines.push("", `${"#".repeat(Math.max(1, headingLevel + 1))} Rationale`, item.rationale)
    }
    return lines.join("\n")
  }
  const reviewHistoryExportMarkdown = (
    items: ReviewHistoryItem[],
    total: number | string,
    totalTruncated: boolean,
    query = reviewHistoryQuery,
  ) => {
    const filterParts = reviewHistoryExportFilterParts(query)
    const lines = [
      `# ${t("settings.claims.reviewHistoryTitle")}`,
      "",
      `- ${new Date().toLocaleString()}`,
      `- ${filterParts.join(" / ")}`,
      `- ${t("settings.claims.reviewHistoryCount", {
        shown: items.length,
        total,
      })}`,
      "",
    ]
    if (totalTruncated) {
      lines.push(t("settings.claims.reviewHistoryExportTruncated", { count: items.length }))
      lines.push("")
    }
    for (const item of items) {
      lines.push(reviewHistoryItemMarkdown(item, 2), "")
    }
    return lines.join("\n")
  }
  const copyReviewHistoryItem = async (item: ReviewHistoryItem) => {
    try {
      await navigator.clipboard.writeText(reviewHistoryItemMarkdown(item, 1))
      toast.success(t("settings.claims.reviewHistoryCopyItemDone"))
    } catch (e) {
      logger.warn("settings", "ClaimsBetaView::reviewHistoryCopyItem", "Clipboard write failed", e)
      const failureToast = claimClipboardErrorToast("copyReviewHistoryItem", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }
  const copyReviewHistoryExport = async () => {
    if (filteredReviewHistory.length === 0) {
      toast.error(t("settings.claims.reviewHistoryExportEmpty"))
      return
    }
    try {
      await navigator.clipboard.writeText(
        reviewHistoryExportMarkdown(
          filteredReviewHistory,
          reviewHistoryDisplayTotal,
          reviewHistoryTotalIsCurrent && reviewHistoryTotalTruncated,
        ),
      )
      toast.success(t("settings.claims.reviewHistoryExportDone"))
    } catch (e) {
      logger.warn("settings", "ClaimsBetaView::reviewHistoryExport", "Clipboard write failed", e)
      const failureToast = claimClipboardErrorToast("copyReviewHistoryExport", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }
  const copyAllReviewHistoryExport = async () => {
    if (reviewHistoryExportingAll) return
    setReviewHistoryExportingAll(true)
    try {
      const tx = getTransport()
      const query = normalizeClaimSearch(reviewHistoryQuery)
      const now = Date.now()
      const exported: ReviewHistoryItem[] = []
      const seen = new Set<string>()
      let offset = 0
      let total = 0
      let totalTruncated = false
      while (true) {
        let response: DreamingDecisionListResponse | null = null
        try {
          response = await tx.call<DreamingDecisionListResponse>(
            "dreaming_list_decisions_page",
            reviewHistoryBackendArgs(offset, REVIEW_HISTORY_EXPORT_PAGE_SIZE, query),
          )
        } catch (e) {
          logger.warn(
            "settings",
            "ClaimsBetaView::reviewHistoryExportAllLoad",
            "Failed to load all review history for export",
            e,
          )
          const failureToast = claimOwnerOperationErrorToast("loadReviewHistory", t, e)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
          return
        }
        const items = response?.items ?? []
        total = Math.max(total, response?.total ?? 0, offset + items.length)
        totalTruncated = totalTruncated || response?.totalTruncated === true
        const mapped = items
          .filter(isReviewHistoryDecision)
          .map(reviewHistoryItemFromDecisionListItem)
          .filter(
            (item) =>
              reviewHistoryItemMatchesTimeRange(item, reviewHistoryTimeRange, now) &&
              reviewHistoryItemMatchesScope(item, reviewHistoryScopeFilter) &&
              reviewHistoryItemMatchesQuery(
                item,
                query,
                item.scopeType
                  ? scopeFilterLabel(scopeFilterValue(item.scopeType, item.scopeId))
                  : "",
              ),
          )
        for (const item of mapped) {
          if (seen.has(item.id)) continue
          seen.add(item.id)
          exported.push(item)
        }
        offset += items.length
        if (items.length === 0) break
        if (!totalTruncated && offset >= total) break
        if (items.length < REVIEW_HISTORY_EXPORT_PAGE_SIZE) break
      }
      if (exported.length === 0) {
        toast.error(t("settings.claims.reviewHistoryExportEmpty"))
        return
      }
      try {
        await navigator.clipboard.writeText(
          reviewHistoryExportMarkdown(
            exported,
            totalTruncated
              ? `${Math.max(total, exported.length)}+`
              : Math.max(total, exported.length),
            totalTruncated,
            reviewHistoryQuery,
          ),
        )
      } catch (e) {
        logger.warn(
          "settings",
          "ClaimsBetaView::reviewHistoryExportAllClipboard",
          "Clipboard write failed",
          e,
        )
        const failureToast = claimClipboardErrorToast("copyReviewHistoryExport", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
        return
      }
      toast.success(t("settings.claims.reviewHistoryExportAllDone", { count: exported.length }))
    } finally {
      setReviewHistoryExportingAll(false)
    }
  }
  useEffect(() => {
    if (!detail?.claim.id || detail.claim.status !== "needs_review") {
      setDetailConflictClaims([])
      setConflictMatrixDetails([])
      setConflictEvidenceErrorDetail(null)
      setConflictCandidatesErrorDetail(null)
      setDetailConflictLoading(false)
      return
    }
    let cancelled = false
    setDetailConflictClaims([])
    setConflictMatrixDetails([])
    setConflictEvidenceErrorDetail(null)
    setConflictCandidatesErrorDetail(null)
    setDetailConflictLoading(true)
    const tx = getTransport()
    void Promise.allSettled([
      tx.call<ClaimRecord[]>("claim_conflicts", { id: detail.claim.id, limit: 100 }),
      tx.call<ClaimDetail[]>("claim_conflict_details", { id: detail.claim.id, limit: 5 }),
    ])
      .then(([conflictsResult, detailsResult]) => {
        if (cancelled) return
        const conflicts =
          conflictsResult.status === "fulfilled" ? (conflictsResult.value ?? []) : []
        const details = detailsResult.status === "fulfilled" ? (detailsResult.value ?? []) : []
        if (conflictsResult.status === "rejected") {
          logger.warn(
            "settings",
            "ClaimsBetaView::claimConflicts",
            "Failed to load claim conflicts",
            conflictsResult.reason,
          )
          setConflictCandidatesErrorDetail(claimOwnerOperationErrorDetail(conflictsResult.reason))
        }
        if (detailsResult.status === "rejected") {
          logger.warn(
            "settings",
            "ClaimsBetaView::claimConflictDetails",
            "Failed to load claim conflict details",
            detailsResult.reason,
          )
          setConflictEvidenceErrorDetail(claimOwnerOperationErrorDetail(detailsResult.reason))
        }
        setConflictMatrixDetails(details)
        setDetailConflictClaims(
          mergeConflictMatches(
            conflicts.map((claim) => ({ claim, kind: matchKindForClaim(claim) })),
            details.map((item) => ({ claim: item.claim, kind: matchKindForClaim(item.claim) })),
          ).map((match) => match.claim),
        )
      })
      .finally(() => {
        if (!cancelled) setDetailConflictLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [detail?.claim.id, detail?.claim.status])
  useEffect(() => {
    const strongestId = conflictInsight?.strongest?.id
    if (!strongestId || strongestId === detail?.claim.id) {
      setConflictCompareDetail(null)
      setConflictEvidenceErrorDetail(null)
      setConflictCompareLoading(false)
      return
    }
    const matrixDetail = conflictMatrixDetails.find((item) => item.claim.id === strongestId)
    if (matrixDetail) {
      setConflictCompareDetail(matrixDetail)
      setConflictCompareLoading(false)
      return
    }
    let cancelled = false
    setConflictEvidenceErrorDetail(null)
    setConflictCompareLoading(true)
    void getTransport()
      .call<ClaimDetail | null>("claim_get", { id: strongestId })
      .then((nextDetail) => {
        if (cancelled) return
        setConflictCompareDetail(nextDetail ?? null)
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "ClaimsBetaView::conflictEvidenceDiff",
          "Failed to load strongest conflicting claim detail",
          e,
        )
        if (!cancelled) {
          setConflictCompareDetail(null)
          setConflictEvidenceErrorDetail(claimOwnerOperationErrorDetail(e))
        }
      })
      .finally(() => {
        if (!cancelled) setConflictCompareLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [conflictInsight?.strongest?.id, conflictMatrixDetails, detail?.claim.id])

  useEffect(() => {
    setMemoryFocusUrl(currentClaimFocusTarget(selectedId))
  }, [currentClaimFocusTarget, selectedId])
  useEffect(() => {
    setBatchSelectedIds((prev) => {
      if (prev.size === 0) return prev
      if (!isReviewQueue) return new Set()
      const visibleIds = new Set(visibleClaims.map((claim) => claim.id))
      const next = new Set([...prev].filter((id) => visibleIds.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [isReviewQueue, visibleClaims])

  const toggleBatchSelectAll = () => {
    setBatchSelectedIds((prev) => {
      if (visibleClaims.length === 0) return prev
      const allSelected = visibleClaims.every((claim) => prev.has(claim.id))
      if (allSelected) {
        const next = new Set(prev)
        for (const claim of visibleClaims) next.delete(claim.id)
        return next
      }
      const next = new Set(prev)
      for (const claim of visibleClaims) next.add(claim.id)
      return next
    })
  }
  const scopeHelp = (c: { scopeType: string; scopeId?: string | null }): string => {
    if (c.scopeType === "global") return t("settings.claims.scopeHelp.global")
    if (c.scopeType === "agent") return t("settings.claims.scopeHelp.agent")
    if (c.scopeType === "project") return t("settings.claims.scopeHelp.project")
    return t("settings.claims.scopeHelp.unknown")
  }
  const canOpenScope = (c: { scopeType: string; scopeId?: string | null }): boolean =>
    (c.scopeType === "agent" || c.scopeType === "project") && !!c.scopeId
  const openScope = (c: { scopeType: string; scopeId?: string | null }) => {
    if (!canOpenScope(c) || !c.scopeId) return
    requestMemoryScopeFocus({ kind: c.scopeType as "agent" | "project", id: c.scopeId })
  }
  const openSourceLabel = (evidence: EvidenceRecord): string => {
    if (evidence.sessionId) return t("settings.claims.openChatSource")
    if (evidence.filePath) return t("settings.claims.openFileSource")
    if (evidence.url) return t("settings.claims.openUrlSource")
    return t("chat.memoryTrace.openSource")
  }
  const keepExistingConflict = async () => {
    if (conflictResolving || detail?.claim.status !== "needs_review") return
    const currentId = detail.claim.id
    const strongest = conflictInsight?.strongest ?? null
    const loadedStrongest = strongest
      ? (conflictMatrixDetails.find((item) => item.claim.id === strongest.id) ??
        (conflictCompareDetail?.claim.id === strongest.id ? conflictCompareDetail : null))
      : null
    const note = conflictResolutionNote({
      action: "keep_existing",
      current: detail.claim,
      existing: strongest,
      currentTrustKey: detailTrustKey,
      currentStats: detailEvidenceStats,
      currentEvidenceCount: detail.evidence.length,
      existingTrustKey: loadedStrongest
        ? claimTrustKey(loadedStrongest.claim, loadedStrongest.evidence)
        : null,
      existingStats: loadedStrongest ? evidenceTrustStats(loadedStrongest.evidence) : null,
      existingEvidenceCount: loadedStrongest ? loadedStrongest.evidence.length : null,
      activeConflictCount: conflictInsight?.activeCount,
      archivedConflictCount: 1,
    })
    setConflictResolving(true)
    try {
      await getTransport().call("claim_forget", {
        id: currentId,
        permanent: false,
        note,
      })
      setBatchSelectedIds((prev) => {
        if (!prev.has(currentId)) return prev
        const next = new Set(prev)
        next.delete(currentId)
        return next
      })
      setSelectedId(null)
      setDetail(null)
      await fetchClaims()
      if (reviewHistoryOpen) await loadReviewHistory()
      showArchiveRestoreToast(t("settings.claims.conflictKeptExisting"), [currentId])
    } catch (e) {
      logger.error(
        "settings",
        "ClaimsBetaView::keepExistingConflict",
        "Failed to archive conflicting review claim",
        e,
      )
      const failureToast = claimOwnerOperationErrorToast("conflictResolve", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setConflictResolving(false)
    }
  }

  const confirmUseCurrentConflict = async () => {
    if (conflictResolving || detail?.claim.status !== "needs_review") return
    const currentId = detail.claim.id
    const targets = detailConflictMatches.filter(({ claim }) => claim.id !== currentId)
    const strongest = conflictInsight?.strongest ?? null
    const loadedStrongest = strongest
      ? (conflictMatrixDetails.find((item) => item.claim.id === strongest.id) ??
        (conflictCompareDetail?.claim.id === strongest.id ? conflictCompareDetail : null))
      : null
    const currentNote = conflictResolutionNote({
      action: "use_current",
      current: detail.claim,
      existing: strongest,
      currentTrustKey: detailTrustKey,
      currentStats: detailEvidenceStats,
      currentEvidenceCount: detail.evidence.length,
      existingTrustKey: loadedStrongest
        ? claimTrustKey(loadedStrongest.claim, loadedStrongest.evidence)
        : null,
      existingStats: loadedStrongest ? evidenceTrustStats(loadedStrongest.evidence) : null,
      existingEvidenceCount: loadedStrongest ? loadedStrongest.evidence.length : null,
      activeConflictCount: conflictInsight?.activeCount,
      archivedConflictCount: targets.length,
    })
    setConflictResolving(true)
    try {
      const tx = getTransport()
      await tx.call("claim_update", {
        id: currentId,
        status: "active",
        note: currentNote,
      })

      const archivedIds = new Set<string>()
      let firstArchiveError: unknown = null
      for (const { claim } of targets) {
        try {
          const targetDetail =
            conflictMatrixDetails.find((item) => item.claim.id === claim.id) ??
            (conflictCompareDetail?.claim.id === claim.id ? conflictCompareDetail : null)
          await tx.call("claim_forget", {
            id: claim.id,
            permanent: false,
            note: conflictResolutionNote({
              action: "archive_superseded",
              current: detail.claim,
              existing: claim,
              currentTrustKey: detailTrustKey,
              currentStats: detailEvidenceStats,
              currentEvidenceCount: detail.evidence.length,
              existingTrustKey: targetDetail
                ? claimTrustKey(targetDetail.claim, targetDetail.evidence)
                : null,
              existingStats: targetDetail ? evidenceTrustStats(targetDetail.evidence) : null,
              existingEvidenceCount: targetDetail ? targetDetail.evidence.length : null,
              activeConflictCount: conflictInsight?.activeCount,
              archivedConflictCount: 1,
            }),
          })
          archivedIds.add(claim.id)
        } catch (e) {
          logger.warn(
            "settings",
            "ClaimsBetaView::useCurrentConflict",
            "One conflicting claim failed to archive",
            e,
          )
          firstArchiveError ??= e
        }
      }

      setBatchSelectedIds((prev) => {
        if (prev.size === 0) return prev
        const next = new Set(prev)
        next.delete(currentId)
        for (const id of archivedIds) next.delete(id)
        return next
      })
      setConflictResolveOpen(false)
      setSelectedId(null)
      setDetail(null)
      await fetchClaims()
      await refreshActiveClaimsForConflict()
      if (reviewHistoryOpen) await loadReviewHistory()

      const failed = targets.length - archivedIds.size
      if (failed > 0) {
        const failureToast = claimOwnerOperationErrorToast("conflictResolve", t, firstArchiveError)
        toast.warning(
          t("settings.claims.conflictResolvePartial", {
            count: archivedIds.size,
            failed,
          }),
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } else {
        toast.success(
          t("settings.claims.conflictUsedCurrent", {
            count: archivedIds.size,
          }),
        )
      }
    } catch (e) {
      logger.error(
        "settings",
        "ClaimsBetaView::useCurrentConflict",
        "Failed to enable current conflicting claim",
        e,
      )
      const failureToast = claimOwnerOperationErrorToast("conflictResolve", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setConflictResolving(false)
    }
  }

  const renderEvidenceDiffColumn = (
    title: string,
    claim: ClaimRecord,
    evidence: EvidenceRecord[],
    trustKey: ClaimTrustKey,
    stats: EvidenceTrustStats,
    strongestEvidence: EvidenceRecord | null,
  ) => (
    <div className="min-w-0 rounded border border-border/40 bg-background/70 px-2 py-2">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="truncate text-[11px] font-medium text-foreground">{title}</div>
        <span className="shrink-0 rounded bg-secondary/70 px-1.5 py-0.5 text-[10px]">
          {trustLabel(trustKey)}
        </span>
      </div>
      <div className="mt-1 truncate text-[11px] font-medium">{claim.object}</div>
      <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
        <span className="rounded bg-secondary/60 px-1.5 py-0.5">
          {t("settings.claims.evidenceCount", { count: evidence.length })}
        </span>
        {stats.confirmed > 0 && (
          <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300">
            {t("settings.claims.confirmedEvidenceCount", { count: stats.confirmed })}
          </span>
        )}
        {stats.sourceBacked > 0 && (
          <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-700 dark:text-sky-300">
            {t("settings.claims.sourceBackedEvidenceCount", { count: stats.sourceBacked })}
          </span>
        )}
        {stats.inferred > 0 && (
          <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">
            {t("settings.claims.inferredEvidenceCount", { count: stats.inferred })}
          </span>
        )}
      </div>
      {strongestEvidence ? (
        <div className="mt-2 rounded border border-border/30 bg-secondary/20 px-2 py-1.5">
          <div className="flex flex-wrap gap-1 text-[10px] text-muted-foreground">
            <span>{t("settings.claims.conflictBestEvidence")}</span>
            <span>·</span>
            <span>{evidenceSourceLabel(strongestEvidence.sourceType)}</span>
            <span>·</span>
            <span>{evidenceClassLabel(strongestEvidence.evidenceClass)}</span>
          </div>
          <div className="mt-1 max-h-8 overflow-hidden text-[10px] leading-relaxed text-muted-foreground">
            {strongestEvidence.quote ||
              strongestEvidence.filePath ||
              strongestEvidence.url ||
              strongestEvidence.sourceId}
          </div>
        </div>
      ) : (
        <div className="mt-2 rounded border border-dashed border-border/50 px-2 py-1.5 text-[10px] text-muted-foreground">
          {t("settings.claims.conflictNoEvidencePreview")}
        </div>
      )}
    </div>
  )

  const renderConflictMatrixItem = (item: ClaimDetail) => {
    const trustKey = claimTrustKey(item.claim, item.evidence)
    const stats = evidenceTrustStats(item.evidence)
    const evidence = bestEvidence(item.evidence)

    return (
      <button
        key={item.claim.id}
        type="button"
        className="min-w-0 rounded border border-border/40 bg-background/70 px-2 py-2 text-left transition-colors hover:bg-secondary/40"
        onClick={() => {
          setSelectedId(item.claim.id)
          setDetail(null)
        }}
      >
        <div className="flex min-w-0 items-center justify-between gap-2">
          <div className="truncate text-[11px] font-medium">{item.claim.object}</div>
          <span className="shrink-0 rounded bg-secondary/70 px-1.5 py-0.5 text-[10px]">
            {trustLabel(trustKey)}
          </span>
        </div>
        <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
          <span className="rounded bg-secondary/60 px-1.5 py-0.5">
            {t("settings.claims.evidenceCount", { count: item.evidence.length })}
          </span>
          {stats.confirmed > 0 && (
            <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300">
              {t("settings.claims.confirmedEvidenceCount", { count: stats.confirmed })}
            </span>
          )}
          {stats.sourceBacked > 0 && (
            <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-700 dark:text-sky-300">
              {t("settings.claims.sourceBackedEvidenceCount", { count: stats.sourceBacked })}
            </span>
          )}
          {stats.inferred > 0 && (
            <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">
              {t("settings.claims.inferredEvidenceCount", { count: stats.inferred })}
            </span>
          )}
        </div>
        {evidence && (
          <div className="mt-1 truncate text-[10px] text-muted-foreground">
            {evidence.quote || evidence.filePath || evidence.url || evidence.sourceId}
          </div>
        )}
      </button>
    )
  }

  const renderReviewHistoryItem = (item: ReviewHistoryItem) => {
    const rationale = item.rationale && item.rationale !== item.content ? item.rationale : null
    const rationalePreview = rationale ? compactRationalePreview(rationale) : null
    const hasLongRationale = !!rationale && rationalePreview !== rationale

    return (
      <div
        key={item.id}
        className="min-w-0 rounded-md border border-border/50 bg-background/70 text-xs"
      >
        <div className="flex min-w-0 items-start gap-1 px-3 py-2">
          <button
            type="button"
            disabled={!item.targetId}
            onClick={() => {
              if (!item.targetId) return
              setSelectedId(item.targetId)
              setDetail(null)
              setMemoryFocusUrl(currentClaimFocusTarget(item.targetId))
            }}
            className="min-w-0 flex-1 text-left transition-colors hover:text-foreground disabled:cursor-default disabled:hover:text-inherit"
          >
            <div className="truncate font-medium">
              {item.content || rationalePreview || item.targetId || item.id}
            </div>
            <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
              <span className="font-mono">{formatActivityTime(item.createdAt)}</span>
              <span>{decisionTypeLabel(item.decisionType)}</span>
              <span>{t(`dashboard.dreaming.trigger.${item.trigger}`, item.trigger)}</span>
              {item.phase && <span>{item.phase}</span>}
              {item.status && <span>{item.status}</span>}
              {item.targetId && (
                <span className="text-primary">{t("settings.claims.reviewHistoryOpenClaim")}</span>
              )}
            </div>
          </button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
            aria-label={t("settings.claims.reviewHistoryCopyItem")}
            data-ha-title-tip={t("settings.claims.reviewHistoryCopyItem")}
            onClick={() => void copyReviewHistoryItem(item)}
          >
            <Copy className="h-3 w-3" />
          </Button>
        </div>
        {rationale && (
          <div className="border-t border-border/40 px-3 py-1.5 text-[10px] text-muted-foreground">
            {hasLongRationale ? (
              <details>
                <summary className="cursor-pointer truncate">
                  {rationalePreview} · {t("chat.details")}
                </summary>
                <div className="mt-1 whitespace-pre-wrap break-words leading-relaxed">
                  {rationale}
                </div>
              </details>
            ) : (
              <div className="break-words leading-relaxed">{rationale}</div>
            )}
          </div>
        )}
      </div>
    )
  }

  const renderClaimRow = (c: ClaimRecord) => {
    const reviewProjection = isReviewQueue ? reviewProjectionForVisibleClaim(c) : null
    const reviewKey = reviewProjection?.primary ?? null
    const conflictSummary = isReviewQueue ? claimConflictSummaries.get(c.id) : null
    const evidenceSummary = claimEvidenceSummaries.get(c.id)
    const conflictExamples = conflictSummary?.examples ?? []
    const searchDiagnostic = claimSearchDiagnosticForClaim(c)
    const searchMatches = searchDiagnostic?.matches ?? []
    const rankSignals = searchDiagnostic?.rankSignals ?? []

    return (
      <div
        key={c.id}
        className={`flex w-full items-start gap-2 px-3 py-2 text-xs transition-colors border-b border-border/30 hover:bg-secondary/40 ${
          selectedId === c.id ? "bg-secondary/60 font-medium" : ""
        }`}
      >
        {isReviewQueue && (
          <button
            type="button"
            aria-pressed={batchSelectedIds.has(c.id)}
            aria-label={t("settings.claims.batchToggle")}
            className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors ${
              batchSelectedIds.has(c.id)
                ? "border-primary bg-primary text-primary-foreground"
                : "border-border bg-background text-transparent"
            }`}
            disabled={!!batchBusy}
            onClick={() => toggleBatchSelection(c.id)}
          >
            <CheckCircle2 className="h-3 w-3" />
          </button>
        )}
        <button
          type="button"
          onClick={() => {
            setSelectedId(c.id)
            setMemoryFocusUrl(currentClaimFocusTarget(c.id))
          }}
          className="min-w-0 flex-1 text-left hover:text-foreground"
        >
          <div className="flex min-w-0 items-center gap-2">
            <span
              className={`h-2 w-2 rounded-full shrink-0 ${STATUS_DOT[c.status] ?? "bg-muted-foreground/50"}`}
            />
            <span className="truncate">{c.content}</span>
          </div>
          <div className="text-[10px] text-muted-foreground mt-0.5 font-mono">
            {c.claimType} · {scopeLabel(c)} · {(c.confidence * 100).toFixed(0)}% ·{" "}
            {(c.salience * 100).toFixed(0)}%
          </div>
          {evidenceSummary && (
            <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
              <span
                className="rounded bg-secondary/70 px-1.5 py-0.5"
                data-ha-title-tip={trustDetail(evidenceSummary.trust)}
              >
                {trustLabel(evidenceSummary.trust)}
              </span>
              <span className="rounded bg-secondary/60 px-1.5 py-0.5">
                {t("settings.claims.evidenceCount", { count: evidenceSummary.evidenceCount })}
              </span>
              {evidenceSummary.confirmedCount > 0 && (
                <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300">
                  {t("settings.claims.confirmedEvidenceCount", {
                    count: evidenceSummary.confirmedCount,
                  })}
                </span>
              )}
              {evidenceSummary.sourceBackedCount > 0 && (
                <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-700 dark:text-sky-300">
                  {t("settings.claims.sourceBackedEvidenceCount", {
                    count: evidenceSummary.sourceBackedCount,
                  })}
                </span>
              )}
              {evidenceSummary.inferredCount > 0 && (
                <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">
                  {t("settings.claims.inferredEvidenceCount", {
                    count: evidenceSummary.inferredCount,
                  })}
                </span>
              )}
            </div>
          )}
          {searchMatches.length > 0 && (
            <div className="mt-1 flex flex-wrap items-center gap-1 text-[10px]">
              <span className="text-muted-foreground">{t("settings.claims.searchMatchedBy")}</span>
              {searchMatches.map((match) => (
                <span key={match.kind} className="rounded bg-primary/10 px-1.5 py-0.5 text-primary">
                  {t(`settings.claims.searchMatch_${match.kind}`)}
                </span>
              ))}
            </div>
          )}
          {rankSignals.length > 0 && (
            <div className="mt-1 flex flex-wrap items-center gap-1 text-[10px]">
              <span className="text-muted-foreground">
                {t("settings.memorySearchMatch_ranked", "Ranked")}
              </span>
              {rankSignals.map((signal) => (
                <span
                  key={`${signal.kind}-${signal.direction}`}
                  className="rounded bg-secondary/70 px-1.5 py-0.5 text-muted-foreground"
                >
                  {claimRankSignalLabel(signal)}
                </span>
              ))}
            </div>
          )}
          {isReviewQueue && reviewKey && (
            <div className="mt-1 flex min-w-0 items-start gap-1.5 text-[10px] text-muted-foreground">
              <span
                className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-full ${
                  REVIEW_BUCKET_TONE[reviewKey]
                }`}
              />
              <span className="min-w-0">
                <span className="font-medium text-foreground/80">
                  {reviewReasonLabel(reviewKey)}
                </span>
                <span className="text-muted-foreground"> · {reviewReasonDetail(reviewKey)}</span>
              </span>
            </div>
          )}
          {isReviewQueue && reviewProjection && (
            <div className="mt-1 flex min-w-0 flex-wrap gap-1">
              {reviewProjection.risks.slice(0, 4).map((risk) => (
                <span
                  key={risk}
                  className={`rounded border px-1.5 py-0.5 text-[10px] ${REVIEW_RISK_TONE[risk]}`}
                >
                  {reviewRiskLabel(risk)}
                </span>
              ))}
              {reviewProjection.risks.length > 4 && (
                <span className="rounded border border-border/40 bg-secondary/60 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  +{reviewProjection.risks.length - 4}
                </span>
              )}
            </div>
          )}
          {isReviewQueue && reviewKey === "conflict" && conflictExamples.length > 0 && (
            <div className="mt-1 flex min-w-0 flex-wrap gap-1 text-[10px] text-muted-foreground">
              <span className="shrink-0 text-muted-foreground">
                {t("settings.claims.conflictPreviewLabel")}
              </span>
              {conflictExamples.slice(0, 3).map((example) => (
                <span
                  key={example.claimId}
                  className="min-w-0 max-w-[180px] truncate rounded bg-secondary/70 px-1.5 py-0.5"
                  data-ha-title-tip={example.content || example.object}
                >
                  {example.object || example.content} ·{" "}
                  {example.status === "active"
                    ? t("settings.claims.conflictKindActive")
                    : t("settings.claims.conflictKindReview")}
                </span>
              ))}
              {conflictSummary && conflictSummary.conflictCount > conflictExamples.length && (
                <span className="rounded bg-secondary/70 px-1.5 py-0.5">
                  {t("settings.claims.conflictPreviewMore", {
                    count: conflictSummary.conflictCount - conflictExamples.length,
                  })}
                </span>
              )}
            </div>
          )}
        </button>
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-sm font-medium flex items-center gap-1.5">
            {t("settings.claims.title")}
            <span className="text-[9px] uppercase tracking-wide rounded bg-primary/15 text-primary px-1 py-0.5">
              {t("settings.memoryStructuredBadge")}
            </span>
          </div>
          <div className="text-xs text-muted-foreground">{t("settings.claims.desc")}</div>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-1.5 text-xs"
            onClick={openBackfill}
          >
            <DatabaseZap className="h-3.5 w-3.5" />
            {t("settings.claims.backfill.button")}
          </Button>
          <Select
            value={statusFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setStatusFilter(value)
            }}
          >
            <SelectTrigger className="h-8 w-[140px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("settings.claims.statusAll")}</SelectItem>
              <SelectItem value="active">{t("settings.claims.status.active")}</SelectItem>
              <SelectItem value="superseded">{t("settings.claims.status.superseded")}</SelectItem>
              <SelectItem value="expired">{t("settings.claims.status.expired")}</SelectItem>
              <SelectItem value="archived">{t("settings.claims.status.archived")}</SelectItem>
              <SelectItem value="needs_review">
                {t("settings.claims.status.needs_review")}
              </SelectItem>
            </SelectContent>
          </Select>
          <Select
            value={claimTypeFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setClaimTypeFilter(normalizeClaimTypeFilter(value, claimSchema))
            }}
          >
            <SelectTrigger className="h-8 w-[150px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {claimTypeFilterOptions.map((value) => (
                <SelectItem key={value} value={value}>
                  {claimTypeFilterLabel(value)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={scopeFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setScopeFilter(value)
            }}
          >
            <SelectTrigger className="h-8 w-[180px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {scopeFilterOptions.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={confidenceSourceFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setConfidenceSourceFilter(normalizeConfidenceSourceFilter(value, claimSchema))
            }}
          >
            <SelectTrigger className="h-8 w-[160px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {confidenceSourceFilterOptions.map((value) => (
                <SelectItem key={value} value={value}>
                  {confidenceSourceFilterLabel(value)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={evidenceClassFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setEvidenceClassFilter(normalizeEvidenceClassFilter(value, claimSchema))
            }}
          >
            <SelectTrigger className="h-8 w-[170px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {evidenceClassFilterOptions.map((value) => (
                <SelectItem key={value} value={value}>
                  {evidenceClassFilterLabel(value)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={evidenceSourceFilter}
            onValueChange={(value) => {
              resetClaimListPosition()
              setEvidenceSourceFilter(normalizeEvidenceSourceFilter(value, claimSchema))
            }}
          >
            <SelectTrigger className="h-8 w-[155px] text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {evidenceSourceFilterOptions.map((value) => (
                <SelectItem key={value} value={value}>
                  {evidenceSourceFilterLabel(value)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="space-y-1">
            <Select
              value={claimListSort}
              onValueChange={(value) => {
                resetClaimListPosition()
                setClaimListSort(normalizeClaimListSort(value))
              }}
            >
              <SelectTrigger className="h-8 w-[155px] text-xs" data-ha-title-tip={claimListSortRuntimeText}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CLAIM_LIST_SORT_VALUES.map((value) => (
                  <SelectItem key={value} value={value}>
                    {claimListSortLabel(value)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="max-w-[180px] text-[10px] leading-snug text-muted-foreground">
              {claimListSortRuntimeText}
            </div>
          </div>
        </div>
      </div>

      {claimSchemaErrorToast && (
        <div className="rounded-md border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div className="min-w-0 space-y-1">
              <div className="font-medium">{claimSchemaErrorToast.title}</div>
              {claimSchemaErrorToast.description && (
                <div className="break-words text-amber-900/80 dark:text-amber-100/80">
                  {claimSchemaErrorToast.description}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
      {scopeNameErrorToasts.length > 0 && (
        <div className="rounded-md border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div className="min-w-0 space-y-1">
              <div className="font-medium">{scopeNameErrorToasts[0].title}</div>
              {scopeNameErrorToasts.map((failure, index) =>
                failure.description ? (
                  <div
                    key={`${failure.description}-${index}`}
                    className="break-words text-amber-900/80 dark:text-amber-100/80"
                  >
                    {failure.description}
                  </div>
                ) : null,
              )}
            </div>
          </div>
        </div>
      )}

      <div className="rounded-lg border border-border/60 bg-secondary/20 px-3 py-3">
        <div className="text-xs font-medium text-foreground">
          {t("settings.claims.explainer.title")}
        </div>
        <div className="mt-1 text-xs leading-relaxed text-muted-foreground">
          {t("settings.claims.explainer.intro")}
        </div>
        <div className="mt-3 grid gap-3 md:grid-cols-3">
          {(["affects", "review", "backfill"] as const).map((item) => (
            <div key={item} className="min-w-0">
              <div className="text-[11px] font-medium text-foreground">
                {t(`settings.claims.explainer.${item}Title`)}
              </div>
              <div className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                {t(`settings.claims.explainer.${item}Desc`)}
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-[1fr_1fr] gap-4">
        {/* Claim list */}
        <div className="border border-border/60 rounded-lg overflow-hidden">
          <div className="flex items-center justify-between gap-2 px-3 py-2 border-b border-border/60 bg-secondary/20 text-xs font-medium">
            <span>
              {t("settings.claims.list")} ({claimListCountLabel})
            </span>
            {isReviewQueue && visibleClaims.length > 0 && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-[10px]"
                disabled={!!batchBusy}
                onClick={toggleBatchSelectAll}
              >
                {allBatchSelected
                  ? t("settings.claims.batchClear")
                  : t("settings.claims.batchSelectAll")}
              </Button>
            )}
          </div>
          <div className="border-b border-border/60 bg-background px-3 py-2">
            <div className="flex min-w-0 items-center gap-1.5">
              <div className="relative min-w-[160px] flex-1">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <SearchInput
                  type="search"
                  value={claimSearchQuery}
                  onChange={(event) => {
                    resetClaimListPosition()
                    setClaimSearchQuery(event.target.value)
                  }}
                  placeholder={t("settings.claims.searchPlaceholder")}
                  className="h-8 pr-8 pl-8 text-xs"
                />
                {claimSearchQuery && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={t("common.clear")}
                    className="absolute right-1 top-1/2 h-6 w-6 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    onClick={() => {
                      resetClaimListPosition()
                      setClaimSearchQuery("")
                    }}
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-8 shrink-0 gap-1 px-2 text-[10px]"
                onClick={saveClaimListFilterPreset}
              >
                <BookmarkPlus className="h-3 w-3" />
                {t("settings.claims.filterPresetSave")}
              </Button>
            </div>
            {claimListFilterPresets.length > 0 && (
              <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px]">
                <span className="text-muted-foreground">{t("settings.claims.filterPresets")}</span>
                {claimListFilterPresets.map((preset) => {
                  const label = claimListPresetLabel(preset)
                  const active = preset.id === currentClaimListPresetId
                  return (
                    <span
                      key={preset.id}
                      className={[
                        "inline-flex max-w-full items-center rounded-md border border-border/70",
                        active ? "bg-primary/10 text-foreground" : "bg-background",
                      ].join(" ")}
                    >
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-6 min-w-0 max-w-[220px] justify-start truncate px-2 text-[10px]"
                        data-ha-title-tip={label}
                        onClick={() => applyClaimListFilterPreset(preset)}
                      >
                        <span className="truncate">{label}</span>
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        aria-label={t("settings.claims.filterPresetRemove")}
                        className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                        onClick={() => removeClaimListFilterPreset(preset.id)}
                      >
                        <X className="h-3 w-3" />
                      </Button>
                    </span>
                  )
                })}
              </div>
            )}
          </div>
          {isReviewQueue && (
            <div className="border-b border-border/60 bg-background px-3 py-2">
              <div className="flex min-w-0 items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex min-w-0 items-center gap-1.5 text-xs font-medium">
                    <History className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate">{t("settings.claims.reviewHistoryTitle")}</span>
                  </div>
                  <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                    {t("settings.claims.reviewHistoryDesc")}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  {reviewHistoryOpen && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 gap-1 px-2 text-[10px]"
                      disabled={reviewHistoryLoading || reviewHistoryExportingAll}
                      onClick={() => void copyAllReviewHistoryExport()}
                    >
                      {reviewHistoryExportingAll ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Copy className="h-3 w-3" />
                      )}
                      {t("settings.claims.reviewHistoryExportAll")}
                    </Button>
                  )}
                  {reviewHistoryOpen && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 gap-1 px-2 text-[10px]"
                      disabled={
                        reviewHistoryLoading ||
                        reviewHistoryExportingAll ||
                        filteredReviewHistory.length === 0
                      }
                      onClick={() => void copyReviewHistoryExport()}
                    >
                      <Copy className="h-3 w-3" />
                      {t("settings.claims.reviewHistoryExport")}
                    </Button>
                  )}
                  {reviewHistoryOpen && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 px-2 text-[10px]"
                      disabled={reviewHistoryLoading}
                      onClick={() => void loadReviewHistory()}
                    >
                      {reviewHistoryLoading ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        t("settings.claims.reviewHistoryRefresh")
                      )}
                    </Button>
                  )}
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-[10px]"
                    aria-expanded={reviewHistoryOpen}
                    onClick={() => setReviewHistoryOpen((open) => !open)}
                  >
                    {reviewHistoryOpen
                      ? t("settings.claims.reviewHistoryHide")
                      : t("settings.claims.reviewHistoryShow")}
                  </Button>
                </div>
              </div>
              {reviewHistoryOpen && (
                <div className="mt-2 space-y-1.5">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <Select
                      value={reviewHistoryDecisionFilter}
                      onValueChange={(value) =>
                        setReviewHistoryDecisionFilter(normalizeReviewHistoryDecisionFilter(value))
                      }
                    >
                      <SelectTrigger className="h-7 w-[128px] text-[10px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {reviewHistoryDecisionOptions.map((value) => (
                          <SelectItem key={value} value={value}>
                            {reviewHistoryDecisionFilterLabel(value)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Select
                      value={reviewHistoryTimeRange}
                      onValueChange={(value) =>
                        setReviewHistoryTimeRange(normalizeReviewHistoryTimeRange(value))
                      }
                    >
                      <SelectTrigger className="h-7 w-[112px] text-[10px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {REVIEW_HISTORY_TIME_RANGES.map((value) => (
                          <SelectItem key={value} value={value}>
                            {reviewHistoryTimeRangeLabel(value)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Select
                      value={reviewHistoryScopeFilter}
                      onValueChange={setReviewHistoryScopeFilter}
                    >
                      <SelectTrigger className="h-7 w-[150px] text-[10px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {reviewHistoryScopeOptions.map((option) => (
                          <SelectItem key={option.value} value={option.value}>
                            {option.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <div className="relative min-w-[160px] flex-1">
                      <Search className="absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
                      <SearchInput
                        type="search"
                        value={reviewHistoryQuery}
                        onChange={(event) => setReviewHistoryQuery(event.target.value)}
                        placeholder={t("settings.claims.reviewHistorySearchPlaceholder")}
                        className="h-7 pr-7 pl-7 text-[10px]"
                      />
                      {reviewHistoryQuery && (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          aria-label={t("common.clear")}
                          className="absolute right-0.5 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                          onClick={() => setReviewHistoryQuery("")}
                        >
                          <X className="h-3 w-3" />
                        </Button>
                      )}
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 gap-1 px-2 text-[10px]"
                      onClick={saveReviewHistoryFilterPreset}
                    >
                      <BookmarkPlus className="h-3 w-3" />
                      {t("settings.claims.reviewHistoryPresetSave")}
                    </Button>
                    {reviewHistory.length > 0 && (
                      <span className="text-[10px] text-muted-foreground">
                        {t("settings.claims.reviewHistoryCount", {
                          shown: filteredReviewHistory.length,
                          total: reviewHistoryDisplayTotal,
                        })}
                      </span>
                    )}
                  </div>
                  {reviewHistoryFilterPresets.length > 0 && (
                    <div className="flex flex-wrap items-center gap-1.5 text-[10px]">
                      <span className="text-muted-foreground">
                        {t("settings.claims.reviewHistoryPresets")}
                      </span>
                      {reviewHistoryFilterPresets.map((preset) => {
                        const label = reviewHistoryPresetLabel(preset)
                        const active = preset.id === currentReviewHistoryPresetId
                        return (
                          <span
                            key={preset.id}
                            className={[
                              "inline-flex max-w-full items-center rounded-md border border-border/70",
                              active ? "bg-primary/10 text-foreground" : "bg-background",
                            ].join(" ")}
                          >
                            <Button
                              type="button"
                              variant="ghost"
                              size="sm"
                              className="h-6 min-w-0 max-w-[220px] justify-start truncate px-2 text-[10px]"
                              data-ha-title-tip={label}
                              onClick={() => applyReviewHistoryFilterPreset(preset)}
                            >
                              <span className="truncate">{label}</span>
                            </Button>
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon"
                              aria-label={t("settings.claims.reviewHistoryPresetRemove")}
                              className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                              onClick={() => removeReviewHistoryFilterPreset(preset.id)}
                            >
                              <X className="h-3 w-3" />
                            </Button>
                          </span>
                        )
                      })}
                    </div>
                  )}
                  {reviewHistoryErrorToast && (
                    <div className="rounded-md border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
                      <div className="flex items-start gap-2">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        <div className="min-w-0 space-y-1">
                          <div className="font-medium">{reviewHistoryErrorToast.title}</div>
                          {reviewHistoryErrorToast.description && (
                            <div className="break-words text-amber-900/80 dark:text-amber-100/80">
                              {reviewHistoryErrorToast.description}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  )}
                  {reviewHistoryLoading ? (
                    <div className="flex items-center justify-center gap-1 rounded-md border border-dashed border-border/70 px-3 py-4 text-xs text-muted-foreground">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      {t("common.loading")}
                    </div>
                  ) : reviewHistory.length === 0 ? (
                    <div className="rounded-md border border-dashed border-border/70 px-3 py-4 text-center text-xs text-muted-foreground">
                      {t("settings.claims.reviewHistoryEmpty")}
                    </div>
                  ) : filteredReviewHistory.length === 0 ? (
                    <div className="rounded-md border border-dashed border-border/70 px-3 py-4 text-center text-xs text-muted-foreground">
                      {t("settings.claims.reviewHistoryNoMatches")}
                    </div>
                  ) : (
                    filteredReviewHistory.map(renderReviewHistoryItem)
                  )}
                  {!reviewHistoryLoading && reviewHistoryHasMore && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 w-full gap-1 text-[10px]"
                      disabled={reviewHistoryLoadingMore}
                      onClick={() => void loadReviewHistory(true)}
                    >
                      {reviewHistoryLoadingMore && <Loader2 className="h-3 w-3 animate-spin" />}
                      {t("settings.claims.reviewHistoryLoadMore")}
                    </Button>
                  )}
                </div>
              )}
            </div>
          )}
          {isReviewQueue && batchSelectedCount > 0 && (
            <div className="flex flex-wrap items-center gap-2 border-b border-border/60 bg-background px-3 py-2">
              <span className="min-w-0 flex-1 text-[11px] text-muted-foreground">
                {t("settings.claims.batchSelected", { count: batchSelectedCount })}
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-7 gap-1 text-xs"
                disabled={!!batchBusy}
                onClick={() => void runBatchAction("approve")}
              >
                {batchBusy === "approve" ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <CheckCircle2 className="h-3 w-3 text-emerald-500" />
                )}
                {t("settings.claims.batchApprove")}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-7 gap-1 text-xs text-muted-foreground"
                disabled={!!batchBusy}
                onClick={() => void runBatchAction("archive")}
              >
                {batchBusy === "archive" ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <Archive className="h-3 w-3" />
                )}
                {t("settings.claims.batchArchive")}
              </Button>
            </div>
          )}
          {claimListErrorToast && (
            <div className="border-b border-amber-500/20 bg-amber-500/8 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
              <div className="flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <div className="min-w-0 space-y-1">
                  <div className="font-medium">{claimListErrorToast.title}</div>
                  {claimListErrorToast.description && (
                    <div className="break-words text-amber-900/80 dark:text-amber-100/80">
                      {claimListErrorToast.description}
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}
          {claimListSummaryErrorToasts.length > 0 && (
            <div className="border-b border-amber-500/20 bg-amber-500/8 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
              <div className="flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <div className="min-w-0 space-y-1">
                  <div className="font-medium">{claimListSummaryErrorToasts[0].title}</div>
                  {claimListSummaryErrorToasts.map((failure, index) =>
                    failure.description ? (
                      <div
                        key={`${failure.description}-${index}`}
                        className="break-words text-amber-900/80 dark:text-amber-100/80"
                      >
                        {failure.description}
                      </div>
                    ) : null,
                  )}
                </div>
              </div>
            </div>
          )}
          <div className="max-h-[460px] overflow-y-auto">
            {loading ? (
              <div className="px-3 py-6 text-xs text-muted-foreground text-center inline-flex items-center gap-1 w-full justify-center">
                <Loader2 className="h-3 w-3 animate-spin" />
                {t("common.loading")}
              </div>
            ) : claims.length === 0 && claimListErrorToast ? null : claims.length === 0 ? (
              <div className="px-3 py-6 text-xs text-muted-foreground text-center">
                {hasClaimSearch ? t("settings.claims.searchEmpty") : t("settings.claims.empty")}
              </div>
            ) : visibleClaims.length === 0 ? (
              <div className="px-3 py-6 text-xs text-muted-foreground text-center">
                {t("settings.claims.searchEmpty")}
              </div>
            ) : isReviewQueue ? (
              reviewBuckets.map((bucket) => (
                <div key={bucket.key} className="border-b border-border/40 last:border-0">
                  <div className="flex items-center gap-2 border-b border-border/30 bg-secondary/10 px-3 py-1.5 text-[11px] font-medium text-muted-foreground">
                    <span
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${REVIEW_BUCKET_TONE[bucket.key]}`}
                    />
                    <span className="truncate">
                      {t(`settings.claims.reviewBuckets.${bucket.key}`)}
                    </span>
                    <span className="ml-auto tabular-nums">{bucket.claims.length}</span>
                  </div>
                  {bucket.claims.map(renderClaimRow)}
                </div>
              ))
            ) : (
              visibleClaims.map(renderClaimRow)
            )}
            {!loading && claimHasMore && visibleClaims.length > 0 && (
              <div className="border-t border-border/40 p-2">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-8 w-full gap-1 text-[10px] text-muted-foreground"
                  disabled={claimLoadingMore}
                  onClick={() => void loadMoreClaims()}
                >
                  {claimLoadingMore && <Loader2 className="h-3 w-3 animate-spin" />}
                  {t("settings.claims.loadMore")}
                </Button>
              </div>
            )}
          </div>
        </div>

        {/* Claim detail */}
        <div className="border border-border/60 rounded-lg p-3 max-h-[460px] overflow-y-auto">
          {detail ? (
            <div className="text-xs space-y-3">
              <section className="space-y-1">
                <div className="flex items-center justify-between gap-2">
                  <div className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("settings.claims.detailMemory")}
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-6 shrink-0 gap-1 px-2 text-[10px] text-muted-foreground"
                    onClick={() => void copyClaimLink(detail.claim)}
                    data-ha-title-tip={t("settings.claims.copyLink")}
                    aria-label={t("settings.claims.copyLink")}
                  >
                    <Link2 className="h-3 w-3" />
                    {t("settings.claims.copyLink")}
                  </Button>
                </div>
                <div className="text-sm font-medium leading-relaxed">{detail.claim.content}</div>
              </section>

              <div className="grid gap-2 border-y border-border/40 py-2 sm:grid-cols-3">
                <div className="min-w-0">
                  <div className="text-[10px] text-muted-foreground">
                    {t("settings.claims.usageScope")}
                  </div>
                  <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
                    <div className="truncate font-mono text-[11px]">{scopeLabel(detail.claim)}</div>
                    {canOpenScope(detail.claim) && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-5 shrink-0 px-1"
                        onClick={() => openScope(detail.claim)}
                        data-ha-title-tip={t("settings.claims.openScope")}
                        aria-label={t("settings.claims.openScope")}
                      >
                        <ExternalLink className="h-3 w-3" />
                      </Button>
                    )}
                  </div>
                  <div className="mt-0.5 text-[10px] leading-snug text-muted-foreground">
                    {scopeHelp(detail.claim)}
                  </div>
                </div>
                <div>
                  <div className="text-[10px] text-muted-foreground">
                    {t("settings.claims.confidence")}
                  </div>
                  <div className="mt-0.5 font-medium tabular-nums">
                    {(detail.claim.confidence * 100).toFixed(0)}%
                  </div>
                </div>
                <div>
                  <div className="text-[10px] text-muted-foreground">
                    {t("settings.claims.salience")}
                  </div>
                  <div className="mt-0.5 font-medium tabular-nums">
                    {(detail.claim.salience * 100).toFixed(0)}%
                  </div>
                </div>
              </div>

              {detailTrustKey && detailEvidenceStats && (
                <div className="rounded border border-border/50 bg-secondary/20 px-2 py-2">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                      {t("settings.claims.trustSignal")}
                    </span>
                    <span className="rounded bg-background px-1.5 py-0.5 text-[10px] font-medium">
                      {trustLabel(detailTrustKey)}
                    </span>
                  </div>
                  <div className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                    {trustDetail(detailTrustKey)}
                  </div>
                  <div className="mt-1.5 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                    <span className="rounded bg-background px-1.5 py-0.5">
                      {t("settings.claims.evidenceCount", { count: detail.evidence.length })}
                    </span>
                    {detailEvidenceStats.confirmed > 0 && (
                      <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300">
                        {t("settings.claims.confirmedEvidenceCount", {
                          count: detailEvidenceStats.confirmed,
                        })}
                      </span>
                    )}
                    {detailEvidenceStats.sourceBacked > 0 && (
                      <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-700 dark:text-sky-300">
                        {t("settings.claims.sourceBackedEvidenceCount", {
                          count: detailEvidenceStats.sourceBacked,
                        })}
                      </span>
                    )}
                    {detailEvidenceStats.inferred > 0 && (
                      <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">
                        {t("settings.claims.inferredEvidenceCount", {
                          count: detailEvidenceStats.inferred,
                        })}
                      </span>
                    )}
                  </div>
                </div>
              )}

              {detail.claim.status === "needs_review" && (
                <div className="border-l-2 border-sky-500/60 pl-2">
                  <div className="text-[10px] text-muted-foreground">
                    {t("settings.claims.reviewReason")}
                  </div>
                  {detailReviewProjection && (
                    <>
                      <div className="mt-0.5 font-medium">
                        {reviewReasonLabel(detailReviewProjection.primary)}
                      </div>
                      <div className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                        {reviewReasonDetail(detailReviewProjection.primary)}
                      </div>
                      <div className="mt-2">
                        <div className="text-[10px] text-muted-foreground">
                          {t("settings.claims.reviewRiskSignals")}
                        </div>
                        <div className="mt-1 flex flex-wrap gap-1">
                          {detailReviewProjection.risks.map((risk) => (
                            <span
                              key={risk}
                              className={`rounded border px-1.5 py-0.5 text-[10px] ${
                                REVIEW_RISK_TONE[risk]
                              }`}
                            >
                              {reviewRiskLabel(risk)}
                            </span>
                          ))}
                        </div>
                      </div>
                    </>
                  )}
                  {(detailConflictMatches.length > 0 ||
                    detailConflictLoading ||
                    conflictCandidatesErrorDetail) && (
                    <div className="mt-2 space-y-1">
                      <div className="text-[10px] text-muted-foreground">
                        {t("settings.claims.conflictWith")}
                      </div>
                      {detailConflictLoading && (
                        <div className="flex items-center gap-1 rounded border border-dashed border-border/60 px-2 py-1.5 text-[11px] text-muted-foreground">
                          <Loader2 className="h-3 w-3 animate-spin" />
                          {t("common.loading")}
                        </div>
                      )}
                      {!detailConflictLoading &&
                        conflictCandidatesErrorDetail &&
                        detailConflictMatches.length === 0 && (
                          <div className="rounded border border-dashed border-amber-500/35 bg-amber-500/5 px-2 py-2 text-[11px] text-amber-800 dark:text-amber-200">
                            <div>{t("settings.claims.conflictCandidatesUnavailable")}</div>
                            <div className="mt-1 break-all text-[10px] text-amber-900/80 dark:text-amber-100/80">
                              {t("settings.claims.operationErrors.detail", {
                                defaultValue: "Details: {{error}}",
                                error: conflictCandidatesErrorDetail,
                              })}
                            </div>
                          </div>
                        )}
                      {conflictInsight && (
                        <div className="space-y-2 rounded border border-amber-500/30 bg-amber-500/5 px-2 py-2">
                          <div className="flex items-center gap-1.5 text-[11px] font-medium text-foreground">
                            <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
                            {t("settings.claims.conflictWhyTitle")}
                          </div>
                          <div className="text-[11px] leading-relaxed text-muted-foreground">
                            {t("settings.claims.conflictWhyDesc")}
                          </div>
                          <div className="grid gap-1.5 sm:grid-cols-2">
                            <div className="min-w-0">
                              <div className="text-[10px] text-muted-foreground">
                                {t("settings.claims.conflictCurrentValue")}
                              </div>
                              <div className="truncate text-[11px] font-medium">
                                {detail.claim.object}
                              </div>
                            </div>
                            <div className="min-w-0">
                              <div className="text-[10px] text-muted-foreground">
                                {t("settings.claims.conflictOtherValues")}
                              </div>
                              <div className="truncate text-[11px] font-medium">
                                {conflictInsight.otherObjects.join(" / ")}
                              </div>
                            </div>
                          </div>
                          {conflictInsight.strongest && (
                            <div className="space-y-0.5 text-[10px] text-muted-foreground">
                              <div className="font-medium text-foreground/80">
                                {t("settings.claims.conflictSignal")}
                              </div>
                              <div>
                                {t("settings.claims.conflictSignalConfidence", {
                                  current: (detail.claim.confidence * 100).toFixed(0),
                                  other: (conflictInsight.strongest.confidence * 100).toFixed(0),
                                })}
                              </div>
                              <div>
                                {t("settings.claims.conflictSignalSalience", {
                                  current: (detail.claim.salience * 100).toFixed(0),
                                  other: (conflictInsight.strongest.salience * 100).toFixed(0),
                                })}
                              </div>
                              {conflictInsight.activeCount > 0 && (
                                <div>
                                  {t("settings.claims.conflictSignalActive", {
                                    count: conflictInsight.activeCount,
                                  })}
                                </div>
                              )}
                            </div>
                          )}
                          {conflictInsight.strongest && detailTrustKey && detailEvidenceStats && (
                            <div className="space-y-2 rounded border border-border/40 bg-background/50 px-2 py-2">
                              <div>
                                <div className="text-[11px] font-medium text-foreground">
                                  {t("settings.claims.conflictEvidenceDiffTitle")}
                                </div>
                                <div className="mt-0.5 text-[10px] leading-relaxed text-muted-foreground">
                                  {t("settings.claims.conflictEvidenceDiffDesc")}
                                </div>
                              </div>
                              {conflictCompareLoading ? (
                                <div className="flex items-center justify-center gap-1 rounded border border-dashed border-border/60 px-2 py-3 text-[11px] text-muted-foreground">
                                  <Loader2 className="h-3 w-3 animate-spin" />
                                  {t("common.loading")}
                                </div>
                              ) : conflictCompareDetail &&
                                conflictCompareTrustKey &&
                                conflictCompareEvidenceStats ? (
                                <div className="grid gap-2 sm:grid-cols-2">
                                  {renderEvidenceDiffColumn(
                                    t("settings.claims.conflictCurrentCandidate"),
                                    detail.claim,
                                    detail.evidence,
                                    detailTrustKey,
                                    detailEvidenceStats,
                                    strongestCurrentEvidence,
                                  )}
                                  {renderEvidenceDiffColumn(
                                    t("settings.claims.conflictExistingCandidate"),
                                    conflictCompareDetail.claim,
                                    conflictCompareDetail.evidence,
                                    conflictCompareTrustKey,
                                    conflictCompareEvidenceStats,
                                    strongestConflictEvidence,
                                  )}
                                </div>
                              ) : (
                                <div className="rounded border border-dashed border-border/60 px-2 py-3 text-center text-[11px] text-muted-foreground">
                                  <div>{t("settings.claims.conflictEvidenceDiffUnavailable")}</div>
                                  {conflictEvidenceErrorDetail && (
                                    <div className="mt-1 break-all text-[10px]">
                                      {t("settings.claims.operationErrors.detail", {
                                        defaultValue: "Details: {{error}}",
                                        error: conflictEvidenceErrorDetail,
                                      })}
                                    </div>
                                  )}
                                </div>
                              )}
                              {conflictMatrixDetails.length > 1 && (
                                <div className="space-y-1.5 border-t border-border/40 pt-2">
                                  <div className="text-[10px] font-medium text-muted-foreground">
                                    {t("settings.claims.conflictWith")}
                                  </div>
                                  <div className="grid gap-1.5">
                                    {conflictMatrixDetails.map(renderConflictMatrixItem)}
                                  </div>
                                </div>
                              )}
                            </div>
                          )}
                          <div className="rounded border border-border/40 bg-background/60 px-2 py-1.5">
                            <div className="flex items-center gap-1.5 text-[11px] font-medium">
                              <Lightbulb className="h-3.5 w-3.5 text-primary" />
                              {t("settings.claims.conflictSuggestionTitle")}
                            </div>
                            <div className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                              {t(conflictSuggestionLabelKey(conflictInsight.suggestion))}
                            </div>
                          </div>
                        </div>
                      )}
                      {visibleConflictActionTargets.map(({ claim, kind }) => (
                        <button
                          key={`${kind}-${claim.id}`}
                          type="button"
                          className="block w-full rounded border border-border/40 px-2 py-1 text-left transition-colors hover:bg-secondary/40"
                          onClick={() => {
                            setSelectedId(claim.id)
                            setDetail(null)
                          }}
                        >
                          <div className="truncate text-[11px] font-medium">{claim.content}</div>
                          <div className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
                            {t(`settings.claims.status.${claim.status}`)} · {scopeLabel(claim)} ·{" "}
                            {kind === "active"
                              ? t("settings.claims.conflictKindActive")
                              : t("settings.claims.conflictKindReview")}
                          </div>
                        </button>
                      ))}
                      {hiddenConflictActionTargetCount > 0 && (
                        <div className="rounded border border-border/40 bg-secondary/20 px-2 py-1.5 text-[11px] text-muted-foreground">
                          {t("settings.claims.conflictMoreLoaded", {
                            count: hiddenConflictActionTargetCount,
                          })}
                        </div>
                      )}
                      <div className="flex flex-wrap gap-1.5 pt-1">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1 text-xs"
                          disabled={conflictResolving}
                          onClick={() => void keepExistingConflict()}
                        >
                          {conflictResolving ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <Archive className="h-3 w-3" />
                          )}
                          {t("settings.claims.conflictKeepExisting")}
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1 text-xs"
                          disabled={conflictResolving}
                          onClick={() => setConflictResolveOpen(true)}
                        >
                          <CheckCircle2 className="h-3 w-3 text-emerald-500" />
                          {t("settings.claims.conflictUseCurrent")}
                        </Button>
                      </div>
                    </div>
                  )}
                </div>
              )}

              {detail.claim.tags.length > 0 && (
                <div className="flex flex-wrap gap-1">
                  {detail.claim.tags.map((tag) => (
                    <span
                      key={tag}
                      className="rounded bg-secondary/60 px-1.5 py-0.5 text-[10px] text-muted-foreground"
                    >
                      {tag}
                    </span>
                  ))}
                </div>
              )}

              <div className="rounded border border-border/50 bg-secondary/10 px-2 py-2">
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5 text-[11px] font-medium">
                      <Link2 className="h-3.5 w-3.5 text-muted-foreground" />
                      {t("settings.claims.entityContextTitle")}
                    </div>
                    <div className="mt-0.5 text-[10px] leading-relaxed text-muted-foreground">
                      {t("settings.claims.entityContextDesc")}
                    </div>
                  </div>
                  {claimGraphLoading && (
                    <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
                  )}
                </div>
                {claimGraph && claimGraph.edges.length > 0 ? (
                  <div className="mt-2 space-y-1.5">
                    <div className="flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                      <span className="rounded bg-background px-1.5 py-0.5">
                        {t("settings.claims.entityContextNodes", {
                          count: claimGraph.nodes.length,
                        })}
                      </span>
                      <span className="rounded bg-background px-1.5 py-0.5">
                        {t("settings.claims.entityContextEdges", {
                          count: claimGraph.edges.length,
                        })}
                      </span>
                      {claimGraph.truncated && (
                        <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">
                          {t("settings.claims.entityContextTruncated")}
                        </span>
                      )}
                    </div>
                    <div className="grid gap-1">
                      {claimGraphEdges.map((edge) => (
                        <button
                          key={edge.id}
                          type="button"
                          className="rounded border border-border/40 bg-background/60 px-2 py-1.5 text-left transition-colors hover:bg-secondary/40"
                          onClick={() => {
                            setSelectedId(edge.claimId)
                            setDetail(null)
                          }}
                        >
                          <div className="min-w-0 truncate text-[11px] font-medium">
                            {claimGraphNodeLabels.get(edge.source) ?? edge.source} ·{" "}
                            {edge.predicate} ·{" "}
                            {claimGraphNodeLabels.get(edge.target) ?? edge.target}
                          </div>
                          <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                            {edge.content}
                          </div>
                        </button>
                      ))}
                    </div>
                  </div>
                ) : (
                  <div className="mt-2 rounded border border-dashed border-border/50 px-2 py-2 text-[11px] text-muted-foreground">
                    <div>
                      {claimGraphLoading
                        ? t("settings.claims.entityContextLoading")
                        : claimGraphErrorDetail
                          ? t("settings.claims.entityContextUnavailable")
                          : t("settings.claims.entityContextEmpty")}
                    </div>
                    {!claimGraphLoading && claimGraphErrorDetail && (
                      <div className="mt-1 break-all text-[10px]">
                        {t("settings.claims.operationErrors.detail", {
                          defaultValue: "Details: {{error}}",
                          error: claimGraphErrorDetail,
                        })}
                      </div>
                    )}
                  </div>
                )}
              </div>

              <div className="pt-1.5 border-t border-border/40">
                <ClaimReviewActions claim={detail.claim} onChanged={onClaimChanged} />
              </div>

              <details className="border-t border-border/40 pt-2">
                <summary className="cursor-pointer text-[11px] font-medium text-muted-foreground hover:text-foreground">
                  {t("settings.claims.technicalDetails")}
                </summary>
                <div className="mt-2 space-y-1 font-mono text-[10px] text-muted-foreground">
                  <div>
                    {detail.claim.claimType} · {detail.claim.subject} · {detail.claim.predicate} ·{" "}
                    {detail.claim.object}
                  </div>
                  <div>
                    {detail.claim.confidenceSource}
                    {detail.claim.validUntil ? ` · ${detail.claim.validUntil}` : ""}
                  </div>
                </div>
              </details>

              <div className="font-medium pt-1">
                {t("settings.claims.evidence")} ({detail.evidence.length})
              </div>
              {detail.evidence.length === 0 ? (
                <div className="text-muted-foreground">{t("settings.claims.noEvidence")}</div>
              ) : (
                <ul className="space-y-1">
                  {detail.evidence.map((e) => (
                    <li
                      key={e.id}
                      className="rounded border border-border/40 px-2 py-1.5 text-[10px]"
                    >
                      <div className="flex flex-wrap items-center gap-1.5 text-muted-foreground">
                        <span className="rounded bg-secondary/60 px-1.5 py-0.5">
                          {evidenceSourceLabel(e.sourceType)}
                        </span>
                        <span className="rounded bg-secondary/60 px-1.5 py-0.5">
                          {evidenceClassLabel(e.evidenceClass)}
                        </span>
                        {e.sessionId && (
                          <span className="font-mono">session {e.sessionId.slice(0, 8)}…</span>
                        )}
                        {e.messageId && <span className="font-mono">#{e.messageId}</span>}
                      </div>
                      <div className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                        {trustDetail(evidenceTrustKey(e))}
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {e.sessionId && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => openEvidenceSource(e)}
                          >
                            {openSourceLabel(e)}
                          </Button>
                        )}
                        {e.filePath && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => openEvidenceFile(e)}
                          >
                            {openSourceLabel({ ...e, sessionId: null })}
                          </Button>
                        )}
                        {e.url && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => openEvidenceUrl(e)}
                          >
                            {openSourceLabel({ ...e, sessionId: null, filePath: null })}
                          </Button>
                        )}
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-6 gap-1 px-1.5 text-[10px]"
                          onClick={() => void copyEvidenceDetails(detail.claim, e)}
                        >
                          <Copy className="h-3 w-3" />
                          {t("settings.claims.copyEvidence")}
                        </Button>
                      </div>
                      {e.quote && (
                        <blockquote className="mt-1 border-l-2 border-border pl-2 text-xs leading-relaxed text-foreground break-words">
                          {e.quote}
                        </blockquote>
                      )}
                    </li>
                  ))}
                </ul>
              )}

              {detail.links.length > 0 && (
                <>
                  <div className="font-medium pt-1">
                    {t("settings.claims.links")} ({detail.links.length})
                  </div>
                  <ul className="space-y-1">
                    {detail.links.map((l) => (
                      <li
                        key={`${l.claimId}-${l.memoryId}`}
                        className="font-mono text-[10px] text-muted-foreground"
                      >
                        memory #{l.memoryId} · {l.syncMode}
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>
          ) : (
            <>
              {selectedId && detailErrorToast ? (
                <div className="py-10 text-center text-xs">
                  <div className="font-medium text-foreground">{detailErrorToast.title}</div>
                  {detailErrorToast.description && (
                    <div className="mt-1 break-all text-[11px] text-muted-foreground">
                      {detailErrorToast.description}
                    </div>
                  )}
                </div>
              ) : (
                <div className="text-xs text-muted-foreground text-center py-12">
                  {t("settings.claims.selectClaim")}
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {/* Confirm before enabling one conflicting claim and archiving loaded alternatives. */}
      <Dialog
        open={conflictResolveOpen}
        onOpenChange={(o) => {
          if (!conflictResolving) setConflictResolveOpen(o)
        }}
      >
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("settings.claims.conflictUseCurrentTitle")}</DialogTitle>
            <DialogDescription className="leading-relaxed">
              {t("settings.claims.conflictUseCurrentDesc", {
                count: detailConflictMatches.length,
              })}
            </DialogDescription>
          </DialogHeader>
          {detail && (
            <div className="space-y-2 text-xs">
              <div className="rounded border border-emerald-500/30 bg-emerald-500/5 px-2 py-1.5">
                <div className="text-[10px] font-medium uppercase text-muted-foreground">
                  {t("settings.claims.conflictUseCurrent")}
                </div>
                <div className="mt-0.5 break-words font-medium">{detail.claim.content}</div>
              </div>
              <div className="max-h-48 space-y-1 overflow-y-auto">
                {detailConflictMatches.map(({ claim, kind }) => (
                  <div
                    key={`${kind}-${claim.id}`}
                    className="rounded border border-border/50 px-2 py-1.5"
                  >
                    <div className="break-words font-medium">{claim.content}</div>
                    <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
                      {t(`settings.claims.status.${claim.status}`)} · {scopeLabel(claim)} ·{" "}
                      {kind === "active"
                        ? t("settings.claims.conflictKindActive")
                        : t("settings.claims.conflictKindReview")}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={conflictResolving}
              onClick={() => setConflictResolveOpen(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="button"
              size="sm"
              className="gap-1.5"
              disabled={conflictResolving}
              onClick={() => void confirmUseCurrentConflict()}
            >
              {conflictResolving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {t("settings.claims.conflictUseCurrent")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Backfill dry-run preview + apply */}
      <Dialog
        open={backfillOpen}
        onOpenChange={(o) => {
          if (!applying) setBackfillOpen(o)
        }}
      >
        <DialogContent className="flex max-h-[85vh] w-[calc(100vw-2rem)] max-w-3xl flex-col overflow-hidden">
          <DialogHeader className="min-w-0 pr-6">
            <DialogTitle>{t("settings.claims.backfill.title")}</DialogTitle>
            <DialogDescription className="break-words leading-relaxed">
              {t("settings.claims.backfill.desc")}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 min-w-0 overflow-y-auto pr-1">
            {planLoading ? (
              <div className="py-10 text-center text-xs text-muted-foreground inline-flex items-center justify-center gap-1.5 w-full">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.loading")}
              </div>
            ) : plan ? (
              <div className="min-w-0 space-y-3">
                <div className="grid min-w-0 grid-cols-2 gap-2 text-center sm:grid-cols-5">
                  {(
                    [
                      ["summaryTotal", plan.summary.totalMemories],
                      ["summaryLinked", plan.summary.alreadyLinked],
                      ["summaryCandidates", plan.summary.candidates],
                      ["summaryActive", plan.summary.autoActive],
                      ["summaryReview", plan.summary.needsReview],
                    ] as const
                  ).map(([key, value]) => (
                    <div key={key} className="min-w-0 rounded-lg border border-border/60 px-2 py-2">
                      <div className="text-sm font-semibold tabular-nums">{value}</div>
                      <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                        {t(`settings.claims.backfill.${key}`)}
                      </div>
                    </div>
                  ))}
                </div>

                <div className="max-h-[42vh] min-w-0 overflow-y-auto overflow-x-hidden rounded-lg border border-border/60 sm:max-h-[360px]">
                  {plan.candidates.length === 0 ? (
                    <div className="px-3 py-8 text-xs text-muted-foreground text-center">
                      {t("settings.claims.backfill.empty")}
                    </div>
                  ) : (
                    plan.candidates.map((c) => (
                      <div
                        key={c.memoryId}
                        className="min-w-0 px-3 py-2 text-xs border-b border-border/30 last:border-0"
                      >
                        <div className="flex min-w-0 items-center gap-2">
                          <span
                            className={`h-2 w-2 rounded-full shrink-0 ${STATUS_DOT[c.proposedStatus] ?? "bg-muted-foreground/50"}`}
                          />
                          <span className="min-w-0 flex-1 truncate">{c.content}</span>
                        </div>
                        <div className="mt-0.5 min-w-0 truncate font-mono text-[10px] text-muted-foreground">
                          {c.claimType} · {scopeLabel(c)} ·{" "}
                          {t(`settings.claims.status.${c.proposedStatus}`)}
                          {c.pinned ? " · pinned" : ""}
                        </div>
                      </div>
                    ))
                  )}
                </div>
                {plan.previewTruncated && (
                  <div className="text-[10px] text-muted-foreground text-center">
                    {t("settings.claims.backfill.previewTruncated", {
                      shown: plan.candidates.length,
                      total: plan.summary.candidates,
                    })}
                  </div>
                )}
              </div>
            ) : null}
          </div>

          <DialogFooter className="shrink-0">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setBackfillOpen(false)}
              disabled={applying}
            >
              {t("common.cancel")}
            </Button>
            <Button size="sm" onClick={runApply} disabled={applying || noCandidates}>
              {applying ? (
                <span className="inline-flex items-center gap-1.5">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("settings.claims.backfill.applying")}
                </span>
              ) : (
                t("settings.claims.backfill.apply", { count: plan?.summary.candidates ?? 0 })
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
