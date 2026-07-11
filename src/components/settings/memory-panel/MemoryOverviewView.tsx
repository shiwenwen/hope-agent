import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  Activity,
  AlertTriangle,
  Archive,
  ArrowRight,
  BookmarkPlus,
  Brain,
  CheckCircle2,
  Copy,
  Database,
  ExternalLink,
  Eye,
  FolderKanban,
  History,
  Loader2,
  Pencil,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  UserRound,
  Workflow,
  X,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import {
  MEMORY_SOURCE_FILTERS,
  MEMORY_SOURCE_FILTER_SOURCES,
  MEMORY_TYPES,
  MEMORY_TYPE_ICONS,
  type MemoryEpisodeListPage,
  type MemoryEpisodePatch,
  type MemoryEpisodeRecord,
  type MemoryExperienceHistoryListPage,
  type MemoryExperienceHistoryRecord,
  type MemoryHealth,
  type MemoryHistoryListResponse,
  type MemoryRepairReport,
  type MemoryRepairAction,
  type MemoryRepairArtifactFile,
  type MemoryDbSnapshotRestoreReport,
  type MemoryDbSnapshotRestorePreview,
  type MemoryEntry,
  type MemoryHistoryAction,
  type MemoryHistoryQuery,
  type MemoryHistoryRecord,
  type MemoryProcedureListPage,
  type MemoryProcedurePatch,
  type MemoryProcedureRecord,
  type MemoryScope,
  type NewMemoryEpisode,
  type NewMemoryProcedure,
  type MemorySourceFilter,
} from "./types"
import {
  DEFAULT_ACTIVE_MEMORY,
  type ActiveMemoryConfig,
  type AgentConfig,
} from "../types"
import {
  isRecommendedActiveMemory,
  withRecommendedActiveMemory,
} from "../agent-panel/activeMemoryPreset"
import {
  activeMemoryReadinessItems,
  activeMemorySummaryItems,
  type ActiveMemoryReadinessItem,
  type ActiveMemorySummaryItem,
} from "../agent-panel/activeMemorySummary"
import {
  setMemoryFocusUrl,
  type MemoryOverviewFocus,
} from "./memoryFocus"
import {
  formatMemoryUseInRepliesError,
  memoryUseInRepliesErrorDescription,
  normalizeDefaultMemoryAgentId,
} from "./defaultMemoryAgent"
import {
  DEFAULT_CLAIM_SCHEMA,
  normalizeClaimSchema,
  type ClaimRecord,
  type ClaimSchemaMetadata,
} from "./claimTypes"
import { requestMemoryScopeFocus } from "./scopeFocus"
import type { useMemoryData } from "./useMemoryData"
import type { AgentInfo } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import {
  formatDeepResolverHealthSummary,
  formatMemoryHealthDiagnostics,
} from "./memoryHealthFormat"
import {
  externalMemoryProviderOverview,
  externalMemoryProviderSyncBlockSummary,
} from "./externalMemoryProviderReadiness"
import { externalMemoryProviderSyncBlockReasonLabel } from "./externalMemoryProviderLabels"
import { memoryHealthRepairHints, memoryHealthRepairPolicy } from "./memoryHealthRepairHints"
import {
  memoryExperienceOperationErrorToast,
  type MemoryExperienceOperationErrorToast,
} from "./memoryExperienceOperationFeedback"
import {
  formatMemoryDbSnapshotRestorePreviewDiagnostics,
  formatMemorySnapshotArtifactDiagnostics,
  memorySnapshotArtifactSummaryParts,
} from "./memorySnapshotArtifactFormat"
import {
  memoryRepairOperationErrorToast,
  type MemoryRepairOperationErrorToast,
} from "./memoryRepairOperationFeedback"
import {
  countMemoryAuditActivity,
  includeCrossSourceAudit,
  mergeMemoryAuditActivity,
  splitMemoryAuditPage,
  type MemoryAuditPageItem,
} from "./memoryAuditActivity"
import {
  memoryAuditDegradedIssue,
  memoryAuditDegradedWarning,
  memoryAuditOperationErrorToast,
  type MemoryAuditDegradedIssue,
} from "./memoryAuditOperationFeedback"
import {
  memoryOverviewInsightsIssue,
  memoryOverviewInsightsWarning,
  memoryOverviewLoadIssue,
  memoryOverviewLoadWarning,
  memoryOverviewOpenMemoryErrorToast,
  memoryOverviewPendingClaimsErrorToast,
  type MemoryOverviewInsightsIssue,
  type MemoryOverviewLoadIssue,
  type MemoryOverviewOperationErrorToast,
} from "./memoryOverviewOperationFeedback"

type MemoryData = ReturnType<typeof useMemoryData>
type MemoryCenterTab = "overview" | "settings" | "manage" | "dreaming" | "profile" | "claims"

function experienceDomId(kind: "episode" | "procedure", id: string): string {
  return `memory-experience-${kind}-${encodeURIComponent(id)}`
}

interface MemoryOverviewViewProps {
  data: MemoryData
  isAgentMode: boolean
  onSelectTab: (tab: MemoryCenterTab) => void
  onOpenClaims: (focus?: {
    statusFilter?: string
    claimType?: string | null
    scopeType?: string | null
    scopeId?: string | null
    selectedId?: string | null
  }) => void
  focus?: { nonce: number; kind: "episode" | "procedure"; id: string } | null
  auditFocus?: ({ nonce: number } & MemoryOverviewFocus) | null
}

interface ProfileSnapshotRecord {
  scopeType: string
  scopeId?: string | null
  version: number
  bodyMd: string
  sourceRunId: string
  createdAt: string
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

interface MemoryAuditSourceSummary {
  included: boolean
  total: number
  totalTruncated?: boolean
}

interface MemoryAuditPageResponse {
  items: MemoryAuditPageItem<
    MemoryHistoryRecord,
    MemoryExperienceHistoryRecord,
    DreamingDecisionListItem
  >[]
  total: number
  totalTruncated?: boolean
  sources: {
    legacyMemory: MemoryAuditSourceSummary
    experience: MemoryAuditSourceSummary
    claimDecision: MemoryAuditSourceSummary
  }
}

interface RecentCorrectionItem {
  id: string
  decisionType: string
  targetType: string
  targetId?: string | null
  trigger: string
  phase: string
  status: string
  rationale: string
  content?: string | null
  createdAt: string
}

interface MemoryAuditFilterPreset {
  id: string
  query: string
  action: MemoryHistoryAction | "all"
  updatedAt: number
}

interface EpisodeDraft {
  title: string
  situation: string
  actions: string
  outcome: string
  lesson: string
  tags: string
}

interface ProcedureDraft {
  title: string
  trigger: string
  stepsMarkdown: string
  constraintsMarkdown: string
  confidencePercent: string
  tags: string
}

interface ExperienceScopeDraft {
  kind: "global" | "agent" | "project"
  id: string
}

interface ExperienceScopeFilterDraft {
  kind: "all" | "global" | "agent" | "project"
  id: string
}

type ExperienceDetail =
  | { kind: "episode"; record: MemoryEpisodeRecord }
  | { kind: "procedure"; record: MemoryProcedureRecord }

type ExperienceStatusFilter = "active" | "archived" | "all"
type ExperienceSort = "updated_desc" | "updated_asc" | "title_asc" | "quality_desc"

type RecentUnifiedActivityItem =
  | {
      kind: "memory_event"
      key: string
      createdAt: string
      title: string
      subtitle: string[]
      detail?: string | null
      event: MemoryHistoryRecord
      disabled?: boolean
    }
  | {
      kind: "memory"
      key: string
      createdAt: string
      title: string
      subtitle: string[]
      detail?: string | null
      memory: MemoryEntry
    }
  | {
      kind: "decision"
      key: string
      createdAt: string
      title: string
      subtitle: string[]
      detail?: string | null
      decision: RecentCorrectionItem
    }
  | {
      kind: "claim"
      key: string
      createdAt: string
      title: string
      subtitle: string[]
      detail?: string | null
      claim: ClaimRecord
    }
  | {
      kind: "experience_event"
      key: string
      createdAt: string
      title: string
      subtitle: string[]
      detail?: string | null
      event: MemoryExperienceHistoryRecord
    }

function pct(part: number, total: number): number {
  if (total <= 0) return 0
  return Math.round((part / total) * 100)
}

function statusText(enabled: boolean, on: string, off: string): string {
  return enabled ? on : off
}

function activeMemoryReadinessClassName(tone: ActiveMemoryReadinessItem["tone"]): string {
  switch (tone) {
    case "ok":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
    case "warning":
      return "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
    case "notice":
      return "border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300"
  }
}

const RECENT_CORRECTION_STATUSES = ["archived", "expired", "superseded"] as const
const ACTIVITY_WINDOW_MS = 7 * 24 * 60 * 60 * 1000
const MEMORY_AUDIT_PAGE_SIZE = 20
const EXPERIENCE_PAGE_SIZE = 4
const EXPERIENCE_DEFAULT_STATUS: ExperienceStatusFilter = "active"
const EXPERIENCE_DEFAULT_SORT: ExperienceSort = "updated_desc"
const PROCEDURE_GUIDANCE_DEFAULT_MIN_CONFIDENCE = 0.7
const EXPERIENCE_STATUS_FILTERS: ExperienceStatusFilter[] = ["active", "archived", "all"]
const EXPERIENCE_SORTS: ExperienceSort[] = [
  "updated_desc",
  "updated_asc",
  "title_asc",
  "quality_desc",
]
const EMPTY_EPISODE_DRAFT: EpisodeDraft = {
  title: "",
  situation: "",
  actions: "",
  outcome: "",
  lesson: "",
  tags: "",
}
const EMPTY_PROCEDURE_DRAFT: ProcedureDraft = {
  title: "",
  trigger: "",
  stepsMarkdown: "",
  constraintsMarkdown: "",
  confidencePercent: "80",
  tags: "",
}
const EMPTY_EXPERIENCE_SCOPE_DRAFT: ExperienceScopeDraft = {
  kind: "global",
  id: "",
}
const EMPTY_EXPERIENCE_SCOPE_FILTER: ExperienceScopeFilterDraft = {
  kind: "all",
  id: "",
}

function lines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
}

function commaList(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
}

function experienceStatusParam(status: ExperienceStatusFilter): string | null {
  return status === "all" ? null : status
}

function scopeFromExperienceDraft(draft: ExperienceScopeDraft): MemoryScope | null {
  if (draft.kind === "global") return { kind: "global" }
  const id = draft.id.trim()
  if (!id) return null
  return { kind: draft.kind, id }
}

function scopeFromExperienceFilter(draft: ExperienceScopeFilterDraft): MemoryScope | null {
  if (draft.kind === "all") return null
  if (draft.kind === "global") return { kind: "global" }
  const id = draft.id.trim()
  if (!id) return null
  return { kind: draft.kind, id }
}

function experienceDraftFromScope(scope: MemoryScope): ExperienceScopeDraft {
  if (scope.kind === "global") return EMPTY_EXPERIENCE_SCOPE_DRAFT
  return { kind: scope.kind, id: scope.id }
}
const MEMORY_AUDIT_EXPORT_PAGE_SIZE = 100
const MEMORY_AUDIT_EXPORT_MAX_EVENTS = 5000
const MEMORY_AUDIT_PRESET_LIMIT = 6
const MEMORY_AUDIT_PRESET_STORAGE_KEY = "hope.memory.auditFilterPresets.v1"
const MEMORY_AUDIT_ACTIONS: Array<MemoryHistoryAction | "all"> = [
  "all",
  "add",
  "update",
  "delete",
  "pin",
  "unpin",
  "import",
]

function normalizeMemoryAuditAction(value: unknown): MemoryHistoryAction | "all" {
  return typeof value === "string" &&
    MEMORY_AUDIT_ACTIONS.includes(value as MemoryHistoryAction | "all")
    ? (value as MemoryHistoryAction | "all")
    : "all"
}

function memoryAuditPresetId(query: string, action: MemoryHistoryAction | "all"): string {
  return [query.trim().toLocaleLowerCase(), action].join("|")
}

function normalizeMemoryAuditPreset(raw: unknown): MemoryAuditFilterPreset | null {
  if (!raw || typeof raw !== "object") return null
  const value = raw as Record<string, unknown>
  const query = typeof value.query === "string" ? value.query.trim().slice(0, 200) : ""
  const action = normalizeMemoryAuditAction(value.action)
  const updatedAt = Number(value.updatedAt)
  return {
    id: memoryAuditPresetId(query, action),
    query,
    action,
    updatedAt: Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : Date.now(),
  }
}

function loadMemoryAuditFilterPresets(): MemoryAuditFilterPreset[] {
  if (typeof window === "undefined") return []
  try {
    const raw = window.localStorage.getItem(MEMORY_AUDIT_PRESET_STORAGE_KEY)
    const parsed = raw ? JSON.parse(raw) : []
    if (!Array.isArray(parsed)) return []
    const deduped = new Map<string, MemoryAuditFilterPreset>()
    for (const item of parsed) {
      const preset = normalizeMemoryAuditPreset(item)
      if (!preset) continue
      const existing = deduped.get(preset.id)
      if (!existing || existing.updatedAt < preset.updatedAt) {
        deduped.set(preset.id, preset)
      }
    }
    return [...deduped.values()]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, MEMORY_AUDIT_PRESET_LIMIT)
  } catch {
    return []
  }
}

function persistMemoryAuditFilterPresets(presets: MemoryAuditFilterPreset[]) {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      MEMORY_AUDIT_PRESET_STORAGE_KEY,
      JSON.stringify(presets.slice(0, MEMORY_AUDIT_PRESET_LIMIT)),
    )
  } catch {
    // localStorage may be unavailable in private / restricted contexts.
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

function isWithinActivityWindow(value: string): boolean {
  const parsed = timeValue(value)
  const ageMs = Date.now() - parsed
  return parsed > 0 && ageMs >= 0 && ageMs <= ACTIVITY_WINDOW_MS
}

function healthTone(status: MemoryHealth["status"] | undefined): {
  badge: string
  icon: string
} {
  switch (status) {
    case "error":
      return {
        badge: "bg-destructive/10 text-destructive",
        icon: "text-destructive",
      }
    case "warning":
      return {
        badge: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
        icon: "text-amber-600 dark:text-amber-300",
      }
    default:
      return {
        badge: "bg-green-500/10 text-green-700 dark:text-green-300",
        icon: "text-green-600 dark:text-green-300",
      }
  }
}

function sourceFilterFor(rawSource: string): MemorySourceFilter | null {
  for (const source of MEMORY_SOURCE_FILTERS) {
    if (MEMORY_SOURCE_FILTER_SOURCES[source].includes(rawSource)) return source
  }
  return null
}

function memoryHistoryActionFallback(action: MemoryHistoryRecord["action"]): string {
  switch (action) {
    case "import":
      return "Imported"
    case "update":
      return "Updated"
    case "delete":
      return "Deleted"
    case "pin":
      return "Pinned"
    case "unpin":
      return "Unpinned"
    default:
      return "Added"
  }
}

function compareClaimsForOverview(a: ClaimRecord, b: ClaimRecord): number {
  const salience = b.salience - a.salience
  if (Math.abs(salience) > 0.001) return salience
  const confidence = b.confidence - a.confidence
  if (Math.abs(confidence) > 0.001) return confidence
  return timeValue(b.updatedAt) - timeValue(a.updatedAt)
}

function sortClaimsForOverview(claims: ClaimRecord[]): ClaimRecord[] {
  return [...claims].sort(compareClaimsForOverview)
}

function dreamingStatusDot(status: string): string {
  switch (status) {
    case "running":
      return "bg-amber-500"
    case "completed":
      return "bg-emerald-500"
    case "failed":
      return "bg-red-500"
    default:
      return "bg-muted-foreground/50"
  }
}

function isCorrectionDecision(decision: DreamingDecisionRecord): boolean {
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

function decisionContent(decision: DreamingDecisionRecord): string | null {
  return (
    textFieldFromJson(decision.afterJson, "content") ??
    textFieldFromJson(decision.beforeJson, "content")
  )
}

function recentCorrectionFromDecisionListItem(item: DreamingDecisionListItem): RecentCorrectionItem {
  return {
    id: item.id,
    decisionType: item.decisionType,
    targetType: item.targetType,
    targetId: item.targetId ?? null,
    trigger: item.runTrigger,
    phase: item.runPhase,
    status: item.runStatus,
    rationale: item.rationale,
    content: item.content ?? decisionContent(item),
    createdAt: item.createdAt,
  }
}

function profileSnapshotLines(bodyMd: string): string[] {
  return bodyMd
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map((line) => line.replace(/^[-*]\s+/, "").trim())
    .filter(Boolean)
    .slice(0, 3)
}

async function resolveDefaultMemoryAgentId(): Promise<string> {
  try {
    const id = await getTransport().call<string | null>("get_default_agent_id")
    return normalizeDefaultMemoryAgentId(id)
  } catch (e) {
    logger.warn(
      "settings",
      "MemoryOverviewView::resolveDefaultMemoryAgentId",
      "Failed to load default agent id",
      e,
    )
    return DEFAULT_AGENT_ID
  }
}

export default function MemoryOverviewView({
  data,
  isAgentMode,
  onSelectTab,
  onOpenClaims,
  focus,
  auditFocus,
}: MemoryOverviewViewProps) {
  const { t } = useTranslation()
  const [pendingClaims, setPendingClaims] = useState<ClaimRecord[]>([])
  const [pendingLoading, setPendingLoading] = useState(false)
  const [pendingClaimsError, setPendingClaimsError] =
    useState<MemoryOverviewOperationErrorToast | null>(null)
  const [recentMemories, setRecentMemories] = useState<MemoryEntry[]>([])
  const [recentMemoryEvents, setRecentMemoryEvents] = useState<MemoryHistoryRecord[]>([])
  const [memoryAuditOpen, setMemoryAuditOpen] = useState(false)
  const [memoryAuditQuery, setMemoryAuditQuery] = useState("")
  const [memoryAuditAction, setMemoryAuditAction] = useState<MemoryHistoryAction | "all">("all")
  const [memoryAuditResults, setMemoryAuditResults] = useState<MemoryHistoryRecord[]>([])
  const [memoryAuditLoading, setMemoryAuditLoading] = useState(false)
  const [memoryAuditHasMore, setMemoryAuditHasMore] = useState(false)
  const [memoryAuditTotal, setMemoryAuditTotal] = useState<number | null>(null)
  const [memoryAuditTotalTruncated, setMemoryAuditTotalTruncated] = useState(false)
  const [memoryAuditExperienceResults, setMemoryAuditExperienceResults] = useState<
    MemoryExperienceHistoryRecord[]
  >([])
  const [memoryAuditExperienceHasMore, setMemoryAuditExperienceHasMore] = useState(false)
  const [memoryAuditExperienceTotal, setMemoryAuditExperienceTotal] = useState<number | null>(null)
  const [memoryAuditExperienceTotalTruncated, setMemoryAuditExperienceTotalTruncated] =
    useState(false)
  const [memoryAuditDecisionResults, setMemoryAuditDecisionResults] = useState<
    RecentCorrectionItem[]
  >([])
  const [memoryAuditDecisionHasMore, setMemoryAuditDecisionHasMore] = useState(false)
  const [memoryAuditDecisionTotal, setMemoryAuditDecisionTotal] = useState<number | null>(null)
  const [memoryAuditDecisionTotalTruncated, setMemoryAuditDecisionTotalTruncated] = useState(false)
  const [memoryAuditDegradedIssues, setMemoryAuditDegradedIssues] = useState<
    MemoryAuditDegradedIssue[]
  >([])
  const [memoryAuditExportingAll, setMemoryAuditExportingAll] = useState(false)
  const [memoryAuditPresets, setMemoryAuditPresets] = useState<MemoryAuditFilterPreset[]>(() =>
    loadMemoryAuditFilterPresets(),
  )
  const [recentCorrections, setRecentCorrections] = useState<ClaimRecord[]>([])
  const [recentCorrectionDecisions, setRecentCorrectionDecisions] = useState<
    RecentCorrectionItem[]
  >([])
  const [recentDreamingRuns, setRecentDreamingRuns] = useState<DreamingRunRecord[]>([])
  const [recentEpisodes, setRecentEpisodes] = useState<MemoryEpisodeRecord[]>([])
  const [recentProcedures, setRecentProcedures] = useState<MemoryProcedureRecord[]>([])
  const [recentExperienceEvents, setRecentExperienceEvents] = useState<
    MemoryExperienceHistoryRecord[]
  >([])
  const [experienceQuery, setExperienceQuery] = useState("")
  const [experienceAppliedQuery, setExperienceAppliedQuery] = useState("")
  const [experienceStatus, setExperienceStatus] =
    useState<ExperienceStatusFilter>(EXPERIENCE_DEFAULT_STATUS)
  const [experienceSort, setExperienceSort] = useState<ExperienceSort>(EXPERIENCE_DEFAULT_SORT)
  const [experienceScopeFilter, setExperienceScopeFilter] =
    useState<ExperienceScopeFilterDraft>(EMPTY_EXPERIENCE_SCOPE_FILTER)
  const [experienceEpisodeTotal, setExperienceEpisodeTotal] = useState(0)
  const [experienceProcedureTotal, setExperienceProcedureTotal] = useState(0)
  const [experienceSearchLoading, setExperienceSearchLoading] = useState(false)
  const [experienceLoadingMore, setExperienceLoadingMore] = useState<
    "episode" | "procedure" | null
  >(null)
  const [experienceFocusHighlight, setExperienceFocusHighlight] = useState<{
    kind: "episode" | "procedure"
    id: string
  } | null>(null)
  const [experienceDetail, setExperienceDetail] = useState<ExperienceDetail | null>(null)
  const [experienceHistory, setExperienceHistory] = useState<MemoryExperienceHistoryRecord[]>([])
  const [experienceHistoryLoading, setExperienceHistoryLoading] = useState(false)
  const [experienceHistoryError, setExperienceHistoryError] =
    useState<MemoryExperienceOperationErrorToast | null>(null)
  const [experienceStatusSaving, setExperienceStatusSaving] = useState(false)
  const [experienceSourceOpening, setExperienceSourceOpening] = useState<string | null>(null)
  const lastExperienceFocusNonceRef = useRef(0)
  const lastAuditFocusNonceRef = useRef(0)
  const recentEpisodesRef = useRef<MemoryEpisodeRecord[]>([])
  const recentProceduresRef = useRef<MemoryProcedureRecord[]>([])
  const [episodeDialogOpen, setEpisodeDialogOpen] = useState(false)
  const [episodeSaving, setEpisodeSaving] = useState(false)
  const [episodeEditingId, setEpisodeEditingId] = useState<string | null>(null)
  const [episodeDraft, setEpisodeDraft] = useState<EpisodeDraft>(EMPTY_EPISODE_DRAFT)
  const [episodeScopeDraft, setEpisodeScopeDraft] = useState<ExperienceScopeDraft>(
    EMPTY_EXPERIENCE_SCOPE_DRAFT,
  )
  const [procedureDialogOpen, setProcedureDialogOpen] = useState(false)
  const [procedureSaving, setProcedureSaving] = useState(false)
  const [procedureEditingId, setProcedureEditingId] = useState<string | null>(null)
  const [procedureDraft, setProcedureDraft] = useState<ProcedureDraft>(EMPTY_PROCEDURE_DRAFT)
  const [procedureScopeDraft, setProcedureScopeDraft] = useState<ExperienceScopeDraft>(
    EMPTY_EXPERIENCE_SCOPE_DRAFT,
  )
  const [activitySummary, setActivitySummary] = useState({ learned7d: 0, rejected7d: 0 })
  const [activityLoading, setActivityLoading] = useState(false)
  const [activityLoadIssues, setActivityLoadIssues] = useState<MemoryOverviewLoadIssue[]>([])
  const [memoryHealth, setMemoryHealth] = useState<MemoryHealth | null>(null)
  const [healthLoading, setHealthLoading] = useState(false)
  const [memoryHealthLoadError, setMemoryHealthLoadError] =
    useState<MemoryRepairOperationErrorToast | null>(null)
  const [repairingFts, setRepairingFts] = useState(false)
  const [repairingClaimFts, setRepairingClaimFts] = useState(false)
  const [repairingClaimGraph, setRepairingClaimGraph] = useState(false)
  const [repairingExperienceGraph, setRepairingExperienceGraph] = useState(false)
  const [repairingDreamingState, setRepairingDreamingState] = useState(false)
  const [repairingDbSnapshot, setRepairingDbSnapshot] = useState(false)
  const [checkingDbSnapshotRestore, setCheckingDbSnapshotRestore] = useState(false)
  const [restoringDbSnapshot, setRestoringDbSnapshot] = useState(false)
  const [dbSnapshotRestorePreview, setDbSnapshotRestorePreview] =
    useState<MemoryDbSnapshotRestorePreview | null>(null)
  const [lastDbSnapshotPath, setLastDbSnapshotPath] = useState<string | null>(null)
  const [lastDbSnapshotFiles, setLastDbSnapshotFiles] = useState<MemoryRepairArtifactFile[]>([])
  const [profileClaims, setProfileClaims] = useState<ClaimRecord[]>([])
  const [projectClaims, setProjectClaims] = useState<ClaimRecord[]>([])
  const [profileSnapshots, setProfileSnapshots] = useState<ProfileSnapshotRecord[]>([])
  const [insightsLoading, setInsightsLoading] = useState(false)
  const [insightsLoadIssues, setInsightsLoadIssues] = useState<MemoryOverviewInsightsIssue[]>([])
  const [claimSchema, setClaimSchema] = useState<ClaimSchemaMetadata>(DEFAULT_CLAIM_SCHEMA)
  const [agentNames, setAgentNames] = useState<Map<string, string>>(() => new Map())
  const [projectNames, setProjectNames] = useState<Map<string, string>>(() => new Map())
  const [agentOptions, setAgentOptions] = useState<AgentInfo[]>([])
  const [projectOptions, setProjectOptions] = useState<ProjectMeta[]>([])
  const [activeMemoryAgentId, setActiveMemoryAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [targetAgentMemoryEnabled, setTargetAgentMemoryEnabled] = useState(true)
  const [targetAgentActiveMemory, setTargetAgentActiveMemory] = useState<ActiveMemoryConfig | null>(
    null,
  )
  const [activeMemoryLoading, setActiveMemoryLoading] = useState(false)
  const [activeMemorySaving, setActiveMemorySaving] = useState(false)
  const [activeMemoryError, setActiveMemoryError] = useState<string | null>(null)
  const stats = data.stats
  const total = stats?.total ?? data.totalCount
  const reloadMemories = data.loadMemories
  const memoryEnabled = data.effectiveMemoryEnabled
  const canReviewClaims = !isAgentMode && data.effectiveExtractClaims
  const activityLoadWarning = useMemo(
    () => memoryOverviewLoadWarning(activityLoadIssues, t),
    [activityLoadIssues, t],
  )
  const insightsLoadWarning = useMemo(
    () => memoryOverviewInsightsWarning(insightsLoadIssues, t),
    [insightsLoadIssues, t],
  )
  const pendingClaimsCountLabel = pendingClaimsError ? "--" : String(pendingClaims.length)
  const vectorPct = stats ? pct(stats.withEmbedding, stats.total) : 0
  const activeEmbeddingModel =
    data.memoryEmbeddingState.currentModel?.name ||
    data.memoryEmbeddingState.currentModel?.id ||
    null
  const embeddingReady = data.memoryEmbeddingState.selection.enabled && !!activeEmbeddingModel
  const profileClaimTypes = claimSchema.profileClaimTypes
  const projectClaimType = claimSchema.projectClaimType
  const typeRows = useMemo(
    () =>
      MEMORY_TYPES.map((type) => {
        const Icon = MEMORY_TYPE_ICONS[type]
        return {
          type,
          Icon,
          count: stats?.byType[type] ?? 0,
        }
      }).filter((row) => row.count > 0),
    [stats],
  )
  const sourceRows = useMemo(
    () =>
      MEMORY_SOURCE_FILTERS.map((source) => {
        const count = MEMORY_SOURCE_FILTER_SOURCES[source].reduce(
          (sum, rawSource) => sum + (stats?.bySource?.[rawSource] ?? 0),
          0,
        )
        return {
          source,
          count,
          pct: pct(count, total),
        }
      }).filter((row) => row.count > 0),
    [stats, total],
  )
  const recentSourceRows = useMemo(() => {
    const counts = new Map<string, { label: string; count: number }>()
    const addSource = (rawSource: string) => {
      const filter = sourceFilterFor(rawSource)
      const normalizedSource = rawSource.trim()
      const key = filter ?? (normalizedSource || "unknown")
      const label = filter
        ? t(`settings.memorySource_${filter}`)
        : normalizedSource || t("common.unknown", "Unknown")
      const current = counts.get(key)
      counts.set(key, { label, count: (current?.count ?? 0) + 1 })
    }
    if (recentMemoryEvents.length > 0) {
      for (const event of recentMemoryEvents) {
        if (!["add", "import", "update"].includes(event.action)) continue
        if (!isWithinActivityWindow(event.createdAt)) continue
        addSource(event.source)
      }
    } else {
      for (const memory of recentMemories) {
        if (!isWithinActivityWindow(memory.updatedAt)) continue
        addSource(memory.source)
      }
    }
    const totalRecent = [...counts.values()].reduce((sum, row) => sum + row.count, 0)
    return [...counts.entries()]
      .map(([key, row]) => ({
        key,
        label: row.label,
        count: row.count,
        pct: pct(row.count, totalRecent),
      }))
      .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label))
      .slice(0, 4)
  }, [recentMemories, recentMemoryEvents, t])

  const onLabel = t("common.on", "On")
  const offLabel = t("common.off", "Off")
  const activeMemoryConfig = targetAgentActiveMemory ?? DEFAULT_ACTIVE_MEMORY
  const activeMemoryAgentName =
    agentOptions.find((agent) => agent.id === activeMemoryAgentId)?.name ?? activeMemoryAgentId
  const activeMemorySummary = useMemo(
    () => activeMemorySummaryItems(activeMemoryConfig),
    [activeMemoryConfig],
  )
  const activeMemoryReadiness = useMemo(
    () =>
      activeMemoryReadinessItems(activeMemoryConfig, {
        agentMemoryEnabled: targetAgentMemoryEnabled,
      }),
    [activeMemoryConfig, targetAgentMemoryEnabled],
  )
  const activeMemorySummaryLabel = useCallback(
    (id: ActiveMemorySummaryItem["id"]) => {
      switch (id) {
        case "timeout":
          return t("settings.memoryUseInRepliesSummaryTimeout", "Timeout")
        case "cache":
          return t("settings.memoryUseInRepliesSummaryCache", "Cache")
        case "candidates":
          return t("settings.memoryUseInRepliesSummaryCandidates", "Candidates")
        case "maxChars":
          return t("settings.memoryUseInRepliesSummaryMaxChars", "Max chars")
        case "claims":
          return t("settings.memoryUseInRepliesSummaryClaims", "Claims")
      }
    },
    [t],
  )
  const activeMemoryReadinessLabel = useCallback(
    (id: ActiveMemoryReadinessItem["id"]) => {
      switch (id) {
        case "recommended":
          return t(
            "settings.memoryUseInRepliesReadinessRecommended",
            "Ready: low-latency recall includes structured memories and safe timeout fallback.",
          )
        case "agentMemoryOff":
          return t(
            "settings.memoryUseInRepliesReadinessAgentOff",
            "Agent memory is off. Use recommended recall will turn it back on for the default agent.",
          )
        case "disabled":
          return t(
            "settings.memoryUseInRepliesReadinessDisabled",
            "Off: the default agent will not proactively look up memories before answering.",
          )
        case "claimsOff":
          return t(
            "settings.memoryUseInRepliesReadinessClaimsOff",
            "Structured memories are excluded from proactive recall.",
          )
        case "slowTimeout":
          return t(
            "settings.memoryUseInRepliesReadinessSlowTimeout",
            "Timeout is above the recommended low-latency preset.",
          )
        case "tightBudget":
          return t(
            "settings.memoryUseInRepliesReadinessTightBudget",
            "Recall budget is lower than the recommended preset.",
          )
        case "lowCandidates":
          return t(
            "settings.memoryUseInRepliesReadinessLowCandidates",
            "Candidate limit is below the recommended recall breadth.",
          )
        case "shortSnippets":
          return t(
            "settings.memoryUseInRepliesReadinessShortSnippets",
            "Memory snippets are shorter than the recommended preset.",
          )
        case "custom":
          return t(
            "settings.memoryUseInRepliesReadinessCustom",
            "Custom recall is on. Tune only if answers feel slow or miss context.",
          )
      }
    },
    [t],
  )
  const activeMemoryUsesRecommended =
    targetAgentMemoryEnabled && isRecommendedActiveMemory(activeMemoryConfig)
  const activeMemoryStatusLabel = activeMemoryLoading
    ? t("settings.memoryUseInRepliesStatusLoading", "Checking...")
    : !targetAgentMemoryEnabled
      ? t("settings.memoryUseInRepliesStatusAgentOff", "Agent memory off")
      : activeMemoryUsesRecommended
      ? t("settings.memoryUseInRepliesStatusRecommended", "Recommended preset on")
      : activeMemoryConfig.enabled
        ? t("settings.memoryUseInRepliesStatusOn", "Custom recall on")
        : t("settings.memoryUseInRepliesStatusOff", "Off")
  const activeMemoryStatusClassName = activeMemoryLoading
    ? "border-border/60 bg-secondary text-muted-foreground"
    : !targetAgentMemoryEnabled
      ? "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
      : activeMemoryUsesRecommended
      ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
      : activeMemoryConfig.enabled
        ? "border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300"
        : "border-border/60 bg-secondary text-muted-foreground"

  const applyRecommendedDefaultAgentActiveMemory = useCallback(async () => {
    if (activeMemorySaving || isAgentMode || !memoryEnabled) return
    setActiveMemorySaving(true)
    setActiveMemoryError(null)
    try {
      const agentId = await resolveDefaultMemoryAgentId()
      setActiveMemoryAgentId(agentId)
      const config = await getTransport().call<AgentConfig>("get_agent_config", {
        id: agentId,
      })
      const updated = withRecommendedActiveMemory(config)
      await getTransport().call("save_agent_config_cmd", {
        id: agentId,
        config: updated,
      })
      setTargetAgentMemoryEnabled(updated.memory?.enabled ?? true)
      setTargetAgentActiveMemory(updated.memory?.activeMemory ?? DEFAULT_ACTIVE_MEMORY)
      toast.success(t("settings.memoryUseInRepliesUpdated", "Recommended recall enabled"))
    } catch (e) {
      const message = formatMemoryUseInRepliesError(t, "update", e)
      setActiveMemoryError(message)
      logger.error(
        "settings",
        "MemoryOverviewView::applyRecommendedActiveMemory",
        "Failed to update default agent active memory",
        e,
      )
      const description = memoryUseInRepliesErrorDescription(t, e)
      toast.error(
        t("settings.memoryUseInRepliesUpdateFailed", "Could not update active recall"),
        description ? { description } : undefined,
      )
    } finally {
      setActiveMemorySaving(false)
    }
  }, [activeMemorySaving, isAgentMode, memoryEnabled, t])

  const memorySourceLabel = useCallback(
    (source: string) => {
      const filter = sourceFilterFor(source)
      return filter
        ? t(`settings.memorySource_${filter}`)
        : source || t("common.unknown", "Unknown")
    },
    [t],
  )
  const memoryScopeLabel = useCallback(
    (scope: MemoryHistoryRecord["scope"]) => {
      if (scope.kind === "global") return t("dashboard.dreaming.review.scopeGlobal")
      if (scope.kind === "agent") {
        const name = agentNames.get(scope.id)
        return name ? `${t("dashboard.dreaming.review.scopeAgent")}: ${name}` : `agent:${scope.id}`
      }
      const name = projectNames.get(scope.id)
      return name
        ? `${t("dashboard.dreaming.review.scopeProject")}: ${name}`
        : `project:${scope.id}`
    },
    [agentNames, projectNames, t],
  )
  const memoryAuditActionLabel = useCallback(
    (action: MemoryHistoryAction | "all") => {
      if (action === "all") {
        return t("settings.memoryAuditAllActions", "All changes")
      }
      return t(`settings.memoryHistoryAction_${action}`, memoryHistoryActionFallback(action))
    },
    [t],
  )
  const memoryAuditRequest = useCallback(
    (query: string, action: MemoryHistoryAction | "all", limit: number, offset: number) => {
      const request: MemoryHistoryQuery = { limit, offset }
      const trimmedQuery = query.trim()
      if (trimmedQuery) request.query = trimmedQuery
      if (action !== "all") request.actions = [action]
      return request
    },
    [],
  )
  const memoryAuditDegradedToast = useMemo(
    () => memoryAuditDegradedWarning(memoryAuditDegradedIssues, t),
    [memoryAuditDegradedIssues, t],
  )
  const decisionTypeLabel = useCallback(
    (decisionType: string) =>
      t(`settings.memoryDecisionTypes.${decisionType}`, decisionType.replace(/_/g, " ")),
    [t],
  )
  const claimTypeLabel = useCallback(
    (claimType: string) => t(`settings.claimType_${claimType}`, claimType),
    [t],
  )
  const experienceStatusLabel = useCallback(
    (status: ExperienceStatusFilter) => {
      switch (status) {
        case "archived":
          return t("settings.memoryExperienceStatusArchived", "Archived")
        case "all":
          return t("settings.memoryExperienceStatusAll", "All")
        default:
          return t("settings.memoryExperienceStatusActive", "Active")
      }
    },
    [t],
  )
  const experienceSortLabel = useCallback(
    (sort: ExperienceSort) => {
      switch (sort) {
        case "updated_asc":
          return t("settings.memoryExperienceSortOldest", "Oldest")
        case "title_asc":
          return t("settings.memoryExperienceSortTitle", "Title")
        case "quality_desc":
          return t("settings.memoryExperienceSortQuality", "Quality")
        default:
          return t("settings.memoryExperienceSortRecent", "Recent")
      }
    },
    [t],
  )
  const procedureGuidanceInfo = useCallback(
    (procedure: MemoryProcedureRecord) => {
      if (procedure.status === "archived") {
        return {
          kind: "archived" as const,
          label: t("settings.memoryProcedureGuidanceArchived", "Not used"),
          description: t(
            "settings.memoryProcedureGuidanceArchivedDesc",
            "Archived workflows are never used in replies.",
          ),
          className: "border-border/60 bg-secondary text-muted-foreground",
        }
      }
      if (procedure.confidence < PROCEDURE_GUIDANCE_DEFAULT_MIN_CONFIDENCE) {
        return {
          kind: "low" as const,
          label: t("settings.memoryProcedureGuidanceLowConfidence", "Below guidance threshold"),
          description: t(
            "settings.memoryProcedureGuidanceLowConfidenceDesc",
            "Default workflow guidance requires at least 70% confidence; raise confidence or adjust the Agent setting.",
          ),
          className:
            "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
        }
      }
      return {
        kind: "eligible" as const,
        label: t("settings.memoryProcedureGuidanceEligible", "Guidance-ready"),
        description: t(
          "settings.memoryProcedureGuidanceEligibleDesc",
          "Relevant turns may use this workflow as bounded soft guidance; current instructions and safety policies still win.",
        ),
        className:
          "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
      }
    },
    [t],
  )
  const renderProcedureGuidanceNotice = useCallback(
    (procedure: MemoryProcedureRecord) => {
      const guidance = procedureGuidanceInfo(procedure)
      const Icon = guidance.kind === "eligible" ? CheckCircle2 : AlertTriangle
      return (
        <div
          className={cn(
            "flex gap-2 rounded-md border px-3 py-2 text-xs",
            guidance.className,
          )}
        >
          <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div className="min-w-0">
            <div className="font-medium">{guidance.label}</div>
            <div className="mt-0.5 opacity-80">{guidance.description}</div>
          </div>
        </div>
      )
    },
    [procedureGuidanceInfo],
  )
  const renderExperienceScopePicker = useCallback(
    (
      idPrefix: string,
      draft: ExperienceScopeDraft,
      setDraft: Dispatch<SetStateAction<ExperienceScopeDraft>>,
    ) => {
      const scopedOptions = draft.kind === "agent" ? agentOptions : projectOptions
      const selectedId = draft.kind === "global" ? "" : draft.id
      const setKind = (kind: ExperienceScopeDraft["kind"]) => {
        if (kind === "global") {
          setDraft(EMPTY_EXPERIENCE_SCOPE_DRAFT)
          return
        }
        const firstId =
          kind === "agent" ? (agentOptions[0]?.id ?? "") : (projectOptions[0]?.id ?? "")
        setDraft({ kind, id: firstId })
      }

      return (
        <div className="grid gap-1.5">
          <div className="text-xs font-medium">
            {t("settings.memoryExperienceScope", "Scope")}
          </div>
          <div className="flex flex-wrap gap-1.5">
            <Button
              type="button"
              variant={draft.kind === "global" ? "secondary" : "outline"}
              size="sm"
              className="h-7 px-2 text-[11px]"
              onClick={() => setKind("global")}
            >
              {t("dashboard.dreaming.review.scopeGlobal")}
            </Button>
            <Button
              type="button"
              variant={draft.kind === "agent" ? "secondary" : "outline"}
              size="sm"
              className="h-7 px-2 text-[11px]"
              disabled={agentOptions.length === 0}
              onClick={() => setKind("agent")}
            >
              {t("dashboard.dreaming.review.scopeAgent")}
            </Button>
            <Button
              type="button"
              variant={draft.kind === "project" ? "secondary" : "outline"}
              size="sm"
              className="h-7 px-2 text-[11px]"
              disabled={projectOptions.length === 0}
              onClick={() => setKind("project")}
            >
              {t("dashboard.dreaming.review.scopeProject")}
            </Button>
          </div>
          {draft.kind !== "global" && (
            <select
              id={`${idPrefix}-scope-target`}
              value={selectedId}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  id: event.target.value,
                }))
              }
              className="h-9 rounded-md border border-input bg-background px-3 text-sm"
            >
              {scopedOptions.length === 0 ? (
                <option value="">
                  {draft.kind === "agent"
                    ? t("settings.memoryExperienceNoAgents", "No agents available")
                    : t("settings.memoryExperienceNoProjects", "No projects available")}
                </option>
              ) : (
                scopedOptions.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.name}
                  </option>
                ))
              )}
            </select>
          )}
        </div>
      )
    },
    [agentOptions, projectOptions, t],
  )
  const claimScopeLabel = useCallback(
    (claim: ClaimRecord) => {
      if (claim.scopeType === "global") return t("dashboard.dreaming.review.scopeGlobal")
      const id = claim.scopeId ?? "?"
      if (claim.scopeType === "agent") {
        const name = claim.scopeId ? agentNames.get(claim.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeAgent")}: ${name}` : `agent:${id}`
      }
      if (claim.scopeType === "project") {
        const name = claim.scopeId ? projectNames.get(claim.scopeId) : null
        return name ? `${t("dashboard.dreaming.review.scopeProject")}: ${name}` : `project:${id}`
      }
      return `${claim.scopeType}:${id}`
    },
    [agentNames, projectNames, t],
  )
  const projectClaimGroups = useMemo(() => {
    const groups = new Map<
      string,
      {
        key: string
        label: string
        scopeType: string
        scopeId: string | null
        total: number
        claims: ClaimRecord[]
      }
    >()
    for (const claim of projectClaims) {
      const key = `${claim.scopeType}:${claim.scopeId ?? ""}`
      const existing = groups.get(key)
      if (existing) {
        existing.total += 1
        existing.claims.push(claim)
      } else {
        groups.set(key, {
          key,
          label: claimScopeLabel(claim),
          scopeType: claim.scopeType,
          scopeId: claim.scopeId ?? null,
          total: 1,
          claims: [claim],
        })
      }
    }
    return [...groups.values()]
      .map((group) => ({
        ...group,
        claims: sortClaimsForOverview(group.claims).slice(0, 2),
      }))
      .sort((a, b) => {
        const aTop = a.claims[0]
        const bTop = b.claims[0]
        if (!aTop && !bTop) return 0
        if (!aTop) return 1
        if (!bTop) return -1
        return compareClaimsForOverview(aTop, bTop)
      })
      .slice(0, 3)
  }, [claimScopeLabel, projectClaims])
  const primaryProfileSnapshot = useMemo(() => {
    const globalSnapshot = profileSnapshots.find((snapshot) => snapshot.scopeType === "global")
    if (globalSnapshot) return globalSnapshot
    return (
      [...profileSnapshots].sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))[0] ??
      null
    )
  }, [profileSnapshots])
  const primaryProfileSnapshotLines = useMemo(
    () => profileSnapshotLines(primaryProfileSnapshot?.bodyMd ?? ""),
    [primaryProfileSnapshot],
  )

  const loadMemoryHealth = useCallback(async () => {
    if (isAgentMode) {
      setMemoryHealth(null)
      setMemoryHealthLoadError(null)
      setHealthLoading(false)
      return
    }
    setHealthLoading(true)
    setMemoryHealthLoadError(null)
    try {
      const health = await getTransport().call<MemoryHealth>("memory_health")
      setMemoryHealth(health ?? null)
      setMemoryHealthLoadError(null)
    } catch (e) {
      logger.warn("settings", "MemoryOverviewView::memoryHealth", "Failed to load memory health", e)
      setMemoryHealth(null)
      setMemoryHealthLoadError(memoryRepairOperationErrorToast("loadHealth", t, e))
    } finally {
      setHealthLoading(false)
    }
  }, [isAgentMode, t])

  const rebuildMemoryFts = useCallback(async () => {
    if (repairingFts) return
    setRepairingFts(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "rebuild_fts",
      })
      setMemoryHealth(report.after)
      toast.success(
        report.after.ftsMissingRows === 0
          ? t("settings.memoryRepairFtsDone", "Keyword index rebuilt")
          : t("settings.memoryRepairFtsPartial", "Keyword index rebuilt with remaining gaps"),
      )
      void reloadMemories()
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::rebuildMemoryFts", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("rebuildFts", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingFts(false)
    }
  }, [reloadMemories, repairingFts, t])

  const repairClaimGraph = useCallback(async () => {
    if (repairingClaimGraph) return
    setRepairingClaimGraph(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "repair_claim_graph",
      })
      setMemoryHealth(report.after)
      const remaining = report.after.orphanEvidenceRows + report.after.orphanClaimLinks
      toast.success(
        remaining === 0
          ? t("settings.memoryRepairClaimGraphDone", "Claim graph links repaired")
          : t(
              "settings.memoryRepairClaimGraphPartial",
              "Claim graph repair finished with remaining gaps",
            ),
      )
      void reloadMemories()
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::repairClaimGraph", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("repairClaimGraph", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingClaimGraph(false)
    }
  }, [reloadMemories, repairingClaimGraph, t])

  const repairExperienceGraph = useCallback(async () => {
    if (repairingExperienceGraph) return
    setRepairingExperienceGraph(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "repair_experience_graph",
      })
      setMemoryHealth(report.after)
      toast.success(
        report.after.orphanProcedureEpisodeRefs === 0
          ? t("settings.memoryRepairExperienceGraphDone", "Experience links repaired")
          : t(
              "settings.memoryRepairExperienceGraphPartial",
              "Experience repair finished with remaining gaps",
            ),
      )
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::repairExperienceGraph", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("repairExperienceGraph", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingExperienceGraph(false)
    }
  }, [repairingExperienceGraph, t])

  const rebuildClaimFts = useCallback(async () => {
    if (repairingClaimFts) return
    setRepairingClaimFts(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "rebuild_claim_fts",
      })
      setMemoryHealth(report.after)
      toast.success(
        report.after.claimFtsMissingRows === 0 && (report.after.evidenceFtsMissingRows ?? 0) === 0
          ? t("settings.memoryRepairClaimFtsDone", "Structured memory index rebuilt")
          : t(
              "settings.memoryRepairClaimFtsPartial",
              "Structured memory index rebuilt with remaining gaps",
            ),
      )
      void reloadMemories()
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::rebuildClaimFts", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("rebuildClaimFts", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingClaimFts(false)
    }
  }, [reloadMemories, repairingClaimFts, t])

  const recoverDreamingState = useCallback(async () => {
    if (repairingDreamingState) return
    setRepairingDreamingState(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "recover_dreaming_state",
      })
      setMemoryHealth(report.after)
      toast.success(
        report.after.dreamingStaleRuns === 0 && report.after.dreamingStaleLocks === 0
          ? t("settings.memoryRepairDreamingStateDone", "Dreaming maintenance state recovered")
          : t(
              "settings.memoryRepairDreamingStatePartial",
              "Dreaming recovery finished with remaining stale state",
            ),
      )
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::recoverDreamingState", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("recoverDreamingState", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingDreamingState(false)
    }
  }, [repairingDreamingState, t])

  const currentDbSnapshotPath = lastDbSnapshotPath ?? memoryHealth?.latestDbSnapshot?.path ?? null
  const currentDbSnapshotFiles = lastDbSnapshotPath
    ? lastDbSnapshotFiles
    : (memoryHealth?.latestDbSnapshot?.files ?? [])
  const currentDbSnapshotStatus = lastDbSnapshotPath
    ? "ok"
    : (memoryHealth?.latestDbSnapshot?.status ?? "ok")
  const currentDbSnapshotIssues = lastDbSnapshotPath
    ? []
    : (memoryHealth?.latestDbSnapshot?.issues ?? [])

  useEffect(() => {
    setDbSnapshotRestorePreview((previous) =>
      previous?.snapshotPath === currentDbSnapshotPath ? previous : null,
    )
  }, [currentDbSnapshotPath])

  const createDbSnapshot = useCallback(async () => {
    if (repairingDbSnapshot) return
    setRepairingDbSnapshot(true)
    try {
      const report = await getTransport().call<MemoryRepairReport>("memory_repair", {
        action: "create_db_snapshot",
      })
      setMemoryHealth(report.after)
      setLastDbSnapshotPath(report.artifactPath ?? null)
      setLastDbSnapshotFiles(report.artifactFiles ?? [])
      setDbSnapshotRestorePreview(null)
      toast.success(t("settings.memoryRepairDbSnapshotDone", "Database snapshot created"), {
        description: report.artifactPath ?? undefined,
      })
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::createDbSnapshot", "Repair failed", e)
      const failureToast = memoryRepairOperationErrorToast("createDbSnapshot", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRepairingDbSnapshot(false)
    }
  }, [repairingDbSnapshot, t])

  const checkDbSnapshotRestorePreview = useCallback(async () => {
    if (!currentDbSnapshotPath || checkingDbSnapshotRestore) return
    setCheckingDbSnapshotRestore(true)
    try {
      const preview = await getTransport().call<MemoryDbSnapshotRestorePreview>(
        "memory_db_snapshot_restore_preview",
        { snapshotPath: currentDbSnapshotPath },
      )
      setDbSnapshotRestorePreview(preview)
      toast.success(
        preview.canRestore
          ? t(
              "settings.memoryRepairDbSnapshotRestorePreviewReady",
              "Snapshot verified for recovery planning",
            )
          : t(
              "settings.memoryRepairDbSnapshotRestorePreviewBlocked",
              "Snapshot is not safe to restore",
            ),
      )
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::checkDbSnapshotRestorePreview",
        "Snapshot restore preview failed",
        e,
      )
      const failureToast = memoryRepairOperationErrorToast("restorePreview", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setCheckingDbSnapshotRestore(false)
    }
  }, [checkingDbSnapshotRestore, currentDbSnapshotPath, t])

  const copyDbSnapshotPath = useCallback(async () => {
    if (!currentDbSnapshotPath) return
    try {
      await navigator.clipboard.writeText(currentDbSnapshotPath)
      toast.success(t("settings.memoryRepairDbSnapshotPathCopied", "Snapshot path copied"))
    } catch (e) {
      logger.warn("settings", "MemoryOverviewView::copyDbSnapshotPath", "Clipboard write failed", e)
      const failureToast = memoryRepairOperationErrorToast("copyDbSnapshotPath", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [currentDbSnapshotPath, t])

  const copyDbSnapshotVerification = useCallback(async () => {
    if (!currentDbSnapshotPath) return
    try {
      await navigator.clipboard.writeText(
        formatMemorySnapshotArtifactDiagnostics(currentDbSnapshotPath, currentDbSnapshotFiles),
      )
      toast.success(
        t("settings.memoryRepairDbSnapshotVerificationCopied", "Snapshot verification copied"),
      )
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::copyDbSnapshotVerification",
        "Clipboard write failed",
        e,
      )
      const failureToast = memoryRepairOperationErrorToast("copyDbSnapshotVerification", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [currentDbSnapshotFiles, currentDbSnapshotPath, t])

  const copyDbSnapshotRestorePreview = useCallback(async () => {
    if (
      !dbSnapshotRestorePreview ||
      dbSnapshotRestorePreview.snapshotPath !== currentDbSnapshotPath
    ) {
      return
    }
    try {
      await navigator.clipboard.writeText(
        formatMemoryDbSnapshotRestorePreviewDiagnostics(dbSnapshotRestorePreview),
      )
      toast.success(
        t(
          "settings.memoryRepairDbSnapshotRestorePreviewCopied",
          "Restore preflight report copied",
        ),
      )
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::copyDbSnapshotRestorePreview",
        "Clipboard write failed",
        e,
      )
      const failureToast = memoryRepairOperationErrorToast("copyRestorePreview", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [currentDbSnapshotPath, dbSnapshotRestorePreview, t])

  const restoreDbSnapshot = useCallback(async () => {
    if (
      restoringDbSnapshot ||
      !dbSnapshotRestorePreview ||
      dbSnapshotRestorePreview.snapshotPath !== currentDbSnapshotPath
    ) {
      return
    }
    if (!dbSnapshotRestorePreview.canRestore) {
      toast.error(
        t(
          "settings.memoryRepairDbSnapshotRestoreNotReady",
          "Run a passing restore preflight before restoring this snapshot.",
        ),
      )
      return
    }
    const expected = "RESTORE"
    const entered = window.prompt(
      t("settings.memoryRepairDbSnapshotRestoreConfirmPrompt", {
        defaultValue:
          "This will restore memory.db from the verified snapshot and create a rollback snapshot first. Type {{word}} to continue.",
        word: expected,
      }),
    )
    if (entered !== expected) return
    setRestoringDbSnapshot(true)
    try {
      const report = await getTransport().call<MemoryDbSnapshotRestoreReport>(
        "memory_db_snapshot_restore",
        { snapshotPath: dbSnapshotRestorePreview.snapshotPath },
      )
      setMemoryHealth(report.after)
      setLastDbSnapshotPath(report.rollbackSnapshotPath)
      setLastDbSnapshotFiles(report.rollbackSnapshotFiles ?? [])
      setDbSnapshotRestorePreview(null)
      toast.success(
        t("settings.memoryRepairDbSnapshotRestoreDone", "Database snapshot restored"),
        {
          description: t("settings.memoryRepairDbSnapshotRestoreRollback", {
            defaultValue: "Rollback snapshot: {{path}}",
            path: report.rollbackSnapshotPath,
          }),
        },
      )
      void reloadMemories()
    } catch (e) {
      logger.error(
        "settings",
        "MemoryOverviewView::restoreDbSnapshot",
        "Snapshot restore failed",
        e,
      )
      const failureToast = memoryRepairOperationErrorToast("restoreDbSnapshot", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setRestoringDbSnapshot(false)
    }
  }, [
    currentDbSnapshotPath,
    dbSnapshotRestorePreview,
    reloadMemories,
    restoringDbSnapshot,
    t,
  ])

  const copyMemoryHealthDiagnostics = useCallback(async () => {
    if (!memoryHealth) return
    try {
      await navigator.clipboard.writeText(formatMemoryHealthDiagnostics(memoryHealth))
      toast.success(t("settings.memoryHealthCopyDone", "Memory health report copied"))
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::copyMemoryHealthDiagnostics",
        "Clipboard write failed",
        e,
      )
      const failureToast = memoryRepairOperationErrorToast("copyHealthDiagnostics", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [memoryHealth, t])

  const loadPendingClaims = useCallback(async () => {
    if (!canReviewClaims) {
      setPendingClaims([])
      setPendingClaimsError(null)
      setPendingLoading(false)
      return
    }
    setPendingLoading(true)
    setPendingClaimsError(null)
    try {
      const list = await getTransport().call<ClaimRecord[]>("claim_list", {
        status: "needs_review",
        limit: 100,
      })
      setPendingClaims(list ?? [])
    } catch (e) {
      logger.error(
        "settings",
        "MemoryOverviewView::pendingClaims",
        "Failed to list pending claims",
        e,
      )
      setPendingClaimsError(memoryOverviewPendingClaimsErrorToast(t, e))
    } finally {
      setPendingLoading(false)
    }
  }, [canReviewClaims, t])

  const loadRecentActivity = useCallback(async () => {
    if (isAgentMode) {
      setRecentMemories([])
      setRecentMemoryEvents([])
      setRecentCorrections([])
      setRecentCorrectionDecisions([])
      setRecentDreamingRuns([])
      setRecentEpisodes([])
      setRecentProcedures([])
      setRecentExperienceEvents([])
      setExperienceEpisodeTotal(0)
      setExperienceProcedureTotal(0)
      setActivitySummary({ learned7d: 0, rejected7d: 0 })
      setActivityLoading(false)
      setActivityLoadIssues([])
      return
    }
    const refreshExperienceLists =
      experienceAppliedQuery.trim().length === 0 &&
      experienceStatus === EXPERIENCE_DEFAULT_STATUS &&
      experienceSort === EXPERIENCE_DEFAULT_SORT &&
      experienceScopeFilter.kind === "all"
    const loadIssues: MemoryOverviewLoadIssue[] = []
    const recordLoadIssue = (source: MemoryOverviewLoadIssue["source"], error: unknown) => {
      loadIssues.push(memoryOverviewLoadIssue(source, error))
    }
    setActivityLoading(true)
    setActivityLoadIssues([])
    try {
      const memoryHistoryPromise = getTransport()
        .call<MemoryHistoryRecord[]>("memory_history", {
          limit: 20,
          offset: 0,
        })
        .catch((e) => {
          logger.warn(
            "settings",
            "MemoryOverviewView::memoryHistory",
            "Failed to list memory history",
            e,
          )
          recordLoadIssue("history", e)
          return []
        })

      const memoryPromise = getTransport()
        .call<MemoryEntry[]>("memory_list", {
          limit: 100,
          offset: 0,
        })
        .catch((e) => {
          logger.warn(
            "settings",
            "MemoryOverviewView::recentMemories",
            "Failed to list recent memories",
            e,
          )
          recordLoadIssue("memories", e)
          return []
        })

      const correctionPromise = canReviewClaims
        ? Promise.all(
            RECENT_CORRECTION_STATUSES.map((status) =>
              getTransport()
                .call<ClaimRecord[]>("claim_list", {
                  status,
                  limit: 3,
                })
                .catch((e) => {
                  logger.warn(
                    "settings",
                    "MemoryOverviewView::recentCorrections",
                    "Failed to list recent correction claims",
                    e,
                  )
                  recordLoadIssue("corrections", e)
                  return []
                }),
            ),
          ).then((groups) =>
            groups
              .flat()
              .sort((a, b) => timeValue(b.updatedAt) - timeValue(a.updatedAt))
              .slice(0, 4),
          )
        : Promise.resolve([])

      const dreamingRunsPromise = getTransport()
        .call<DreamingRunRecord[]>("dreaming_list_runs", {
          limit: 10,
          offset: 0,
        })
        .catch((e) => {
          logger.warn(
            "settings",
            "MemoryOverviewView::dreamingRuns",
            "Failed to list Dreaming runs",
            e,
          )
          recordLoadIssue("dreamingRuns", e)
          return []
        })

      const correctionDecisionsPromise = canReviewClaims
        ? dreamingRunsPromise
            .then((runs) =>
              Promise.all(
                (runs ?? [])
                  .filter((run) => run.decisionCount > 0)
                  .slice(0, 10)
                  .map((run) =>
                    getTransport()
                      .call<DreamingRunDetail | null>("dreaming_get_run", { id: run.id })
                      .catch((e) => {
                        logger.warn(
                          "settings",
                          "MemoryOverviewView::dreamingRunDetail",
                          "Failed to load Dreaming run detail",
                          e,
                        )
                        recordLoadIssue("dreamingRunDetail", e)
                        return null
                      }),
                  ),
              ),
            )
            .then((details) =>
              details
                .flatMap((detail) =>
                  detail
                    ? detail.decisions.filter(isCorrectionDecision).map((decision) => ({
                        id: decision.id,
                        decisionType: decision.decisionType,
                        targetType: decision.targetType,
                        targetId: decision.targetId ?? null,
                        trigger: detail.run.trigger,
                        phase: detail.run.phase,
                        status: detail.run.status,
                        rationale: decision.rationale,
                        content: decisionContent(decision),
                        createdAt: decision.createdAt,
                      }))
                    : [],
                )
                .sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))
                .slice(0, 4),
            )
        : Promise.resolve([])

      const episodesPromise = refreshExperienceLists
        ? getTransport()
            .call<MemoryEpisodeListPage>("memory_episode_list_page", {
              query: {
                status: experienceStatusParam(EXPERIENCE_DEFAULT_STATUS),
                sort: EXPERIENCE_DEFAULT_SORT,
                scope: null,
                limit: EXPERIENCE_PAGE_SIZE,
                offset: 0,
              },
            })
            .catch((e) => {
              logger.warn(
                "settings",
                "MemoryOverviewView::episodes",
                "Failed to list memory episodes",
                e,
              )
              recordLoadIssue("episodes", e)
              return { items: [], total: 0 }
            })
        : Promise.resolve({ items: [], total: 0 })

      const proceduresPromise = refreshExperienceLists
        ? getTransport()
            .call<MemoryProcedureListPage>("memory_procedure_list_page", {
              query: {
                status: experienceStatusParam(EXPERIENCE_DEFAULT_STATUS),
                sort: EXPERIENCE_DEFAULT_SORT,
                scope: null,
                limit: EXPERIENCE_PAGE_SIZE,
                offset: 0,
              },
            })
            .catch((e) => {
              logger.warn(
                "settings",
                "MemoryOverviewView::procedures",
                "Failed to list memory procedures",
                e,
              )
              recordLoadIssue("procedures", e)
              return { items: [], total: 0 }
            })
        : Promise.resolve({ items: [], total: 0 })

      const experienceHistoryPromise = getTransport()
        .call<MemoryExperienceHistoryListPage>("memory_experience_history_page", {
          query: {
            limit: 8,
            offset: 0,
          },
        })
        .catch((e) => {
          logger.warn(
            "settings",
            "MemoryOverviewView::recentExperienceHistory",
            "Failed to list recent experience history",
            e,
          )
          recordLoadIssue("experienceHistory", e)
          return { items: [], total: 0 }
        })

      const [
        history,
        memories,
        corrections,
        dreamingRuns,
        correctionDecisions,
        episodesPage,
        proceduresPage,
        experienceHistoryPage,
      ] = await Promise.all([
        memoryHistoryPromise,
        memoryPromise,
        correctionPromise,
        dreamingRunsPromise,
        correctionDecisionsPromise,
        episodesPromise,
        proceduresPromise,
        experienceHistoryPromise,
      ])
      const allHistory = (history ?? []).sort(
        (a, b) => timeValue(b.createdAt) - timeValue(a.createdAt),
      )
      const allMemories = (memories ?? []).sort(
        (a, b) => timeValue(b.updatedAt) - timeValue(a.updatedAt),
      )
      const allCorrections = corrections ?? []
      const allCorrectionDecisions = correctionDecisions ?? []
      const allExperienceHistory = (experienceHistoryPage.items ?? []).sort(
        (a, b) => timeValue(b.createdAt) - timeValue(a.createdAt),
      )
      setRecentMemoryEvents(allHistory.slice(0, 4))
      setRecentMemories(allMemories.slice(0, 4))
      setRecentCorrections(allCorrections)
      setRecentCorrectionDecisions(allCorrectionDecisions)
      setRecentDreamingRuns((dreamingRuns ?? []).slice(0, 3))
      setRecentExperienceEvents(allExperienceHistory.slice(0, 8))
      if (refreshExperienceLists) {
        const episodeItems = episodesPage.items ?? []
        const procedureItems = proceduresPage.items ?? []
        setRecentEpisodes(episodeItems.slice(0, EXPERIENCE_PAGE_SIZE))
        setRecentProcedures(procedureItems.slice(0, EXPERIENCE_PAGE_SIZE))
        setExperienceEpisodeTotal(Math.max(episodesPage.total ?? 0, episodeItems.length))
        setExperienceProcedureTotal(Math.max(proceduresPage.total ?? 0, procedureItems.length))
      }
      const learnedEvents = allHistory.filter((event) =>
        ["add", "import", "update"].includes(event.action),
      )
      const learnedExperienceEvents = allExperienceHistory.filter((event) =>
        ["add", "promote", "update", "restore_import"].includes(event.action),
      )
      const learnedMemoryCount =
        allHistory.length > 0
          ? learnedEvents.filter((event) => isWithinActivityWindow(event.createdAt)).length
          : allMemories.filter((memory) =>
              isWithinActivityWindow(memory.createdAt || memory.updatedAt),
            ).length
      setActivitySummary({
        learned7d:
          learnedMemoryCount +
          learnedExperienceEvents.filter((event) => isWithinActivityWindow(event.createdAt))
            .length,
        rejected7d:
          allCorrectionDecisions.length > 0
            ? allCorrectionDecisions.filter((item) => isWithinActivityWindow(item.createdAt)).length
            : allCorrections.filter((claim) => isWithinActivityWindow(claim.updatedAt)).length,
      })
      setActivityLoadIssues(loadIssues)
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::recentActivity",
        "Failed to load recent memory activity",
        e,
      )
      setActivityLoadIssues([memoryOverviewLoadIssue("recentActivity", e)])
    } finally {
      setActivityLoading(false)
    }
  }, [
    canReviewClaims,
    experienceAppliedQuery,
    experienceScopeFilter.kind,
    experienceSort,
    experienceStatus,
    isAgentMode,
  ])

  const loadExperienceLists = useCallback(
    async (
      query: string,
      options?: {
        status?: ExperienceStatusFilter
        sort?: ExperienceSort
        scopeFilter?: ExperienceScopeFilterDraft
      },
    ) => {
      if (isAgentMode) return
      const trimmed = query.trim()
      const status = options?.status ?? experienceStatus
      const sort = options?.sort ?? experienceSort
      const scopeFilter = options?.scopeFilter ?? experienceScopeFilter
      const scope = scopeFromExperienceFilter(scopeFilter)
      setExperienceSearchLoading(true)
      try {
        const [episodesPage, proceduresPage] = await Promise.all([
          getTransport().call<MemoryEpisodeListPage>("memory_episode_list_page", {
            query: {
              scope,
              status: experienceStatusParam(status),
              query: trimmed || null,
              sort,
              limit: EXPERIENCE_PAGE_SIZE,
              offset: 0,
            },
          }),
          getTransport().call<MemoryProcedureListPage>("memory_procedure_list_page", {
            query: {
              scope,
              status: experienceStatusParam(status),
              query: trimmed || null,
              sort,
              limit: EXPERIENCE_PAGE_SIZE,
              offset: 0,
            },
          }),
        ])
        const episodeItems = episodesPage.items ?? []
        const procedureItems = proceduresPage.items ?? []
        setRecentEpisodes(episodeItems.slice(0, EXPERIENCE_PAGE_SIZE))
        setRecentProcedures(procedureItems.slice(0, EXPERIENCE_PAGE_SIZE))
        setExperienceEpisodeTotal(Math.max(episodesPage.total ?? 0, episodeItems.length))
        setExperienceProcedureTotal(Math.max(proceduresPage.total ?? 0, procedureItems.length))
      } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::experienceSearch",
        "Failed to search experience memory",
        e,
      )
        const failureToast = memoryExperienceOperationErrorToast("search", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setExperienceSearchLoading(false)
      }
    },
    [experienceScopeFilter, experienceSort, experienceStatus, isAgentMode, t],
  )

  const applyExperienceView = useCallback(
    async (next?: {
      status?: ExperienceStatusFilter
      sort?: ExperienceSort
      scopeFilter?: ExperienceScopeFilterDraft
    }) => {
      const status = next?.status ?? experienceStatus
      const sort = next?.sort ?? experienceSort
      const scopeFilter = next?.scopeFilter ?? experienceScopeFilter
      const trimmed = experienceQuery.trim()
      setExperienceStatus(status)
      setExperienceSort(sort)
      setExperienceScopeFilter(scopeFilter)
      setExperienceAppliedQuery(trimmed)
      await loadExperienceLists(trimmed, { status, sort, scopeFilter })
    },
    [experienceQuery, experienceScopeFilter, experienceSort, experienceStatus, loadExperienceLists],
  )

  const runExperienceSearch = useCallback(async () => {
    await applyExperienceView()
  }, [applyExperienceView])

  const clearExperienceSearch = useCallback(async () => {
    setExperienceQuery("")
    setExperienceAppliedQuery("")
    setExperienceStatus(EXPERIENCE_DEFAULT_STATUS)
    setExperienceSort(EXPERIENCE_DEFAULT_SORT)
    setExperienceScopeFilter(EMPTY_EXPERIENCE_SCOPE_FILTER)
    await loadExperienceLists("", {
      status: EXPERIENCE_DEFAULT_STATUS,
      sort: EXPERIENCE_DEFAULT_SORT,
      scopeFilter: EMPTY_EXPERIENCE_SCOPE_FILTER,
    })
  }, [loadExperienceLists])

  const loadMoreExperience = useCallback(
    async (kind: "episode" | "procedure") => {
      if (isAgentMode || experienceLoadingMore) return
      const trimmed = experienceAppliedQuery.trim()
      const scope = scopeFromExperienceFilter(experienceScopeFilter)
      setExperienceLoadingMore(kind)
      try {
        if (kind === "episode") {
          const page = await getTransport().call<MemoryEpisodeListPage>("memory_episode_list_page", {
            query: {
              scope,
              status: experienceStatusParam(experienceStatus),
              query: trimmed || null,
              sort: experienceSort,
              limit: EXPERIENCE_PAGE_SIZE,
              offset: recentEpisodesRef.current.length,
            },
          })
          const items = page.items ?? []
          setRecentEpisodes((prev) => {
            const seen = new Set(prev.map((item) => item.id))
            return [...prev, ...items.filter((item) => !seen.has(item.id))]
          })
          setExperienceEpisodeTotal(
            Math.max(page.total ?? 0, recentEpisodesRef.current.length + items.length),
          )
        } else {
          const page = await getTransport().call<MemoryProcedureListPage>(
            "memory_procedure_list_page",
            {
              query: {
                scope,
                status: experienceStatusParam(experienceStatus),
                query: trimmed || null,
                sort: experienceSort,
                limit: EXPERIENCE_PAGE_SIZE,
                offset: recentProceduresRef.current.length,
              },
            },
          )
          const items = page.items ?? []
          setRecentProcedures((prev) => {
            const seen = new Set(prev.map((item) => item.id))
            return [...prev, ...items.filter((item) => !seen.has(item.id))]
          })
          setExperienceProcedureTotal(
            Math.max(page.total ?? 0, recentProceduresRef.current.length + items.length),
          )
        }
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::experienceLoadMore",
          "Failed to load more experience memory",
          e,
        )
        const failureToast = memoryExperienceOperationErrorToast("loadMore", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setExperienceLoadingMore(null)
      }
    },
    [
      experienceAppliedQuery,
      experienceLoadingMore,
      experienceScopeFilter,
      experienceSort,
      experienceStatus,
      isAgentMode,
      t,
    ],
  )

  const refreshExperienceAfterChange = useCallback(async () => {
    const trimmed = experienceAppliedQuery.trim()
    if (
      trimmed ||
      experienceScopeFilter.kind !== "all" ||
      experienceStatus !== EXPERIENCE_DEFAULT_STATUS ||
      experienceSort !== EXPERIENCE_DEFAULT_SORT
    ) {
      await loadExperienceLists(trimmed, {
        status: experienceStatus,
        sort: experienceSort,
        scopeFilter: experienceScopeFilter,
      })
    } else {
      await loadRecentActivity()
    }
  }, [
    experienceAppliedQuery,
    experienceScopeFilter,
    experienceSort,
    experienceStatus,
    loadExperienceLists,
    loadRecentActivity,
  ])

  const resetEpisodeDraft = useCallback(() => {
    setEpisodeDraft(EMPTY_EPISODE_DRAFT)
    setEpisodeScopeDraft(EMPTY_EXPERIENCE_SCOPE_DRAFT)
    setEpisodeEditingId(null)
  }, [])

  const resetProcedureDraft = useCallback(() => {
    setProcedureDraft(EMPTY_PROCEDURE_DRAFT)
    setProcedureScopeDraft(EMPTY_EXPERIENCE_SCOPE_DRAFT)
    setProcedureEditingId(null)
  }, [])

  const submitEpisodeDraft = useCallback(async () => {
    const title = episodeDraft.title.trim()
    const situation = episodeDraft.situation.trim()
    if (!title || !situation) {
      toast.error(
        t("settings.memoryEpisodeRequired", "Add a title and situation before saving."),
      )
      return
    }
    const scope = scopeFromExperienceDraft(episodeScopeDraft)
    if (!scope) {
      toast.error(
        t("settings.memoryExperienceScopeRequired", "Choose a scope target before saving."),
      )
      return
    }
    setEpisodeSaving(true)
    try {
      if (episodeEditingId) {
        const patch: MemoryEpisodePatch = {
          scope,
          title,
          situation,
          actions: lines(episodeDraft.actions),
          outcome: episodeDraft.outcome.trim(),
          lesson: episodeDraft.lesson.trim(),
          tags: commaList(episodeDraft.tags),
        }
        const updated = await getTransport().call<MemoryEpisodeRecord | null>("memory_episode_update", {
          id: episodeEditingId,
          patch,
        })
        if (!updated) {
          throw new Error(`episode not found: ${episodeEditingId}`)
        }
        toast.success(t("settings.memoryEpisodeUpdated", "Episode updated"))
      } else {
        const episode: NewMemoryEpisode = {
          scope,
          title,
          situation,
          actions: lines(episodeDraft.actions),
          outcome: episodeDraft.outcome.trim(),
          lesson: episodeDraft.lesson.trim(),
          tags: commaList(episodeDraft.tags),
          successScore: 0.8,
        }
        await getTransport().call<MemoryEpisodeRecord>("memory_episode_add", { episode })
        toast.success(t("settings.memoryEpisodeSaved", "Episode saved"))
      }
      setEpisodeDialogOpen(false)
      resetEpisodeDraft()
      await refreshExperienceAfterChange()
    } catch (e) {
      logger.error("settings", "MemoryOverviewView::episodeAdd", "Failed to save episode", e)
      const failureToast = memoryExperienceOperationErrorToast("saveEpisode", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setEpisodeSaving(false)
    }
  }, [
    episodeDraft,
    episodeEditingId,
    episodeScopeDraft,
    refreshExperienceAfterChange,
    resetEpisodeDraft,
    t,
  ])

  const submitProcedureDraft = useCallback(async () => {
    const title = procedureDraft.title.trim()
    const trigger = procedureDraft.trigger.trim()
    const stepsMarkdown = procedureDraft.stepsMarkdown.trim()
    if (!title || !trigger || !stepsMarkdown) {
      toast.error(
        t(
          "settings.memoryProcedureRequired",
          "Add a title, trigger, and steps before saving.",
        ),
      )
      return
    }
    const scope = scopeFromExperienceDraft(procedureScopeDraft)
    if (!scope) {
      toast.error(
        t("settings.memoryExperienceScopeRequired", "Choose a scope target before saving."),
      )
      return
    }

    let confidence: number | null = null
    const rawConfidence = procedureDraft.confidencePercent.trim()
    if (rawConfidence) {
      const parsed = Number(rawConfidence)
      if (!Number.isFinite(parsed)) {
        toast.error(
          t("settings.memoryProcedureConfidenceInvalid", "Confidence must be a number."),
        )
        return
      }
      confidence = Math.min(100, Math.max(0, parsed)) / 100
    }

    setProcedureSaving(true)
    try {
      if (procedureEditingId) {
        const patch: MemoryProcedurePatch = {
          scope,
          title,
          trigger,
          stepsMarkdown,
          constraintsMarkdown: procedureDraft.constraintsMarkdown.trim(),
          confidence,
          tags: commaList(procedureDraft.tags),
        }
        const updated = await getTransport().call<MemoryProcedureRecord | null>(
          "memory_procedure_update",
          {
            id: procedureEditingId,
            patch,
          },
        )
        if (!updated) {
          throw new Error(`procedure not found: ${procedureEditingId}`)
        }
        toast.success(t("settings.memoryProcedureUpdated", "Procedure updated"))
      } else {
        const procedure: NewMemoryProcedure = {
          scope,
          title,
          trigger,
          stepsMarkdown,
          constraintsMarkdown: procedureDraft.constraintsMarkdown.trim(),
          confidence,
          sourceEpisodeIds: [],
          tags: commaList(procedureDraft.tags),
        }
        await getTransport().call<MemoryProcedureRecord>("memory_procedure_add", { procedure })
        toast.success(t("settings.memoryProcedureSaved", "Procedure saved"))
      }
      setProcedureDialogOpen(false)
      resetProcedureDraft()
      await refreshExperienceAfterChange()
    } catch (e) {
      logger.error(
        "settings",
        "MemoryOverviewView::procedureAdd",
        "Failed to save procedure",
        e,
      )
      const failureToast = memoryExperienceOperationErrorToast("saveProcedure", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setProcedureSaving(false)
    }
  }, [
    procedureDraft,
    procedureEditingId,
    procedureScopeDraft,
    refreshExperienceAfterChange,
    resetProcedureDraft,
    t,
  ])

  const promoteEpisode = useCallback(
    async (id: string) => {
      try {
        await getTransport().call<MemoryProcedureRecord>("memory_episode_promote_procedure", {
          id,
          options: {},
        })
        toast.success(t("settings.memoryProcedureSaved", "Procedure saved"))
        await refreshExperienceAfterChange()
      } catch (e) {
        logger.error(
          "settings",
          "MemoryOverviewView::episodePromote",
          "Failed to promote episode",
          e,
        )
        const failureToast = memoryExperienceOperationErrorToast("promoteEpisode", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      }
    },
    [refreshExperienceAfterChange, t],
  )

  const loadExperienceHistory = useCallback(
    async (kind: "episode" | "procedure", id: string) => {
      if (isAgentMode) {
        setExperienceHistory([])
        setExperienceHistoryError(null)
        return
      }
      setExperienceHistoryLoading(true)
      setExperienceHistoryError(null)
      try {
        const page = await getTransport().call<MemoryExperienceHistoryListPage>(
          "memory_experience_history_page",
          {
            query: {
              targetKind: kind,
              targetId: id,
              limit: 8,
              offset: 0,
            },
          },
        )
        setExperienceHistory(page.items ?? [])
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::experienceHistory",
          "Failed to load experience history",
          e,
        )
        setExperienceHistoryError(memoryExperienceOperationErrorToast("loadHistory", t, e))
        setExperienceHistory([])
      } finally {
        setExperienceHistoryLoading(false)
      }
    },
    [isAgentMode, t],
  )

  const openExperienceDetail = useCallback(
    (detail: ExperienceDetail) => {
      setExperienceDetail(detail)
      setExperienceHistory([])
      setExperienceHistoryError(null)
      setExperienceFocusHighlight({ kind: detail.kind, id: detail.record.id })
      void loadExperienceHistory(detail.kind, detail.record.id)
    },
    [loadExperienceHistory],
  )

  const experienceHistoryActionLabel = useCallback(
    (action: MemoryExperienceHistoryRecord["action"]) => {
      switch (action) {
        case "promote":
          return t("settings.memoryExperienceHistoryPromote", "Promoted from episode")
        case "update":
          return t("settings.memoryExperienceHistoryUpdate", "Edited")
        case "archive":
          return t("settings.memoryExperienceHistoryArchive", "Archived")
        case "restore":
          return t("settings.memoryExperienceHistoryRestore", "Restored")
        case "restore_import":
          return t("settings.memoryExperienceHistoryRestoreImport", "Restored from backup")
        case "add":
        default:
          return t("settings.memoryExperienceHistoryAdd", "Created")
      }
    },
    [t],
  )

  const editExperienceDetail = useCallback(() => {
    if (!experienceDetail) return
    if (experienceDetail.kind === "episode") {
      const record = experienceDetail.record
      setEpisodeEditingId(record.id)
      setEpisodeDraft({
        title: record.title,
        situation: record.situation,
        actions: record.actions.join("\n"),
        outcome: record.outcome,
        lesson: record.lesson,
        tags: record.tags.join(", "),
      })
      setEpisodeScopeDraft(experienceDraftFromScope(record.scope))
      setExperienceDetail(null)
      setExperienceHistory([])
      setExperienceHistoryError(null)
      setEpisodeDialogOpen(true)
    } else {
      const record = experienceDetail.record
      setProcedureEditingId(record.id)
      setProcedureDraft({
        title: record.title,
        trigger: record.trigger,
        stepsMarkdown: record.stepsMarkdown,
        constraintsMarkdown: record.constraintsMarkdown,
        confidencePercent: String(Math.round(record.confidence * 100)),
        tags: record.tags.join(", "),
      })
      setProcedureScopeDraft(experienceDraftFromScope(record.scope))
      setExperienceDetail(null)
      setExperienceHistory([])
      setExperienceHistoryError(null)
      setProcedureDialogOpen(true)
    }
  }, [experienceDetail])

  const openSourceEpisode = useCallback(
    async (id: string) => {
      const cached = recentEpisodesRef.current.find((episode) => episode.id === id)
      if (cached) {
        openExperienceDetail({ kind: "episode", record: cached })
        return
      }
      setExperienceSourceOpening(id)
      try {
        const episode = await getTransport().call<MemoryEpisodeRecord | null>(
          "memory_episode_get",
          { id },
        )
        if (!episode) {
          toast.error(
            t(
              "settings.memorySourceEpisodeMissing",
              "Source episode is missing. Run memory health repair if this persists.",
            ),
          )
          return
        }
        setRecentEpisodes((prev) => [
          episode,
          ...prev.filter((item) => item.id !== episode.id),
        ].slice(0, EXPERIENCE_PAGE_SIZE))
        openExperienceDetail({ kind: "episode", record: episode })
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::sourceEpisode",
          "Failed to open source episode",
          e,
        )
        const failureToast = memoryExperienceOperationErrorToast("openSourceEpisode", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      } finally {
        setExperienceSourceOpening(null)
      }
    },
    [openExperienceDetail, t],
  )

  const openExperienceHistoryEvent = useCallback(
    async (event: MemoryExperienceHistoryRecord) => {
      if (event.targetKind === "episode") {
        const cached = recentEpisodesRef.current.find((episode) => episode.id === event.targetId)
        if (cached) {
          openExperienceDetail({ kind: "episode", record: cached })
          return
        }
      } else {
        const cached = recentProceduresRef.current.find(
          (procedure) => procedure.id === event.targetId,
        )
        if (cached) {
          openExperienceDetail({ kind: "procedure", record: cached })
          return
        }
      }

      const command =
        event.targetKind === "episode" ? "memory_episode_get" : "memory_procedure_get"
      try {
        const record = await getTransport().call<MemoryEpisodeRecord | MemoryProcedureRecord | null>(
          command,
          { id: event.targetId },
        )
        if (!record) {
          toast.error(
            t(
              "settings.memoryExperienceOpenMissing",
              "This experience memory no longer exists.",
            ),
          )
          return
        }
        if (event.targetKind === "episode") {
          const episode = record as MemoryEpisodeRecord
          setRecentEpisodes((prev) => [
            episode,
            ...prev.filter((item) => item.id !== episode.id),
          ].slice(0, EXPERIENCE_PAGE_SIZE))
          openExperienceDetail({ kind: "episode", record: episode })
        } else {
          const procedure = record as MemoryProcedureRecord
          setRecentProcedures((prev) => [
            procedure,
            ...prev.filter((item) => item.id !== procedure.id),
          ].slice(0, EXPERIENCE_PAGE_SIZE))
          openExperienceDetail({ kind: "procedure", record: procedure })
        }
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::openExperienceHistoryEvent",
          "Failed to open experience history event",
          e,
        )
        const failureToast = memoryExperienceOperationErrorToast("openExperience", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      }
    },
    [openExperienceDetail, t],
  )

  const changeExperienceDetailStatus = useCallback(async () => {
    if (!experienceDetail) return
    const { kind, record } = experienceDetail
    const restore = record.status === "archived"
    setExperienceStatusSaving(true)
    try {
      const command = restore
        ? kind === "episode"
          ? "memory_episode_restore"
          : "memory_procedure_restore"
        : kind === "episode"
          ? "memory_episode_archive"
          : "memory_procedure_archive"
      const changed = await getTransport().call<boolean>(command, { id: record.id })
      if (changed) {
        if (
          experienceFocusHighlight?.kind === kind &&
          experienceFocusHighlight.id === record.id
        ) {
          setExperienceFocusHighlight(null)
        }
        toast.success(
          restore
            ? t("settings.memoryExperienceRestored", "Experience restored")
            : t("settings.memoryExperienceArchived", "Experience archived"),
        )
        setExperienceDetail(null)
        setExperienceHistory([])
        await refreshExperienceAfterChange()
      }
    } catch (e) {
      logger.error(
        "settings",
        "MemoryOverviewView::experienceStatus",
        "Failed to change experience memory status",
        e,
      )
      const failureToast = memoryExperienceOperationErrorToast(
        restore ? "restoreExperience" : "archiveExperience",
        t,
        e,
      )
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setExperienceStatusSaving(false)
    }
  }, [experienceDetail, experienceFocusHighlight, refreshExperienceAfterChange, t])

  const updateMemoryAuditFocusUrl = useCallback(
    (next?: { open?: boolean; query?: string; action?: MemoryHistoryAction | "all" }) => {
      if (isAgentMode) return
      const query = next?.query ?? memoryAuditQuery
      const action = next?.action ?? memoryAuditAction
      const open = next?.open ?? (memoryAuditOpen || query.trim().length > 0 || action !== "all")
      setMemoryFocusUrl(
        {
          kind: "overview",
          auditOpen: open,
          auditAction: action,
          auditQuery: query.trim() || null,
        },
        true,
      )
    },
    [isAgentMode, memoryAuditAction, memoryAuditOpen, memoryAuditQuery],
  )

  const resetMemoryAuditSearchState = useCallback(() => {
    setMemoryAuditQuery("")
    setMemoryAuditAction("all")
    setMemoryAuditResults(recentMemoryEvents)
    setMemoryAuditExperienceResults(recentExperienceEvents)
    setMemoryAuditDecisionResults(recentCorrectionDecisions)
    setMemoryAuditHasMore(false)
    setMemoryAuditTotal(null)
    setMemoryAuditTotalTruncated(false)
    setMemoryAuditExperienceHasMore(false)
    setMemoryAuditExperienceTotal(null)
    setMemoryAuditExperienceTotalTruncated(false)
    setMemoryAuditDecisionHasMore(false)
    setMemoryAuditDecisionTotal(null)
    setMemoryAuditDecisionTotalTruncated(false)
    setMemoryAuditDegradedIssues([])
  }, [recentCorrectionDecisions, recentExperienceEvents, recentMemoryEvents])

  const runMemoryAuditSearch = useCallback(
    async (next?: { query?: string; action?: MemoryHistoryAction | "all"; append?: boolean }) => {
      if (isAgentMode) return
      const nextQuery = next?.query ?? memoryAuditQuery
      const nextAction = next?.action ?? memoryAuditAction
      const append = next?.append === true
      if (!append) {
        updateMemoryAuditFocusUrl({ open: true, query: nextQuery, action: nextAction })
      }
      const includeExperienceHistory = includeCrossSourceAudit(nextAction)
      const request = memoryAuditRequest(
        nextQuery,
        nextAction,
        MEMORY_AUDIT_PAGE_SIZE,
        append ? memoryAuditResults.length : 0,
      )
      const experienceRequest = {
        query: nextQuery.trim() || null,
        limit: MEMORY_AUDIT_PAGE_SIZE,
        offset: append ? memoryAuditExperienceResults.length : 0,
      }
      const decisionRequest = {
        query: nextQuery.trim() || null,
        targetType: "claim",
        limit: MEMORY_AUDIT_PAGE_SIZE,
        offset: append ? memoryAuditDecisionResults.length : 0,
      }

      setMemoryAuditLoading(true)
      const degradedIssues: MemoryAuditDegradedIssue[] = []
      const recordDegradedIssue = (source: MemoryAuditDegradedIssue["source"], error: unknown) => {
        degradedIssues.push(memoryAuditDegradedIssue(source, error))
      }
      if (!append) {
        setMemoryAuditDegradedIssues([])
      }
      try {
        const useUnifiedAuditPage = nextAction !== "all" || canReviewClaims
        if (useUnifiedAuditPage) {
          try {
            const unifiedOffset = append
              ? countMemoryAuditActivity({
                  action: nextAction,
                  memoryCount: memoryAuditResults.length,
                  experienceCount: memoryAuditExperienceResults.length,
                  decisionCount: memoryAuditDecisionResults.length,
                })
              : 0
            const unifiedResponse = await getTransport().call<MemoryAuditPageResponse>(
              "memory_audit_page",
              {
                query: nextQuery.trim() || null,
                action: nextAction,
                limit: MEMORY_AUDIT_PAGE_SIZE,
                offset: unifiedOffset,
              },
            )
            const buckets = splitMemoryAuditPage({
              items: unifiedResponse.items ?? [],
              mapDecision: recentCorrectionFromDecisionListItem,
            })
            const legacySummary = unifiedResponse.sources.legacyMemory
            const experienceSummary = unifiedResponse.sources.experience
            const decisionSummary = unifiedResponse.sources.claimDecision
            const legacyTotal = Math.max(
              legacySummary.total,
              (append ? memoryAuditResults.length : 0) + buckets.memory.length,
            )
            const experienceTotal = includeExperienceHistory
              ? Math.max(
                  experienceSummary.total,
                  (append ? memoryAuditExperienceResults.length : 0) + buckets.experience.length,
                )
              : 0
            const decisionTotal =
              includeExperienceHistory && canReviewClaims
                ? Math.max(
                    decisionSummary.total,
                    (append ? memoryAuditDecisionResults.length : 0) + buckets.decisions.length,
                  )
                : 0

            setMemoryAuditResults((prev) => {
              if (!append) return buckets.memory
              const seen = new Set(prev.map((event) => event.id))
              return [...prev, ...buckets.memory.filter((event) => !seen.has(event.id))]
            })
            setMemoryAuditExperienceResults((prev) => {
              if (!includeExperienceHistory) return []
              if (!append) return buckets.experience
              const seen = new Set(prev.map((event) => event.id))
              return [...prev, ...buckets.experience.filter((event) => !seen.has(event.id))]
            })
            setMemoryAuditDecisionResults((prev) => {
              if (!includeExperienceHistory || !canReviewClaims) return []
              if (!append) return buckets.decisions
              const seen = new Set(prev.map((event) => event.id))
              return [...prev, ...buckets.decisions.filter((event) => !seen.has(event.id))]
            })
            setMemoryAuditTotal(legacyTotal)
            setMemoryAuditTotalTruncated(legacySummary.totalTruncated === true)
            setMemoryAuditExperienceTotal(includeExperienceHistory ? experienceTotal : null)
            setMemoryAuditExperienceTotalTruncated(
              includeExperienceHistory && experienceSummary.totalTruncated === true,
            )
            setMemoryAuditDecisionTotal(
              includeExperienceHistory && canReviewClaims ? decisionTotal : null,
            )
            setMemoryAuditDecisionTotalTruncated(
              includeExperienceHistory &&
                canReviewClaims &&
                decisionSummary.totalTruncated === true,
            )
            setMemoryAuditExperienceHasMore(false)
            setMemoryAuditDecisionHasMore(false)
            setMemoryAuditHasMore(
              unifiedOffset + (unifiedResponse.items?.length ?? 0) < unifiedResponse.total ||
                (unifiedResponse.totalTruncated === true &&
                  (unifiedResponse.items?.length ?? 0) >= MEMORY_AUDIT_PAGE_SIZE),
            )
            return
          } catch (unifiedError) {
            logger.warn(
              "settings",
              "MemoryOverviewView::memoryAuditUnifiedSearch",
              "Failed to load unified memory audit page; falling back to source queries",
              unifiedError,
            )
            recordDegradedIssue("unified", unifiedError)
          }
        }

        let response: MemoryHistoryListResponse | null = null
        try {
          response = await getTransport().call<MemoryHistoryListResponse>("memory_history_page", {
            ...request,
          })
        } catch (pageError) {
          logger.warn(
            "settings",
            "MemoryOverviewView::memoryAuditSearchPage",
            "Failed to load memory history page; falling back to item query",
            pageError,
          )
          recordDegradedIssue("legacyPage", pageError)
          const legacyItems =
            (await getTransport().call<MemoryHistoryRecord[]>("memory_history", { ...request })) ??
            []
          response = {
            items: legacyItems,
            total: (append ? memoryAuditResults.length : 0) + legacyItems.length,
            totalTruncated: legacyItems.length >= MEMORY_AUDIT_PAGE_SIZE,
          }
        }
        let experienceResponse: MemoryExperienceHistoryListPage | null = null
        if (includeExperienceHistory) {
          try {
            experienceResponse = await getTransport().call<MemoryExperienceHistoryListPage>(
              "memory_experience_history_page",
              { query: experienceRequest },
            )
          } catch (experienceError) {
            logger.warn(
              "settings",
              "MemoryOverviewView::memoryAuditExperienceSearch",
              "Failed to search experience history",
              experienceError,
            )
            recordDegradedIssue("experience", experienceError)
            experienceResponse = {
              items: [],
              total: append ? memoryAuditExperienceResults.length : 0,
              totalTruncated: false,
            }
          }
        }
        let decisionResponse: DreamingDecisionListResponse | null = null
        if (includeExperienceHistory && canReviewClaims) {
          try {
            decisionResponse = await getTransport().call<DreamingDecisionListResponse>(
              "dreaming_list_decisions_page",
              decisionRequest,
            )
          } catch (decisionError) {
            logger.warn(
              "settings",
              "MemoryOverviewView::memoryAuditDecisionSearch",
              "Failed to search claim decision history",
              decisionError,
            )
            recordDegradedIssue("decisions", decisionError)
            decisionResponse = {
              items: [],
              total: append ? memoryAuditDecisionResults.length : 0,
              totalTruncated: false,
            }
          }
        }
        const results = response?.items ?? []
        const page = (results ?? []).sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))
        const offset = append ? memoryAuditResults.length : 0
        const total = Math.max(response?.total ?? 0, offset + page.length)
        const totalTruncated = response?.totalTruncated === true
        const experiencePage = (experienceResponse?.items ?? []).sort(
          (a, b) => timeValue(b.createdAt) - timeValue(a.createdAt),
        )
        const experienceOffset = append ? memoryAuditExperienceResults.length : 0
        const experienceTotal = includeExperienceHistory
          ? Math.max(experienceResponse?.total ?? 0, experienceOffset + experiencePage.length)
          : 0
        const experienceTotalTruncated =
          includeExperienceHistory && experienceResponse?.totalTruncated === true
        const decisionPage = (decisionResponse?.items ?? [])
          .map(recentCorrectionFromDecisionListItem)
          .sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))
        const decisionOffset = append ? memoryAuditDecisionResults.length : 0
        const decisionTotal =
          includeExperienceHistory && canReviewClaims
            ? Math.max(decisionResponse?.total ?? 0, decisionOffset + decisionPage.length)
            : 0
        const decisionTotalTruncated =
          includeExperienceHistory && canReviewClaims && decisionResponse?.totalTruncated === true
        setMemoryAuditResults((prev) => {
          if (!append) return page
          const seen = new Set(prev.map((event) => event.id))
          return [...prev, ...page.filter((event) => !seen.has(event.id))]
        })
        setMemoryAuditExperienceResults((prev) => {
          if (!includeExperienceHistory) return []
          if (!append) return experiencePage
          const seen = new Set(prev.map((event) => event.id))
          return [...prev, ...experiencePage.filter((event) => !seen.has(event.id))]
        })
        setMemoryAuditDecisionResults((prev) => {
          if (!includeExperienceHistory || !canReviewClaims) return []
          if (!append) return decisionPage
          const seen = new Set(prev.map((event) => event.id))
          return [...prev, ...decisionPage.filter((event) => !seen.has(event.id))]
        })
        setMemoryAuditTotal(total)
        setMemoryAuditTotalTruncated(totalTruncated)
        setMemoryAuditExperienceTotal(includeExperienceHistory ? experienceTotal : null)
        setMemoryAuditExperienceTotalTruncated(experienceTotalTruncated)
        setMemoryAuditDecisionTotal(
          includeExperienceHistory && canReviewClaims ? decisionTotal : null,
        )
        setMemoryAuditDecisionTotalTruncated(decisionTotalTruncated)
        setMemoryAuditDecisionHasMore(
          includeExperienceHistory &&
            canReviewClaims &&
            (decisionOffset + decisionPage.length < decisionTotal ||
              (decisionTotalTruncated && decisionPage.length >= MEMORY_AUDIT_PAGE_SIZE)),
        )
        setMemoryAuditExperienceHasMore(
          includeExperienceHistory &&
            (experienceOffset + experiencePage.length < experienceTotal ||
              (experienceTotalTruncated && experiencePage.length >= MEMORY_AUDIT_PAGE_SIZE)),
        )
        setMemoryAuditHasMore(
          offset + page.length < total || (totalTruncated && page.length >= MEMORY_AUDIT_PAGE_SIZE),
        )
        setMemoryAuditDegradedIssues((prev) =>
          append ? [...prev, ...degradedIssues] : degradedIssues,
        )
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::memoryAuditSearch",
          "Failed to search memory history",
          e,
        )
        const failureToast = memoryAuditOperationErrorToast(append ? "loadMore" : "search", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
        if (!append) {
          setMemoryAuditResults([])
          setMemoryAuditExperienceResults([])
          setMemoryAuditDecisionResults([])
          setMemoryAuditHasMore(false)
          setMemoryAuditTotal(null)
          setMemoryAuditTotalTruncated(false)
          setMemoryAuditExperienceHasMore(false)
          setMemoryAuditExperienceTotal(null)
          setMemoryAuditExperienceTotalTruncated(false)
          setMemoryAuditDecisionHasMore(false)
          setMemoryAuditDecisionTotal(null)
          setMemoryAuditDecisionTotalTruncated(false)
          setMemoryAuditDegradedIssues([])
        }
      } finally {
        setMemoryAuditLoading(false)
      }
    },
    [
      isAgentMode,
      canReviewClaims,
      memoryAuditAction,
      memoryAuditQuery,
      memoryAuditDecisionResults.length,
      memoryAuditExperienceResults.length,
      memoryAuditRequest,
      memoryAuditResults.length,
      t,
      updateMemoryAuditFocusUrl,
    ],
  )

  const clearMemoryAuditSearch = useCallback(() => {
    resetMemoryAuditSearchState()
    updateMemoryAuditFocusUrl({ open: true, query: "", action: "all" })
  }, [resetMemoryAuditSearchState, updateMemoryAuditFocusUrl])

  const currentMemoryAuditPresetId = memoryAuditPresetId(memoryAuditQuery, memoryAuditAction)
  const memoryAuditPresetLabel = useCallback(
    (preset: MemoryAuditFilterPreset) => {
      const parts = [memoryAuditActionLabel(preset.action)]
      if (preset.query) parts.push(`"${preset.query}"`)
      return parts.join(" / ")
    },
    [memoryAuditActionLabel],
  )
  const saveMemoryAuditPreset = useCallback(() => {
    const preset: MemoryAuditFilterPreset = {
      id: currentMemoryAuditPresetId,
      query: memoryAuditQuery.trim(),
      action: memoryAuditAction,
      updatedAt: Date.now(),
    }
    setMemoryAuditPresets((prev) => {
      const next = [preset, ...prev.filter((item) => item.id !== preset.id)].slice(
        0,
        MEMORY_AUDIT_PRESET_LIMIT,
      )
      persistMemoryAuditFilterPresets(next)
      return next
    })
    toast.success(t("settings.memoryFilterPresetSaved"))
  }, [currentMemoryAuditPresetId, memoryAuditAction, memoryAuditQuery, t])
  const applyMemoryAuditPreset = useCallback(
    (preset: MemoryAuditFilterPreset) => {
      setMemoryAuditOpen(true)
      setMemoryAuditQuery(preset.query)
      setMemoryAuditAction(preset.action)
      void runMemoryAuditSearch({ query: preset.query, action: preset.action })
    },
    [runMemoryAuditSearch],
  )
  const removeMemoryAuditPreset = useCallback((id: string) => {
    setMemoryAuditPresets((prev) => {
      const next = prev.filter((item) => item.id !== id)
      persistMemoryAuditFilterPresets(next)
      return next
    })
  }, [])

  const openMemoryHistoryEvent = useCallback(
    async (event: MemoryHistoryRecord) => {
      if (event.action === "delete") return
      try {
        const memory = await getTransport().call<MemoryEntry | null>("memory_get", {
          id: event.memoryId,
        })
        if (memory) {
          data.startEdit(memory)
          return
        }
      } catch (e) {
        logger.warn(
          "settings",
          "MemoryOverviewView::openMemoryHistoryEvent",
          "Failed to open memory from history",
          e,
        )
        const failureToast = memoryOverviewOpenMemoryErrorToast(t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      }
      onSelectTab("manage")
    },
    [data, onSelectTab, t],
  )

  const memoryEventActivityItem = useCallback(
    (event: MemoryHistoryRecord): RecentUnifiedActivityItem => ({
      kind: "memory_event",
      key: `memory-event:${event.id}`,
      createdAt: event.createdAt,
      title: event.contentPreview,
      subtitle: [
        formatActivityTime(event.createdAt),
        memoryAuditActionLabel(event.action),
        t(`settings.memoryType_${event.memoryType}`),
        memorySourceLabel(event.source),
      ],
      event,
      disabled: event.action === "delete",
    }),
    [memoryAuditActionLabel, memorySourceLabel, t],
  )

  const experienceEventActivityItem = useCallback(
    (event: MemoryExperienceHistoryRecord): RecentUnifiedActivityItem => {
      const isEpisode = event.targetKind === "episode"
      const kindLabel = isEpisode
        ? t("settings.memoryExperienceKindEpisode", "Episode")
        : t("settings.memoryExperienceKindProcedure", "Workflow")
      const title =
        event.titlePreview ||
        event.contentPreview ||
        `${kindLabel}:${event.targetId.slice(0, 8)}`
      return {
        kind: "experience_event",
        key: `experience-event:${event.id}`,
        createdAt: event.createdAt,
        title,
        subtitle: [
          formatActivityTime(event.createdAt),
          kindLabel,
          experienceHistoryActionLabel(event.action),
          memoryScopeLabel(event.scope),
        ],
        detail:
          event.contentPreview && event.contentPreview !== title ? event.contentPreview : null,
        event,
      }
    },
    [experienceHistoryActionLabel, memoryScopeLabel, t],
  )

  const decisionActivityItem = useCallback(
    (item: RecentCorrectionItem): RecentUnifiedActivityItem => ({
      kind: "decision",
      key: `decision:${item.id}`,
      createdAt: item.createdAt,
      title: item.content || item.rationale || item.targetId || item.id,
      subtitle: [
        formatActivityTime(item.createdAt),
        decisionTypeLabel(item.decisionType),
        t(`dashboard.dreaming.trigger.${item.trigger}`, item.trigger),
        item.phase,
      ].filter(Boolean),
      detail: item.rationale && item.rationale !== item.content ? item.rationale : null,
      decision: item,
    }),
    [decisionTypeLabel, t],
  )

  const recentUnifiedActivity = useMemo<RecentUnifiedActivityItem[]>(() => {
    const memoryItems: RecentUnifiedActivityItem[] =
      recentMemoryEvents.length > 0
        ? recentMemoryEvents.map(memoryEventActivityItem)
        : recentMemories.map((memory) => ({
            kind: "memory",
            key: `memory:${memory.id}`,
            createdAt: memory.updatedAt,
            title: memory.content,
            subtitle: [
              formatActivityTime(memory.updatedAt),
              t(`settings.memoryType_${memory.memoryType}`),
              memorySourceLabel(memory.source),
            ],
            memory,
          }))

    const decisionItems: RecentUnifiedActivityItem[] =
      recentCorrectionDecisions.length > 0
        ? recentCorrectionDecisions.map(decisionActivityItem)
        : recentCorrections.map((claim) => ({
            kind: "claim",
            key: `claim:${claim.id}`,
            createdAt: claim.updatedAt,
            title: claim.content,
            subtitle: [
              formatActivityTime(claim.updatedAt),
              t(`settings.claims.status.${claim.status}`),
              `${(claim.confidence * 100).toFixed(0)}%`,
            ],
            claim,
          }))

    const experienceItems: RecentUnifiedActivityItem[] =
      recentExperienceEvents.map(experienceEventActivityItem)

    return mergeMemoryAuditActivity({
      action: "all",
      memory: memoryItems,
      experience: experienceItems,
      decisions: decisionItems,
    }).slice(0, 4)
  }, [
    decisionActivityItem,
    decisionTypeLabel,
    experienceEventActivityItem,
    experienceHistoryActionLabel,
    memoryAuditActionLabel,
    memoryEventActivityItem,
    memoryScopeLabel,
    memorySourceLabel,
    recentCorrectionDecisions,
    recentCorrections,
    recentExperienceEvents,
    recentMemories,
    recentMemoryEvents,
    t,
  ])

  const visibleAuditActivity = useMemo<RecentUnifiedActivityItem[]>(() => {
    if (!memoryAuditOpen) return []
    return mergeMemoryAuditActivity({
      action: memoryAuditAction,
      memory: memoryAuditResults.map(memoryEventActivityItem),
      experience: memoryAuditExperienceResults.map(experienceEventActivityItem),
      decisions: memoryAuditDecisionResults.map(decisionActivityItem),
    })
  }, [
    decisionActivityItem,
    experienceEventActivityItem,
    memoryAuditAction,
    memoryAuditDecisionResults,
    memoryAuditExperienceResults,
    memoryAuditOpen,
    memoryAuditResults,
    memoryEventActivityItem,
  ])

  const openRecentUnifiedActivity = useCallback(
    (item: RecentUnifiedActivityItem) => {
      if (item.kind === "memory_event") {
        void openMemoryHistoryEvent(item.event)
        return
      }
      if (item.kind === "memory") {
        data.startEdit(item.memory)
        return
      }
      if (item.kind === "decision") {
        const decision = item.decision
        if (decision.targetType === "claim" && decision.targetId) {
          onOpenClaims({
            statusFilter: "all",
            selectedId: decision.targetId,
          })
        } else {
          onSelectTab("dreaming")
        }
        return
      }
      if (item.kind === "experience_event") {
        void openExperienceHistoryEvent(item.event)
        return
      }
      onOpenClaims({
        statusFilter: "all",
        claimType: item.claim.claimType,
        scopeType: item.claim.scopeType,
        scopeId: item.claim.scopeId,
        selectedId: item.claim.id,
      })
    },
    [data, onOpenClaims, onSelectTab, openExperienceHistoryEvent, openMemoryHistoryEvent],
  )

  const loadClaimInsights = useCallback(async () => {
    if (!canReviewClaims) {
      setProfileClaims([])
      setProjectClaims([])
      setProfileSnapshots([])
      setInsightsLoadIssues([])
      setInsightsLoading(false)
      return
    }
    const loadIssues: MemoryOverviewInsightsIssue[] = []
    const recordInsightIssue = (source: MemoryOverviewInsightsIssue["source"], error: unknown) => {
      loadIssues.push(memoryOverviewInsightsIssue(source, error))
    }
    setInsightsLoading(true)
    setInsightsLoadIssues([])
    try {
      const tx = getTransport()
      const [profileGroups, projectList, snapshotList] = await Promise.all([
        Promise.all(
          profileClaimTypes.map((claimType) =>
            tx
              .call<ClaimRecord[]>("claim_list", {
                status: "active",
                claimType,
                limit: 8,
              })
              .catch((e) => {
                logger.warn(
                  "settings",
                  "MemoryOverviewView::profileClaims",
                  "Failed to list profile claims",
                  e,
                )
                recordInsightIssue("profileClaims", e)
                return []
              }),
          ),
        ),
        tx
          .call<ClaimRecord[]>("claim_list", {
            status: "active",
            claimType: projectClaimType,
            limit: 8,
          })
          .catch((e) => {
            logger.warn(
              "settings",
              "MemoryOverviewView::projectClaims",
              "Failed to list project claims",
              e,
            )
            recordInsightIssue("projectClaims", e)
            return []
          }),
        tx.call<ProfileSnapshotRecord[]>("dreaming_list_profile_snapshots").catch((e) => {
          logger.warn(
            "settings",
            "MemoryOverviewView::profileSnapshots",
            "Failed to list profile snapshots",
            e,
          )
          recordInsightIssue("profileSnapshots", e)
          return []
        }),
      ])

      setProfileClaims(sortClaimsForOverview(profileGroups.flat()).slice(0, 4))
      setProjectClaims(sortClaimsForOverview(projectList ?? []).slice(0, 8))
      setProfileSnapshots(snapshotList ?? [])
      setInsightsLoadIssues(loadIssues)
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::claimInsights",
        "Failed to load memory insights",
        e,
      )
      setInsightsLoadIssues([memoryOverviewInsightsIssue("insights", e)])
    } finally {
      setInsightsLoading(false)
    }
  }, [canReviewClaims, profileClaimTypes, projectClaimType])

  useEffect(() => {
    let cancelled = false
    void getTransport()
      .call<ClaimSchemaMetadata>("claim_schema_metadata")
      .then((schema) => {
        if (cancelled) return
        setClaimSchema(normalizeClaimSchema(schema))
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "MemoryOverviewView::claimSchema",
          "Failed to load claim schema metadata",
          e,
        )
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (isAgentMode || !memoryEnabled) {
      setTargetAgentActiveMemory(null)
      setTargetAgentMemoryEnabled(true)
      setActiveMemoryError(null)
      setActiveMemoryLoading(false)
      return undefined
    }

    let cancelled = false
    setActiveMemoryLoading(true)
    setActiveMemoryError(null)
    void resolveDefaultMemoryAgentId()
      .then(async (agentId) => {
        const config = await getTransport().call<AgentConfig>("get_agent_config", { id: agentId })
        return { agentId, config }
      })
      .then((config) => {
        if (cancelled) return
        setActiveMemoryAgentId(config.agentId)
        setTargetAgentMemoryEnabled(config.config.memory?.enabled ?? true)
        setTargetAgentActiveMemory(config.config.memory?.activeMemory ?? DEFAULT_ACTIVE_MEMORY)
      })
      .catch((e) => {
        if (cancelled) return
        const message = formatMemoryUseInRepliesError(t, "load", e)
        setActiveMemoryError(message)
        logger.warn(
          "settings",
          "MemoryOverviewView::loadDefaultAgentActiveMemory",
          "Failed to load default agent active memory",
          e,
        )
      })
      .finally(() => {
        if (!cancelled) setActiveMemoryLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [isAgentMode, memoryEnabled])

  useEffect(() => {
    void loadPendingClaims()
  }, [loadPendingClaims])

  useEffect(() => {
    void loadRecentActivity()
  }, [loadRecentActivity])

  useEffect(() => {
    recentEpisodesRef.current = recentEpisodes
  }, [recentEpisodes])

  useEffect(() => {
    recentProceduresRef.current = recentProcedures
  }, [recentProcedures])

  useEffect(() => {
    if (!focus || isAgentMode || lastExperienceFocusNonceRef.current === focus.nonce) {
      return undefined
    }
    lastExperienceFocusNonceRef.current = focus.nonce
    let cancelled = false

    const scrollToTarget = () => {
      window.setTimeout(() => {
        document
          .getElementById(experienceDomId(focus.kind, focus.id))
          ?.scrollIntoView({ block: "center", behavior: "smooth" })
      }, 50)
    }

    setExperienceFocusHighlight({ kind: focus.kind, id: focus.id })

    if (focus.kind === "episode") {
      const episode = recentEpisodesRef.current.find((episode) => episode.id === focus.id)
      if (episode) {
        openExperienceDetail({ kind: "episode", record: episode })
        scrollToTarget()
        return () => {
          cancelled = true
        }
      }
    }
    if (focus.kind === "procedure") {
      const procedure = recentProceduresRef.current.find((procedure) => procedure.id === focus.id)
      if (procedure) {
        openExperienceDetail({ kind: "procedure", record: procedure })
        scrollToTarget()
        return () => {
          cancelled = true
        }
      }
    }

    const command = focus.kind === "episode" ? "memory_episode_get" : "memory_procedure_get"
    void getTransport()
      .call<MemoryEpisodeRecord | MemoryProcedureRecord | null>(command, { id: focus.id })
      .then((record) => {
        if (cancelled) return
        if (!record) {
          toast.error(
            t(
              "settings.memoryExperienceFocusMissing",
              "That experience memory no longer exists.",
            ),
          )
          return
        }
        if (focus.kind === "episode") {
          const episode = record as MemoryEpisodeRecord
          openExperienceDetail({ kind: "episode", record: episode })
          setRecentEpisodes((prev) => [
            episode,
            ...prev.filter((item) => item.id !== episode.id),
          ].slice(0, EXPERIENCE_PAGE_SIZE))
        } else {
          const procedure = record as MemoryProcedureRecord
          openExperienceDetail({ kind: "procedure", record: procedure })
          setRecentProcedures((prev) => [
            procedure,
            ...prev.filter((item) => item.id !== procedure.id),
          ].slice(0, EXPERIENCE_PAGE_SIZE))
        }
        scrollToTarget()
      })
      .catch((e) => {
        logger.warn(
          "settings",
          "MemoryOverviewView::experienceFocus",
          "Failed to focus experience memory",
          e,
        )
        const failureToast = memoryExperienceOperationErrorToast("focusExperience", t, e)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      })

    return () => {
      cancelled = true
    }
  }, [focus, isAgentMode, openExperienceDetail, t])

  useEffect(() => {
    if (!auditFocus || isAgentMode || lastAuditFocusNonceRef.current === auditFocus.nonce) {
      return
    }
    lastAuditFocusNonceRef.current = auditFocus.nonce
    const nextAction = normalizeMemoryAuditAction(auditFocus.auditAction)
    const nextQuery = (auditFocus.auditQuery ?? "").trim()
    const nextOpen = auditFocus.auditOpen === true || nextQuery.length > 0 || nextAction !== "all"
    setMemoryAuditOpen(nextOpen)
    setMemoryAuditQuery(nextQuery)
    setMemoryAuditAction(nextAction)
    if (nextOpen && (nextQuery.length > 0 || nextAction !== "all")) {
      void runMemoryAuditSearch({ query: nextQuery, action: nextAction })
    } else {
      resetMemoryAuditSearchState()
    }
  }, [auditFocus, isAgentMode, resetMemoryAuditSearchState, runMemoryAuditSearch])

  useEffect(() => {
    void loadMemoryHealth()
  }, [loadMemoryHealth])

  useEffect(() => {
    if (isAgentMode) return undefined
    return getTransport().listen("config:changed", () => {
      void loadMemoryHealth()
    })
  }, [isAgentMode, loadMemoryHealth])

  useEffect(() => {
    void loadClaimInsights()
  }, [loadClaimInsights])

  useEffect(() => {
    if (isAgentMode) return undefined
    let cancelled = false
    const tx = getTransport()
    void Promise.allSettled([
      tx.call<AgentInfo[]>("list_agents"),
      tx.call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true }),
    ]).then(([agentsResult, projectsResult]) => {
      if (cancelled) return
      if (agentsResult.status === "fulfilled") {
        const agents = agentsResult.value ?? []
        setAgentOptions(agents)
        setAgentNames(new Map(agents.map((agent) => [agent.id, agent.name])))
      }
      if (projectsResult.status === "fulfilled") {
        const projects = projectsResult.value ?? []
        setProjectOptions(projects.filter((project) => !project.archived))
        setProjectNames(new Map(projects.map((project) => [project.id, project.name])))
      }
    })
    return () => {
      cancelled = true
    }
  }, [isAgentMode])

  useEffect(() => {
    if (!canReviewClaims) return undefined
    const unlisten = getTransport().listen("memory:claim_changed", () => {
      void reloadMemories()
      void loadPendingClaims()
      void loadRecentActivity()
      void loadClaimInsights()
      void loadMemoryHealth()
    })
    return () => unlisten()
  }, [
    canReviewClaims,
    reloadMemories,
    loadPendingClaims,
    loadRecentActivity,
    loadClaimInsights,
    loadMemoryHealth,
  ])

  useEffect(() => {
    if (isAgentMode) return undefined
    const unlisten = getTransport().listen("memory:changed", () => {
      void reloadMemories()
      void loadRecentActivity()
      void loadMemoryHealth()
    })
    return () => unlisten()
  }, [reloadMemories, isAgentMode, loadRecentActivity, loadMemoryHealth])

  useEffect(() => {
    if (isAgentMode) return undefined
    const refresh = () => {
      void loadRecentActivity()
      void loadMemoryHealth()
    }
    const unlistenComplete = getTransport().listen("dreaming:cycle_complete", refresh)
    const unlistenStarted = getTransport().listen("dreaming:cycle_started", refresh)
    return () => {
      unlistenComplete()
      unlistenStarted()
    }
  }, [isAgentMode, loadMemoryHealth, loadRecentActivity])

  const memoryHealthUnavailable = !healthLoading && !memoryHealth
  const memoryHealthTone = healthTone(
    memoryHealthLoadError || memoryHealthUnavailable ? "error" : memoryHealth?.status,
  )
  const memoryHealthStatusText =
    healthLoading && !memoryHealth
      ? t("common.loading")
      : memoryHealthLoadError || memoryHealthUnavailable
      ? t("settings.memoryHealthUnavailable", "Health check unavailable.")
      : memoryHealth?.status === "error"
      ? t("common.error", "Error")
      : memoryHealth?.status === "warning"
        ? t("settings.memoryHealthNeedsAttention", "Needs attention")
        : t("settings.memoryHealthOk", "OK")
  const claimGraphOrphans =
    (memoryHealth?.orphanEvidenceRows ?? 0) + (memoryHealth?.orphanClaimLinks ?? 0)
  const procedureEpisodeOrphans = memoryHealth?.orphanProcedureEpisodeRefs ?? 0
  const memoryGraphOrphans = claimGraphOrphans + procedureEpisodeOrphans
  const experienceMemoryTotal =
    (memoryHealth?.episodesTotal ?? 0) + (memoryHealth?.proceduresTotal ?? 0)
  const keywordIndexGaps =
    (memoryHealth?.ftsMissingRows ?? 0) +
    (memoryHealth?.claimFtsMissingRows ?? 0) +
    (memoryHealth?.evidenceFtsMissingRows ?? 0)
  const deepResolverHealthSummary = memoryHealth
    ? formatDeepResolverHealthSummary(memoryHealth, t, { onLabel, offLabel })
    : null
  const visibleHealthIssues = (memoryHealth?.issues ?? []).filter(
    (issue) => issue.severity !== "info",
  )
  const topHealthIssues = visibleHealthIssues.slice(0, 2)
  const hiddenHealthIssueCount = Math.max(0, visibleHealthIssues.length - topHealthIssues.length)
  const embeddingProviderStatus = !memoryHealth?.embeddingProviderConfigured
    ? offLabel
    : memoryHealth.embeddingProviderLoaded
      ? t("settings.memoryEmbeddingProviderLoaded", {
          defaultValue: "{{dims}}d provider loaded",
          dims: memoryHealth.embeddingProviderDimensions ?? "?",
        })
      : t("settings.memoryEmbeddingProviderUnavailable", "Provider not loaded")
  const embeddingProviderCapabilities = [
    memoryHealth?.embeddingProviderMultimodal
      ? t("settings.memoryEmbeddingProviderMultimodal", "multimodal")
      : null,
    memoryHealth?.embeddingProviderBatch
      ? t("settings.memoryEmbeddingProviderBatch", "batch")
      : null,
  ].filter((label): label is string => Boolean(label))
  const externalProviderStatus = !memoryHealth?.externalProvidersEnabled
    ? offLabel
    : memoryHealth.externalProviderActiveCount > 0
      ? t("settings.memoryExternalProvidersActive", {
          defaultValue: "{{active}} / {{total}} active",
          active: memoryHealth.externalProviderActiveCount,
          total: memoryHealth.externalProviderCount,
        })
      : t("settings.memoryExternalProvidersNoActive", "Enabled, no active providers")
  const externalProviderOverview = externalMemoryProviderOverview(
    memoryHealth?.externalProvidersEnabled ?? false,
    memoryHealth?.externalProviders ?? [],
  )
  const externalProviderBlockSummary = externalMemoryProviderSyncBlockSummary(
    memoryHealth?.externalProvidersEnabled ?? false,
    memoryHealth?.externalProviders ?? [],
  )
  const externalProviderBlockChips = externalProviderBlockSummary.topReasons
    .slice(0, 3)
    .map((reason) => {
      const count = externalProviderBlockSummary.reasonCounts[reason] ?? 0
      const label = externalMemoryProviderSyncBlockReasonLabel(reason, t)
      return {
        reason,
        label: count > 1 ? `${label} (${count})` : label,
      }
    })
  const hiddenExternalProviderBlockReasonCount = Math.max(
    0,
    externalProviderBlockSummary.topReasons.length - externalProviderBlockChips.length,
  )
  const externalProviderOverviewDetail = (() => {
    switch (externalProviderOverview.state) {
      case "off":
        return t(
          "settings.memoryExternalProvidersOverviewOff",
          "External provider sync is off. No memory data leaves Hope Agent through providers.",
        )
      case "needs_setup":
        return t("settings.memoryExternalProvidersOverviewNeedsSetup", {
          defaultValue:
            "{{count}} provider(s) need endpoint setup. Local memory remains available.",
          count: externalProviderOverview.needsSetupCount,
        })
      case "unsupported_policy":
        return t("settings.memoryExternalProvidersOverviewUnsupportedPolicy", {
          defaultValue:
            "{{count}} provider config(s) use an unsupported sync policy. Choose a supported policy before sync can run.",
          count: externalProviderOverview.unsupportedPolicyCount,
        })
      case "adapter_pending":
        return t("settings.memoryExternalProvidersOverviewAdapterPending", {
          defaultValue:
            "{{count}} provider config(s) are waiting for a runtime adapter. No sync will run; local memory remains available.",
          count: externalProviderOverview.adapterPendingCount,
        })
      case "error":
        return t("settings.memoryExternalProvidersOverviewError", {
          defaultValue: "{{count}} provider(s) report an error. Local memory remains available.",
          count: externalProviderOverview.errorCount,
        })
      case "ready":
        return t("settings.memoryExternalProvidersOverviewReady", {
          defaultValue:
            "{{count}} provider adapter(s) are ready for additive sync. Local memory remains the source of truth.",
          count: externalProviderOverview.readyCount,
        })
      default:
        return t(
          "settings.memoryExternalProvidersOverviewNoActive",
          "Provider sync is enabled globally, but no provider policy is active.",
        )
    }
  })()
  const externalProviderOverviewTone =
    externalProviderOverview.state === "needs_setup" ||
    externalProviderOverview.state === "unsupported_policy" ||
    externalProviderOverview.state === "adapter_pending" ||
    externalProviderOverview.state === "error"
      ? "text-amber-600 dark:text-amber-300"
      : "text-muted-foreground"
  const repairHints = memoryHealth ? memoryHealthRepairHints(memoryHealth) : []
  const repairPolicy = memoryHealth ? memoryHealthRepairPolicy(memoryHealth) : "none"
  const canRebuildFts = repairHints.some((hint) => hint.action === "rebuild_fts")
  const canRebuildClaimFts = repairHints.some((hint) => hint.action === "rebuild_claim_fts")
  const canRepairClaimGraph = repairHints.some((hint) => hint.action === "repair_claim_graph")
  const canRepairExperienceGraph = repairHints.some(
    (hint) => hint.action === "repair_experience_graph",
  )
  const canRecoverDreamingState = repairHints.some(
    (hint) => hint.action === "recover_dreaming_state",
  )
  const canCreateDbSnapshot = repairHints.some((hint) => hint.action === "create_db_snapshot")
  const currentDbSnapshotSummary = memorySnapshotArtifactSummaryParts(currentDbSnapshotFiles)
  const currentDbSnapshotIncomplete = currentDbSnapshotStatus !== "ok"
  const activeDbSnapshotRestorePreview =
    dbSnapshotRestorePreview?.snapshotPath === currentDbSnapshotPath ? dbSnapshotRestorePreview : null
  const dbSnapshotRestoreIssue = activeDbSnapshotRestorePreview?.issues[0] ?? null
  const repairReasonText = (action: MemoryRepairAction): string | null => {
    if (!repairHints.some((hint) => hint.action === action)) return null
    switch (action) {
      case "create_db_snapshot":
        return t(
          "settings.memoryRepairReasonDbSnapshot",
          "SQLite integrity check failed; preserve a raw snapshot before running other repairs.",
        )
      case "rebuild_fts":
        return t(
          "settings.memoryRepairReasonFts",
          "Some long-term memories are missing from keyword search; rebuild the local FTS index.",
        )
      case "rebuild_claim_fts":
        return t(
          "settings.memoryRepairReasonClaimFts",
          "Some structured memories or evidence are missing from keyword search; rebuild the structured indexes.",
        )
      case "repair_claim_graph":
        return t(
          "settings.memoryRepairReasonClaimGraph",
          "Structured memory evidence or links point at missing rows; clean up orphan graph links.",
        )
      case "repair_experience_graph":
        return t(
          "settings.memoryRepairReasonExperienceGraph",
          "Procedure memories reference missing source episodes; clean up those experience links.",
        )
      case "recover_dreaming_state":
        return t(
          "settings.memoryRepairReasonDreamingState",
          "Background memory maintenance has stale runs or locks; recover the scheduler state.",
        )
    }
  }
  const repairActionItems: Array<{
    action: MemoryRepairAction
    visible: boolean
    loading: boolean
    label: string
    reason: string | null
    onClick: () => void
  }> = [
    {
      action: "create_db_snapshot",
      visible: canCreateDbSnapshot,
      loading: repairingDbSnapshot,
      label: t("settings.memoryRepairDbSnapshot", "Create database snapshot"),
      reason: repairReasonText("create_db_snapshot"),
      onClick: () => void createDbSnapshot(),
    },
    {
      action: "rebuild_fts",
      visible: canRebuildFts,
      loading: repairingFts,
      label: t("settings.memoryRepairFts", "Rebuild keyword index"),
      reason: repairReasonText("rebuild_fts"),
      onClick: () => void rebuildMemoryFts(),
    },
    {
      action: "rebuild_claim_fts",
      visible: canRebuildClaimFts,
      loading: repairingClaimFts,
      label: t("settings.memoryRepairClaimFts", "Rebuild structured index"),
      reason: repairReasonText("rebuild_claim_fts"),
      onClick: () => void rebuildClaimFts(),
    },
    {
      action: "repair_claim_graph",
      visible: canRepairClaimGraph,
      loading: repairingClaimGraph,
      label: t("settings.memoryRepairClaimGraph", "Repair claim graph links"),
      reason: repairReasonText("repair_claim_graph"),
      onClick: () => void repairClaimGraph(),
    },
    {
      action: "repair_experience_graph",
      visible: canRepairExperienceGraph,
      loading: repairingExperienceGraph,
      label: t("settings.memoryRepairExperienceGraph", "Repair experience links"),
      reason: repairReasonText("repair_experience_graph"),
      onClick: () => void repairExperienceGraph(),
    },
    {
      action: "recover_dreaming_state",
      visible: canRecoverDreamingState,
      loading: repairingDreamingState,
      label: t("settings.memoryRepairDreamingState", "Recover Dreaming state"),
      reason: repairReasonText("recover_dreaming_state"),
      onClick: () => void recoverDreamingState(),
    },
  ]
  const memoryAuditHasFilters = memoryAuditQuery.trim().length > 0 || memoryAuditAction !== "all"
  const experienceHasSearch = experienceAppliedQuery.trim().length > 0
  const experienceHasCustomView =
    experienceHasSearch ||
    experienceScopeFilter.kind !== "all" ||
    experienceStatus !== EXPERIENCE_DEFAULT_STATUS ||
    experienceSort !== EXPERIENCE_DEFAULT_SORT
  const experienceSearchTotal = experienceEpisodeTotal + experienceProcedureTotal
  const experienceEpisodeHasMore = recentEpisodes.length < experienceEpisodeTotal
  const experienceProcedureHasMore = recentProcedures.length < experienceProcedureTotal
  const memoryAuditCombinedTotal =
    memoryAuditAction === "all"
      ? memoryAuditTotal == null &&
        memoryAuditExperienceTotal == null &&
        memoryAuditDecisionTotal == null
        ? null
        : (memoryAuditTotal ?? memoryAuditResults.length) +
          (memoryAuditExperienceTotal ?? memoryAuditExperienceResults.length) +
          (memoryAuditDecisionTotal ?? memoryAuditDecisionResults.length)
      : memoryAuditTotal
  const memoryAuditCombinedTotalTruncated =
    memoryAuditTotalTruncated ||
    (memoryAuditAction === "all" &&
      (memoryAuditExperienceTotalTruncated || memoryAuditDecisionTotalTruncated))
  const memoryAuditHasMoreCombined =
    memoryAuditHasMore ||
    (memoryAuditAction === "all" &&
      (memoryAuditExperienceHasMore || memoryAuditDecisionHasMore))
  const memoryAuditShownCount = countMemoryAuditActivity({
    action: memoryAuditAction,
    memoryCount: memoryAuditResults.length,
    experienceCount: memoryAuditExperienceResults.length,
    decisionCount: memoryAuditDecisionResults.length,
  })
  const memoryAuditTotalLabel =
    memoryAuditCombinedTotal == null
      ? null
      : memoryAuditCombinedTotalTruncated
        ? `${memoryAuditCombinedTotal}+`
        : `${memoryAuditCombinedTotal}`
  const memoryAuditExportMarkdown = useCallback(
    (
      events: MemoryHistoryRecord[],
      options?: {
        query?: string
        action?: MemoryHistoryAction | "all"
        allMatching?: boolean
        truncated?: boolean
      },
    ) => {
      const exportQuery = options?.query ?? memoryAuditQuery
      const exportAction = options?.action ?? memoryAuditAction
      const normalizedQuery = exportQuery.trim().replace(/\s+/g, " ")
      const filterParts = [memoryAuditActionLabel(exportAction)]
      if (normalizedQuery) filterParts.push(`query: ${normalizedQuery}`)
      const lines: string[] = [
        `# ${t("settings.memoryRecentActivity", "Recent memory activity")}`,
        "",
        `- ${t("settings.memoryAuditExportGeneratedAt", "Generated at")}: ${new Date().toLocaleString()}`,
        `- ${t("settings.memoryAuditExportFilters", "Filters")}: ${filterParts.join(" / ")}`,
        `- ${t("settings.memoryAuditExportCount", "Activity events")}: ${events.length}`,
        `- ${t("settings.memoryAuditExportView", "Export view")}: ${
          options?.allMatching
            ? t("settings.memoryAuditExportAllMatching", "All matching activity")
            : t("settings.memoryAuditExportCurrentView", "Current loaded view")
        }`,
      ]
      if (options?.truncated) {
        lines.push(
          `- ${t(
            "settings.memoryAuditExportTruncated",
            "Export reached the safety limit; more matching activity may exist.",
          )}`,
        )
      }
      lines.push("")

      for (const event of events) {
        const preview = event.contentPreview.trim()
        const heading = (preview || `memory:${event.memoryId}`)
          .replace(/\s+/g, " ")
          .replace(/^#+\s*/, "")
        lines.push(`## ${heading}`)
        lines.push(
          `- ${formatActivityTime(event.createdAt)} · ${memoryAuditActionLabel(event.action)} · ${t(
            `settings.memoryType_${event.memoryType}`,
          )} · ${memorySourceLabel(event.source)}`,
        )
        lines.push(`- memory: ${event.memoryId}`)
        lines.push(`- scope: ${memoryScopeLabel(event.scope)}`)
        lines.push(`- pinned: ${event.pinned ? onLabel : offLabel}`)
        if (event.sourceSessionId) {
          lines.push(`- session: ${event.sourceSessionId}`)
        }
        if (preview) {
          lines.push("")
          lines.push(preview)
        }
        lines.push("")
      }

      return lines.join("\n").trimEnd()
    },
    [
      memoryAuditAction,
      memoryAuditActionLabel,
      memoryAuditQuery,
      memoryScopeLabel,
      memorySourceLabel,
      offLabel,
      onLabel,
      t,
    ],
  )
  const recentUnifiedActivityExportMarkdown = useCallback(
    (
      items: RecentUnifiedActivityItem[],
      options?: {
        query?: string
        action?: MemoryHistoryAction | "all"
        allMatching?: boolean
        truncated?: boolean
      },
    ) => {
      const filterParts = [memoryAuditActionLabel(options?.action ?? "all")]
      const normalizedQuery = (options?.query ?? "").trim().replace(/\s+/g, " ")
      if (normalizedQuery) filterParts.push(`query: ${normalizedQuery}`)
      const lines: string[] = [
        `# ${t("settings.memoryRecentActivity", "Recent memory activity")}`,
        "",
        `- ${t("settings.memoryAuditExportGeneratedAt", "Generated at")}: ${new Date().toLocaleString()}`,
        `- ${t("settings.memoryAuditExportFilters", "Filters")}: ${filterParts.join(" / ")}`,
        `- ${t("settings.memoryAuditExportCount", "Activity events")}: ${items.length}`,
        `- ${t("settings.memoryAuditExportView", "Export view")}: ${
          options?.allMatching
            ? t("settings.memoryAuditExportAllMatching", "All matching activity")
            : t("settings.memoryAuditExportCurrentView", "Current loaded view")
        }`,
      ]
      if (options?.truncated) {
        lines.push(
          `- ${t(
            "settings.memoryAuditExportTruncated",
            "Export reached the safety limit; more matching activity may exist.",
          )}`,
        )
      }
      lines.push("")

      for (const item of items) {
        const heading = (item.title || item.key).replace(/\s+/g, " ").replace(/^#+\s*/, "")
        lines.push(`## ${heading}`)
        if (item.subtitle.length > 0) {
          lines.push(`- ${item.subtitle.join(" · ")}`)
        }
        if (item.kind === "memory_event") {
          lines.push(`- kind: memory event`)
          lines.push(`- memory: ${item.event.memoryId}`)
          lines.push(`- scope: ${memoryScopeLabel(item.event.scope)}`)
          lines.push(`- pinned: ${item.event.pinned ? onLabel : offLabel}`)
          if (item.event.sourceSessionId) {
            lines.push(`- session: ${item.event.sourceSessionId}`)
          }
        } else if (item.kind === "memory") {
          lines.push(`- kind: memory`)
          lines.push(`- memory: ${item.memory.id}`)
          lines.push(`- scope: ${memoryScopeLabel(item.memory.scope)}`)
          lines.push(`- pinned: ${item.memory.pinned ? onLabel : offLabel}`)
          if (item.memory.sourceSessionId) {
            lines.push(`- session: ${item.memory.sourceSessionId}`)
          }
        } else if (item.kind === "decision") {
          lines.push(`- kind: claim decision`)
          lines.push(`- decision: ${item.decision.id}`)
          if (item.decision.targetId) {
            lines.push(`- target: ${item.decision.targetType}:${item.decision.targetId}`)
          }
        } else if (item.kind === "claim") {
          lines.push(`- kind: structured memory`)
          lines.push(`- claim: ${item.claim.id}`)
          lines.push(`- scope: ${claimScopeLabel(item.claim)}`)
          lines.push(`- status: ${item.claim.status}`)
        } else {
          lines.push(`- kind: experience memory`)
          lines.push(`- target: ${item.event.targetKind}:${item.event.targetId}`)
          lines.push(`- scope: ${memoryScopeLabel(item.event.scope)}`)
        }
        if (item.detail) {
          lines.push("")
          lines.push(item.detail)
        }
        lines.push("")
      }

      return lines.join("\n").trimEnd()
    },
    [claimScopeLabel, memoryAuditActionLabel, memoryScopeLabel, offLabel, onLabel, t],
  )
  const hasCurrentActivityExport = memoryAuditOpen
    ? visibleAuditActivity.length > 0
    : recentUnifiedActivity.length > 0
  const copyMemoryAuditExport = useCallback(async () => {
    if (!hasCurrentActivityExport) {
      toast.error(t("settings.memoryAuditExportEmpty", "No memory activity to export"))
      return
    }
    try {
      await navigator.clipboard.writeText(
        memoryAuditOpen
          ? recentUnifiedActivityExportMarkdown(visibleAuditActivity, {
              query: memoryAuditQuery,
              action: memoryAuditAction,
            })
          : recentUnifiedActivityExportMarkdown(recentUnifiedActivity),
      )
      toast.success(t("settings.memoryAuditExportDone", "Memory activity copied as Markdown"))
    } catch (e) {
      logger.warn("settings", "MemoryOverviewView::memoryAuditExport", "Clipboard write failed", e)
      const failureToast = memoryAuditOperationErrorToast("exportCurrent", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    }
  }, [
    hasCurrentActivityExport,
    memoryAuditAction,
    memoryAuditOpen,
    memoryAuditQuery,
    recentUnifiedActivity,
    recentUnifiedActivityExportMarkdown,
    t,
    visibleAuditActivity,
  ])
  const copyAllMemoryAuditExport = useCallback(async () => {
    if (memoryAuditExportingAll) return
    const exportQuery = memoryAuditQuery
    const exportAction = memoryAuditAction
    const includeExperienceHistory = includeCrossSourceAudit(exportAction)
    setMemoryAuditExportingAll(true)
    try {
      const exported: MemoryHistoryRecord[] = []
      const exportedExperience: MemoryExperienceHistoryRecord[] = []
      const exportedDecisions: RecentCorrectionItem[] = []
      const seen = new Set<string>()
      const seenExperience = new Set<string>()
      const seenDecisions = new Set<string>()
      let offset = 0
      let experienceOffset = 0
      let decisionOffset = 0
      let truncated = false
      let experienceTruncated = false
      let decisionTruncated = false
      let total = 0
      let experienceTotal = 0
      let decisionTotal = 0
      let totalTruncated = false
      let experienceTotalTruncated = false
      let decisionTotalTruncated = false

      const useUnifiedAuditPage = exportAction !== "all" || canReviewClaims
      if (useUnifiedAuditPage) {
        try {
          let unifiedOffset = 0
          let unifiedTotal = 0
          while (true) {
            const unifiedResponse = await getTransport().call<MemoryAuditPageResponse>(
              "memory_audit_page",
              {
                query: exportQuery.trim() || null,
                action: exportAction,
                limit: MEMORY_AUDIT_EXPORT_PAGE_SIZE,
                offset: unifiedOffset,
              },
            )
            const rows = unifiedResponse.items ?? []
            const buckets = splitMemoryAuditPage({
              items: rows,
              mapDecision: recentCorrectionFromDecisionListItem,
            })
            total = Math.max(total, unifiedResponse.sources.legacyMemory.total)
            totalTruncated =
              totalTruncated || unifiedResponse.sources.legacyMemory.totalTruncated === true
            experienceTotal = Math.max(
              experienceTotal,
              unifiedResponse.sources.experience.total,
            )
            experienceTotalTruncated =
              experienceTotalTruncated ||
              unifiedResponse.sources.experience.totalTruncated === true
            decisionTotal = Math.max(decisionTotal, unifiedResponse.sources.claimDecision.total)
            decisionTotalTruncated =
              decisionTotalTruncated ||
              unifiedResponse.sources.claimDecision.totalTruncated === true

            for (const event of buckets.memory) {
              if (seen.has(event.id)) continue
              if (
                exported.length + exportedExperience.length + exportedDecisions.length >=
                MEMORY_AUDIT_EXPORT_MAX_EVENTS
              ) {
                truncated = true
                break
              }
              seen.add(event.id)
              exported.push(event)
            }
            for (const event of buckets.experience) {
              if (seenExperience.has(event.id)) continue
              if (
                exported.length + exportedExperience.length + exportedDecisions.length >=
                MEMORY_AUDIT_EXPORT_MAX_EVENTS
              ) {
                experienceTruncated = true
                break
              }
              seenExperience.add(event.id)
              exportedExperience.push(event)
            }
            for (const event of buckets.decisions) {
              if (seenDecisions.has(event.id)) continue
              if (
                exported.length + exportedExperience.length + exportedDecisions.length >=
                MEMORY_AUDIT_EXPORT_MAX_EVENTS
              ) {
                decisionTruncated = true
                break
              }
              seenDecisions.add(event.id)
              exportedDecisions.push(event)
            }

            unifiedTotal = Math.max(unifiedTotal, unifiedResponse.total, unifiedOffset + rows.length)
            if (
              truncated ||
              experienceTruncated ||
              decisionTruncated ||
              exported.length + exportedExperience.length + exportedDecisions.length >=
                MEMORY_AUDIT_EXPORT_MAX_EVENTS
            ) {
              truncated = true
              break
            }
            unifiedOffset += rows.length
            if (!unifiedResponse.totalTruncated && unifiedOffset >= unifiedTotal) break
            if (rows.length < MEMORY_AUDIT_EXPORT_PAGE_SIZE) break
          }

          if (
            exported.length === 0 &&
            exportedExperience.length === 0 &&
            exportedDecisions.length === 0
          ) {
            toast.error(t("settings.memoryAuditExportEmpty", "No memory activity to export"))
            return
          }

          const exportMarkdown =
            includeExperienceHistory
              ? recentUnifiedActivityExportMarkdown(
                  mergeMemoryAuditActivity({
                    action: exportAction,
                    memory: exported.map(memoryEventActivityItem),
                    experience: exportedExperience.map(experienceEventActivityItem),
                    decisions: exportedDecisions.map(decisionActivityItem),
                  }),
                  {
                    query: exportQuery,
                    action: exportAction,
                    allMatching: true,
                    truncated:
                      truncated ||
                      totalTruncated ||
                      experienceTruncated ||
                      experienceTotalTruncated ||
                      decisionTruncated ||
                      decisionTotalTruncated,
                  },
                )
              : memoryAuditExportMarkdown(exported, {
                  query: exportQuery,
                  action: exportAction,
                  allMatching: true,
                  truncated: truncated || totalTruncated,
                })

          await navigator.clipboard.writeText(exportMarkdown)
          toast.success(
            t("settings.memoryAuditExportAllDone", {
              defaultValue: "{{count}} memory activity events copied as Markdown",
              count: exported.length + exportedExperience.length + exportedDecisions.length,
            }),
          )
          return
        } catch (unifiedError) {
          logger.warn(
            "settings",
            "MemoryOverviewView::memoryAuditUnifiedExportAll",
            "Failed to export unified memory audit page; falling back to source queries",
            unifiedError,
          )
          exported.length = 0
          exportedExperience.length = 0
          exportedDecisions.length = 0
          seen.clear()
          seenExperience.clear()
          seenDecisions.clear()
          truncated = false
          experienceTruncated = false
          decisionTruncated = false
          total = 0
          experienceTotal = 0
          decisionTotal = 0
          totalTruncated = false
          experienceTotalTruncated = false
          decisionTotalTruncated = false
        }
      }

      while (true) {
        const request = memoryAuditRequest(
          exportQuery,
          exportAction,
          MEMORY_AUDIT_EXPORT_PAGE_SIZE,
          offset,
        )
        let response: MemoryHistoryListResponse | null = null
        try {
          response = await getTransport().call<MemoryHistoryListResponse>("memory_history_page", {
            ...request,
          })
        } catch (pageError) {
          logger.warn(
            "settings",
            "MemoryOverviewView::memoryAuditExportAllPage",
            "Failed to export memory history via page query; falling back to item query",
            pageError,
          )
          const legacyItems =
            (await getTransport().call<MemoryHistoryRecord[]>("memory_history", { ...request })) ??
            []
          response = {
            items: legacyItems,
            total: offset + legacyItems.length,
            totalTruncated: legacyItems.length >= MEMORY_AUDIT_EXPORT_PAGE_SIZE,
          }
        }
        const rows = (response.items ?? []).sort(
          (a, b) => timeValue(b.createdAt) - timeValue(a.createdAt),
        )
        total = Math.max(total, response.total ?? 0, offset + rows.length)
        totalTruncated = totalTruncated || response.totalTruncated === true
        for (const event of rows) {
          if (seen.has(event.id)) continue
          if (
            exported.length + exportedExperience.length + exportedDecisions.length >=
            MEMORY_AUDIT_EXPORT_MAX_EVENTS
          ) {
            truncated = true
            break
          }
          seen.add(event.id)
          exported.push(event)
        }
        if (truncated) break
        offset += rows.length
        if (!totalTruncated && offset >= total) break
        if (rows.length < MEMORY_AUDIT_EXPORT_PAGE_SIZE) break
      }

      if (includeExperienceHistory && exported.length < MEMORY_AUDIT_EXPORT_MAX_EVENTS) {
        while (true) {
          let response: MemoryExperienceHistoryListPage | null = null
          try {
            response = await getTransport().call<MemoryExperienceHistoryListPage>(
              "memory_experience_history_page",
              {
                query: {
                  query: exportQuery.trim() || null,
                  limit: MEMORY_AUDIT_EXPORT_PAGE_SIZE,
                  offset: experienceOffset,
                },
              },
            )
          } catch (experienceError) {
            logger.warn(
              "settings",
              "MemoryOverviewView::memoryAuditExperienceExportAll",
              "Failed to export experience history; continuing with memory history",
              experienceError,
            )
            experienceTotalTruncated = true
            break
          }
          const rows = (response.items ?? []).sort(
            (a, b) => timeValue(b.createdAt) - timeValue(a.createdAt),
          )
          experienceTotal = Math.max(
            experienceTotal,
            response.total ?? 0,
            experienceOffset + rows.length,
          )
          experienceTotalTruncated =
            experienceTotalTruncated || response.totalTruncated === true
          for (const event of rows) {
            if (seenExperience.has(event.id)) continue
            if (
              exported.length + exportedExperience.length + exportedDecisions.length >=
              MEMORY_AUDIT_EXPORT_MAX_EVENTS
            ) {
              experienceTruncated = true
              break
            }
            seenExperience.add(event.id)
            exportedExperience.push(event)
          }
          if (experienceTruncated) break
          experienceOffset += rows.length
          if (!experienceTotalTruncated && experienceOffset >= experienceTotal) break
          if (rows.length < MEMORY_AUDIT_EXPORT_PAGE_SIZE) break
        }
      }

      if (
        includeExperienceHistory &&
        canReviewClaims &&
        exported.length + exportedExperience.length < MEMORY_AUDIT_EXPORT_MAX_EVENTS
      ) {
        while (true) {
          let response: DreamingDecisionListResponse | null = null
          try {
            response = await getTransport().call<DreamingDecisionListResponse>(
              "dreaming_list_decisions_page",
              {
                query: exportQuery.trim() || null,
                targetType: "claim",
                limit: MEMORY_AUDIT_EXPORT_PAGE_SIZE,
                offset: decisionOffset,
              },
            )
          } catch (decisionError) {
            logger.warn(
              "settings",
              "MemoryOverviewView::memoryAuditDecisionExportAll",
              "Failed to export claim decision history; continuing with other activity",
              decisionError,
            )
            decisionTotalTruncated = true
            break
          }
          const rows = (response.items ?? [])
            .map(recentCorrectionFromDecisionListItem)
            .sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt))
          decisionTotal = Math.max(
            decisionTotal,
            response.total ?? 0,
            decisionOffset + rows.length,
          )
          decisionTotalTruncated = decisionTotalTruncated || response.totalTruncated === true
          for (const event of rows) {
            if (seenDecisions.has(event.id)) continue
            if (
              exported.length + exportedExperience.length + exportedDecisions.length >=
              MEMORY_AUDIT_EXPORT_MAX_EVENTS
            ) {
              decisionTruncated = true
              break
            }
            seenDecisions.add(event.id)
            exportedDecisions.push(event)
          }
          if (decisionTruncated) break
          decisionOffset += rows.length
          if (!decisionTotalTruncated && decisionOffset >= decisionTotal) break
          if (rows.length < MEMORY_AUDIT_EXPORT_PAGE_SIZE) break
        }
      }

      if (
        exported.length === 0 &&
        exportedExperience.length === 0 &&
        exportedDecisions.length === 0
      ) {
        toast.error(t("settings.memoryAuditExportEmpty", "No memory activity to export"))
        return
      }

      const exportMarkdown =
        includeExperienceHistory
          ? recentUnifiedActivityExportMarkdown(
              mergeMemoryAuditActivity({
                action: exportAction,
                memory: exported.map(memoryEventActivityItem),
                experience: exportedExperience.map(experienceEventActivityItem),
                decisions: exportedDecisions.map(decisionActivityItem),
              }),
              {
                query: exportQuery,
                action: exportAction,
                allMatching: true,
                truncated:
                  truncated ||
                  totalTruncated ||
                  experienceTruncated ||
                  experienceTotalTruncated ||
                  decisionTruncated ||
                  decisionTotalTruncated,
              },
            )
          : memoryAuditExportMarkdown(exported, {
              query: exportQuery,
              action: exportAction,
              allMatching: true,
              truncated: truncated || totalTruncated,
            })

      await navigator.clipboard.writeText(exportMarkdown)
      toast.success(
        t("settings.memoryAuditExportAllDone", {
          defaultValue: "{{count}} memory activity events copied as Markdown",
          count: exported.length + exportedExperience.length + exportedDecisions.length,
        }),
      )
    } catch (e) {
      logger.warn(
        "settings",
        "MemoryOverviewView::memoryAuditExportAll",
        "Clipboard export failed",
        e,
      )
      const failureToast = memoryAuditOperationErrorToast("exportAll", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
    } finally {
      setMemoryAuditExportingAll(false)
    }
  }, [
    memoryAuditAction,
    canReviewClaims,
    decisionActivityItem,
    memoryAuditExportMarkdown,
    memoryAuditExportingAll,
    memoryAuditQuery,
    memoryAuditRequest,
    experienceEventActivityItem,
    memoryEventActivityItem,
    recentUnifiedActivityExportMarkdown,
    t,
  ])

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="w-full space-y-5">
        <div>
          <h2 className="text-lg font-semibold">{t("settings.tabOverview", "Overview")}</h2>
          <p className="mt-1 text-xs text-muted-foreground">{t("settings.memoryDesc")}</p>
        </div>

        {!isAgentMode && !memoryEnabled && (
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-3">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div className="flex min-w-0 gap-2.5">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-300" />
                <div className="min-w-0">
                  <div className="text-sm font-medium">
                    {t("settings.memoryLearningPausedTitle")}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.memoryLearningPausedDesc")}
                  </div>
                </div>
              </div>
              <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                <Button
                  type="button"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => data.applyMemoryLearningMode("manual_only")}
                >
                  {t("settings.memoryLearningResumeManual")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => onSelectTab("settings")}
                >
                  {t("settings.memoryLearningOpenSettings")}
                </Button>
              </div>
            </div>
          </div>
        )}

        <div className="grid gap-3 md:grid-cols-4">
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <Database className="h-3.5 w-3.5" />
              <span>{t("settings.memoryStatsTotal", { count: total })}</span>
            </div>
            <div className="mt-2 text-2xl font-semibold tabular-nums">{total}</div>
          </div>
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <Activity className="h-3.5 w-3.5" />
              <span>{t("settings.memoryAutoExtract")}</span>
            </div>
            <div className="mt-2 text-sm font-medium">
              {statusText(data.effectiveAutoExtract, onLabel, offLabel)}
            </div>
          </div>
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <Brain className="h-3.5 w-3.5" />
              <span>{t("settings.memoryEmbedding")}</span>
            </div>
            <div className="mt-2 truncate text-sm font-medium">
              {embeddingReady ? activeEmbeddingModel : offLabel}
            </div>
          </div>
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <Sparkles className="h-3.5 w-3.5" />
              <span>{t("settings.memoryTabs.claims")}</span>
            </div>
            <div className="mt-2 text-sm font-medium">
              {statusText(memoryEnabled && data.effectiveExtractClaims, onLabel, offLabel)}
            </div>
          </div>
        </div>

        {!isAgentMode && activityLoadWarning && (
          <div className="flex flex-col gap-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-xs sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-300" />
              <div className="min-w-0">
                <div className="font-medium text-foreground">{activityLoadWarning.title}</div>
                <div className="mt-0.5 break-words text-muted-foreground">
                  {activityLoadWarning.description}
                </div>
              </div>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 self-start px-2 text-[11px]"
              disabled={activityLoading}
              onClick={() => void loadRecentActivity()}
            >
              {activityLoading ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
              )}
              {t("settings.memoryOverviewRetry", "Retry")}
            </Button>
          </div>
        )}

        {!isAgentMode && memoryEnabled && (
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <Sparkles className="h-4 w-4 text-primary" />
                    <span>{t("settings.memoryUseInRepliesTitle", "Use memories in replies")}</span>
                  </div>
                  <span
                    className={cn(
                      "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium",
                      activeMemoryStatusClassName,
                    )}
                  >
                    {activeMemoryLoading ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : activeMemoryUsesRecommended ? (
                      <CheckCircle2 className="h-3 w-3" />
                    ) : null}
                    {activeMemoryStatusLabel}
                  </span>
                </div>
                <p className="mt-1 max-w-3xl text-xs text-muted-foreground">
                  {t(
                    "settings.memoryUseInRepliesDesc",
                    "Enable the recommended low-latency recall preset for the default agent. Relevant memories can be brought into answers, and recall still skips safely on timeout.",
                  )}
                </p>
                <p className="mt-1 text-[11px] text-muted-foreground/80">
                  {t("settings.memoryUseInRepliesTarget", {
                    defaultValue: "Default agent: {{agent}}",
                    agent: activeMemoryAgentName,
                  })}
                </p>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {activeMemorySummary.map((item) => (
                    <span
                      key={item.id}
                      className="inline-flex items-center gap-1 rounded border border-border/60 bg-secondary/40 px-1.5 py-0.5 text-[10px] text-muted-foreground"
                    >
                      <span>{activeMemorySummaryLabel(item.id)}</span>
                      <span className="font-medium text-foreground">
                        {"enabled" in item ? (item.enabled ? onLabel : offLabel) : item.value}
                      </span>
                    </span>
                  ))}
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {activeMemoryReadiness.map((item) => (
                    <span
                      key={item.id}
                      className={cn(
                        "inline-flex max-w-full items-center rounded border px-1.5 py-0.5 text-left text-[10px] font-medium leading-snug whitespace-normal",
                        activeMemoryReadinessClassName(item.tone),
                      )}
                    >
                      {activeMemoryReadinessLabel(item.id)}
                    </span>
                  ))}
                </div>
                {activeMemoryError && (
                  <p className="mt-2 text-xs text-destructive">
                    {activeMemoryError}
                  </p>
                )}
              </div>
              <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  onClick={() =>
                    requestMemoryScopeFocus({
                      kind: "agent",
                      id: activeMemoryAgentId,
                      agentTab: "memory",
                    })
                  }
                >
                  <Settings className="mr-1.5 h-3.5 w-3.5" />
                  {t("settings.memoryUseInRepliesTune", "Tune")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  variant={activeMemoryUsesRecommended ? "outline" : "default"}
                  disabled={
                    activeMemoryLoading || activeMemorySaving || activeMemoryUsesRecommended
                  }
                  onClick={applyRecommendedDefaultAgentActiveMemory}
                >
                  {activeMemorySaving && (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  )}
                  {activeMemoryUsesRecommended
                    ? t("settings.memoryUseInRepliesApplied", "Recommended on")
                    : t("settings.memoryUseInRepliesAction", "Use recommended")}
                </Button>
              </div>
            </div>
          </div>
        )}

        {!isAgentMode && (
          <div className="grid gap-3 md:grid-cols-3">
            <button
              type="button"
              onClick={() => onSelectTab("manage")}
              className="rounded-lg border border-border/60 bg-card px-3 py-3 text-left transition-colors hover:bg-muted/40"
            >
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span>{t("settings.memoryRecentLearned")}</span>
                <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px]">7d</span>
              </div>
              <div className="mt-2 text-2xl font-semibold tabular-nums">
                {activitySummary.learned7d}
              </div>
            </button>
            <button
              type="button"
              onClick={() => onOpenClaims({ statusFilter: "needs_review" })}
              disabled={!data.effectiveExtractClaims}
              className="rounded-lg border border-border/60 bg-card px-3 py-3 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span>{t("settings.claims.status.needs_review")}</span>
                <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px]">
                  {t("settings.memoryTabs.claims")}
                </span>
              </div>
              <div
                className={cn(
                  "mt-2 text-2xl font-semibold tabular-nums",
                  pendingClaimsError && "text-amber-700 dark:text-amber-300",
                )}
              >
                {pendingClaimsCountLabel}
              </div>
            </button>
            <button
              type="button"
              onClick={() => onOpenClaims({ statusFilter: "all" })}
              disabled={!data.effectiveExtractClaims}
              className="rounded-lg border border-border/60 bg-card px-3 py-3 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span>{t("settings.memoryRecentCorrections")}</span>
                <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px]">7d</span>
              </div>
              <div className="mt-2 text-2xl font-semibold tabular-nums">
                {activitySummary.rejected7d}
              </div>
            </button>
          </div>
        )}

        {!isAgentMode && recentSourceRows.length > 0 && (
          <div className="rounded-lg border border-border/60 bg-card px-3 py-3">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-sm font-medium">
                  {t("settings.memoryRecentSourceBreakdown", "Recent memory sources")}
                </div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {t(
                    "settings.memoryRecentSourceBreakdownDesc",
                    "Where newly learned or updated memories came from in the last 7 days.",
                  )}
                </div>
              </div>
              <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                7d
              </span>
            </div>
            <div className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
              {recentSourceRows.map((row) => (
                <div
                  key={row.key}
                  className="rounded-md border border-border/50 bg-background/60 px-2.5 py-2"
                >
                  <div className="flex items-center justify-between gap-2 text-xs">
                    <span className="truncate text-muted-foreground">{row.label}</span>
                    <span className="font-medium tabular-nums">{row.count}</span>
                  </div>
                  <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-primary/70"
                      style={{ width: `${row.pct}%` }}
                    />
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,360px)]">
          <div className="space-y-3 rounded-lg border border-border/60 bg-card p-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-medium">{t("settings.memoryTabs.manage")}</div>
                <div className="mt-0.5 text-xs text-muted-foreground">
                  {t("settings.memoryDesc")}
                </div>
              </div>
              <Button size="sm" onClick={data.startAdd}>
                {t("settings.memoryAdd")}
              </Button>
            </div>
            {typeRows.length > 0 ? (
              <div className="space-y-3">
                <div className="grid gap-2 sm:grid-cols-2">
                  {typeRows.map(({ type, Icon, count }) => (
                    <div
                      key={type}
                      className="flex min-w-0 items-center gap-2 rounded-md border border-border/50 bg-background/50 px-3 py-2"
                    >
                      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                      <span className="min-w-0 flex-1 truncate text-sm">
                        {t(`settings.memoryType_${type}`)}
                      </span>
                      <span className="text-sm font-medium tabular-nums">{count}</span>
                    </div>
                  ))}
                </div>
                {sourceRows.length > 0 && (
                  <div className="space-y-2">
                    <div className="text-xs font-medium text-muted-foreground">
                      {t("settings.memorySources", "来源")}
                    </div>
                    <div className="space-y-1.5">
                      {sourceRows.map(({ source, count, pct }) => (
                        <div key={source} className="space-y-1">
                          <div className="flex items-center justify-between gap-3 text-xs">
                            <span className="truncate text-muted-foreground">
                              {t(`settings.memorySource_${source}`)}
                            </span>
                            <span className="font-medium tabular-nums">
                              {count} · {pct}%
                            </span>
                          </div>
                          <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                            <div
                              className="h-full rounded-full bg-primary/70"
                              style={{ width: `${pct}%` }}
                            />
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            ) : (
              <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                {t("settings.memoryEmpty")}
              </div>
            )}
            <Button variant="outline" size="sm" onClick={() => onSelectTab("manage")}>
              {t("settings.memoryTabs.manage")}
              <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </div>

          <div className="space-y-3 rounded-lg border border-border/60 bg-card p-4">
            <div className="text-sm font-medium">{t("settings.memorySearchTuning")}</div>
            <div className="space-y-2 text-xs">
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">{t("settings.memoryEmbedding")}</span>
                <span className="flex items-center gap-1.5 font-medium">
                  {embeddingReady ? (
                    <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
                  ) : (
                    <Settings className="h-3.5 w-3.5 text-muted-foreground" />
                  )}
                  {embeddingReady ? t("settings.memoryVectorEnabled") : offLabel}
                </span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">
                  {t("settings.memoryStatsVec", { pct: vectorPct })}
                </span>
                <span className="font-medium tabular-nums">
                  {stats ? `${stats.withEmbedding}/${stats.total}` : "0/0"}
                </span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">
                  {t("settings.memoryEmbeddingProvider", "Provider")}
                </span>
                <span className="min-w-0 truncate font-medium">
                  {embeddingProviderStatus}
                  {embeddingProviderCapabilities.length > 0
                    ? ` · ${embeddingProviderCapabilities.join(", ")}`
                    : ""}
                </span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">
                  {t("settings.memoryExternalProviders", "External providers")}
                </span>
                <span className="min-w-0 truncate font-medium">{externalProviderStatus}</span>
              </div>
              <div
                className={cn(
                  "rounded-md bg-muted/35 px-2 py-1.5 text-[11px] leading-snug",
                  externalProviderOverviewTone,
                )}
              >
                <div>{externalProviderOverviewDetail}</div>
                {externalProviderBlockChips.length > 0 && (
                  <div className="mt-1 flex flex-wrap items-center gap-1">
                    <span className="text-muted-foreground">
                      {t("settings.memoryExternalProviderBlockedBy", "Blocked by")}
                    </span>
                    {externalProviderBlockChips.map((chip) => (
                      <span
                        key={chip.reason}
                        className="rounded bg-background/80 px-1.5 py-0.5 font-mono text-[10px]"
                      >
                        {chip.label}
                      </span>
                    ))}
                    {hiddenExternalProviderBlockReasonCount > 0 && (
                      <span className="text-muted-foreground">
                        {t("settings.memoryExternalProviderMoreBlockReasons", {
                          defaultValue: "+{{count}} more in health report",
                          count: hiddenExternalProviderBlockReasonCount,
                        })}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
            <div className="rounded-md border border-border/50 bg-background/60 px-3 py-2 text-xs">
              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  {memoryHealthLoadError ||
                  memoryHealth?.status === "warning" ||
                  memoryHealth?.status === "error" ? (
                    <AlertTriangle className={`h-3.5 w-3.5 ${memoryHealthTone.icon}`} />
                  ) : (
                    <CheckCircle2 className={`h-3.5 w-3.5 ${memoryHealthTone.icon}`} />
                  )}
                  <span className="font-medium">
                    {t("settings.memoryHealthTitle", "Memory health")}
                  </span>
                  {healthLoading && (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  )}
                </div>
                <span className={`rounded px-1.5 py-0.5 text-[10px] ${memoryHealthTone.badge}`}>
                  {memoryHealthStatusText}
                </span>
              </div>
              {memoryHealth ? (
                <div className="mt-2 space-y-2">
                  <div className="grid grid-cols-2 gap-2 text-[10px] text-muted-foreground sm:grid-cols-4">
                    <div>
                      <div className="font-medium text-foreground tabular-nums">
                        {memoryHealth.memoriesPendingEmbedding}
                      </div>
                      <div className="truncate">
                        {t("settings.memoryHealthPendingEmbeddings", "Need embed")}
                      </div>
                    </div>
                    <div>
                      <div className="font-medium text-foreground tabular-nums">
                        {keywordIndexGaps}
                      </div>
                      <div className="truncate">
                        {t("settings.memoryHealthFtsMissing", "FTS gaps")}
                      </div>
                    </div>
                    <div>
                      <div className="font-medium text-foreground tabular-nums">
                        {memoryGraphOrphans}
                      </div>
                      <div className="truncate">{t("settings.memoryHealthOrphans", "Orphans")}</div>
                    </div>
                    <div>
                      <div className="font-medium text-foreground tabular-nums">
                        {experienceMemoryTotal}
                      </div>
                      <div className="truncate">
                        {t("settings.memoryHealthExperience", "Experience")}
                      </div>
                    </div>
                  </div>
                  {deepResolverHealthSummary && (
                    <div
                      className={`rounded border px-2 py-1.5 text-[11px] ${
                        deepResolverHealthSummary.tone === "blocked"
                          ? "border-amber-500/20 bg-amber-500/5 text-amber-700 dark:text-amber-300"
                          : deepResolverHealthSummary.tone === "backlog"
                            ? "border-sky-500/20 bg-sky-500/5 text-sky-700 dark:text-sky-300"
                            : "border-emerald-500/20 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300"
                      }`}
                    >
                      <div className="font-medium">{deepResolverHealthSummary.statusText}</div>
                      {deepResolverHealthSummary.detailText && (
                        <div className="mt-0.5 text-muted-foreground">
                          {deepResolverHealthSummary.detailText}
                        </div>
                      )}
                    </div>
                  )}
                  {topHealthIssues.length > 0 ? (
                    <div className="space-y-1">
                      {topHealthIssues.map((issue, index) => (
                        <div
                          key={`${issue.code}-${index}`}
                          className="rounded border border-amber-500/20 bg-amber-500/5 px-2 py-1 text-[11px] text-muted-foreground"
                        >
                          <div className="font-medium text-foreground">{issue.message}</div>
                          {issue.action && <div className="mt-0.5">{issue.action}</div>}
                        </div>
                      ))}
                      {hiddenHealthIssueCount > 0 && (
                        <div className="px-2 text-[11px] text-muted-foreground">
                          {t("settings.memoryHealthMoreIssues", {
                            defaultValue: "+{{count}} more issue(s) in copied report",
                            count: hiddenHealthIssueCount,
                          })}
                        </div>
                      )}
                    </div>
                  ) : (
                    <div className="text-[11px] text-muted-foreground">
                      {t(
                        "settings.memoryHealthAllGood",
                        "Indexes, embeddings, and claim graph look healthy.",
                      )}
                    </div>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="h-7 w-full text-[11px]"
                    onClick={() => void copyMemoryHealthDiagnostics()}
                  >
                    <Copy className="mr-1.5 h-3.5 w-3.5" />
                    {t("settings.memoryHealthCopyReport", "Copy health report")}
                  </Button>
                  {repairPolicy === "snapshot_first" && (
                    <div className="rounded-md border border-amber-500/20 bg-amber-500/5 px-2 py-1.5 text-[11px] leading-snug text-amber-700 dark:text-amber-300">
                      {t(
                        "settings.memoryRepairSnapshotFirstNotice",
                        "Database integrity needs attention. Create a snapshot first; other repairs are hidden until the data is preserved.",
                      )}
                    </div>
                  )}
                  {repairActionItems
                    .filter((item) => item.visible)
                    .map((item) => (
                      <div key={item.action} className="space-y-1">
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="h-7 w-full text-[11px]"
                          onClick={item.onClick}
                          disabled={item.loading}
                        >
                          {item.loading && (
                            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                          )}
                          {item.label}
                        </Button>
                        {item.reason && (
                          <div className="px-1 text-[11px] leading-snug text-muted-foreground">
                            {item.reason}
                          </div>
                        )}
                      </div>
                    ))}
                  {currentDbSnapshotPath && (
                    <div className="rounded-md border border-emerald-500/20 bg-emerald-500/5 px-2 py-1.5 text-[11px] leading-snug">
                      <div className="flex items-start justify-between gap-2">
                        <span className="font-medium text-emerald-700 dark:text-emerald-300">
                          {t("settings.memoryRepairDbSnapshotLatest", "Latest safety snapshot")}
                        </span>
                        <div className="flex shrink-0 flex-wrap justify-end gap-1">
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => void checkDbSnapshotRestorePreview()}
                            disabled={checkingDbSnapshotRestore}
                          >
                            {checkingDbSnapshotRestore ? (
                              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                            ) : (
                              <CheckCircle2 className="mr-1 h-3 w-3" />
                            )}
                            {t(
                              "settings.memoryRepairDbSnapshotCheckRestore",
                              "Check restore plan",
                            )}
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => void copyDbSnapshotPath()}
                          >
                            <Copy className="mr-1 h-3 w-3" />
                            {t("settings.memoryRepairDbSnapshotCopyPath", "Copy path")}
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="h-6 px-1.5 text-[10px]"
                            onClick={() => void copyDbSnapshotVerification()}
                          >
                            <Copy className="mr-1 h-3 w-3" />
                            {t(
                              "settings.memoryRepairDbSnapshotCopyVerification",
                              "Copy verification",
                            )}
                          </Button>
                        </div>
                      </div>
                      <div
                        className="mt-1 truncate font-mono text-[10px] text-muted-foreground"
                        title={currentDbSnapshotPath}
                      >
                        {currentDbSnapshotPath}
                      </div>
                      <div className="mt-1 text-[10px] text-muted-foreground">
                        {currentDbSnapshotSummary
                          ? t("settings.memoryRepairDbSnapshotSummary", {
                              defaultValue:
                                "{{count}} file(s) captured - {{name}} {{size}} sha256 {{sha}}",
                              count: currentDbSnapshotSummary.count,
                              name: currentDbSnapshotSummary.name,
                              size: currentDbSnapshotSummary.size,
                              sha: currentDbSnapshotSummary.sha,
                            })
                          : t(
                              "settings.memoryRepairDbSnapshotNoMetadata",
                              "No file metadata returned.",
                            )}
                      </div>
                      {currentDbSnapshotIncomplete && (
                        <div className="mt-1 rounded border border-amber-500/20 bg-amber-500/5 px-2 py-1 text-[10px] text-amber-700 dark:text-amber-300">
                          <div>
                            {t(
                              "settings.memoryRepairDbSnapshotIncomplete",
                              "Latest snapshot is not fully verifiable. Create a fresh snapshot before recovery.",
                            )}
                          </div>
                          {currentDbSnapshotIssues[0] && (
                            <div
                              className="mt-0.5 truncate font-mono text-[10px]"
                              title={currentDbSnapshotIssues[0]}
                            >
                              {currentDbSnapshotIssues[0]}
                            </div>
                          )}
                        </div>
                      )}
                      {activeDbSnapshotRestorePreview && (
                        <div
                          className={cn(
                            "mt-1 rounded border px-2 py-1 text-[10px]",
                            activeDbSnapshotRestorePreview.canRestore
                              ? "border-emerald-500/20 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300"
                              : "border-amber-500/20 bg-amber-500/5 text-amber-700 dark:text-amber-300",
                          )}
                        >
                          <div>
                            {activeDbSnapshotRestorePreview.canRestore
                              ? t(
                                  "settings.memoryRepairDbSnapshotRestoreReady",
                                  "Restore preflight passed. This snapshot can be used by a future explicit restore flow.",
                                )
                              : t(
                                  "settings.memoryRepairDbSnapshotRestoreBlocked",
                                  "Restore preflight blocked this snapshot. Create a fresh snapshot before recovery.",
                                )}
                          </div>
                          <div className="mt-0.5 text-muted-foreground">
                            {t("settings.memoryRepairDbSnapshotRestoreFiles", {
                              defaultValue: "{{count}} file(s) checked",
                              count: activeDbSnapshotRestorePreview.files.length,
                            })}
                            {" · "}
                            {activeDbSnapshotRestorePreview.status}
                            {" · "}
                            {t("settings.memoryRepairDbSnapshotRestoreQuickCheck", {
                              defaultValue: "quick_check {{value}}",
                              value: activeDbSnapshotRestorePreview.quickCheck || "-",
                            })}
                          </div>
                          {dbSnapshotRestoreIssue && (
                            <div
                              className="mt-0.5 truncate font-mono text-[10px]"
                              title={dbSnapshotRestoreIssue}
                            >
                              {dbSnapshotRestoreIssue}
                            </div>
                          )}
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="mt-1 h-6 px-1.5 text-[10px]"
                            onClick={() => void copyDbSnapshotRestorePreview()}
                          >
                            <Copy className="mr-1 h-3 w-3" />
                            {t(
                              "settings.memoryRepairDbSnapshotCopyRestorePreview",
                              "Copy preflight report",
                            )}
                          </Button>
                          {activeDbSnapshotRestorePreview.canRestore && (
                            <Button
                              type="button"
                              size="sm"
                              variant="destructive"
                              className="mt-1 ml-1 h-6 px-1.5 text-[10px]"
                              onClick={() => void restoreDbSnapshot()}
                              disabled={restoringDbSnapshot}
                            >
                              {restoringDbSnapshot ? (
                                <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                              ) : null}
                              {t(
                                "settings.memoryRepairDbSnapshotRestoreNow",
                                "Restore snapshot",
                              )}
                            </Button>
                          )}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              ) : memoryHealthLoadError ? (
                <div className="mt-2 rounded-md border border-destructive/25 bg-destructive/5 px-2 py-2 text-[11px]">
                  <div className="font-medium text-destructive">
                    {memoryHealthLoadError.title}
                  </div>
                  {memoryHealthLoadError.description && (
                    <div className="mt-1 break-words text-muted-foreground">
                      {memoryHealthLoadError.description}
                    </div>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="mt-2 h-7 px-2 text-[11px]"
                    disabled={healthLoading}
                    onClick={() => void loadMemoryHealth()}
                  >
                    {healthLoading ? (
                      <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                    )}
                    {t("settings.memoryOverviewRetry", "Retry")}
                  </Button>
                </div>
              ) : (
                <div className="mt-2 text-[11px] text-muted-foreground">
                  {healthLoading
                    ? t("common.loading")
                    : t("settings.memoryHealthUnavailable", "Health check unavailable.")}
                </div>
              )}
            </div>
            <Button variant="outline" size="sm" onClick={() => onSelectTab("settings")}>
              {t("settings.memoryTabs.settings")}
              <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
            </Button>
          </div>
        </div>

        {!isAgentMode && (
          <div className="grid gap-4 xl:grid-cols-3">
            {insightsLoadWarning && (
              <div className="flex flex-col gap-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-xs sm:flex-row sm:items-start sm:justify-between xl:col-span-3">
                <div className="flex min-w-0 items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-300" />
                  <div className="min-w-0">
                    <div className="font-medium text-foreground">{insightsLoadWarning.title}</div>
                    <div className="mt-0.5 break-words text-muted-foreground">
                      {insightsLoadWarning.description}
                    </div>
                  </div>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 self-start px-2 text-[11px]"
                  disabled={insightsLoading}
                  onClick={() => void loadClaimInsights()}
                >
                  {insightsLoading ? (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                  )}
                  {t("settings.memoryOverviewRetry", "Retry")}
                </Button>
              </div>
            )}
            <div className="rounded-lg border border-border/60 bg-card p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <UserRound className="h-4 w-4 text-primary" />
                    <span>{t("settings.memoryKnowsAboutMe")}</span>
                    {insightsLoading && (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.memoryKnowsAboutMeDesc")}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onOpenClaims({ statusFilter: "active", claimType: "profile" })}
                  disabled={!data.effectiveExtractClaims}
                >
                  {t("settings.memoryTabs.claims")}
                  <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="mt-3 space-y-2">
                {!data.effectiveExtractClaims ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {t("settings.memoryClaimInsightsDisabled")}
                  </div>
                ) : profileClaims.length === 0 && primaryProfileSnapshotLines.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {insightsLoading ? t("common.loading") : t("settings.memoryKnowsAboutMeEmpty")}
                  </div>
                ) : (
                  <>
                    {primaryProfileSnapshotLines.length > 0 && (
                      <button
                        type="button"
                        onClick={() => onSelectTab("profile")}
                        className="block w-full min-w-0 rounded-md border border-primary/20 bg-primary/5 px-3 py-2 text-left text-xs transition-colors hover:bg-primary/10"
                      >
                        <div className="flex min-w-0 items-center justify-between gap-2">
                          <span className="truncate font-medium">
                            {t("settings.memoryTabs.profile")}
                          </span>
                          <span className="shrink-0 text-[10px] text-muted-foreground">
                            {primaryProfileSnapshot
                              ? t("settings.profile.version", {
                                  version: primaryProfileSnapshot.version,
                                })
                              : ""}
                          </span>
                        </div>
                        <ul className="mt-1 space-y-0.5 text-[11px] leading-relaxed text-muted-foreground">
                          {primaryProfileSnapshotLines.map((line, index) => (
                            <li key={`${index}-${line}`} className="truncate">
                              {line}
                            </li>
                          ))}
                        </ul>
                      </button>
                    )}
                    {profileClaims.map((claim) => (
                      <button
                        key={claim.id}
                        type="button"
                        onClick={() =>
                          onOpenClaims({
                            statusFilter: "active",
                            claimType: claim.claimType,
                            scopeType: claim.scopeType,
                            scopeId: claim.scopeId,
                            selectedId: claim.id,
                          })
                        }
                        className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                      >
                        <div className="truncate font-medium">{claim.content}</div>
                        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                          <span>{claimTypeLabel(claim.claimType)}</span>
                          <span>{claimScopeLabel(claim)}</span>
                          <span>{(claim.confidence * 100).toFixed(0)}%</span>
                        </div>
                      </button>
                    ))}
                  </>
                )}
              </div>
            </div>

            <div className="rounded-lg border border-border/60 bg-card p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <FolderKanban className="h-4 w-4 text-primary" />
                    <span>{t("settings.memoryProjectMemory")}</span>
                    {insightsLoading && (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.memoryProjectMemoryDesc")}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    onOpenClaims({ statusFilter: "active", claimType: projectClaimType })
                  }
                  disabled={!data.effectiveExtractClaims}
                >
                  {t("settings.memoryTabs.claims")}
                  <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="mt-3 space-y-2">
                {!data.effectiveExtractClaims ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {t("settings.memoryClaimInsightsDisabled")}
                  </div>
                ) : projectClaims.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {insightsLoading ? t("common.loading") : t("settings.memoryProjectMemoryEmpty")}
                  </div>
                ) : (
                  projectClaimGroups.map((group) => (
                    <div key={group.key} className="space-y-1">
                      <div className="flex min-w-0 items-center justify-between gap-2 px-1 text-[10px] text-muted-foreground">
                        <div className="flex min-w-0 items-center gap-1.5">
                          <span className="truncate font-medium text-foreground">
                            {group.label}
                          </span>
                          {group.scopeType === "project" && group.scopeId && (
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon"
                              className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                              onClick={() =>
                                requestMemoryScopeFocus({ kind: "project", id: group.scopeId! })
                              }
                              title={t("project.openProject")}
                              aria-label={t("project.openProject")}
                            >
                              <ExternalLink className="h-3.5 w-3.5" />
                            </Button>
                          )}
                        </div>
                        <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 tabular-nums">
                          {group.total}
                        </span>
                      </div>
                      {group.claims.map((claim) => (
                        <button
                          key={claim.id}
                          type="button"
                          onClick={() =>
                            onOpenClaims({
                              statusFilter: "active",
                              claimType: projectClaimType,
                              scopeType: claim.scopeType,
                              scopeId: claim.scopeId,
                              selectedId: claim.id,
                            })
                          }
                          className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                        >
                          <div className="truncate font-medium">{claim.content}</div>
                          <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                            <span>{claimTypeLabel(claim.claimType)}</span>
                            <span>{(claim.confidence * 100).toFixed(0)}%</span>
                          </div>
                        </button>
                      ))}
                    </div>
                  ))
                )}
              </div>
            </div>

            <div className="rounded-lg border border-border/60 bg-card p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <Activity className="h-4 w-4 text-primary" />
                    <span>{t("dashboard.dreaming.runs.title")}</span>
                    {activityLoading && (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("dashboard.dreaming.subtitle")}
                  </div>
                </div>
                <Button variant="ghost" size="sm" onClick={() => onSelectTab("dreaming")}>
                  {t("settings.memoryTabs.dreaming")}
                  <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="mt-3 space-y-2">
                {recentDreamingRuns.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {activityLoading ? t("common.loading") : t("dashboard.dreaming.runs.empty")}
                  </div>
                ) : (
                  recentDreamingRuns.map((run) => (
                    <button
                      key={run.id}
                      type="button"
                      onClick={() => onSelectTab("dreaming")}
                      className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                    >
                      <div className="flex min-w-0 items-center gap-2">
                        <span
                          className={`h-2 w-2 shrink-0 rounded-full ${dreamingStatusDot(
                            run.status,
                          )}`}
                        />
                        <span className="truncate font-medium">
                          {t(`dashboard.dreaming.trigger.${run.trigger}`, run.trigger)}
                          {" · "}
                          {t(`dashboard.dreaming.runs.status.${run.status}`, run.status)}
                        </span>
                        <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                          {formatActivityTime(run.finishedAt || run.startedAt)}
                        </span>
                      </div>
                      <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                        <span>
                          {t("dashboard.dreaming.scanned", {
                            count: run.candidatesScanned,
                          })}
                        </span>
                        <span>
                          {t("dashboard.dreaming.nominated", {
                            count: run.candidatesNominated,
                          })}
                        </span>
                        <span>
                          {t("dashboard.dreaming.promoted", {
                            count: run.promotedCount,
                          })}
                        </span>
                      </div>
                    </button>
                  ))
                )}
              </div>
            </div>
          </div>
        )}

        {!isAgentMode && (
          <div className="rounded-lg border border-border/60 bg-card p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-medium">
                  <Workflow className="h-4 w-4 text-primary" />
                  <span>{t("settings.memoryEpisodesTitle", "Experience & workflows")}</span>
                  {activityLoading && (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  )}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {t(
                    "settings.memoryEpisodesDesc",
                    "Reusable lessons and soft procedures captured from finished work.",
                  )}
                </div>
              </div>
              <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setProcedureDialogOpen(true)}
                >
                  <Workflow className="mr-1.5 h-3.5 w-3.5" />
                  {t("settings.memoryProcedureAdd", "Record workflow")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setEpisodeDialogOpen(true)}
                >
                  <History className="mr-1.5 h-3.5 w-3.5" />
                  {t("settings.memoryEpisodeAdd", "Record episode")}
                </Button>
              </div>
            </div>
            <div className="mt-3 flex flex-col gap-2 sm:flex-row">
              <div className="relative min-w-0 flex-1">
                <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={experienceQuery}
                  onChange={(event) => setExperienceQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      void runExperienceSearch()
                    }
                  }}
                  placeholder={t(
                    "settings.memoryExperienceSearchPlaceholder",
                    "Search episodes and workflows...",
                  )}
                  className="h-8 pl-8 text-xs"
                  disabled={experienceSearchLoading}
                />
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  className="h-8"
                  onClick={() => void runExperienceSearch()}
                  disabled={experienceSearchLoading}
                >
                  {experienceSearchLoading ? (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Search className="mr-1.5 h-3.5 w-3.5" />
                  )}
                  {t("common.search")}
                </Button>
                {(experienceHasCustomView || experienceQuery.trim().length > 0) && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-8"
                    onClick={() => void clearExperienceSearch()}
                    disabled={experienceSearchLoading}
                  >
                    <X className="mr-1.5 h-3.5 w-3.5" />
                    {t("common.clear")}
                  </Button>
                )}
              </div>
            </div>
            <div className="mt-2 flex flex-wrap gap-1.5">
              {EXPERIENCE_STATUS_FILTERS.map((status) => (
                <Button
                  key={status}
                  type="button"
                  variant={experienceStatus === status ? "secondary" : "outline"}
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  disabled={experienceSearchLoading}
                  onClick={() => void applyExperienceView({ status })}
                >
                  {experienceStatusLabel(status)}
                </Button>
              ))}
              <span className="mx-0.5 h-7 w-px bg-border/70" />
              {EXPERIENCE_SORTS.map((sort) => (
                <Button
                  key={sort}
                  type="button"
                  variant={experienceSort === sort ? "secondary" : "outline"}
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  disabled={experienceSearchLoading}
                  onClick={() => void applyExperienceView({ sort })}
                >
                  {experienceSortLabel(sort)}
                </Button>
              ))}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              <Button
                type="button"
                variant={experienceScopeFilter.kind === "all" ? "secondary" : "outline"}
                size="sm"
                className="h-7 px-2 text-[11px]"
                disabled={experienceSearchLoading}
                onClick={() =>
                  void applyExperienceView({ scopeFilter: EMPTY_EXPERIENCE_SCOPE_FILTER })
                }
              >
                {t("settings.memoryExperienceScopeAll", "All scopes")}
              </Button>
              <Button
                type="button"
                variant={experienceScopeFilter.kind === "global" ? "secondary" : "outline"}
                size="sm"
                className="h-7 px-2 text-[11px]"
                disabled={experienceSearchLoading}
                onClick={() =>
                  void applyExperienceView({ scopeFilter: { kind: "global", id: "" } })
                }
              >
                {t("dashboard.dreaming.review.scopeGlobal")}
              </Button>
              <Button
                type="button"
                variant={experienceScopeFilter.kind === "agent" ? "secondary" : "outline"}
                size="sm"
                className="h-7 px-2 text-[11px]"
                disabled={experienceSearchLoading || agentOptions.length === 0}
                onClick={() =>
                  void applyExperienceView({
                    scopeFilter: { kind: "agent", id: agentOptions[0]?.id ?? "" },
                  })
                }
              >
                {t("dashboard.dreaming.review.scopeAgent")}
              </Button>
              <Button
                type="button"
                variant={experienceScopeFilter.kind === "project" ? "secondary" : "outline"}
                size="sm"
                className="h-7 px-2 text-[11px]"
                disabled={experienceSearchLoading || projectOptions.length === 0}
                onClick={() =>
                  void applyExperienceView({
                    scopeFilter: { kind: "project", id: projectOptions[0]?.id ?? "" },
                  })
                }
              >
                {t("dashboard.dreaming.review.scopeProject")}
              </Button>
              {experienceScopeFilter.kind !== "all" &&
                experienceScopeFilter.kind !== "global" && (
                  <select
                    value={experienceScopeFilter.id}
                    onChange={(event) =>
                      void applyExperienceView({
                        scopeFilter: {
                          ...experienceScopeFilter,
                          id: event.target.value,
                        },
                      })
                    }
                    disabled={experienceSearchLoading}
                    className="h-7 max-w-[240px] rounded-md border border-input bg-background px-2 text-[11px]"
                  >
                    {(experienceScopeFilter.kind === "agent"
                      ? agentOptions
                      : projectOptions
                    ).map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.name}
                      </option>
                    ))}
                  </select>
                )}
            </div>
            {experienceHasCustomView && (
              <div className="mt-2 text-[11px] text-muted-foreground">
                {experienceHasSearch
                  ? t("settings.memoryExperienceSearchActive", {
                      defaultValue: 'Showing {{total}} matches for "{{query}}"',
                      total: experienceSearchTotal,
                      query: experienceAppliedQuery,
                    })
                  : t("settings.memoryExperienceViewActive", {
                      defaultValue: "Showing {{total}} experience items",
                      total: experienceSearchTotal,
                    })}
              </div>
            )}
            <div className="mt-3 grid gap-3 lg:grid-cols-2">
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2 text-xs font-medium text-muted-foreground">
                  <span>
                    {experienceHasCustomView
                      ? t("settings.memoryEpisodesMatches", "Episodes")
                      : t("settings.memoryEpisodesRecent", "Recent episodes")}
                  </span>
                  {(experienceEpisodeTotal > 0 || recentEpisodes.length > 0) && (
                    <span className="font-mono text-[10px]">
                      {recentEpisodes.length}/{experienceEpisodeTotal}
                    </span>
                  )}
                </div>
                {recentEpisodes.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {activityLoading || experienceSearchLoading
                      ? t("common.loading")
                      : experienceHasCustomView
                        ? t("settings.memoryEpisodesNoMatches", "No matching episodes.")
                        : t("settings.memoryEpisodesEmpty", "No episodes yet.")}
                  </div>
                ) : (
                  <>
                    {recentEpisodes.map((episode) => (
                      <div
                        id={experienceDomId("episode", episode.id)}
                        key={episode.id}
                        className={cn(
                          "rounded-md border border-border/50 bg-background/70 px-3 py-2 text-xs transition-colors",
                          experienceFocusHighlight?.kind === "episode" &&
                            experienceFocusHighlight.id === episode.id &&
                            "border-primary/40 bg-primary/5 ring-1 ring-primary/20",
                        )}
                      >
                        <div className="flex min-w-0 items-start justify-between gap-2">
                          <div className="min-w-0">
                            <div className="truncate font-medium">{episode.title}</div>
                            <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                              {episode.lesson || episode.outcome || episode.situation}
                            </div>
                          </div>
                          <span className="shrink-0 text-[10px] text-muted-foreground">
                            {formatActivityTime(episode.updatedAt)}
                          </span>
                        </div>
                        <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                          <span className="max-w-full truncate rounded bg-secondary px-1.5 py-0.5">
                            {memoryScopeLabel(episode.scope)}
                          </span>
                          {episode.tags.slice(0, 3).map((tag) => (
                            <span key={tag} className="rounded bg-secondary px-1.5 py-0.5">
                              {tag}
                            </span>
                          ))}
                          {episode.actions.length > 0 && (
                            <span className="rounded bg-secondary px-1.5 py-0.5">
                              {t("settings.memoryEpisodeActions", {
                                count: episode.actions.length,
                              })}
                            </span>
                          )}
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="ml-auto h-6 px-2 text-[10px]"
                            onClick={() =>
                              openExperienceDetail({ kind: "episode", record: episode })
                            }
                          >
                            <Eye className="mr-1 h-3 w-3" />
                            {t("settings.memoryExperienceDetails", "Details")}
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="h-6 px-2 text-[10px]"
                            onClick={() => void promoteEpisode(episode.id)}
                          >
                            {t("settings.memoryEpisodePromote", "Promote")}
                          </Button>
                        </div>
                      </div>
                    ))}
                    {experienceEpisodeHasMore && (
                      <div className="flex justify-center pt-1">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-8 text-xs"
                          disabled={experienceLoadingMore !== null}
                          onClick={() => void loadMoreExperience("episode")}
                        >
                          {experienceLoadingMore === "episode" && (
                            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                          )}
                          {t("settings.memoryExperienceLoadMoreEpisodes", "Load more episodes")}
                        </Button>
                      </div>
                    )}
                  </>
                )}
              </div>
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2 text-xs font-medium text-muted-foreground">
                  <span>
                    {experienceHasCustomView
                      ? t("settings.memoryProceduresMatches", "Workflows")
                      : t("settings.memoryProceduresRecent", "Procedures")}
                  </span>
                  {(experienceProcedureTotal > 0 || recentProcedures.length > 0) && (
                    <span className="font-mono text-[10px]">
                      {recentProcedures.length}/{experienceProcedureTotal}
                    </span>
                  )}
                </div>
                {recentProcedures.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {activityLoading || experienceSearchLoading
                      ? t("common.loading")
                      : experienceHasCustomView
                        ? t("settings.memoryProceduresNoMatches", "No matching workflows.")
                        : t("settings.memoryProceduresEmpty", "No procedures yet.")}
                  </div>
                ) : (
                  <>
                    {recentProcedures.map((procedure) => {
                      const guidance = procedureGuidanceInfo(procedure)
                      const GuidanceIcon =
                        guidance.kind === "eligible" ? CheckCircle2 : AlertTriangle
                      return (
                        <div
                          id={experienceDomId("procedure", procedure.id)}
                          key={procedure.id}
                          className={cn(
                            "rounded-md border border-border/50 bg-background/70 px-3 py-2 text-xs transition-colors",
                            experienceFocusHighlight?.kind === "procedure" &&
                              experienceFocusHighlight.id === procedure.id &&
                              "border-primary/40 bg-primary/5 ring-1 ring-primary/20",
                          )}
                        >
                          <div className="flex min-w-0 items-start justify-between gap-2">
                            <div className="min-w-0">
                              <div className="truncate font-medium">{procedure.title}</div>
                              <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                                {procedure.trigger}
                              </div>
                            </div>
                            <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                              {(procedure.confidence * 100).toFixed(0)}%
                            </span>
                          </div>
                          <div className="mt-1 line-clamp-2 text-[11px] text-muted-foreground">
                            {procedure.stepsMarkdown}
                          </div>
                          <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                            <span className="max-w-full truncate rounded bg-secondary px-1.5 py-0.5">
                              {memoryScopeLabel(procedure.scope)}
                            </span>
                            <span
                              className={cn(
                                "inline-flex max-w-full items-center gap-1 truncate rounded border px-1.5 py-0.5",
                                guidance.className,
                              )}
                              title={guidance.description}
                            >
                              <GuidanceIcon className="h-3 w-3 shrink-0" />
                              <span className="truncate">{guidance.label}</span>
                            </span>
                            {procedure.tags.slice(0, 3).map((tag) => (
                              <span key={tag} className="rounded bg-secondary px-1.5 py-0.5">
                                {tag}
                              </span>
                            ))}
                            <Button
                              type="button"
                              size="sm"
                              variant="ghost"
                              className="ml-auto h-6 px-2 text-[10px]"
                              onClick={() =>
                                openExperienceDetail({ kind: "procedure", record: procedure })
                              }
                            >
                              <Eye className="mr-1 h-3 w-3" />
                              {t("settings.memoryExperienceDetails", "Details")}
                            </Button>
                          </div>
                        </div>
                      )
                    })}
                    {experienceProcedureHasMore && (
                      <div className="flex justify-center pt-1">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-8 text-xs"
                          disabled={experienceLoadingMore !== null}
                          onClick={() => void loadMoreExperience("procedure")}
                        >
                          {experienceLoadingMore === "procedure" && (
                            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                          )}
                          {t("settings.memoryExperienceLoadMoreProcedures", "Load more workflows")}
                        </Button>
                      </div>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>
        )}

        {!isAgentMode && (
          <div
            className={`rounded-lg border p-4 ${
              pendingClaims.length > 0
                ? "border-sky-500/30 bg-sky-500/5"
                : "border-border/60 bg-card"
            }`}
          >
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-medium">
                  <Sparkles className="h-4 w-4 text-primary" />
                  <span>
                    {t("settings.claims.status.needs_review")} · {t("settings.memoryTabs.claims")}
                  </span>
                  {pendingLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  ) : pendingClaimsError ? (
                    <AlertTriangle className="h-3.5 w-3.5 text-amber-600 dark:text-amber-300" />
                  ) : (
                    <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px] tabular-nums text-muted-foreground">
                      {pendingClaims.length}
                    </span>
                  )}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {data.effectiveExtractClaims
                    ? t("settings.claims.explainer.reviewDesc")
                    : t("settings.memoryExtractClaimsDesc")}
                </div>
              </div>
              <Button
                variant={pendingClaims.length > 0 ? "default" : "outline"}
                size="sm"
                onClick={() => onOpenClaims({ statusFilter: "needs_review" })}
                disabled={!data.effectiveExtractClaims}
              >
                {t("settings.memoryTabs.claims")}
                <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
              </Button>
            </div>

            {canReviewClaims && (
              <div className="mt-3">
                {pendingClaimsError ? (
                  <div className="flex flex-col gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs sm:flex-row sm:items-start sm:justify-between">
                    <div className="flex min-w-0 items-start gap-2">
                      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
                      <div className="min-w-0">
                        <div className="font-medium text-foreground">{pendingClaimsError.title}</div>
                        {pendingClaimsError.description && (
                          <div className="mt-0.5 break-words text-muted-foreground">
                            {pendingClaimsError.description}
                          </div>
                        )}
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-7 self-start px-2 text-[11px]"
                      disabled={pendingLoading}
                      onClick={() => void loadPendingClaims()}
                    >
                      {pendingLoading ? (
                        <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                      )}
                      {t("settings.memoryOverviewRetry", "Retry")}
                    </Button>
                  </div>
                ) : pendingLoading && pendingClaims.length === 0 ? (
                  <div className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("common.loading")}
                  </div>
                ) : pendingClaims.length === 0 ? (
                  <div className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                    <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
                    {t("dashboard.dreaming.review.queueEmpty", "No claims need review")}
                  </div>
                ) : (
                  <div className="grid gap-2 md:grid-cols-2">
                    {pendingClaims.slice(0, 4).map((claim) => (
                      <button
                        key={claim.id}
                        type="button"
                        onClick={() =>
                          onOpenClaims({
                            statusFilter: "needs_review",
                            claimType: claim.claimType,
                            scopeType: claim.scopeType,
                            scopeId: claim.scopeId,
                            selectedId: claim.id,
                          })
                        }
                        className="min-w-0 rounded-md border border-border/60 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                      >
                        <div className="truncate font-medium">{claim.content}</div>
                        <div className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground">
                          {claim.claimType} · {(claim.confidence * 100).toFixed(0)}%
                        </div>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {!isAgentMode && (
          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-lg border border-border/60 bg-card p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <History className="h-4 w-4 text-primary" />
                    <span>{t("settings.memoryRecentActivity", "Recent memory activity")}</span>
                    {activityLoading && (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t(
                      "settings.memoryRecentActivityDesc",
                      "Auditable memory and workflow changes, with older stores falling back to newest memories.",
                    )}
                  </div>
                </div>
                <div className="flex shrink-0 flex-wrap items-center justify-end gap-1.5">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => void copyMemoryAuditExport()}
                    disabled={
                      activityLoading ||
                      memoryAuditLoading ||
                      memoryAuditExportingAll ||
                      !hasCurrentActivityExport
                    }
                  >
                    <Copy className="mr-1.5 h-3.5 w-3.5" />
                    {t("settings.memoryAuditExport", "Export")}
                  </Button>
                  <Button
                    variant={memoryAuditOpen ? "secondary" : "ghost"}
                    size="sm"
                    onClick={() => {
                      const nextOpen = !memoryAuditOpen
                      setMemoryAuditOpen(nextOpen)
                      if (nextOpen) {
                        resetMemoryAuditSearchState()
                        updateMemoryAuditFocusUrl({ open: true, query: "", action: "all" })
                      } else {
                        resetMemoryAuditSearchState()
                        updateMemoryAuditFocusUrl({ open: false, query: "", action: "all" })
                      }
                    }}
                  >
                    <Search className="mr-1.5 h-3.5 w-3.5" />
                    {t("common.search")}
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => onSelectTab("manage")}>
                    {t("settings.memoryTabs.manage")}
                    <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
              {memoryAuditOpen && (
                <div className="mt-3 space-y-2 border-t border-border/60 pt-3">
                  <div className="flex flex-col gap-2 sm:flex-row">
                    <div className="relative min-w-0 flex-1">
                      <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        value={memoryAuditQuery}
                        onChange={(event) => setMemoryAuditQuery(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") {
                            void runMemoryAuditSearch()
                          }
                        }}
                        placeholder={t(
                          "settings.memoryAuditSearchPlaceholder",
                          "Search memory activity...",
                        )}
                        className="h-8 pl-8 text-xs"
                      />
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      <Button
                        variant="secondary"
                        size="sm"
                        className="h-8"
                        onClick={() => void runMemoryAuditSearch()}
                        disabled={memoryAuditLoading}
                      >
                        {memoryAuditLoading ? (
                          <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Search className="mr-1.5 h-3.5 w-3.5" />
                        )}
                        {t("common.search")}
                      </Button>
                      {memoryAuditHasFilters && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-8"
                          onClick={clearMemoryAuditSearch}
                        >
                          <X className="mr-1.5 h-3.5 w-3.5" />
                          {t("common.clear")}
                        </Button>
                      )}
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {MEMORY_AUDIT_ACTIONS.map((action) => (
                      <Button
                        key={action}
                        type="button"
                        variant={memoryAuditAction === action ? "secondary" : "outline"}
                        size="sm"
                        className="h-7 px-2 text-[11px]"
                        onClick={() => {
                          setMemoryAuditAction(action)
                          void runMemoryAuditSearch({ action })
                        }}
                      >
                        {memoryAuditActionLabel(action)}
                      </Button>
                    ))}
                  </div>
                  <p className="text-[11px] leading-relaxed text-muted-foreground">
                    {memoryAuditAction === "all"
                      ? t(
                          "settings.memoryAuditScopeHintAll",
                          "All changes searches long-term memories, workflows, and structured-memory decisions.",
                        )
                      : t(
                          "settings.memoryAuditScopeHintMemoryOnly",
                          "Action filters apply to long-term memory history; workflows and structured-memory decisions stay under All changes.",
                        )}
                  </p>
                  {memoryAuditDegradedToast && (
                    <div className="flex flex-col gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs sm:flex-row sm:items-start sm:justify-between">
                      <div className="flex min-w-0 items-start gap-2">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
                        <div className="min-w-0">
                          <div className="font-medium text-foreground">
                            {memoryAuditDegradedToast.title}
                          </div>
                          {memoryAuditDegradedToast.description && (
                            <div className="mt-0.5 break-words text-muted-foreground">
                              {memoryAuditDegradedToast.description}
                            </div>
                          )}
                        </div>
                      </div>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-7 shrink-0 gap-1 px-2 text-xs"
                        disabled={memoryAuditLoading}
                        onClick={() => void runMemoryAuditSearch()}
                      >
                        {memoryAuditLoading ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <RefreshCw className="h-3.5 w-3.5" />
                        )}
                        {t("settings.memoryOverviewRetry", "Retry")}
                      </Button>
                    </div>
                  )}
                  <div className="flex flex-wrap items-center gap-1.5 text-xs">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 gap-1 px-2 text-xs"
                      disabled={memoryAuditExportingAll}
                      onClick={() => void copyAllMemoryAuditExport()}
                    >
                      {memoryAuditExportingAll ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                      {t("settings.memoryAuditExportAll", "Export all")}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 gap-1 px-2 text-xs"
                      onClick={saveMemoryAuditPreset}
                    >
                      <BookmarkPlus className="h-3.5 w-3.5" />
                      {t("settings.memoryFilterPresetSave")}
                    </Button>
                    {memoryAuditPresets.length > 0 && (
                      <>
                        <span className="text-muted-foreground">
                          {t("settings.memoryFilterPresets")}
                        </span>
                        {memoryAuditPresets.map((preset) => {
                          const label = memoryAuditPresetLabel(preset)
                          const active = preset.id === currentMemoryAuditPresetId
                          return (
                            <span
                              key={preset.id}
                              className={`inline-flex max-w-full items-center rounded-md border border-border/70 ${
                                active ? "bg-primary/10 text-foreground" : "bg-background"
                              }`}
                            >
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                className="h-6 min-w-0 max-w-[220px] justify-start truncate px-2 text-xs"
                                title={label}
                                onClick={() => applyMemoryAuditPreset(preset)}
                              >
                                <span className="truncate">{label}</span>
                              </Button>
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon"
                                aria-label={t("settings.memoryFilterPresetRemove")}
                                className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                                onClick={() => removeMemoryAuditPreset(preset.id)}
                              >
                                <X className="h-3 w-3" />
                              </Button>
                            </span>
                          )
                        })}
                      </>
                    )}
                  </div>
                  {memoryAuditTotalLabel && (
                    <div className="text-[11px] text-muted-foreground">
                      {t("settings.memoryAuditResultCount", {
                        shown: memoryAuditShownCount,
                        total: memoryAuditTotalLabel,
                      })}
                    </div>
                  )}
                </div>
              )}
              <div className="mt-3 space-y-2">
                {memoryAuditOpen ? (
                  visibleAuditActivity.length > 0 ? (
                    <>
                      {visibleAuditActivity.map((item) => (
                        <button
                          key={item.key}
                          type="button"
                          onClick={() => openRecentUnifiedActivity(item)}
                          disabled={item.kind === "memory_event" && item.disabled}
                          className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50 disabled:cursor-default disabled:hover:bg-background/70"
                        >
                          <div className="truncate font-medium">{item.title}</div>
                          <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                            {item.subtitle.map((part, index) => (
                              <span
                                key={`${item.key}:audit:${index}`}
                                className={index === 0 ? "font-mono" : undefined}
                              >
                                {part}
                              </span>
                            ))}
                          </div>
                          {item.detail && (
                            <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                              {item.detail}
                            </div>
                          )}
                        </button>
                      ))}
                      {memoryAuditHasMoreCombined && (
                        <div className="flex justify-center pt-1">
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-8 text-xs"
                            disabled={memoryAuditLoading}
                            onClick={() => void runMemoryAuditSearch({ append: true })}
                          >
                            {memoryAuditLoading && (
                              <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                            )}
                            {t("settings.memoryAuditLoadMore", "Load more activity")}
                          </Button>
                        </div>
                      )}
                    </>
                  ) : (
                    <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                      {memoryAuditLoading
                        ? t("common.loading")
                        : memoryAuditHasFilters
                          ? t("settings.memoryAuditNoMatches", "No matching memory activity.")
                          : t("settings.memoryRecentLearnedEmpty")}
                    </div>
                  )
                ) : recentUnifiedActivity.length > 0 ? (
                  recentUnifiedActivity.map((item) => (
                    <button
                      key={item.key}
                      type="button"
                      disabled={item.kind === "memory_event" && item.disabled}
                      onClick={() => openRecentUnifiedActivity(item)}
                      className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50 disabled:cursor-default disabled:hover:bg-background/70"
                    >
                      <div className="truncate font-medium">{item.title}</div>
                      <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                        {item.subtitle.map((part, index) => (
                          <span
                            key={`${item.key}:${index}`}
                            className={index === 0 ? "font-mono" : undefined}
                          >
                            {part}
                          </span>
                        ))}
                      </div>
                      {item.detail && (
                        <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                          {item.detail}
                        </div>
                      )}
                    </button>
                  ))
                ) : (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {activityLoading ? t("common.loading") : t("settings.memoryRecentLearnedEmpty")}
                  </div>
                )}
              </div>
            </div>

            <div className="rounded-lg border border-border/60 bg-card p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <CheckCircle2 className="h-4 w-4 text-primary" />
                    <span>{t("settings.memoryRecentCorrections")}</span>
                    {activityLoading && (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.memoryRecentCorrectionsDesc")}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onOpenClaims({ statusFilter: "all" })}
                  disabled={!data.effectiveExtractClaims}
                >
                  {t("settings.memoryTabs.claims")}
                  <ArrowRight className="ml-1.5 h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="mt-3 space-y-2">
                {!data.effectiveExtractClaims ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {t("settings.memoryRecentCorrectionsDisabled")}
                  </div>
                ) : recentCorrectionDecisions.length === 0 && recentCorrections.length === 0 ? (
                  <div className="rounded-md border border-dashed border-border/70 px-3 py-5 text-center text-xs text-muted-foreground">
                    {activityLoading
                      ? t("common.loading")
                      : t("settings.memoryRecentCorrectionsEmpty")}
                  </div>
                ) : recentCorrectionDecisions.length > 0 ? (
                  recentCorrectionDecisions.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() =>
                        item.targetType === "claim" && item.targetId
                          ? onOpenClaims({
                              statusFilter: "all",
                              selectedId: item.targetId,
                            })
                          : onSelectTab("dreaming")
                      }
                      className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                    >
                      <div className="truncate font-medium">
                        {item.content || item.rationale || item.targetId || item.id}
                      </div>
                      <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                        <span className="font-mono">{formatActivityTime(item.createdAt)}</span>
                        <span>{decisionTypeLabel(item.decisionType)}</span>
                        <span>{t(`dashboard.dreaming.trigger.${item.trigger}`, item.trigger)}</span>
                        {item.phase && <span>{item.phase}</span>}
                      </div>
                      {item.rationale && item.rationale !== item.content && (
                        <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
                          {item.rationale}
                        </div>
                      )}
                    </button>
                  ))
                ) : (
                  recentCorrections.map((claim) => (
                    <button
                      key={claim.id}
                      type="button"
                      onClick={() =>
                        onOpenClaims({
                          statusFilter: "all",
                          claimType: claim.claimType,
                          scopeType: claim.scopeType,
                          scopeId: claim.scopeId,
                          selectedId: claim.id,
                        })
                      }
                      className="block w-full min-w-0 rounded-md border border-border/50 bg-background/70 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/50"
                    >
                      <div className="truncate font-medium">{claim.content}</div>
                      <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                        <span className="font-mono">{formatActivityTime(claim.updatedAt)}</span>
                        <span>{t(`settings.claims.status.${claim.status}`)}</span>
                        <span>{(claim.confidence * 100).toFixed(0)}%</span>
                      </div>
                    </button>
                  ))
                )}
              </div>
            </div>
          </div>
        )}

        {!isAgentMode && (
          <div className="grid gap-3 md:grid-cols-3">
            <button
              type="button"
              onClick={() => onOpenClaims({ statusFilter: "needs_review" })}
              disabled={!data.effectiveExtractClaims}
              className="min-h-24 rounded-lg border border-border/60 bg-card p-4 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <Sparkles className="h-4 w-4 text-primary" />
              <div className="mt-2 text-sm font-medium">{t("settings.memoryTabs.claims")}</div>
              <div className="mt-1 text-xs text-muted-foreground">{t("settings.claims.desc")}</div>
            </button>
            <button
              type="button"
              onClick={() => onSelectTab("profile")}
              className="min-h-24 rounded-lg border border-border/60 bg-card p-4 text-left transition-colors hover:bg-muted/40"
            >
              <UserRound className="h-4 w-4 text-primary" />
              <div className="mt-2 text-sm font-medium">{t("settings.memoryTabs.profile")}</div>
              <div className="mt-1 text-xs text-muted-foreground">{t("settings.profile.desc")}</div>
            </button>
            <button
              type="button"
              onClick={() => onSelectTab("dreaming")}
              className="min-h-24 rounded-lg border border-border/60 bg-card p-4 text-left transition-colors hover:bg-muted/40"
            >
              <Activity className="h-4 w-4 text-primary" />
              <div className="mt-2 text-sm font-medium">{t("settings.memoryTabs.dreaming")}</div>
              <div className="mt-1 text-xs text-muted-foreground">
                {t("settings.dreaming.desc")}
              </div>
            </button>
          </div>
        )}

        <Dialog
          open={!!experienceDetail}
          onOpenChange={(open) => {
            if (!open && !experienceStatusSaving) {
              setExperienceDetail(null)
              setExperienceHistory([])
            }
          }}
        >
          <DialogContent className="max-w-2xl">
            {experienceDetail && (
              <>
                <DialogHeader>
                  <DialogTitle>{experienceDetail.record.title}</DialogTitle>
                  <DialogDescription>
                    {experienceDetail.kind === "episode"
                      ? t("settings.memoryEpisodeDetailDesc", "Recorded task experience")
                      : t("settings.memoryProcedureDetailDesc", "Reusable workflow memory")}
                    {" · "}
                    {memoryScopeLabel(experienceDetail.record.scope)}
                    {" · "}
                    {experienceDetail.record.status === "archived"
                      ? t("settings.memoryExperienceStatusArchived", "Archived")
                      : t("settings.memoryExperienceStatusActive", "Active")}
                    {" · "}
                    {formatActivityTime(experienceDetail.record.updatedAt)}
                  </DialogDescription>
                </DialogHeader>
                <div className="max-h-[60vh] space-y-4 overflow-y-auto pr-1 text-sm">
                  {experienceDetail.kind === "episode" ? (
                    <>
                      <div className="grid gap-1.5">
                        <div className="text-xs font-medium text-muted-foreground">
                          {t("settings.memoryEpisodeSituation", "Situation")}
                        </div>
                        <div className="rounded-md border border-border/60 bg-background/60 p-3 text-sm">
                          {experienceDetail.record.situation}
                        </div>
                      </div>
                      {experienceDetail.record.actions.length > 0 && (
                        <div className="grid gap-1.5">
                          <div className="text-xs font-medium text-muted-foreground">
                            {t("settings.memoryEpisodeActionsLabel", "Actions")}
                          </div>
                          <ol className="list-decimal space-y-1 rounded-md border border-border/60 bg-background/60 py-3 pl-8 pr-3 text-sm">
                            {experienceDetail.record.actions.map((action, idx) => (
                              <li key={`${idx}-${action}`}>{action}</li>
                            ))}
                          </ol>
                        </div>
                      )}
                      {(experienceDetail.record.outcome || experienceDetail.record.lesson) && (
                        <div className="grid gap-3 sm:grid-cols-2">
                          {experienceDetail.record.outcome && (
                            <div className="grid gap-1.5">
                              <div className="text-xs font-medium text-muted-foreground">
                                {t("settings.memoryEpisodeOutcome", "Outcome")}
                              </div>
                              <div className="rounded-md border border-border/60 bg-background/60 p-3 text-sm">
                                {experienceDetail.record.outcome}
                              </div>
                            </div>
                          )}
                          {experienceDetail.record.lesson && (
                            <div className="grid gap-1.5">
                              <div className="text-xs font-medium text-muted-foreground">
                                {t("settings.memoryEpisodeLesson", "Lesson")}
                              </div>
                              <div className="rounded-md border border-border/60 bg-background/60 p-3 text-sm">
                                {experienceDetail.record.lesson}
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                      <div className="flex flex-wrap gap-1.5 text-xs text-muted-foreground">
                        <span className="rounded bg-secondary px-1.5 py-0.5">
                          {Math.round(experienceDetail.record.successScore * 100)}%
                        </span>
                        {experienceDetail.record.tags.map((tag) => (
                          <span key={tag} className="rounded bg-secondary px-1.5 py-0.5">
                            {tag}
                          </span>
                        ))}
                      </div>
                    </>
                  ) : (
                    <>
                      {renderProcedureGuidanceNotice(experienceDetail.record)}
                      <div className="grid gap-1.5">
                        <div className="text-xs font-medium text-muted-foreground">
                          {t("settings.memoryProcedureTrigger", "Trigger")}
                        </div>
                        <div className="rounded-md border border-border/60 bg-background/60 p-3 text-sm">
                          {experienceDetail.record.trigger}
                        </div>
                      </div>
                      <div className="grid gap-1.5">
                        <div className="text-xs font-medium text-muted-foreground">
                          {t("settings.memoryProcedureSteps", "Steps")}
                        </div>
                        <pre className="whitespace-pre-wrap rounded-md border border-border/60 bg-background/60 p-3 text-sm font-sans">
                          {experienceDetail.record.stepsMarkdown}
                        </pre>
                      </div>
                      {experienceDetail.record.constraintsMarkdown && (
                        <div className="grid gap-1.5">
                          <div className="text-xs font-medium text-muted-foreground">
                            {t("settings.memoryProcedureConstraints", "Constraints")}
                          </div>
                          <pre className="whitespace-pre-wrap rounded-md border border-border/60 bg-background/60 p-3 text-sm font-sans">
                            {experienceDetail.record.constraintsMarkdown}
                          </pre>
                        </div>
                      )}
                      <div className="flex flex-wrap gap-1.5 text-xs text-muted-foreground">
                        <span className="rounded bg-secondary px-1.5 py-0.5">
                          {Math.round(experienceDetail.record.confidence * 100)}%
                        </span>
                        {experienceDetail.record.sourceEpisodeIds.map((id) => (
                          <Button
                            key={id}
                            type="button"
                            variant="secondary"
                            size="sm"
                            className="h-6 max-w-full px-1.5 py-0.5 text-[10px] font-normal"
                            title={id}
                            disabled={experienceSourceOpening === id}
                            onClick={() => void openSourceEpisode(id)}
                          >
                            {experienceSourceOpening === id && (
                              <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                            )}
                            <span className="truncate">
                              {t("settings.memoryProcedureSourceEpisode", {
                                defaultValue: "episode:{{id}}",
                                id: id.slice(0, 8),
                              })}
                            </span>
                          </Button>
                        ))}
                        {experienceDetail.record.tags.map((tag) => (
                          <span key={tag} className="rounded bg-secondary px-1.5 py-0.5">
                            {tag}
                          </span>
                        ))}
                      </div>
                    </>
                  )}
                  <div className="grid gap-2 border-t border-border/60 pt-3">
                    <div className="flex items-center justify-between gap-2">
                      <div className="text-xs font-medium text-muted-foreground">
                        {t("settings.memoryExperienceHistoryTitle", "Recent changes")}
                      </div>
                      {experienceHistoryLoading && (
                        <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                      )}
                    </div>
                    {experienceHistoryError ? (
                      <div className="flex flex-col gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs sm:flex-row sm:items-start sm:justify-between">
                        <div className="flex min-w-0 items-start gap-2">
                          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-300" />
                          <div className="min-w-0">
                            <div className="font-medium text-foreground">
                              {experienceHistoryError.title}
                            </div>
                            {experienceHistoryError.description && (
                              <div className="mt-0.5 break-words text-muted-foreground">
                                {experienceHistoryError.description}
                              </div>
                            )}
                          </div>
                        </div>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-7 self-start px-2 text-[11px]"
                          disabled={experienceHistoryLoading}
                          onClick={() =>
                            void loadExperienceHistory(
                              experienceDetail.kind,
                              experienceDetail.record.id,
                            )
                          }
                        >
                          {experienceHistoryLoading ? (
                            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                          )}
                          {t("settings.memoryOverviewRetry", "Retry")}
                        </Button>
                      </div>
                    ) : !experienceHistoryLoading && experienceHistory.length === 0 ? (
                      <div className="rounded-md border border-dashed border-border/70 px-3 py-2 text-xs text-muted-foreground">
                        {t(
                          "settings.memoryExperienceHistoryEmpty",
                          "No recorded changes yet.",
                        )}
                      </div>
                    ) : (
                      <div className="space-y-2">
                        {experienceHistory.map((event) => (
                          <div
                            key={event.id}
                            className="rounded-md border border-border/60 bg-background/60 p-2.5"
                          >
                            <div className="flex flex-wrap items-center gap-1.5 text-xs">
                              <span className="font-medium">
                                {experienceHistoryActionLabel(event.action)}
                              </span>
                              <span className="text-muted-foreground">
                                {formatActivityTime(event.createdAt)}
                              </span>
                              <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                {memoryScopeLabel(event.scope)}
                              </span>
                            </div>
                            {event.contentPreview && (
                              <div className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                                {event.contentPreview}
                              </div>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
                <DialogFooter>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() => {
                      setExperienceDetail(null)
                      setExperienceHistory([])
                    }}
                    disabled={experienceStatusSaving}
                  >
                    {t("common.close", "Close")}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={editExperienceDetail}
                    disabled={experienceStatusSaving}
                  >
                    <Pencil className="mr-1.5 h-3.5 w-3.5" />
                    {t("common.edit", "Edit")}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    className={
                      experienceDetail.record.status === "archived"
                        ? undefined
                        : "text-destructive hover:text-destructive"
                    }
                    onClick={() => void changeExperienceDetailStatus()}
                    disabled={experienceStatusSaving}
                  >
                    {experienceStatusSaving && (
                      <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                    )}
                    {!experienceStatusSaving &&
                      (experienceDetail.record.status === "archived" ? (
                        <CheckCircle2 className="mr-1.5 h-3.5 w-3.5" />
                      ) : (
                        <Archive className="mr-1.5 h-3.5 w-3.5" />
                      ))}
                    {experienceDetail.record.status === "archived"
                      ? t("settings.memoryExperienceRestore", "Restore")
                      : t("settings.memoryExperienceArchive", "Archive")}
                  </Button>
                </DialogFooter>
              </>
            )}
          </DialogContent>
        </Dialog>

        <Dialog
          open={episodeDialogOpen}
          onOpenChange={(open) => {
            setEpisodeDialogOpen(open)
            if (!open && !episodeSaving) resetEpisodeDraft()
          }}
        >
          <DialogContent className="max-w-2xl">
            <DialogHeader>
              <DialogTitle>
                {episodeEditingId
                  ? t("settings.memoryEpisodeEditTitle", "Edit episode")
                  : t("settings.memoryEpisodeDialogTitle", "Record an episode")}
              </DialogTitle>
              <DialogDescription>
                {episodeEditingId
                  ? t(
                      "settings.memoryEpisodeEditDesc",
                      "Correct the saved lesson, scope, or tags.",
                    )
                  : t(
                      "settings.memoryEpisodeDialogDesc",
                      "Save a reusable lesson from finished work.",
                    )}
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-3">
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-episode-title">
                  {t("settings.memoryEpisodeTitle", "Title")}
                </label>
                <Input
                  id="memory-episode-title"
                  value={episodeDraft.title}
                  onChange={(event) =>
                    setEpisodeDraft((draft) => ({ ...draft, title: event.target.value }))
                  }
                  placeholder={t(
                    "settings.memoryEpisodeTitlePlaceholder",
                    "Fixed flaky release check",
                  )}
                />
              </div>
              {renderExperienceScopePicker(
                "memory-episode",
                episodeScopeDraft,
                setEpisodeScopeDraft,
              )}
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-episode-situation">
                  {t("settings.memoryEpisodeSituation", "Situation")}
                </label>
                <Textarea
                  id="memory-episode-situation"
                  className="min-h-20"
                  value={episodeDraft.situation}
                  onChange={(event) =>
                    setEpisodeDraft((draft) => ({ ...draft, situation: event.target.value }))
                  }
                  placeholder={t(
                    "settings.memoryEpisodeSituationPlaceholder",
                    "What was the task or problem?",
                  )}
                />
              </div>
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-episode-actions">
                  {t("settings.memoryEpisodeActionsLabel", "Actions")}
                </label>
                <Textarea
                  id="memory-episode-actions"
                  className="min-h-24"
                  value={episodeDraft.actions}
                  onChange={(event) =>
                    setEpisodeDraft((draft) => ({ ...draft, actions: event.target.value }))
                  }
                  placeholder={t(
                    "settings.memoryEpisodeActionsPlaceholder",
                    "One action per line",
                  )}
                />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="grid gap-1.5">
                  <label className="text-xs font-medium" htmlFor="memory-episode-outcome">
                    {t("settings.memoryEpisodeOutcome", "Outcome")}
                  </label>
                  <Textarea
                    id="memory-episode-outcome"
                    className="min-h-20"
                    value={episodeDraft.outcome}
                    onChange={(event) =>
                      setEpisodeDraft((draft) => ({ ...draft, outcome: event.target.value }))
                    }
                  />
                </div>
                <div className="grid gap-1.5">
                  <label className="text-xs font-medium" htmlFor="memory-episode-lesson">
                    {t("settings.memoryEpisodeLesson", "Lesson")}
                  </label>
                  <Textarea
                    id="memory-episode-lesson"
                    className="min-h-20"
                    value={episodeDraft.lesson}
                    onChange={(event) =>
                      setEpisodeDraft((draft) => ({ ...draft, lesson: event.target.value }))
                    }
                  />
                </div>
              </div>
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-episode-tags">
                  {t("settings.memoryEpisodeTags", "Tags")}
                </label>
                <Input
                  id="memory-episode-tags"
                  value={episodeDraft.tags}
                  onChange={(event) =>
                    setEpisodeDraft((draft) => ({ ...draft, tags: event.target.value }))
                  }
                  placeholder={t("settings.memoryEpisodeTagsPlaceholder", "release, ci")}
                />
              </div>
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => setEpisodeDialogOpen(false)}
                disabled={episodeSaving}
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="button"
                onClick={() => void submitEpisodeDraft()}
                disabled={episodeSaving}
              >
                {episodeSaving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                {t("common.save")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog
          open={procedureDialogOpen}
          onOpenChange={(open) => {
            setProcedureDialogOpen(open)
            if (!open && !procedureSaving) resetProcedureDraft()
          }}
        >
          <DialogContent className="max-w-2xl">
            <DialogHeader>
              <DialogTitle>
                {procedureEditingId
                  ? t("settings.memoryProcedureEditTitle", "Edit workflow")
                  : t("settings.memoryProcedureDialogTitle", "Record a workflow")}
              </DialogTitle>
              <DialogDescription>
                {procedureEditingId
                  ? t(
                      "settings.memoryProcedureEditDesc",
                      "Correct the saved procedure, scope, or tags.",
                    )
                  : t(
                      "settings.memoryProcedureDialogDesc",
                      "Save a reusable procedure for repeated work.",
                    )}
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-3">
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-procedure-title">
                  {t("settings.memoryEpisodeTitle", "Title")}
                </label>
                <Input
                  id="memory-procedure-title"
                  value={procedureDraft.title}
                  onChange={(event) =>
                    setProcedureDraft((draft) => ({ ...draft, title: event.target.value }))
                  }
                  placeholder={t(
                    "settings.memoryProcedureTitlePlaceholder",
                    "Release verification workflow",
                  )}
                />
              </div>
              {renderExperienceScopePicker(
                "memory-procedure",
                procedureScopeDraft,
                setProcedureScopeDraft,
              )}
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-procedure-trigger">
                  {t("settings.memoryProcedureTrigger", "Trigger")}
                </label>
                <Textarea
                  id="memory-procedure-trigger"
                  className="min-h-20"
                  value={procedureDraft.trigger}
                  onChange={(event) =>
                    setProcedureDraft((draft) => ({ ...draft, trigger: event.target.value }))
                  }
                  placeholder={t(
                    "settings.memoryProcedureTriggerPlaceholder",
                    "When this workflow should be considered",
                  )}
                />
              </div>
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-procedure-steps">
                  {t("settings.memoryProcedureSteps", "Steps")}
                </label>
                <Textarea
                  id="memory-procedure-steps"
                  className="min-h-32"
                  value={procedureDraft.stepsMarkdown}
                  onChange={(event) =>
                    setProcedureDraft((draft) => ({
                      ...draft,
                      stepsMarkdown: event.target.value,
                    }))
                  }
                  placeholder={t(
                    "settings.memoryProcedureStepsPlaceholder",
                    "- Inspect the failing check\n- Apply the known fix\n- Re-run the narrow verifier",
                  )}
                />
              </div>
              <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
                <div className="grid gap-1.5">
                  <label className="text-xs font-medium" htmlFor="memory-procedure-constraints">
                    {t("settings.memoryProcedureConstraints", "Constraints")}
                  </label>
                  <Textarea
                    id="memory-procedure-constraints"
                    className="min-h-20"
                    value={procedureDraft.constraintsMarkdown}
                    onChange={(event) =>
                      setProcedureDraft((draft) => ({
                        ...draft,
                        constraintsMarkdown: event.target.value,
                      }))
                    }
                    placeholder={t(
                      "settings.memoryProcedureConstraintsPlaceholder",
                      "When not to use this workflow",
                    )}
                  />
                </div>
                <div className="grid gap-1.5">
                  <label className="text-xs font-medium" htmlFor="memory-procedure-confidence">
                    {t("settings.memoryProcedureConfidence", "Confidence")}
                  </label>
                  <Input
                    id="memory-procedure-confidence"
                    type="number"
                    min={0}
                    max={100}
                    step={5}
                    value={procedureDraft.confidencePercent}
                    onChange={(event) =>
                      setProcedureDraft((draft) => ({
                        ...draft,
                        confidencePercent: event.target.value,
                      }))
                    }
                  />
                </div>
              </div>
              <div className="grid gap-1.5">
                <label className="text-xs font-medium" htmlFor="memory-procedure-tags">
                  {t("settings.memoryEpisodeTags", "Tags")}
                </label>
                <Input
                  id="memory-procedure-tags"
                  value={procedureDraft.tags}
                  onChange={(event) =>
                    setProcedureDraft((draft) => ({ ...draft, tags: event.target.value }))
                  }
                  placeholder={t("settings.memoryEpisodeTagsPlaceholder", "release, ci")}
                />
              </div>
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => setProcedureDialogOpen(false)}
                disabled={procedureSaving}
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="button"
                onClick={() => void submitProcedureDraft()}
                disabled={procedureSaving}
              >
                {procedureSaving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                {t("common.save")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </div>
  )
}
