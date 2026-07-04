import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  BarChart3,
  BookText,
  Bot,
  Brain,
  CalendarClock,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  CheckCircle2,
  ClipboardCheck,
  CircleAlert,
  Clock,
  Copy,
  Cpu,
  Database,
  Eye,
  EyeOff,
  FileText,
  Files,
  FolderGit2,
  FolderOpen,
  Gauge,
  GitCompare,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  Globe,
  HardDrive,
  Hash,
  Layers,
  LayoutDashboard,
  Lightbulb,
  Loader2,
  Lock,
  MessageCircle,
  MessageSquare,
  Monitor,
  Pause,
  Pencil,
  Play,
  Plus,
  Radio,
  RefreshCw,
  Search,
  Server,
  Shield,
  ShieldAlert,
  Sparkles,
  X,
  type LucideIcon,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import { basename } from "@/lib/path"
import { logger } from "@/lib/logger"
import { useAppVersion } from "@/lib/appMeta"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { useSafeFavicon } from "@/hooks/useSafeFavicon"
import { getTransport } from "@/lib/transport-provider"
import { useDangerousModeStatus } from "@/hooks/useDangerousModeStatus"
import { type BackgroundJobSnapshot, isBackgroundJobActive } from "@/types/background-jobs"
import { SessionBackgroundJobsList } from "../background-jobs/SessionBackgroundJobsList"
import type {
  CodingFailureBucket,
  CodingImprovementActionPlan,
  CodingImprovementPromotionPlan,
  CodingImprovementProposal,
  CodingMetricBucket,
  CodingTrendReport,
  CodingWorkflowRetro,
  ContextCandidate,
  ContextCandidateKind,
  CreateOwnerAskUserQuestionInput,
  DomainApprovalGate,
  DomainArtifactExportGuardReport,
  DomainConnectorActionGuardReport,
  DomainEvidenceItem,
  DomainEvidenceRequirement,
  DomainOperationalGateReport,
  DomainQualityCheck,
  DomainQualityCheckStatus,
  DomainQualityRunSnapshot,
  DomainQualityRunState,
  DomainQualitySeverity,
  DomainSoakReport,
  DomainVerificationRule,
  DomainWorkflowDraft,
  DomainWorkflowTemplate,
  GenerateCodingImprovementProposalsResult,
  LspDiagnostic,
  ManagedWorktree,
  RecordDomainEvidenceInput,
  ReviewFinding,
  ReviewFindingStatus,
  ReviewRunSnapshot,
  ReviewSeverity,
  ReviewVerdict,
  VerificationRisk,
  VerificationRunSnapshot,
  VerificationStep,
  VerificationStepState,
  WorkspaceGitSnapshot,
} from "@/lib/transport"
import {
  computeContextUsage,
  contextUsageBarClass,
  formatMessageTime,
  type ContextUsageInfo,
} from "../chatUtils"
import { formatCacheUsageDisplay, formatCompactTokenCount } from "../cacheUsageDisplay"
import {
  type CompactResult,
  compactResultMessage,
  computeCacheStats,
  resolveCurrentModel,
  runViewContext,
} from "../sessionStatus"
import type { CommandResult } from "../slash-commands/types"
import type {
  ActiveModel,
  AvailableModel,
  FileChangeMetadata,
  FileChangesMetadata,
  Message,
  SessionMeta,
  SessionMode,
  Task,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import { FileMimeIcon } from "@/components/chat/message/FileCard"
import { FileDeltaCounter } from "@/components/chat/message/FileDeltaCounter"
import { FileContextMenu, FileActionsMoreButton } from "@/components/chat/files/FileActionMenu"
import { useFileActions } from "@/components/chat/files/useFileActions"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import type { PlanModeState } from "@/components/chat/plan-mode/usePlanMode"
import type { SessionFileEntry } from "./useSessionFileChanges"
import type { SessionUrlSource } from "./useSessionUrlSources"
import { useWorkspaceArtifacts } from "./useWorkspaceArtifacts"
import { useWorkspaceEnvironment } from "./useWorkspaceEnvironment"
import { useScrollPagedRender } from "./useScrollPagedRender"
import { useSessionKnowledge } from "./useSessionKnowledge"
import { useManagedWorktrees } from "./useManagedWorktrees"
import { useContextRetrieval } from "./useContextRetrieval"
import { useDomainQualityRuns, type DomainQualityRunsState } from "./useDomainQualityRuns"
import { useLspDiagnostics } from "./useLspDiagnostics"
import { useReviewRuns, type ReviewRunsState } from "./useReviewRuns"
import { useVerificationRuns, type VerificationRunsState } from "./useVerificationRuns"
import { useCodingTrendReport } from "./useCodingTrendReport"
import {
  useWorkflowRuns,
  type WorkflowEvent,
  type WorkflowGateIssue,
  type WorkflowPermissionPreview,
  type WorkflowOp,
  type WorkflowRun,
  type WorkflowRunSnapshot,
  type WorkflowRunState,
  type WorkflowRunsState,
  type WorkflowScriptPreview,
} from "./useWorkflowRuns"
import {
  useGoal,
  type Goal,
  type GoalBudgetSnapshot,
  type GoalCriterionAudit,
  type GoalCriterionStatus,
  type GoalEvidenceItem,
  type GoalSnapshot,
  type GoalState,
  type GoalStateSnapshot,
  type GoalTimelineItem,
} from "./useGoal"
import {
  useLoopSchedules,
  type LoopExecutionStrategy,
  type LoopRun,
  type LoopSnapshot,
  type LoopSchedule,
  type LoopSchedulesState,
  type LoopRunState,
  type LoopState,
  type LoopTriggerKind,
} from "./useLoopSchedules"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"
import { PANEL_SCROLL_FADE } from "../right-panel/panelFade"
import {
  formatGitRef,
  resolveWorkspaceEnvironmentStatus,
  workingDirSourceLabelKey,
} from "./workspaceEnvironment"

interface WorkspacePanelProps {
  taskSnapshot: TaskProgressSnapshot | null
  taskExecutionState?: WorkspaceTaskExecutionState
  /** 会话消息 —— 当前轮 live tail 在面板内部聚合,与后端历史全量合并。 */
  messages: Message[]
  contextUsageOverride?: ContextUsageInfo | null
  /** 改写类文件「查看 diff」→ 右侧 diff 面板。 */
  onOpenDiff: (payload: FileChangeMetadata | FileChangesMetadata) => void
  /** 预览文件 → 右侧预览面板（与下挂文件 / Markdown 链接同一策略）。 */
  onPreviewFile?: (target: PreviewTarget) => void
  /** 当前会话 id,后端聚合 + 文件作用域解析都需要它。 */
  sessionId?: string | null
  /** 当前会话元信息,用于渲染项目/IM/Cron/Subagent/权限等环境上下文。 */
  sessionMeta?: SessionMeta | null
  /** 当前会话所属项目(若有),由 ChatScreen 传入避免面板内部散查全局状态。 */
  project?: ProjectMeta | null
  effectiveWorkingDir?: string | null
  workingDirSource?: "session" | "project"
  permissionMode?: SessionMode
  planState?: PlanModeState
  activeModel?: ActiveModel | null
  /** 会话卡（复刻状态悬浮窗）所需:Agent 名 / Think 档 / 模型解析 / 会话动作回调。 */
  agentName?: string
  reasoningEffort?: string
  availableModels?: AvailableModel[]
  currentAgentId?: string
  /** 「查看上下文」把 `/context` 结果回派给 ChatScreen（与悬浮窗共用入口）。 */
  onCommandAction?: (result: CommandResult) => void
  compacting?: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  /** 「查看系统提示词」—— 复用 ChatScreen 的 loadSystemPrompt。 */
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
  /** 无痕会话:跳过后端聚合,只用 live tail（守「关闭即焚」）。 */
  incognito?: boolean
  /** 当前会话是否正在跑一轮:true→false 跳变时面板重新拉后端聚合。 */
  turnActive?: boolean
  /** 标题栏常驻订阅到的 workflow runs；面板打开时复用它，避免重复轮询。 */
  workflowRunsState?: WorkflowRunsState
  /** R4:本会话后台任务（由 ChatScreen 的 useBackgroundJobs 传入,与头部徽标 / 独立面板共用一份订阅）。 */
  backgroundJobs?: BackgroundJobSnapshot[]
  /** R4:后台任务展开状态,与独立面板共享,避免工作台和面板交互分叉。 */
  backgroundJobExpansionOverrides?: Record<string, boolean>
  onBackgroundJobExpandedChange?: (jobId: string, expanded: boolean) => void
  /** R4:打开独立「后台任务」面板（完整列表和单项管理在那里处理）。 */
  onOpenBackgroundJobs?: () => void
  /** 打开子 agent 实时会话弹层，不切换当前主会话。 */
  onViewSubagentSession?: (sessionId: string) => void
  /** 草稿态新对话里创建 workflow 前,由 ChatScreen 物化一个真实会话并切过去。 */
  onEnsureSession?: () => Promise<string | null>
  onClose: () => void
}

/** 每段初始渲染条数;滚到底自动 +此值（无「加载更多」按钮）。 */
const RENDER_STEP = 20

function domainOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "")
  } catch {
    return url
  }
}

/** Collapsible card section matching TaskProgressPanel's visual language. */
function WorkspaceSection({
  title,
  count,
  icon: Icon,
  children,
  meta,
  defaultExpanded = true,
  expandSignal,
}: {
  title: string
  count?: number
  icon: LucideIcon
  children: ReactNode
  meta?: ReactNode
  defaultExpanded?: boolean
  expandSignal?: number
}) {
  const [expanded, setExpanded] = useState(defaultExpanded)
  const lastExpandSignalRef = useRef(expandSignal ?? 0)

  useEffect(() => {
    const nextSignal = expandSignal ?? 0
    if (nextSignal === lastExpandSignalRef.current) return
    lastExpandSignalRef.current = nextSignal
    setExpanded(true)
  }, [expandSignal])

  return (
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-surface-floating shadow-sm">
      <button
        type="button"
        aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-secondary/45"
        onClick={() => setExpanded((v) => !v)}
      >
        <Icon className="h-4 w-4 shrink-0 text-blue-500" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {title}
          {typeof count === "number" ? (
            <>
              <span className="px-1.5 font-normal text-muted-foreground">·</span>
              <span className="font-normal text-muted-foreground tabular-nums">{count}</span>
            </>
          ) : null}
        </span>
        {meta}
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
      </button>
      <AnimatedCollapse open={expanded}>
        <div className="border-t border-border/60 px-2 py-2">{children}</div>
      </AnimatedCollapse>
    </div>
  )
}

/** 段内末尾小字:被后端安全上限截断时提示「仅显示最近 N 条」。 */
function TruncatedNote() {
  const { t } = useTranslation()
  return (
    <div className="px-2 pt-1.5 text-center text-[11px] text-muted-foreground/60">
      {t("workspace.truncatedNote", "仅显示最近 1000 条")}
    </div>
  )
}

/**
 * 文件行 —— 操作与消息下挂文件 / Markdown 链接完全一致:主点击按类型 × 模式决议
 * (预览 / 打开 / 下载),右键 + ⋯ 出完整菜单。窗口内文件带结构化 diff 时额外保留
 * 一个「查看 diff」按钮(工作台独有);窗口外(后端摘要)文件无 diff,点击走预览当前
 * 内容。工作台在消息树外,故 sessionId / onPreviewFile 通过 overrides 显式传入。
 */
function FileRow({
  entry,
  sessionId,
  onOpenDiff,
  onPreviewFile,
}: {
  entry: SessionFileEntry
  sessionId?: string | null
  onOpenDiff: (payload: FileChangeMetadata | FileChangesMetadata) => void
  onPreviewFile?: (target: PreviewTarget) => void
}) {
  const { t } = useTranslation()
  const name = basename(entry.path)
  const diff = entry.diff
  // `+N -M` shows for any modified file with a known line delta (backend
  // summary or live diff); the diff *button* needs the structured `diff`.
  const showDelta = entry.kind === "modified" && (entry.linesAdded > 0 || entry.linesRemoved > 0)
  const target = useMemo<PreviewTarget>(
    () => ({ kind: "path", path: entry.path, name }),
    [entry.path, name],
  )
  const overrides = useMemo(() => ({ sessionId, onPreviewFile }), [sessionId, onPreviewFile])
  const { primary, run } = useFileActions(target, overrides)
  const btnClass =
    "p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"

  return (
    <FileContextMenu target={target} overrides={overrides}>
      <div className="flex items-center gap-2 rounded-md border border-border/50 bg-secondary/30 px-2.5 py-1.5 transition-colors hover:bg-secondary/50">
        <FileMimeIcon mime="" name={name} className="h-4 w-4 shrink-0 text-muted-foreground" />
        <IconTip label={entry.path}>
          <button
            type="button"
            onClick={() => run(primary)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left transition-colors hover:text-foreground"
          >
            <span className="truncate text-xs font-medium text-foreground/90">{name}</span>
            {showDelta ? (
              <FileDeltaCounter
                linesAdded={entry.linesAdded}
                linesRemoved={entry.linesRemoved}
                className="text-[10px]"
              />
            ) : entry.kind === "read" ? (
              <span className="shrink-0 text-[10px] text-muted-foreground/70">
                {t("workspace.action.read")}
              </span>
            ) : null}
          </button>
        </IconTip>
        <div className="flex shrink-0 items-center gap-0.5">
          {diff && (
            <IconTip label={t("diffPanel.openDiff", "查看 diff")}>
              <button type="button" onClick={() => onOpenDiff(diff)} className={btnClass}>
                <GitCompare className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          )}
          <FileActionsMoreButton target={target} overrides={overrides} />
        </div>
      </div>
    </FileContextMenu>
  )
}

function SourceRow({ source }: { source: SessionUrlSource }) {
  const { t } = useTranslation()
  const faviconUrl = useSafeFavicon(source.url)
  return (
    <IconTip label={source.url}>
      <button
        type="button"
        onClick={() => openExternalUrl(source.url)}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-secondary/45"
      >
        {faviconUrl ? (
          <img
            src={faviconUrl}
            alt=""
            className="h-3.5 w-3.5 shrink-0 rounded-[3px] bg-background/70 object-contain"
          />
        ) : (
          <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <span className="min-w-0 flex-1 truncate text-xs text-foreground/90">
          {domainOf(source.url)}
        </span>
        {source.origin === "web_search" && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            <Search className="h-2.5 w-2.5" />
            {t("workspace.sourceFromSearch", "搜索")}
          </span>
        )}
      </button>
    </IconTip>
  )
}

function EmptyHint({ children }: { children: ReactNode }) {
  return <div className="px-2 py-3 text-center text-xs text-muted-foreground/70">{children}</div>
}

type StatusTone = "muted" | "good" | "warn" | "danger" | "info"

const STATUS_TONE_CLASS: Record<StatusTone, string> = {
  muted: "border-border bg-muted/50 text-muted-foreground",
  good: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warn: "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  danger: "border-destructive/35 bg-destructive/10 text-destructive",
  info: "border-blue-500/35 bg-blue-500/10 text-blue-700 dark:text-blue-300",
}

function StatusPill({
  label,
  tone,
  loading,
}: {
  label: string
  tone: StatusTone
  loading?: boolean
}) {
  return (
    <span
      className={cn(
        "inline-flex max-w-[8rem] shrink-0 items-center rounded-full border px-2 py-0.5 text-[10px] font-medium",
        STATUS_TONE_CLASS[tone],
        loading && "animate-pulse",
      )}
    >
      <span className="truncate">{label}</span>
    </span>
  )
}

function EnvRow({
  icon: Icon,
  label,
  value,
  detail,
  tone = "muted",
  title,
  onClick,
  disabled,
}: {
  icon: LucideIcon
  label: string
  value: ReactNode
  detail?: ReactNode
  tone?: "muted" | "good" | "warn" | "danger" | "info"
  title?: string
  onClick?: () => void
  disabled?: boolean
}) {
  const iconClass =
    tone === "good"
      ? "text-emerald-600 dark:text-emerald-400"
      : tone === "warn"
        ? "text-amber-600 dark:text-amber-400"
        : tone === "danger"
          ? "text-destructive"
          : tone === "info"
            ? "text-blue-500"
            : "text-muted-foreground"
  const className = cn(
    "flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-secondary/35",
    onClick && "w-full text-left",
    disabled && "cursor-not-allowed opacity-60",
  )
  const content = (
    <>
      <Icon className={cn("h-3.5 w-3.5 shrink-0", iconClass)} />
      <span className="w-14 shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">{value}</span>
      {detail ? (
        <span className="max-w-[45%] shrink-0 truncate text-muted-foreground">{detail}</span>
      ) : null}
    </>
  )
  const row = onClick ? (
    <button type="button" className={className} onClick={onClick} disabled={disabled}>
      {content}
    </button>
  ) : (
    <div className={className}>{content}</div>
  )
  return title ? <IconTip label={title}>{row}</IconTip> : row
}

function planStateLabel(t: ReturnType<typeof useTranslation>["t"], state: PlanModeState): string {
  switch (state) {
    case "planning":
      return t("planMode.planning", "正在制定计划...")
    case "review":
      return t("workspace.environment.planReview", "等待审核")
    case "executing":
      return t("planMode.executing", "正在按计划执行...")
    case "completed":
      return t("planMode.completed", "执行完成")
    case "off":
      return t("workspace.environment.planOff", "关闭")
  }
}

function gitSyncLabel(
  t: ReturnType<typeof useTranslation>["t"],
  git: WorkspaceGitSnapshot | null,
): string | null {
  if (!git) return null
  const { sync } = git
  switch (sync.state) {
    case "ahead":
      return t("workspace.environment.syncAhead", "领先 {{count}}", { count: sync.ahead })
    case "behind":
      return t("workspace.environment.syncBehind", "落后 {{count}}", { count: sync.behind })
    case "diverged":
      return t("workspace.environment.syncDiverged", "领先 {{ahead}} / 落后 {{behind}}", {
        ahead: sync.ahead,
        behind: sync.behind,
      })
    case "upToDate":
      return sync.upstream ? t("workspace.environment.syncUpToDate", "已同步") : null
    case "noUpstream":
      return t("workspace.environment.syncNoUpstream", "无 upstream")
    case "unknown":
      return sync.upstream ? t("workspace.environment.syncUnknown", "同步状态未知") : null
  }
}

/** Shared action-button styling for the session card (matches the status popover). */
const SESSION_ACTION_BTN =
  "rounded-md border border-border/50 px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground disabled:opacity-50"

/**
 * 会话卡 —— 把标题栏状态悬浮窗的能力「复刻一份」到工作台。模型 / 认证、上下文用量条
 * + 压缩 / 查看上下文、Agent 作为核心常驻;缓存、会话名 / ID、消息数、思考模式、更新
 * 时间、查看系统提示词折进「展开更多」。动作走与悬浮窗完全相同的 transport
 * (`compact_context_now` / `/context` / `get_system_prompt`);上下文数值与悬浮窗共用
 * `computeContextUsage`,两处永不漂移。App 版本不在此卡(归「环境」卡)。
 */
function SessionSection({
  sessionId,
  sessionMeta,
  agentName,
  reasoningEffort,
  activeModel,
  availableModels,
  messages,
  contextUsageOverride,
  currentAgentId,
  turnActive,
  compacting = false,
  onCompactContext,
  onCommandAction,
  onViewSystemPrompt,
  systemPromptLoading,
}: {
  sessionId?: string | null
  sessionMeta?: SessionMeta | null
  agentName?: string
  reasoningEffort?: string
  activeModel?: ActiveModel | null
  availableModels?: AvailableModel[]
  messages: Message[]
  contextUsageOverride?: ContextUsageInfo | null
  currentAgentId?: string
  turnActive?: boolean
  compacting?: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  onCommandAction?: (result: CommandResult) => void
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
}) {
  const { t } = useTranslation()
  const [showMore, setShowMore] = useState(false)
  const [copied, setCopied] = useState(false)
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  // Clear the "copied" reset timer on unmount so it can't fire after the card
  // is closed / the session switched (leaked timer + stale setState).
  useEffect(
    () => () => {
      if (copyTimer.current) clearTimeout(copyTimer.current)
    },
    [],
  )

  const currentModel = useMemo(
    () => resolveCurrentModel(activeModel, availableModels),
    [activeModel, availableModels],
  )
  const usage = useMemo(
    () =>
      contextUsageOverride ??
      (currentModel ? computeContextUsage(messages, currentModel.contextWindow) : null),
    [contextUsageOverride, currentModel, messages],
  )
  const cache = useMemo(() => computeCacheStats(messages), [messages])

  const modelLabel = currentModel
    ? `${currentModel.providerName}/${currentModel.modelName || currentModel.modelId}`
    : activeModel?.modelId || "—"
  const authLabel = (currentModel?.apiType ?? "") === "codex" ? "oauth" : "api-key"
  const sessionTitle = sessionMeta?.title
    ? sessionMeta.title
    : sessionId
      ? sessionId.slice(0, 8)
      : t("chat.statusNewSession")

  const handleCopyId = useCallback(async () => {
    if (!sessionId) return
    try {
      await navigator.clipboard.writeText(sessionId)
    } catch (e) {
      logger.error("ui", "WorkspaceSession::copyId", "Copy session id failed", e)
      return
    }
    setCopied(true)
    if (copyTimer.current) clearTimeout(copyTimer.current)
    copyTimer.current = setTimeout(() => setCopied(false), 1500)
  }, [sessionId])

  const handleCompact = useCallback(async () => {
    if (!sessionId) return
    try {
      const result = await onCompactContext?.()
      if (result) {
        toast.success(compactResultMessage(t, result))
      }
    } catch (e) {
      logger.error("ui", "WorkspaceSession::compact", "Compact failed", e)
      toast.error(t("chat.compactFailed"))
    }
  }, [sessionId, onCompactContext, t])

  const handleViewContext = useCallback(async () => {
    if (!sessionId) return
    try {
      onCommandAction?.(await runViewContext(sessionId, currentAgentId))
    } catch (e) {
      logger.error("ui", "WorkspaceSession::viewContext", "View context failed", e)
    }
  }, [sessionId, currentAgentId, onCommandAction])

  return (
    <WorkspaceSection title={t("workspace.sectionSession", "会话")} icon={MessageCircle}>
      <div className="space-y-2">
        {/* 核心 — 模型 + 认证 */}
        <div className="space-y-0.5">
          <EnvRow
            icon={Brain}
            label={t("chat.statusModel")}
            value={modelLabel}
            detail={authLabel}
            title={modelLabel}
          />
        </div>

        {/* 核心 — 上下文用量条 + 压缩 / 查看上下文 */}
        {usage ? (
          <div className="space-y-1.5 rounded-md border border-border/40 bg-secondary/25 px-2.5 py-2">
            <div className="flex items-center justify-between gap-2 text-xs">
              <span className="text-muted-foreground">{t("chat.statusContext")}</span>
              <span className="font-medium tabular-nums text-foreground">
                {usage.usedK}k/{usage.ctxK}k ({usage.pct}%)
              </span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-secondary">
              <div
                className={cn(
                  "h-full rounded-full transition-all duration-300",
                  contextUsageBarClass(usage.pct),
                )}
                style={{ width: `${Math.min(usage.pct, 100)}%` }}
              />
            </div>
            <div className="flex gap-1.5 pt-0.5">
              {usage.usedTokens > 0 ? (
                <button
                  type="button"
                  className={cn(SESSION_ACTION_BTN, "flex-1")}
                  disabled={compacting || turnActive}
                  onClick={handleCompact}
                >
                  {compacting ? t("chat.compacting") : t("chat.compactNow")}
                </button>
              ) : null}
              <button
                type="button"
                className={cn(
                  SESSION_ACTION_BTN,
                  "inline-flex flex-1 items-center justify-center gap-1",
                )}
                onClick={handleViewContext}
              >
                <BarChart3 className="h-3 w-3" />
                {t("chat.viewContext", "View context")}
              </button>
            </div>
          </div>
        ) : null}

        {/* 核心 — Agent */}
        <div className="space-y-0.5">
          <EnvRow
            icon={Bot}
            label={t("chat.statusAgent")}
            value={agentName || t("chat.mainAgent")}
          />
        </div>

        {/* 展开更多 / 收起 */}
        <button
          type="button"
          aria-expanded={showMore}
          onClick={() => setShowMore((v) => !v)}
          className="flex w-full items-center justify-center gap-1 rounded-md py-1 text-[11px] text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground"
        >
          {showMore ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          {showMore
            ? t("workspace.sessionShowLess", "收起")
            : t("workspace.sessionShowMore", "展开更多")}
        </button>

        <AnimatedCollapse open={showMore}>
          <div className="space-y-0.5 pt-0.5">
            {cache ? (
              <EnvRow
                icon={Database}
                label={t("chat.statusCache")}
                value={formatCacheUsageDisplay({
                  created: cache.created,
                  read: cache.read,
                  writeLabel: t("chat.statusCacheWrite"),
                  hitLabel: t("chat.statusCacheHit"),
                })}
                detail={
                  cache.lastInput != null ? formatCompactTokenCount(cache.lastInput) : undefined
                }
              />
            ) : null}

            <EnvRow icon={MessageCircle} label={t("chat.statusSession")} value={sessionTitle} />

            {sessionId ? (
              <IconTip label={copied ? t("chat.copied") : t("chat.copy")}>
                <div
                  role="button"
                  tabIndex={0}
                  onClick={handleCopyId}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault()
                      void handleCopyId()
                    }
                  }}
                  className="flex min-w-0 cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-secondary/35"
                >
                  <Hash className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="w-14 shrink-0 text-muted-foreground">
                    {t("chat.statusSessionId")}
                  </span>
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/90">
                    {sessionId}
                  </span>
                  {copied ? (
                    <Check className="h-3.5 w-3.5 shrink-0 text-green-600 dark:text-green-500" />
                  ) : (
                    <Copy className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
                  )}
                </div>
              </IconTip>
            ) : null}

            <div className="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs text-muted-foreground">
              <MessageSquare className="h-3.5 w-3.5 shrink-0" />
              <span>{t("chat.statusMessages", { count: messages.length })}</span>
            </div>

            <EnvRow
              icon={Gauge}
              label={t("chat.statusThinking")}
              value={reasoningEffort ? t(`effort.${reasoningEffort}`) : "—"}
            />

            {sessionMeta?.updatedAt ? (
              <EnvRow
                icon={Clock}
                label={t("chat.statusUpdated")}
                value={formatMessageTime(sessionMeta.updatedAt)}
              />
            ) : null}

            {onViewSystemPrompt ? (
              <button
                type="button"
                className={cn(
                  SESSION_ACTION_BTN,
                  "mt-1 inline-flex w-full items-center justify-center gap-1.5",
                )}
                disabled={systemPromptLoading}
                onClick={onViewSystemPrompt}
              >
                {systemPromptLoading ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <FileText className="h-3 w-3" />
                )}
                {t("chat.viewSystemPrompt")}
              </button>
            ) : null}
          </div>
        </AnimatedCollapse>
      </div>
    </WorkspaceSection>
  )
}

function EnvironmentSection({
  sessionId,
  sessionMeta,
  project,
  effectiveWorkingDir,
  workingDirSource,
  permissionMode = "default",
  planState = "off",
  turnActive,
  onOpenDiff,
}: {
  sessionId?: string | null
  sessionMeta?: SessionMeta | null
  project?: ProjectMeta | null
  effectiveWorkingDir?: string | null
  workingDirSource?: "session" | "project"
  permissionMode?: SessionMode
  planState?: PlanModeState
  turnActive?: boolean
  onOpenDiff?: (payload: FileChangeMetadata | FileChangesMetadata) => void
}) {
  const { t } = useTranslation()
  const [gitDiffLoading, setGitDiffLoading] = useState(false)
  const appVersion = useAppVersion()
  const environmentRefreshKey = useMemo(
    () =>
      [
        effectiveWorkingDir ?? "",
        workingDirSource ?? "",
        sessionMeta?.projectId ?? "",
        project?.workingDir ?? "",
      ].join("\u001f"),
    [effectiveWorkingDir, workingDirSource, sessionMeta?.projectId, project?.workingDir],
  )
  const env = useWorkspaceEnvironment(sessionId, { turnActive, refreshKey: environmentRefreshKey })
  const dangerous = useDangerousModeStatus()
  const isLocalRuntime = useMemo(() => getTransport().supportsLocalFileOps(), [])
  const status = resolveWorkspaceEnvironmentStatus(env.snapshot, effectiveWorkingDir, !!env.error)
  const statusLabel = t(status.labelKey, status.fallback)
  const workingDir = env.snapshot?.workingDir.path ?? effectiveWorkingDir ?? null
  const workingDirName = env.snapshot?.workingDir.name ?? (workingDir ? basename(workingDir) : null)
  const source =
    env.snapshot?.workingDir.source ??
    (workingDirSource === "project"
      ? "project"
      : workingDirSource === "session"
        ? "session"
        : "none")
  const sourceLabel = workingDirSourceLabelKey(source)
  const git = env.snapshot?.git ?? null
  const currentWorktree = git?.worktrees.find((w) => w.isCurrent) ?? null
  const syncLabel = git ? gitSyncLabel(t, git) : null
  const managedWorktreesState = useManagedWorktrees(sessionId, {
    incognito: sessionMeta?.incognito,
    turnActive,
  })
  const managedWorktrees = managedWorktreesState.worktrees
  const activeManagedWorktree =
    managedWorktrees.find((wt) => wt.state !== "archived" && wt.path === workingDir) ?? null
  const [worktreeActionKey, setWorktreeActionKey] = useState<string | null>(null)
  const canOpenGitDiff = !!sessionId && !!git && !git.status.clean && !!onOpenDiff
  const handleOpenGitDiff = useCallback(async () => {
    if (!sessionId || !onOpenDiff || gitDiffLoading) return
    setGitDiffLoading(true)
    try {
      const payload = await getTransport().loadSessionGitDiff(sessionId)
      if (payload.changes.length === 0) {
        toast.info(t("workspace.environment.noTextDiff", "没有可展示的文本 diff"))
        return
      }
      onOpenDiff(payload)
    } catch (e) {
      logger.error("ui", "WorkspaceEnvironment::gitDiff", "Load git diff failed", e)
      toast.error(t("workspace.environment.gitDiffFailed", "读取 Git diff 失败"))
    } finally {
      setGitDiffLoading(false)
    }
  }, [gitDiffLoading, onOpenDiff, sessionId, t])
  const createManagedWorktree = useCallback(async () => {
    if (!sessionId || !workingDir || worktreeActionKey) return
    setWorktreeActionKey("create")
    try {
      await getTransport().call<ManagedWorktree>("create_managed_worktree", {
        sessionId,
        sourceWorkingDir: workingDir,
        label: t("workspace.worktree.manualLabel", "Manual worktree"),
        purpose: "manual",
      })
      managedWorktreesState.refresh()
      toast.success(t("workspace.worktree.created", "已创建隔离工作树"))
    } catch (e) {
      logger.error("ui", "EnvironmentSection::createManagedWorktree", "Create failed", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setWorktreeActionKey(null)
    }
  }, [managedWorktreesState, sessionId, t, workingDir, worktreeActionKey])
  const runManagedWorktreeAction = useCallback(
    async (worktree: ManagedWorktree, action: "archive" | "restore" | "handoff") => {
      if (worktreeActionKey) return
      const command =
        action === "archive"
          ? "archive_managed_worktree"
          : action === "restore"
            ? "restore_managed_worktree"
            : "handoff_managed_worktree"
      setWorktreeActionKey(`${action}:${worktree.id}`)
      try {
        await getTransport().call<ManagedWorktree>(command, { worktreeId: worktree.id })
        managedWorktreesState.refresh()
        toast.success(
          action === "archive"
            ? t("workspace.worktree.archived", "已归档工作树")
            : action === "restore"
              ? t("workspace.worktree.restored", "已恢复工作树")
              : t("workspace.worktree.handoffDone", "已交接到当前会话"),
        )
      } catch (e) {
        logger.error("ui", "EnvironmentSection::managedWorktreeAction", `${action} failed`, e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setWorktreeActionKey(null)
      }
    },
    [managedWorktreesState, t, worktreeActionKey],
  )

  const sessionSource = sessionMeta?.channelInfo
    ? {
        icon: MessageCircle,
        value: sessionMeta.channelInfo.channelId,
        detail:
          sessionMeta.channelInfo.senderName ||
          sessionMeta.channelInfo.chatId ||
          sessionMeta.channelInfo.chatType,
      }
    : sessionMeta?.isCron
      ? {
          icon: CalendarClock,
          value: t("workspace.environment.sourceCron", "定时任务"),
          detail: null,
        }
      : sessionMeta?.parentSessionId
        ? {
            icon: Bot,
            value: t("workspace.environment.sourceSubagent", "子 Agent"),
            detail: sessionMeta.parentSessionId.slice(0, 8),
          }
        : { icon: Radio, value: t("workspace.environment.sourceChat", "普通会话"), detail: null }

  return (
    <WorkspaceSection
      title={t("workspace.sectionEnvironment", "环境")}
      icon={Cpu}
      meta={<StatusPill label={statusLabel} tone={status.tone} loading={env.loading} />}
    >
      <div className="space-y-0.5">
        <EnvRow
          icon={Monitor}
          label={t("workspace.environment.version", "版本")}
          value={`v${appVersion}`}
        />

        <EnvRow
          icon={isLocalRuntime ? HardDrive : Server}
          label={t("workspace.environment.runtime", "运行")}
          value={
            isLocalRuntime
              ? t("workspace.environment.runtimeLocal", "本机桌面")
              : t("workspace.environment.runtimeRemote", "远端服务")
          }
        />

        <EnvRow
          icon={FolderOpen}
          label={t("workspace.environment.workingDir", "目录")}
          value={workingDirName || t("workspace.environment.noWorkingDir", "未设置")}
          detail={t(sourceLabel.key, sourceLabel.fallback)}
          title={workingDir ?? undefined}
          tone={status.kind === "missingWorkingDir" ? "danger" : "muted"}
        />

        {project ? (
          <EnvRow
            icon={FolderGit2}
            label={t("workspace.environment.project", "项目")}
            value={project.name}
            detail={project.archived ? t("workspace.environment.archived", "已归档") : undefined}
          />
        ) : null}

        <EnvRow
          icon={sessionSource.icon}
          label={t("workspace.environment.source", "来源")}
          value={sessionSource.value}
          detail={sessionSource.detail}
        />

        {sessionMeta?.incognito ? (
          <EnvRow
            icon={EyeOff}
            label={t("workspace.environment.privacy", "隐私")}
            value={t("chat.incognito", "无痕")}
            detail={t("workspace.environment.incognitoDetail", "不读取历史产物")}
            tone="info"
          />
        ) : null}

        <EnvRow
          icon={dangerous.active ? ShieldAlert : Shield}
          label={t("workspace.environment.permission", "权限")}
          value={t(`chat.permissionMode.${permissionMode}.label`, permissionMode)}
          detail={
            dangerous.active ? t("workspace.environment.dangerousMode", "危险模式") : undefined
          }
          tone={dangerous.active || permissionMode === "yolo" ? "danger" : "muted"}
        />

        {planState !== "off" ? (
          <EnvRow
            icon={GitPullRequest}
            label={t("workspace.environment.plan", "计划")}
            value={planStateLabel(t, planState)}
            tone={planState === "executing" ? "info" : planState === "completed" ? "good" : "muted"}
          />
        ) : null}

        {env.error ? (
          <EnvRow
            icon={CircleAlert}
            label={t("workspace.environment.statusLabel", "状态")}
            value={t("workspace.environment.unavailable", "无法读取环境状态")}
            detail={env.error}
            tone="warn"
          />
        ) : null}

        {git ? (
          <>
            <EnvRow
              icon={GitBranch}
              label={t("workspace.environment.branch", "分支")}
              value={formatGitRef(git)}
              detail={
                git.detached ? t("fileBrowser.gitDetached", "detached") : (git.head ?? undefined)
              }
            />
            {currentWorktree || git.worktrees.length > 1 ? (
              <EnvRow
                icon={FolderGit2}
                label={t("workspace.environment.worktree", "工作树")}
                value={currentWorktree ? basename(currentWorktree.path) : basename(git.root)}
                detail={
                  git.worktrees.length > 1
                    ? t("workspace.environment.worktreeCount", "{{count}} 个", {
                        count: git.worktrees.length,
                      })
                    : undefined
                }
                title={currentWorktree?.path ?? git.root}
              />
            ) : null}
            <ManagedWorktreesMiniPanel
              worktrees={managedWorktrees}
              activeWorktree={activeManagedWorktree}
              loading={managedWorktreesState.loading}
              error={managedWorktreesState.error}
              actionKey={worktreeActionKey}
              canCreate={Boolean(sessionId && workingDir && git)}
              onCreate={() => void createManagedWorktree()}
              onAction={(worktree, action) => void runManagedWorktreeAction(worktree, action)}
            />
            <EnvRow
              icon={git.status.clean ? CheckCircle2 : GitCompare}
              label={t("workspace.environment.changes", "变更")}
              value={
                gitDiffLoading ? (
                  <span className="inline-flex items-center gap-1">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {t("workspace.environment.loadingDiff", "读取 diff")}
                  </span>
                ) : git.status.clean ? (
                  t("workspace.environment.clean", "无本地变更")
                ) : (
                  t("workspace.environment.changedFiles", "{{count}} 个文件", {
                    count: git.status.changedFiles,
                  })
                )
              }
              detail={
                git.status.linesAdded > 0 || git.status.linesRemoved > 0 ? (
                  <FileDeltaCounter
                    linesAdded={git.status.linesAdded}
                    linesRemoved={git.status.linesRemoved}
                    className="text-[10px]"
                  />
                ) : git.status.conflictedFiles > 0 ? (
                  t("workspace.environment.conflictCount", "{{count}} 个冲突", {
                    count: git.status.conflictedFiles,
                  })
                ) : undefined
              }
              tone={git.status.conflictedFiles > 0 ? "danger" : git.status.clean ? "good" : "warn"}
              onClick={canOpenGitDiff ? handleOpenGitDiff : undefined}
              disabled={gitDiffLoading}
            />
            {(syncLabel || git.sync.upstream || git.sync.remote) && (
              <EnvRow
                icon={GitPullRequest}
                label={t("workspace.environment.sync", "同步")}
                value={
                  syncLabel ??
                  git.sync.upstream ??
                  t("workspace.environment.syncUnknown", "同步状态未知")
                }
                detail={git.sync.upstream ?? git.sync.remote ?? undefined}
                tone={
                  git.sync.state === "diverged"
                    ? "danger"
                    : git.sync.state === "behind"
                      ? "warn"
                      : git.sync.state === "ahead"
                        ? "info"
                        : "muted"
                }
                title={git.sync.remote ?? git.sync.upstream ?? undefined}
              />
            )}
            {git.lastCommit ? (
              <EnvRow
                icon={GitCommitHorizontal}
                label={t("workspace.environment.commit", "提交")}
                value={git.lastCommit.subject}
                detail={git.lastCommit.hash}
              />
            ) : null}
          </>
        ) : env.snapshot && workingDir ? (
          <EnvRow
            icon={GitBranch}
            label={t("workspace.environment.git", "Git")}
            value={t("workspace.environment.nonGit", "非 Git 工作目录")}
          />
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function ManagedWorktreesMiniPanel({
  worktrees,
  activeWorktree,
  loading,
  error,
  actionKey,
  canCreate,
  onCreate,
  onAction,
}: {
  worktrees: ManagedWorktree[]
  activeWorktree?: ManagedWorktree | null
  loading?: boolean
  error?: string | null
  actionKey?: string | null
  canCreate?: boolean
  onCreate: () => void
  onAction: (worktree: ManagedWorktree, action: "archive" | "restore" | "handoff") => void
}) {
  const { t } = useTranslation()
  const visible = worktrees.slice(0, 4)
  const createBusy = actionKey === "create"
  return (
    <div className="rounded-md border border-border/55 bg-secondary/15">
      <div className="flex min-w-0 items-center gap-2 px-2 py-1.5">
        <FolderGit2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/85">
          {t("workspace.worktree.managed", "Managed worktrees")}
        </span>
        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
        <IconTip label={t("workspace.worktree.create", "创建隔离工作树")}>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-6 w-6"
            disabled={!canCreate || Boolean(actionKey)}
            onClick={onCreate}
          >
            {createBusy ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Plus className="h-3 w-3" />
            )}
          </Button>
        </IconTip>
      </div>
      {error ? (
        <div className="border-t border-border/60 px-2 py-1.5 text-[10px] text-destructive">
          {truncateMiddle(error, 120)}
        </div>
      ) : visible.length === 0 ? (
        <div className="border-t border-border/60 px-2 py-1.5 text-[10px] text-muted-foreground">
          {t("workspace.worktree.empty", "暂无 managed worktree")}
        </div>
      ) : (
        <div className="space-y-1 border-t border-border/60 p-1.5">
          {visible.map((worktree) => {
            const isActive = activeWorktree?.id === worktree.id
            const busyPrefix = actionKey?.endsWith(`:${worktree.id}`)
              ? actionKey.split(":")[0]
              : null
            return (
              <div
                key={worktree.id}
                className={cn(
                  "flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-[10px]",
                  isActive ? "bg-primary/10" : "bg-background/35",
                )}
                title={worktree.path}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className="min-w-0 truncate font-medium text-foreground/85">
                      {worktree.label || basename(worktree.path)}
                    </span>
                    <StatusPill
                      label={managedWorktreeStateLabel(t, worktree.state)}
                      tone={managedWorktreeStateTone(worktree.state)}
                    />
                  </div>
                  <div className="mt-0.5 flex min-w-0 gap-1.5 text-muted-foreground">
                    <span className="truncate">
                      {managedWorktreePurposeLabel(t, worktree.purpose)}
                    </span>
                    <span className="shrink-0 text-muted-foreground/45">·</span>
                    <span className="truncate">{worktreeDirtySummary(t, worktree)}</span>
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-0.5">
                  {worktree.state === "archived" || !worktree.pathExists ? (
                    <IconTip label={t("workspace.worktree.restore", "恢复")}>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-6 w-6"
                        disabled={Boolean(actionKey)}
                        onClick={() => onAction(worktree, "restore")}
                      >
                        {busyPrefix === "restore" ? (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        ) : (
                          <Play className="h-3 w-3" />
                        )}
                      </Button>
                    </IconTip>
                  ) : (
                    <>
                      <IconTip label={t("workspace.worktree.handoff", "交接到当前会话")}>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="h-6 w-6"
                          disabled={Boolean(actionKey) || isActive}
                          onClick={() => onAction(worktree, "handoff")}
                        >
                          {busyPrefix === "handoff" ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <GitPullRequest className="h-3 w-3" />
                          )}
                        </Button>
                      </IconTip>
                      <IconTip label={t("workspace.worktree.archive", "归档")}>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="h-6 w-6 text-muted-foreground hover:text-destructive"
                          disabled={Boolean(actionKey)}
                          onClick={() => onAction(worktree, "archive")}
                        >
                          {busyPrefix === "archive" ? (
                            <Loader2 className="h-3 w-3 animate-spin" />
                          ) : (
                            <X className="h-3 w-3" />
                          )}
                        </Button>
                      </IconTip>
                    </>
                  )}
                </div>
              </div>
            )
          })}
          {worktrees.length > visible.length ? (
            <div className="px-1.5 pb-0.5 text-[10px] text-muted-foreground">
              {t("workspace.worktree.more", "另有 {{count}} 个", {
                count: worktrees.length - visible.length,
              })}
            </div>
          ) : null}
        </div>
      )}
    </div>
  )
}

function managedWorktreeStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: ManagedWorktree["state"],
): string {
  switch (state) {
    case "active":
      return t("workspace.worktree.stateActive", "Active")
    case "archived":
      return t("workspace.worktree.stateArchived", "Archived")
    case "handoff":
      return t("workspace.worktree.stateHandoff", "Handoff")
  }
}

function managedWorktreeStateTone(state: ManagedWorktree["state"]): StatusTone {
  if (state === "active") return "good"
  if (state === "handoff") return "info"
  return "muted"
}

function managedWorktreePurposeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  purpose: ManagedWorktree["purpose"],
): string {
  switch (purpose) {
    case "workflow":
      return t("workspace.worktree.purposeWorkflow", "Workflow")
    case "subagent":
      return t("workspace.worktree.purposeSubagent", "Subagent")
    case "manual":
      return t("workspace.worktree.purposeManual", "Manual")
  }
}

function worktreeDirtySummary(
  t: ReturnType<typeof useTranslation>["t"],
  worktree: ManagedWorktree,
): string {
  const dirty = worktree.dirtySnapshot
  if (!worktree.pathExists) return t("workspace.worktree.pathMissing", "路径已清理")
  if (!dirty) return worktree.baseBranch || worktree.baseRef || worktree.baseSha?.slice(0, 8) || "-"
  if (dirty.clean) return t("workspace.worktree.clean", "无本地变更")
  return t("workspace.worktree.changed", "{{count}} 个变更", { count: dirty.changedFiles })
}

/**
 * 知识空间段:① 本会话挂载的知识空间(owner 平面 list_session_kbs_cmd,带读/写徽章
 * + 项目来源 + 外部锁);② 本会话的笔记活动(live-tail:写入 / 读取的笔记 + 检索次数)。
 * 无痕会话不拉挂载列表(D10 关闭即焚),活动走 live-tail 自然为空。
 */
function KnowledgeSection({
  sessionId,
  projectId,
  incognito,
  messages,
}: {
  sessionId?: string | null
  projectId?: string | null
  incognito?: boolean
  messages: Message[]
}) {
  const { t } = useTranslation()
  const { attachments, activity } = useSessionKnowledge(sessionId, projectId, {
    incognito,
    messages,
  })
  const hasContent =
    attachments.length > 0 || activity.entries.length > 0 || activity.searchCount > 0

  return (
    <WorkspaceSection
      title={t("workspace.sectionKnowledge", "知识空间")}
      count={attachments.length}
      icon={BookText}
    >
      {hasContent ? (
        <div className="space-y-2">
          {attachments.length > 0 && (
            <div className="space-y-1">
              {attachments.map((kb) => {
                const external = !!kb.rootDir
                return (
                  <div
                    key={kb.id}
                    className="flex items-center gap-2 rounded-md border border-border/50 bg-secondary/30 px-2.5 py-1.5"
                  >
                    <span className="shrink-0 text-sm leading-none">{kb.emoji || "📚"}</span>
                    <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                      {kb.name}
                    </span>
                    {external && (
                      <IconTip label={t("knowledge.picker.external", "外部库")}>
                        <Lock className="h-3 w-3 shrink-0 text-muted-foreground/70" />
                      </IconTip>
                    )}
                    {kb.via === "project" && (
                      <span className="shrink-0 text-[10px] text-muted-foreground/60">
                        {t("workspace.kbViaProject", "项目")}
                      </span>
                    )}
                    <span className="shrink-0 rounded bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {kb.access === "write"
                        ? t("knowledge.picker.accessWrite", "读写")
                        : t("knowledge.picker.accessRead", "只读")}
                    </span>
                  </div>
                )
              })}
            </div>
          )}

          {activity.entries.length > 0 && (
            <div className="space-y-0.5">
              {activity.entries.map((e) => (
                <IconTip key={e.key} label={e.ref}>
                  <div className="flex items-center gap-2 rounded-md px-2 py-1 hover:bg-secondary/40">
                    <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate text-xs text-foreground/85">
                      {basename(e.ref)}
                    </span>
                    <span
                      className={cn(
                        "shrink-0 text-[10px]",
                        e.kind === "write"
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-muted-foreground/70",
                      )}
                    >
                      {e.kind === "write"
                        ? t("workspace.kbWrote", "写入")
                        : t("workspace.kbRead", "读取")}
                    </span>
                  </div>
                </IconTip>
              ))}
            </div>
          )}

          {activity.searchCount > 0 && (
            <div className="px-2 pt-0.5 text-[10px] text-muted-foreground/60">
              {t("workspace.kbSearchCount", {
                n: activity.searchCount,
                defaultValue: "检索 {{n}} 次",
              })}
            </div>
          )}
        </div>
      ) : (
        <EmptyHint>{t("workspace.emptyKnowledge", "未挂载知识空间")}</EmptyHint>
      )}
    </WorkspaceSection>
  )
}

function contextKindIcon(kind: ContextCandidateKind): LucideIcon {
  switch (kind) {
    case "file":
      return FileText
    case "symbol":
      return Hash
    case "diagnostic":
      return CircleAlert
    case "review_finding":
      return GitPullRequest
    case "verification_step":
      return CheckCircle2
    case "goal_evidence":
      return Brain
    case "task":
      return Check
    case "workflow_op":
      return Layers
    case "ide_context":
      return Monitor
    case "url_source":
      return Globe
    case "document":
      return FileText
    case "email_thread":
      return MessageCircle
    case "calendar_event":
      return CalendarClock
    case "sheet_range":
      return Database
    case "knowledge_note":
      return BookText
    case "web_source":
      return Globe
    case "decision":
      return Brain
    case "artifact":
      return Files
  }
}

function contextKindLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: ContextCandidateKind,
): string {
  switch (kind) {
    case "file":
      return t("workspace.context.kindFile", "文件")
    case "symbol":
      return t("workspace.context.kindSymbol", "符号")
    case "diagnostic":
      return t("workspace.context.kindDiagnostic", "诊断")
    case "review_finding":
      return t("workspace.context.kindReview", "审查")
    case "verification_step":
      return t("workspace.context.kindVerification", "验证")
    case "goal_evidence":
      return t("workspace.context.kindGoal", "Goal")
    case "task":
      return t("workspace.context.kindTask", "任务")
    case "workflow_op":
      return t("workspace.context.kindWorkflow", "工作流")
    case "ide_context":
      return t("workspace.context.kindIde", "IDE")
    case "url_source":
      return t("workspace.context.kindUrl", "来源")
    case "document":
      return t("workspace.context.kindDocument", "文档")
    case "email_thread":
      return t("workspace.context.kindEmailThread", "邮件")
    case "calendar_event":
      return t("workspace.context.kindCalendarEvent", "日程")
    case "sheet_range":
      return t("workspace.context.kindSheetRange", "表格")
    case "knowledge_note":
      return t("workspace.context.kindKnowledgeNote", "笔记")
    case "web_source":
      return t("workspace.context.kindWebSource", "网页")
    case "decision":
      return t("workspace.context.kindDecision", "决策")
    case "artifact":
      return t("workspace.context.kindArtifact", "产物")
  }
}

function contextCandidateTone(candidate: ContextCandidate): StatusTone {
  const status = candidate.status ?? ""
  if (
    status.includes("p0") ||
    status.includes("p1") ||
    status.includes("error") ||
    status.includes("failed") ||
    status.includes("timed_out") ||
    status.includes("blocked")
  ) {
    return "danger"
  }
  if (
    status.includes("p2") ||
    status.includes("warning") ||
    status.includes("pending") ||
    status.includes("skipped") ||
    status.includes("awaiting")
  ) {
    return "warn"
  }
  if (status.includes("passed") || status.includes("completed")) return "good"
  if (status.includes("running") || status.includes("in_progress")) return "info"
  if (candidate.kind === "symbol") return "info"
  return "muted"
}

function contextLocationLabel(candidate: ContextCandidate): string | null {
  const path = candidate.path ?? candidate.url ?? candidate.subtitle ?? null
  if (!path) return null
  const base = candidate.url ? domainOf(candidate.url) : basename(path)
  if (candidate.line != null) return `${base}:${candidate.line}`
  return base
}

type ContextFocusedAction = "review" | "verify"

function contextCandidateFocusPaths(candidate: ContextCandidate): string[] {
  const actions = candidate.metadata?.actions
  if (actions && typeof actions === "object" && !Array.isArray(actions)) {
    const focusPaths = (actions as Record<string, unknown>).focusPaths
    if (Array.isArray(focusPaths)) {
      const paths = focusPaths.filter(
        (path): path is string => typeof path === "string" && path.length > 0,
      )
      if (paths.length > 0) return paths
    }
  }
  if (
    candidate.kind === "document" ||
    candidate.kind === "email_thread" ||
    candidate.kind === "calendar_event" ||
    candidate.kind === "sheet_range" ||
    candidate.kind === "knowledge_note" ||
    candidate.kind === "web_source" ||
    candidate.kind === "decision" ||
    candidate.kind === "artifact"
  ) {
    return []
  }
  return candidate.path ? [candidate.path] : []
}

function ContextCandidateActions({
  candidate,
  disabled,
  actionKey,
  onAction,
}: {
  candidate: ContextCandidate
  disabled?: boolean
  actionKey?: string | null
  onAction?: (candidate: ContextCandidate, action: ContextFocusedAction) => void
}) {
  const { t } = useTranslation()
  const focusPaths = contextCandidateFocusPaths(candidate)
  if (!onAction || focusPaths.length === 0) return null
  const reviewKey = `${candidate.id}:review`
  const verifyKey = `${candidate.id}:verify`
  const busy = Boolean(actionKey)
  return (
    <div className="flex shrink-0 items-center gap-1">
      <IconTip label={t("workspace.context.focusReview", "聚焦审查")}>
        <button
          type="button"
          disabled={disabled || busy}
          onClick={(e) => {
            e.stopPropagation()
            onAction(candidate, "review")
          }}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border/50 bg-background/65 text-muted-foreground transition-colors hover:border-primary/45 hover:text-foreground disabled:opacity-45"
          aria-label={t("workspace.context.focusReview", "聚焦审查")}
        >
          {actionKey === reviewKey ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <GitPullRequest className="h-3.5 w-3.5" />
          )}
        </button>
      </IconTip>
      <IconTip label={t("workspace.context.focusVerify", "聚焦验证")}>
        <button
          type="button"
          disabled={disabled || busy}
          onClick={(e) => {
            e.stopPropagation()
            onAction(candidate, "verify")
          }}
          className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border/50 bg-background/65 text-muted-foreground transition-colors hover:border-primary/45 hover:text-foreground disabled:opacity-45"
          aria-label={t("workspace.context.focusVerify", "聚焦验证")}
        >
          {actionKey === verifyKey ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Gauge className="h-3.5 w-3.5" />
          )}
        </button>
      </IconTip>
    </div>
  )
}

function contextDomainActions(candidate: ContextCandidate): Record<string, unknown> | null {
  const actions = candidate.metadata?.domainActions
  if (!actions || typeof actions !== "object" || Array.isArray(actions)) return null
  return actions as Record<string, unknown>
}

function contextCitationText(candidate: ContextCandidate): string {
  const location = candidate.url ?? candidate.path ?? candidate.subtitle ?? ""
  const status = candidate.status ? ` (${candidate.status})` : ""
  return location ? `${candidate.title}${status}: ${location}` : `${candidate.title}${status}`
}

function contextCandidateStringMetadata(candidate: ContextCandidate, key: string): string | null {
  const value = candidate.metadata?.[key]
  return typeof value === "string" && value.trim().length > 0 ? value : null
}

function contextCandidateNumberMetadata(candidate: ContextCandidate, key: string): number | null {
  const value = candidate.metadata?.[key]
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function contextCandidateDomain(candidate: ContextCandidate): string {
  return contextCandidateStringMetadata(candidate, "domain") ?? "general"
}

function contextCandidateEvidenceType(candidate: ContextCandidate): string {
  const explicit = contextCandidateStringMetadata(candidate, "evidenceType")
  if (explicit) return explicit
  switch (candidate.kind) {
    case "decision":
      return "user_decision"
    case "artifact":
      return "artifact_created"
    case "sheet_range":
      return "data_quality_checked"
    case "calendar_event":
      return "meeting_context_collected"
    case "email_thread":
      return "source_cited"
    case "web_source":
    case "document":
    case "knowledge_note":
    case "goal_evidence":
    default:
      return "source_cited"
  }
}

function contextCandidateAccessScope(candidate: ContextCandidate): string {
  const explicit = contextCandidateStringMetadata(candidate, "accessScope")
  if (explicit) return explicit
  switch (candidate.kind) {
    case "email_thread":
    case "calendar_event":
    case "sheet_range":
      return "connector"
    case "web_source":
      return "public"
    case "document":
    case "knowledge_note":
    case "artifact":
      return "project"
    case "decision":
    case "goal_evidence":
    default:
      return "session"
  }
}

function contextCandidateRedactionStatus(candidate: ContextCandidate): string {
  return contextCandidateStringMetadata(candidate, "redactionStatus") ?? "none"
}

function contextCandidateSourceMetadata(candidate: ContextCandidate): Record<string, unknown> {
  return {
    source: "context_retrieval",
    candidateId: candidate.id,
    candidateKind: candidate.kind,
    title: candidate.title,
    subtitle: candidate.subtitle ?? null,
    path: candidate.path ?? null,
    line: candidate.line ?? null,
    url: candidate.url ?? null,
    status: candidate.status ?? null,
    reasons: candidate.reasons,
    sources: candidate.sources,
    originalMetadata: candidate.metadata ?? {},
  }
}

function contextCandidateEvidenceInput(
  candidate: ContextCandidate,
  sessionId: string,
): RecordDomainEvidenceInput {
  return {
    sessionId,
    domain: contextCandidateDomain(candidate),
    evidenceType: contextCandidateEvidenceType(candidate),
    title: candidate.title,
    summary: candidate.subtitle ?? candidate.reasons[0] ?? null,
    sourceMetadata: contextCandidateSourceMetadata(candidate),
    confidence: contextCandidateNumberMetadata(candidate, "confidence"),
    accessScope: contextCandidateAccessScope(candidate),
    redactionStatus: contextCandidateRedactionStatus(candidate),
  }
}

function contextCandidateSummary(candidate: ContextCandidate): string {
  const lines: string[] = []
  if (candidate.subtitle) lines.push(`摘要线索：${candidate.subtitle}`)
  const location = contextLocationLabel(candidate)
  if (location) lines.push(`位置：${location}`)
  if (candidate.status) lines.push(`状态：${candidate.status}`)
  if (candidate.reasons.length > 0) lines.push(`推荐原因：${candidate.reasons.join("；")}`)
  if (candidate.sources.length > 0) lines.push(`来源信号：${candidate.sources.join(", ")}`)
  return lines.join("\n") || candidate.title
}

function contextCandidateSummaryEvidenceInput(
  candidate: ContextCandidate,
  sessionId: string,
): RecordDomainEvidenceInput {
  return {
    sessionId,
    domain: contextCandidateDomain(candidate),
    evidenceType: "artifact_created",
    title: `上下文摘要：${candidate.title}`,
    summary: contextCandidateSummary(candidate),
    sourceMetadata: {
      ...contextCandidateSourceMetadata(candidate),
      action: "summarize",
      artifactKind: "context_summary",
    },
    confidence: contextCandidateNumberMetadata(candidate, "confidence"),
    accessScope: contextCandidateAccessScope(candidate),
    redactionStatus: contextCandidateRedactionStatus(candidate),
  }
}

function contextCandidateConflictEvidenceInput(
  candidate: ContextCandidate,
  sessionId: string,
): RecordDomainEvidenceInput {
  return {
    sessionId,
    domain: contextCandidateDomain(candidate),
    evidenceType: "claim_checked",
    title: `冲突待复核：${candidate.title}`,
    summary: candidate.subtitle ?? candidate.reasons[0] ?? null,
    sourceMetadata: {
      ...contextCandidateSourceMetadata(candidate),
      action: "mark_conflict",
      verdict: "conflict",
      conflict: true,
      requiresUserReview: true,
    },
    confidence: contextCandidateNumberMetadata(candidate, "confidence"),
    accessScope: contextCandidateAccessScope(candidate),
    redactionStatus: contextCandidateRedactionStatus(candidate),
  }
}

function contextCandidateAskUserInput(
  candidate: ContextCandidate,
  sessionId: string,
): CreateOwnerAskUserQuestionInput {
  const title = candidate.title
  const context = candidate.subtitle
    ? `请确认这条上下文是否应该作为当前任务决策依据：${candidate.subtitle}`
    : "请确认这条上下文是否应该作为当前任务决策依据。"
  return {
    sessionId,
    source: "workspace_context",
    context,
    questions: [
      {
        questionId: "context_confirmation",
        header: "上下文",
        text: `是否采用「${title}」作为当前任务的有效上下文？`,
        allowCustom: true,
        multiSelect: false,
        options: [
          {
            value: "confirm",
            label: "采用",
            description: "把这条上下文记录为用户确认的有效依据。",
            recommended: true,
          },
          {
            value: "reject",
            label: "不采用",
            description: "把这条上下文记录为用户拒绝或暂不采纳。",
          },
        ],
      },
    ],
    ownerResponse: {
      action: "record_domain_evidence",
      domainEvidence: {
        sessionId,
        domain: contextCandidateDomain(candidate),
        evidenceType: "user_decision",
        title: `用户确认：${title}`,
        summary: candidate.subtitle ?? candidate.reasons[0] ?? null,
        sourceMetadata: {
          ...contextCandidateSourceMetadata(candidate),
          action: "ask_user_confirmation",
        },
        confidence: contextCandidateNumberMetadata(candidate, "confidence"),
        accessScope: contextCandidateAccessScope(candidate),
        redactionStatus: contextCandidateRedactionStatus(candidate),
      },
    },
  }
}

function contextCanAddEvidence(candidate: ContextCandidate): boolean {
  const actions = contextDomainActions(candidate)
  if (!actions?.canAddEvidence) return false
  return contextCandidateStringMetadata(candidate, "origin") !== "domain_evidence"
}

function contextCanSummarize(candidate: ContextCandidate): boolean {
  const actions = contextDomainActions(candidate)
  return Boolean(actions?.canSummarize)
}

function contextCanAskUser(candidate: ContextCandidate): boolean {
  const actions = contextDomainActions(candidate)
  return Boolean(actions?.canAskUser)
}

function contextCanMarkConflict(candidate: ContextCandidate): boolean {
  const actions = contextDomainActions(candidate)
  return Boolean(actions?.canMarkConflict)
}

function contextCanCreateTask(candidate: ContextCandidate): boolean {
  const actions = contextDomainActions(candidate)
  return Boolean(actions?.canCreateTask)
}

function contextCandidateTaskContent(candidate: ContextCandidate): string {
  const location = contextLocationLabel(candidate)
  const evidenceType = contextCandidateEvidenceType(candidate).replace(/_/g, " ")
  if (location) {
    return `处理上下文：${candidate.title} (${location})`
  }
  if (candidate.subtitle) {
    return `处理上下文：${candidate.title} - ${candidate.subtitle}`
  }
  return `处理上下文：${candidate.title} (${evidenceType})`
}

function contextCandidateTaskActiveForm(candidate: ContextCandidate): string {
  const location = candidate.url ?? candidate.path ?? candidate.subtitle ?? candidate.title
  return `正在处理上下文：${location}`
}

function DomainContextActionChips({
  candidate,
  sessionId,
  disabled,
  actionKey,
  onSummarize,
  onAskUser,
  onAddEvidence,
  onMarkConflict,
  onCreateTask,
}: {
  candidate: ContextCandidate
  sessionId?: string | null
  disabled?: boolean
  actionKey?: string | null
  onSummarize?: (candidate: ContextCandidate) => void
  onAskUser?: (candidate: ContextCandidate) => void
  onAddEvidence?: (candidate: ContextCandidate) => void
  onMarkConflict?: (candidate: ContextCandidate) => void
  onCreateTask?: (candidate: ContextCandidate) => void
}) {
  const { t } = useTranslation()
  const actions = contextDomainActions(candidate)
  if (!actions) return null
  const chips: string[] = []
  const canCite = Boolean(actions.canCite)
  const canSummarize = contextCanSummarize(candidate)
  const canAskUser = contextCanAskUser(candidate)
  const canAddEvidence = contextCanAddEvidence(candidate)
  const canMarkConflict = contextCanMarkConflict(candidate)
  const canCreateTask = contextCanCreateTask(candidate)
  if (
    !canCite &&
    !canSummarize &&
    !canAskUser &&
    !canAddEvidence &&
    !canMarkConflict &&
    !canCreateTask &&
    chips.length === 0
  ) {
    return null
  }
  const summaryKey = `${candidate.id}:summary`
  const summaryBusy = actionKey === summaryKey
  const askKey = `${candidate.id}:ask`
  const askBusy = actionKey === askKey
  const evidenceKey = `${candidate.id}:evidence`
  const evidenceBusy = actionKey === evidenceKey
  const conflictKey = `${candidate.id}:conflict`
  const conflictBusy = actionKey === conflictKey
  const taskKey = `${candidate.id}:task`
  const taskBusy = actionKey === taskKey

  const copyCitation = async (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation()
    try {
      await navigator.clipboard.writeText(contextCitationText(candidate))
      toast.success(t("workspace.context.citationCopied", "已复制引用"))
    } catch (e) {
      logger.error("ui", "DomainContextActionChips", "Copy citation failed", e)
      toast.error(t("workspace.context.citationCopyFailed", "复制引用失败"))
    }
  }

  return (
    <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1">
      {canCite ? (
        <button
          type="button"
          onClick={copyCitation}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground"
        >
          <Copy className="h-3 w-3" />
          <span>{t("workspace.context.actionCite", "引用")}</span>
        </button>
      ) : null}
      {canSummarize ? (
        <button
          type="button"
          disabled={disabled || !sessionId || !onSummarize || Boolean(actionKey)}
          onClick={(event) => {
            event.stopPropagation()
            onSummarize?.(candidate)
          }}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {summaryBusy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <BookText className="h-3 w-3" />
          )}
          <span>{t("workspace.context.actionSummarize", "摘要")}</span>
        </button>
      ) : null}
      {canAskUser ? (
        <button
          type="button"
          disabled={disabled || !sessionId || !onAskUser || Boolean(actionKey)}
          onClick={(event) => {
            event.stopPropagation()
            onAskUser?.(candidate)
          }}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {askBusy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <MessageCircle className="h-3 w-3" />
          )}
          <span>{t("workspace.context.actionAsk", "确认")}</span>
        </button>
      ) : null}
      {canAddEvidence ? (
        <button
          type="button"
          disabled={disabled || !sessionId || !onAddEvidence || Boolean(actionKey)}
          onClick={(event) => {
            event.stopPropagation()
            onAddEvidence?.(candidate)
          }}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {evidenceBusy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <Database className="h-3 w-3" />
          )}
          <span>{t("workspace.context.actionEvidence", "证据")}</span>
        </button>
      ) : null}
      {canMarkConflict ? (
        <button
          type="button"
          disabled={disabled || !sessionId || !onMarkConflict || Boolean(actionKey)}
          onClick={(event) => {
            event.stopPropagation()
            onMarkConflict?.(candidate)
          }}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-amber-500/50 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {conflictBusy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <ShieldAlert className="h-3 w-3" />
          )}
          <span>{t("workspace.context.actionConflict", "冲突")}</span>
        </button>
      ) : null}
      {canCreateTask ? (
        <button
          type="button"
          disabled={disabled || !sessionId || !onCreateTask || Boolean(actionKey)}
          onClick={(event) => {
            event.stopPropagation()
            onCreateTask?.(candidate)
          }}
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {taskBusy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <Plus className="h-3 w-3" />
          )}
          <span>{t("workspace.context.actionTask", "转任务")}</span>
        </button>
      ) : null}
      {chips.slice(0, 4).map((chip) => (
        <span
          key={chip}
          className="rounded border border-border/40 bg-background/45 px-1.5 py-0.5 text-[10px] text-muted-foreground/80"
        >
          {chip}
        </span>
      ))}
    </div>
  )
}

function ContextFileCandidateRow({
  candidate,
  sessionId,
  onPreviewFile,
  actionKey,
  actionsDisabled,
  onAction,
  onSummarize,
  onAskUser,
  onAddEvidence,
  onMarkConflict,
  onCreateTask,
}: {
  candidate: ContextCandidate
  sessionId?: string | null
  onPreviewFile?: (target: PreviewTarget) => void
  actionKey?: string | null
  actionsDisabled?: boolean
  onAction?: (candidate: ContextCandidate, action: ContextFocusedAction) => void
  onSummarize?: (candidate: ContextCandidate) => void
  onAskUser?: (candidate: ContextCandidate) => void
  onAddEvidence?: (candidate: ContextCandidate) => void
  onMarkConflict?: (candidate: ContextCandidate) => void
  onCreateTask?: (candidate: ContextCandidate) => void
}) {
  const { t } = useTranslation()
  const Icon = contextKindIcon(candidate.kind)
  const path = candidate.path ?? ""
  const target = useMemo<PreviewTarget>(
    () => ({ kind: "path", path, name: basename(path) || candidate.title }),
    [candidate.title, path],
  )
  const overrides = useMemo(() => ({ sessionId, onPreviewFile }), [sessionId, onPreviewFile])
  const { primary, run } = useFileActions(target, overrides)
  return (
    <div className="flex w-full min-w-0 items-stretch rounded-md border border-border/50 bg-secondary/25 transition-colors hover:bg-secondary/40">
      <IconTip label={path}>
        <div
          role="button"
          tabIndex={0}
          onClick={() => run(primary)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault()
              run(primary)
            }
          }}
          className="flex min-w-0 flex-1 items-start gap-2 px-2.5 py-1.5 text-left"
        >
          <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-1.5">
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                {candidate.title}
              </span>
              {candidate.status ? (
                <StatusPill label={candidate.status} tone={contextCandidateTone(candidate)} />
              ) : null}
            </div>
            <div className="mt-1 line-clamp-2 text-[11px] leading-snug text-muted-foreground">
              {candidate.reasons[0] ?? contextKindLabel(t, candidate.kind)}
            </div>
            <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground/65">
              <span className="truncate">
                {contextLocationLabel(candidate) ?? candidate.subtitle}
              </span>
              <span className="shrink-0">{contextKindLabel(t, candidate.kind)}</span>
            </div>
            <DomainContextActionChips
              candidate={candidate}
              sessionId={sessionId}
              disabled={actionsDisabled}
              actionKey={actionKey}
              onSummarize={onSummarize}
              onAskUser={onAskUser}
              onAddEvidence={onAddEvidence}
              onMarkConflict={onMarkConflict}
              onCreateTask={onCreateTask}
            />
          </div>
        </div>
      </IconTip>
      <div className="flex shrink-0 items-start px-1.5 py-1.5">
        <ContextCandidateActions
          candidate={candidate}
          disabled={actionsDisabled}
          actionKey={actionKey}
          onAction={onAction}
        />
      </div>
    </div>
  )
}

function ContextGenericCandidateRow({
  candidate,
  sessionId,
  actionKey,
  actionsDisabled,
  onSummarize,
  onAskUser,
  onAddEvidence,
  onMarkConflict,
  onCreateTask,
}: {
  candidate: ContextCandidate
  sessionId?: string | null
  actionKey?: string | null
  actionsDisabled?: boolean
  onSummarize?: (candidate: ContextCandidate) => void
  onAskUser?: (candidate: ContextCandidate) => void
  onAddEvidence?: (candidate: ContextCandidate) => void
  onMarkConflict?: (candidate: ContextCandidate) => void
  onCreateTask?: (candidate: ContextCandidate) => void
}) {
  const { t } = useTranslation()
  const Icon = contextKindIcon(candidate.kind)
  const label = candidate.url ?? candidate.subtitle ?? candidate.path ?? candidate.title
  const clickable = Boolean(candidate.url)
  const content = (
    <>
      <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {candidate.title}
          </span>
          {candidate.status ? (
            <StatusPill label={candidate.status} tone={contextCandidateTone(candidate)} />
          ) : null}
        </div>
        <div className="mt-1 line-clamp-2 text-[11px] leading-snug text-muted-foreground">
          {candidate.reasons[0] ?? contextKindLabel(t, candidate.kind)}
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground/65">
          <span className="truncate">{contextLocationLabel(candidate) ?? candidate.subtitle}</span>
          <span className="shrink-0">{contextKindLabel(t, candidate.kind)}</span>
        </div>
        <DomainContextActionChips
          candidate={candidate}
          sessionId={sessionId}
          disabled={actionsDisabled}
          actionKey={actionKey}
          onSummarize={onSummarize}
          onAskUser={onAskUser}
          onAddEvidence={onAddEvidence}
          onMarkConflict={onMarkConflict}
          onCreateTask={onCreateTask}
        />
      </div>
    </>
  )
  return (
    <IconTip label={label}>
      {clickable ? (
        <div
          role="button"
          tabIndex={0}
          onClick={() => candidate.url && openExternalUrl(candidate.url)}
          onKeyDown={(event) => {
            if ((event.key === "Enter" || event.key === " ") && candidate.url) {
              event.preventDefault()
              openExternalUrl(candidate.url)
            }
          }}
          className="flex w-full min-w-0 items-start gap-2 rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5 text-left transition-colors hover:bg-secondary/45"
        >
          {content}
        </div>
      ) : (
        <div className="flex min-w-0 items-start gap-2 rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
          {content}
        </div>
      )}
    </IconTip>
  )
}

function ContextRetrievalSection({
  sessionId,
  incognito,
  turnActive,
  workingDir,
  onPreviewFile,
  onDomainEvidenceRecorded,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  onPreviewFile?: (target: PreviewTarget) => void
  onDomainEvidenceRecorded?: () => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const { snapshot, loading, error, refresh } = useContextRetrieval(sessionId, {
    incognito,
    turnActive,
    query,
    limit: 24,
  })
  const candidates = snapshot?.candidates ?? []
  const visible = candidates.slice(0, 8)
  const stats = snapshot?.stats
  const disabled = !sessionId || incognito
  const [contextActionKey, setContextActionKey] = useState<string | null>(null)

  const runFocusedContextAction = useCallback(
    async (candidate: ContextCandidate, action: ContextFocusedAction) => {
      if (!sessionId || disabled) return
      const focusPaths = contextCandidateFocusPaths(candidate)
      if (focusPaths.length === 0) return
      const actionKey = `${candidate.id}:${action}`
      setContextActionKey(actionKey)
      try {
        if (action === "review") {
          await getTransport().call<ReviewRunSnapshot>("run_code_review", {
            sessionId,
            scope: "local",
            focusPaths,
          })
          toast.success(t("workspace.context.focusReviewStarted", "已完成聚焦审查"))
        } else {
          await getTransport().call<VerificationRunSnapshot>("run_smart_verification", {
            sessionId,
            scope: "local",
            focusPaths,
          })
          toast.success(t("workspace.context.focusVerifyStarted", "已启动聚焦验证"))
        }
        refresh()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "ContextRetrievalSection", "Focused context action failed", e)
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, refresh, sessionId, t],
  )

  const recordContextCandidateEvidence = useCallback(
    async (candidate: ContextCandidate) => {
      if (!sessionId || disabled || !contextCanAddEvidence(candidate)) return
      const actionKey = `${candidate.id}:evidence`
      setContextActionKey(actionKey)
      try {
        const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
          input: contextCandidateEvidenceInput(candidate, sessionId),
        })
        toast.success(
          t("workspace.context.evidenceRecorded", "已加入证据：{{title}}", {
            title: item.title,
          }),
        )
        refresh()
        onDomainEvidenceRecorded?.()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error(
          "ui",
          "ContextRetrievalSection",
          "Record context candidate evidence failed",
          e,
        )
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, onDomainEvidenceRecorded, refresh, sessionId, t],
  )

  const summarizeContextCandidate = useCallback(
    async (candidate: ContextCandidate) => {
      if (!sessionId || disabled || !contextCanSummarize(candidate)) return
      const actionKey = `${candidate.id}:summary`
      setContextActionKey(actionKey)
      try {
        const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
          input: contextCandidateSummaryEvidenceInput(candidate, sessionId),
        })
        toast.success(
          t("workspace.context.summaryRecorded", "已生成摘要：{{title}}", {
            title: item.title,
          }),
        )
        refresh()
        onDomainEvidenceRecorded?.()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("ui", "ContextRetrievalSection", "Summarize context candidate failed", e)
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, onDomainEvidenceRecorded, refresh, sessionId, t],
  )

  const askContextCandidateConfirmation = useCallback(
    async (candidate: ContextCandidate) => {
      if (!sessionId || disabled || !contextCanAskUser(candidate)) return
      const actionKey = `${candidate.id}:ask`
      setContextActionKey(actionKey)
      try {
        await getTransport().call("create_owner_ask_user_question", {
          input: contextCandidateAskUserInput(candidate, sessionId),
        })
        toast.success(
          t("workspace.context.confirmationRequested", "已请求确认：{{title}}", {
            title: candidate.title,
          }),
        )
        refresh()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error(
          "ui",
          "ContextRetrievalSection",
          "Request context candidate confirmation failed",
          e,
        )
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, refresh, sessionId, t],
  )

  const markContextCandidateConflict = useCallback(
    async (candidate: ContextCandidate) => {
      if (!sessionId || disabled || !contextCanMarkConflict(candidate)) return
      const actionKey = `${candidate.id}:conflict`
      setContextActionKey(actionKey)
      try {
        const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
          input: contextCandidateConflictEvidenceInput(candidate, sessionId),
        })
        toast.warning(
          t("workspace.context.conflictRecorded", "已标记冲突：{{title}}", {
            title: item.title,
          }),
        )
        refresh()
        onDomainEvidenceRecorded?.()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error(
          "ui",
          "ContextRetrievalSection",
          "Mark context candidate conflict failed",
          e,
        )
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, onDomainEvidenceRecorded, refresh, sessionId, t],
  )

  const createContextCandidateTask = useCallback(
    async (candidate: ContextCandidate) => {
      if (!sessionId || disabled || !contextCanCreateTask(candidate)) return
      const actionKey = `${candidate.id}:task`
      setContextActionKey(actionKey)
      try {
        await getTransport().call<Task[]>("create_session_task", {
          sessionId,
          content: contextCandidateTaskContent(candidate),
          activeForm: contextCandidateTaskActiveForm(candidate),
        })
        toast.success(
          t("workspace.context.taskCreated", "已创建任务：{{title}}", {
            title: candidate.title,
          }),
        )
        refresh()
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error(
          "ui",
          "ContextRetrievalSection",
          "Create context candidate task failed",
          e,
        )
        toast.error(message)
      } finally {
        setContextActionKey(null)
      }
    },
    [disabled, refresh, sessionId, t],
  )

  const meta =
    loading && !snapshot ? (
      <StatusPill label={t("workspace.context.loading", "召回中")} tone="info" loading />
    ) : error ? (
      <StatusPill label={t("workspace.context.failed", "失败")} tone="danger" />
    ) : candidates.length > 0 ? (
      <StatusPill
        label={t("workspace.context.count", "{{count}} 条", { count: candidates.length })}
        tone="info"
      />
    ) : disabled ? (
      <StatusPill label={t("workspace.context.disabled", "未启用")} tone="muted" />
    ) : (
      <StatusPill label={t("workspace.context.emptyMeta", "待召回")} tone="muted" />
    )

  return (
    <WorkspaceSection
      title={t("workspace.context.title", "推荐上下文")}
      count={candidates.length}
      icon={Search}
      meta={meta}
      defaultExpanded={candidates.length > 0 || !!error}
    >
      <div className="space-y-2">
        <form
          className="flex items-center gap-1.5"
          onSubmit={(e) => {
            e.preventDefault()
            refresh()
          }}
        >
          <div className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              disabled={disabled}
              className="h-8 pl-7 text-xs"
              placeholder={t("workspace.context.searchPlaceholder", "资料、来源、文件、任务")}
            />
          </div>
          <IconTip label={t("workspace.context.refresh", "刷新推荐上下文")}>
            <button
              type="submit"
              disabled={disabled || loading}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
        </form>

        {stats ? (
          <div className="grid grid-cols-4 gap-1.5">
            {[
              [t("workspace.context.statDiff", "diff"), stats.gitChanges],
              [
                t("workspace.context.statFiles", "文件"),
                stats.artifactFiles + stats.fileSearchMatches,
              ],
              [
                t("workspace.context.statSignals", "信号"),
                stats.diagnostics + stats.reviewFindings + stats.ideContextSignals,
              ],
              [
                t("workspace.context.statControl", "闭环"),
                stats.verificationSteps +
                  stats.goalEvidence +
                  stats.tasks +
                  stats.workflowOps +
                  stats.symbols,
              ],
              [
                t("workspace.context.statDomain", "领域"),
                stats.domainCandidates + stats.domainEvidence,
              ],
              [t("workspace.context.statAccess", "缺口"), stats.accessIssues],
            ].map(([label, count]) => (
              <div
                key={label as string}
                className="rounded-md border border-border/50 bg-secondary/25 px-2 py-1.5"
              >
                <div className="truncate text-[10px] text-muted-foreground">{label as string}</div>
                <div className="text-xs font-medium tabular-nums text-foreground">
                  {count as number}
                </div>
              </div>
            ))}
          </div>
        ) : null}

        {snapshot?.domainContext ? (
          <div className="rounded-md border border-primary/20 bg-primary/5 px-2.5 py-1.5 text-[11px] text-muted-foreground">
            <div className="flex min-w-0 items-center gap-1.5">
              <Brain className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1 truncate">
                {snapshot.domainContext.templateTitle ??
                  t("workspace.context.domainFallback", "{{domain}} 领域", {
                    domain: snapshot.domainContext.domain,
                  })}
              </span>
              {snapshot.domainContext.templateId ? (
                <StatusPill
                  label={
                    snapshot.domainContext.templateVersion
                      ? `${snapshot.domainContext.templateId}@${snapshot.domainContext.templateVersion}`
                      : snapshot.domainContext.templateId
                  }
                  tone="muted"
                />
              ) : null}
              <StatusPill label={snapshot.domainContext.source} tone="info" />
            </div>
            {snapshot.accessIssues.length ? (
              <div className="mt-1 space-y-0.5">
                {snapshot.accessIssues.slice(0, 2).map((issue) => (
                  <div
                    key={`${issue.kind}:${issue.title}`}
                    className="truncate text-amber-700 dark:text-amber-300"
                  >
                    {issue.title} · {issue.action}
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}

        {incognito ? (
          <EmptyHint>{t("workspace.context.incognito", "无痕会话不读取历史上下文")}</EmptyHint>
        ) : error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : visible.length > 0 ? (
          <div className="space-y-1">
            {visible.map((candidate) =>
              candidate.path ? (
                <ContextFileCandidateRow
                  key={candidate.id}
                  candidate={candidate}
                  sessionId={sessionId}
                  onPreviewFile={onPreviewFile}
                  actionKey={contextActionKey}
                  actionsDisabled={disabled || Boolean(contextActionKey)}
                  onAction={runFocusedContextAction}
                  onSummarize={summarizeContextCandidate}
                  onAskUser={askContextCandidateConfirmation}
                  onAddEvidence={recordContextCandidateEvidence}
                  onMarkConflict={markContextCandidateConflict}
                  onCreateTask={createContextCandidateTask}
                />
              ) : (
                <ContextGenericCandidateRow
                  key={candidate.id}
                  candidate={candidate}
                  sessionId={sessionId}
                  actionKey={contextActionKey}
                  actionsDisabled={disabled || Boolean(contextActionKey)}
                  onSummarize={summarizeContextCandidate}
                  onAskUser={askContextCandidateConfirmation}
                  onAddEvidence={recordContextCandidateEvidence}
                  onMarkConflict={markContextCandidateConflict}
                  onCreateTask={createContextCandidateTask}
                />
              ),
            )}
            {candidates.length > visible.length || snapshot?.truncated ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.context.more", "还有 {{count}} 条", {
                  count: Math.max(candidates.length - visible.length, 0),
                })}
              </div>
            ) : null}
          </div>
        ) : (
          <EmptyHint>
            {workingDir
              ? t("workspace.context.empty", "暂无推荐上下文")
              : t(
                  "workspace.context.emptyNoWorkspace",
                  "暂无推荐上下文；未设置工作目录时会跳过文件搜索",
                )}
          </EmptyHint>
        )}

        {stats?.warnings.length ? (
          <div className="px-2 text-[10px] text-muted-foreground/60">
            {stats.warnings.slice(0, 2).join(" · ")}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

const DOMAIN_SOURCE_EVIDENCE_TYPES = new Set([
  "source_cited",
  "connector_context_collected",
  "data_quality_checked",
])
const DOMAIN_DRAFT_EVIDENCE_TYPES = new Set([
  "artifact_created",
  "connector_draft_created",
])
const DOMAIN_REVIEW_EVIDENCE_TYPES = new Set([
  "artifact_reviewed",
  "connector_action_verified",
])
const DOMAIN_DECISION_EVIDENCE_TYPES = new Set([
  "user_decision",
  "message_draft_approved",
])

function domainEvidenceTypeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  type: string,
): string {
  switch (type) {
    case "source_cited":
      return t("workspace.domainWorkbench.evidenceSourceCited", "引用来源")
    case "connector_context_collected":
      return t("workspace.domainWorkbench.evidenceConnectorContext", "连接器上下文")
    case "data_quality_checked":
      return t("workspace.domainWorkbench.evidenceDataQuality", "数据质量")
    case "artifact_created":
      return t("workspace.domainWorkbench.evidenceArtifactCreated", "产物草稿")
    case "artifact_reviewed":
      return t("workspace.domainWorkbench.evidenceArtifactReviewed", "产物复核")
    case "connector_draft_created":
      return t("workspace.domainWorkbench.evidenceConnectorDraft", "外部动作草稿")
    case "connector_action_verified":
      return t("workspace.domainWorkbench.evidenceConnectorVerified", "外部动作验证")
    case "message_draft_approved":
      return t("workspace.domainWorkbench.evidenceMessageApproved", "消息已批准")
    case "user_decision":
      return t("workspace.domainWorkbench.evidenceUserDecision", "用户决策")
    default:
      return type.replace(/_/g, " ")
  }
}

function domainWorkbenchEvidenceLocation(item: DomainEvidenceItem): string {
  const metadata = asRecord(item.sourceMetadata)
  return (
    stringField(metadata, "url") ??
    stringField(metadata, "file") ??
    stringField(metadata, "path") ??
    stringField(metadata, "connector") ??
    stringField(metadata, "tool") ??
    item.domain
  )
}

function domainWorkbenchEvidenceTone(item: DomainEvidenceItem): StatusTone {
  if (item.redactionStatus && item.redactionStatus !== "none" && item.redactionStatus !== "clean") {
    return "warn"
  }
  if (item.accessScope && item.accessScope !== "public") return "info"
  return "muted"
}

function domainWorkbenchMetricTone(value: number, blocked = false): StatusTone {
  if (blocked) return "danger"
  return value > 0 ? "good" : "muted"
}

function domainWorkbenchOverallTone(args: {
  incognito?: boolean
  evidenceCount: number
  sourceCount: number
  draftCount: number
  blockingReviewFindings: number
  failedVerification: number
  domainFailed: number
  domainNeedsUser: number
  exportStatus?: string | null
  connectorStatus?: string | null
  operationalStatus?: string | null
  soakStatus?: string | null
}): StatusTone {
  if (args.incognito) return "muted"
  if (
    args.blockingReviewFindings > 0 ||
    args.failedVerification > 0 ||
    args.domainFailed > 0 ||
    args.exportStatus === "failed" ||
    args.connectorStatus === "failed" ||
    args.operationalStatus === "failed" ||
    args.soakStatus === "failed"
  ) {
    return "danger"
  }
  if (
    args.domainNeedsUser > 0 ||
    args.evidenceCount === 0 ||
    args.sourceCount === 0 ||
    args.draftCount === 0 ||
    args.exportStatus === "insufficient_data" ||
    args.connectorStatus === "insufficient_data" ||
    args.operationalStatus === "insufficient_data" ||
    args.soakStatus === "insufficient_data"
  ) {
    return "warn"
  }
  return "good"
}

function domainWorkbenchOverallLabel(
  t: ReturnType<typeof useTranslation>["t"],
  tone: StatusTone,
  loading: boolean,
): string {
  if (loading) return t("workspace.domainWorkbench.loading", "同步中")
  if (tone === "danger") return t("workspace.domainWorkbench.blocked", "需处理")
  if (tone === "warn") return t("workspace.domainWorkbench.needsEvidence", "待补证据")
  if (tone === "good") return t("workspace.domainWorkbench.ready", "闭环健康")
  return t("workspace.domainWorkbench.idle", "待开始")
}

function domainWorkbenchNextSteps(
  t: ReturnType<typeof useTranslation>["t"],
  args: {
    incognito?: boolean
    evidenceCount: number
    sourceCount: number
    draftCount: number
    reviewCount: number
    failedVerification: number
    domainFailed: number
    domainNeedsUser: number
    exportGuard: DomainArtifactExportGuardReport | null
    connectorGuard: DomainConnectorActionGuardReport | null
    operationalGate: DomainOperationalGateReport | null
    soakReport: DomainSoakReport | null
  },
): string[] {
  if (args.incognito) {
    return [t("workspace.domainWorkbench.nextIncognito", "无痕会话不会持久化通用任务证据。")]
  }
  const steps: string[] = []
  if (args.evidenceCount === 0) {
    steps.push(t("workspace.domainWorkbench.nextEvidence", "先让模型记录来源、草稿或决策证据。"))
  }
  if (args.sourceCount === 0) {
    steps.push(t("workspace.domainWorkbench.nextSources", "补齐来源或连接器上下文，避免无依据产物。"))
  }
  if (args.draftCount === 0) {
    steps.push(t("workspace.domainWorkbench.nextDraft", "生成可审查的草稿、文件或外部动作草案。"))
  }
  if (args.reviewCount === 0 && args.draftCount > 0) {
    steps.push(t("workspace.domainWorkbench.nextReview", "对草稿做复核，再进入交付或外部执行。"))
  }
  if (args.domainFailed > 0 || args.domainNeedsUser > 0) {
    steps.push(t("workspace.domainWorkbench.nextDomainQuality", "处理领域复核里的阻塞项或用户确认项。"))
  }
  if (args.failedVerification > 0) {
    steps.push(t("workspace.domainWorkbench.nextVerification", "查看验证失败步骤并重新验证。"))
  }
  if (args.exportGuard?.status && args.exportGuard.status !== "passed") {
    steps.push(
      args.exportGuard.recommendedNextSteps[0] ??
        t("workspace.domainWorkbench.nextExportGuard", "补齐最终产物、复核和脱敏证据。"),
    )
  }
  if (args.connectorGuard?.status && args.connectorGuard.status !== "passed") {
    steps.push(
      args.connectorGuard.recommendedNextSteps[0] ??
        t("workspace.domainWorkbench.nextConnectorGuard", "补齐外部动作批准和回滚证据。"),
    )
  }
  if (args.operationalGate?.status && args.operationalGate.status !== "passed") {
    steps.push(
      args.operationalGate.recommendedNextSteps[0] ??
        t("workspace.domainWorkbench.nextOperationalGate", "等待工作流排空，或处理失败/阻塞的运行。"),
    )
  }
  if (args.soakReport?.status && args.soakReport.status !== "passed") {
    steps.push(
      args.soakReport.recommendedNextSteps[0] ??
        t("workspace.domainWorkbench.nextSoakReport", "查看长跑审计里的事故和未排空任务。"),
    )
  }
  return steps.length > 0
    ? steps.slice(0, 4)
    : [t("workspace.domainWorkbench.nextReady", "证据链健康；交付或外部动作前仍会要求用户最终确认。")]
}

function DomainWorkbenchMetric({
  icon: Icon,
  label,
  value,
  tone,
}: {
  icon: LucideIcon
  label: string
  value: number | string
  tone: StatusTone
}) {
  return (
    <div className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone])}>
      <div className="flex min-w-0 items-center gap-1.5 text-[10px]">
        <Icon className="h-3 w-3 shrink-0" />
        <span className="truncate">{label}</span>
      </div>
      <div className="mt-0.5 text-xs font-semibold tabular-nums">{value}</div>
    </div>
  )
}

function DomainWorkbenchEvidenceRow({ item }: { item: DomainEvidenceItem }) {
  const { t } = useTranslation()
  return (
    <IconTip label={domainWorkbenchEvidenceLocation(item)}>
      <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <Database className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {item.title}
          </span>
          <StatusPill
            label={domainEvidenceTypeLabel(t, item.evidenceType)}
            tone={domainWorkbenchEvidenceTone(item)}
          />
        </div>
        <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
          {item.summary || domainWorkbenchEvidenceLocation(item)}
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5 text-[10px] text-muted-foreground/65">
          <span className="truncate">{domainLabel(item.domain)}</span>
          <span className="truncate">{item.accessScope}</span>
          <span className="truncate">{item.redactionStatus}</span>
          <span className="shrink-0">{formatMessageTime(item.createdAt)}</span>
        </div>
      </div>
    </IconTip>
  )
}

function DomainTaskWorkbenchSection({
  sessionId,
  incognito,
  workingDir,
  reviewRunsState,
  verificationRunsState,
  domainQualityRunsState,
  domainWorkbenchState,
  focusRequest,
}: {
  sessionId?: string | null
  incognito?: boolean
  workingDir?: string | null
  reviewRunsState: ReviewRunsState
  verificationRunsState: VerificationRunsState
  domainQualityRunsState: DomainQualityRunsState
  domainWorkbenchState: DomainTaskWorkbenchState
  focusRequest?: { target: "operational" | "soak"; nonce: number } | null
}) {
  const { t } = useTranslation()
  const {
    evidence,
    evidenceLoading,
    evidenceError,
    exportGuard,
    exportGuardLoading,
    exportGuardError,
    connectorGuard,
    connectorGuardLoading,
    connectorGuardError,
    operationalGate,
    operationalGateLoading,
    operationalGateError,
    soakReport,
    soakReportLoading,
    soakReportError,
    refreshAll,
  } = domainWorkbenchState
  const evidenceCount = evidence.length
  const sourceCount = evidence.filter((item) =>
    DOMAIN_SOURCE_EVIDENCE_TYPES.has(item.evidenceType),
  ).length
  const draftCount = evidence.filter((item) =>
    DOMAIN_DRAFT_EVIDENCE_TYPES.has(item.evidenceType),
  ).length
  const reviewEvidenceCount = evidence.filter((item) =>
    DOMAIN_REVIEW_EVIDENCE_TYPES.has(item.evidenceType),
  ).length
  const decisionEvidenceCount =
    evidence.filter((item) => DOMAIN_DECISION_EVIDENCE_TYPES.has(item.evidenceType)).length +
    (connectorGuard?.summary.approvalEvidence ?? 0)
  const openReviewFindings =
    reviewRunsState.snapshot?.findings.filter((finding) => finding.status === "open") ?? []
  const blockingReviewFindings = openReviewFindings.filter(
    (finding) => finding.severity === "p0" || finding.severity === "p1",
  ).length
  const verificationSteps = verificationRunsState.snapshot?.steps ?? []
  const failedVerification = verificationStatsNumber(verificationRunsState.snapshot, "failed")
  const passedVerification = verificationStatsNumber(verificationRunsState.snapshot, "passed")
  const domainFailed = domainQualityStatsNumber(domainQualityRunsState.snapshot, "failed")
  const domainNeedsUser = domainQualityStatsNumber(domainQualityRunsState.snapshot, "needsUser")
  const domainPassed = domainQualityStatsNumber(domainQualityRunsState.snapshot, "passed")
  const loading =
    evidenceLoading ||
    exportGuardLoading ||
    connectorGuardLoading ||
    operationalGateLoading ||
    soakReportLoading ||
    reviewRunsState.loading ||
    verificationRunsState.loading ||
    domainQualityRunsState.loading
  const busy =
    loading ||
    reviewRunsState.running ||
    verificationRunsState.planning ||
    verificationRunsState.running ||
    domainQualityRunsState.running
  const disabled = !sessionId || incognito
  const tone = domainWorkbenchOverallTone({
    incognito,
    evidenceCount,
    sourceCount,
    draftCount,
    blockingReviewFindings,
    failedVerification,
    domainFailed,
    domainNeedsUser,
    exportStatus: exportGuard?.status,
    connectorStatus: connectorGuard?.status,
    operationalStatus: operationalGate?.status,
    soakStatus: soakReport?.status,
  })
  const nextSteps = domainWorkbenchNextSteps(t, {
    incognito,
    evidenceCount,
    sourceCount,
    draftCount,
    reviewCount: reviewEvidenceCount,
    failedVerification,
    domainFailed,
    domainNeedsUser,
    exportGuard,
    connectorGuard,
    operationalGate,
    soakReport,
  })
  const recentEvidence = evidence.slice(0, 4)
  const error =
    evidenceError ??
    exportGuardError ??
    connectorGuardError ??
    operationalGateError ??
    soakReportError
  const count =
    evidenceCount +
    openReviewFindings.length +
    verificationSteps.length +
    (domainQualityRunsState.snapshot?.checks.length ?? 0) +
    (operationalGate?.checks?.length ?? 0) +
    (soakReport?.incidents?.length ?? 0)

  const focusSignal = focusRequest?.nonce ?? 0

  const handleRefresh = () => {
    void refreshAll()
    reviewRunsState.refresh()
    verificationRunsState.refresh()
    domainQualityRunsState.refresh()
  }

  const handleDomainQuality = async () => {
    const next = await domainQualityRunsState.runDomainQuality()
    if (!next) return
    if (next.run.state === "completed") {
      toast.success(t("workspace.domainWorkbench.domainQualityClean", "领域复核通过"))
    } else if (next.run.state === "needs_user") {
      toast.warning(t("workspace.domainWorkbench.domainQualityNeedsUser", "领域复核需要确认"))
    } else {
      toast.error(t("workspace.domainWorkbench.domainQualityBlocked", "领域复核发现阻塞项"))
    }
    void refreshAll()
  }

  const handlePlanVerification = async () => {
    const next = await verificationRunsState.planVerification()
    if (next) {
      toast.success(
        t("workspace.domainWorkbench.planVerificationDone", "已推荐 {{count}} 条验证", {
          count: next.steps.length,
        }),
      )
    }
  }

  const handleRunVerification = async () => {
    const next = await verificationRunsState.runVerification()
    if (next) {
      toast.success(t("workspace.domainWorkbench.runVerificationDone", "验证已开始"))
    }
  }

  return (
    <WorkspaceSection
      title={t("workspace.domainWorkbench.title", "通用任务工作台")}
      count={count}
      icon={Layers}
      expandSignal={focusSignal}
      meta={
        <StatusPill
          label={domainWorkbenchOverallLabel(t, tone, loading)}
          tone={tone}
          loading={loading}
        />
      }
      defaultExpanded={tone !== "muted" || Boolean(error)}
    >
      <div className="space-y-2">
        <div className="grid grid-cols-3 gap-1.5">
          <DomainWorkbenchMetric
            icon={Globe}
            label={t("workspace.domainWorkbench.sources", "来源")}
            value={sourceCount}
            tone={domainWorkbenchMetricTone(sourceCount)}
          />
          <DomainWorkbenchMetric
            icon={Database}
            label={t("workspace.domainWorkbench.evidence", "证据")}
            value={evidenceCount}
            tone={domainWorkbenchMetricTone(evidenceCount)}
          />
          <DomainWorkbenchMetric
            icon={FileText}
            label={t("workspace.domainWorkbench.drafts", "草稿")}
            value={draftCount}
            tone={domainWorkbenchMetricTone(draftCount)}
          />
          <DomainWorkbenchMetric
            icon={ClipboardCheck}
            label={t("workspace.domainWorkbench.review", "复核")}
            value={reviewEvidenceCount + Math.max(openReviewFindings.length, 0)}
            tone={domainWorkbenchMetricTone(
              reviewEvidenceCount,
              blockingReviewFindings > 0 || openReviewFindings.length > 0,
            )}
          />
          <DomainWorkbenchMetric
            icon={CheckCircle2}
            label={t("workspace.domainWorkbench.verification", "验证")}
            value={`${passedVerification}/${verificationSteps.length}`}
            tone={domainWorkbenchMetricTone(passedVerification, failedVerification > 0)}
          />
          <DomainWorkbenchMetric
            icon={MessageSquare}
            label={t("workspace.domainWorkbench.decisions", "决策")}
            value={decisionEvidenceCount}
            tone={domainWorkbenchMetricTone(decisionEvidenceCount)}
          />
        </div>

        <div className="grid grid-cols-2 gap-1.5">
          <button
            type="button"
            onClick={handleDomainQuality}
            disabled={disabled || domainQualityRunsState.running || domainQualityRunsState.loading}
            className="inline-flex min-w-0 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {domainQualityRunsState.running ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ClipboardCheck className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.domainWorkbench.runQuality", "运行领域复核")}</span>
          </button>
          <button
            type="button"
            onClick={handlePlanVerification}
            disabled={
              disabled ||
              !workingDir ||
              verificationRunsState.planning ||
              verificationRunsState.running ||
              verificationRunsState.loading
            }
            className="inline-flex min-w-0 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {verificationRunsState.planning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Gauge className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.domainWorkbench.planVerify", "推荐验证")}</span>
          </button>
          <button
            type="button"
            onClick={handleRunVerification}
            disabled={
              disabled ||
              !workingDir ||
              verificationRunsState.planning ||
              verificationRunsState.running ||
              verificationRunsState.loading
            }
            className="inline-flex min-w-0 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {verificationRunsState.running ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.domainWorkbench.runVerify", "运行验证")}</span>
          </button>
          <button
            type="button"
            onClick={handleRefresh}
            disabled={busy}
            className="inline-flex min-w-0 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/25 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-55"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.domainWorkbench.refresh", "刷新工作台")}</span>
          </button>
        </div>

        {error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : incognito ? (
          <EmptyHint>{t("workspace.domainWorkbench.incognito", "无痕会话不持久化通用任务证据")}</EmptyHint>
        ) : null}

        <div className="rounded-md border border-border/50 bg-secondary/20 px-2.5 py-2">
          <div className="mb-1.5 flex min-w-0 items-center gap-1.5 text-xs font-medium text-foreground/90">
            <Lightbulb className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("workspace.domainWorkbench.nextSteps", "下一步")}</span>
          </div>
          <div className="space-y-1">
            {nextSteps.map((step, index) => (
              <div
                key={`${index}:${step}`}
                className="line-clamp-2 text-[11px] leading-snug text-muted-foreground"
              >
                {step}
              </div>
            ))}
          </div>
        </div>

        <div className="grid grid-cols-1 gap-2">
          <DomainOperationalGatePanel
            report={operationalGate}
            loading={operationalGateLoading}
            error={operationalGateError}
            disabled={disabled || operationalGateLoading}
            onRefresh={domainWorkbenchState.refreshOperationalGate}
          />
          <DomainSoakReportPanel
            report={soakReport}
            loading={soakReportLoading}
            error={soakReportError}
            disabled={disabled || soakReportLoading}
            onRefresh={domainWorkbenchState.refreshSoakReport}
          />
          <DomainArtifactExportGuardPanel
            report={exportGuard}
            loading={exportGuardLoading}
            error={exportGuardError}
            disabled={disabled || exportGuardLoading}
            onRefresh={domainWorkbenchState.refreshExportGuard}
          />
          <DomainConnectorActionGuardPanel
            report={connectorGuard}
            loading={connectorGuardLoading}
            error={connectorGuardError}
            disabled={disabled || connectorGuardLoading}
            onRefresh={domainWorkbenchState.refreshConnectorGuard}
          />
        </div>

        {domainQualityRunsState.snapshot ? (
          <div className="flex min-w-0 flex-wrap gap-1">
            <StatusPill
              label={t("workspace.domainWorkbench.domainPassed", "{{count}} 通过", {
                count: domainPassed,
              })}
              tone="good"
            />
            {domainFailed > 0 ? (
              <StatusPill
                label={t("workspace.domainWorkbench.domainFailed", "{{count}} 阻塞", {
                  count: domainFailed,
                })}
                tone="danger"
              />
            ) : null}
            {domainNeedsUser > 0 ? (
              <StatusPill
                label={t("workspace.domainWorkbench.domainNeedsUser", "{{count}} 确认", {
                  count: domainNeedsUser,
                })}
                tone="warn"
              />
            ) : null}
          </div>
        ) : null}

        {recentEvidence.length > 0 ? (
          <div className="space-y-1">
            {recentEvidence.map((item) => (
              <DomainWorkbenchEvidenceRow key={item.id} item={item} />
            ))}
            {evidence.length > recentEvidence.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.domainWorkbench.moreEvidence", "还有 {{count}} 条证据", {
                  count: evidence.length - recentEvidence.length,
                })}
              </div>
            ) : null}
          </div>
        ) : !incognito && !loading && !error ? (
          <EmptyHint>{t("workspace.domainWorkbench.empty", "还没有通用任务证据")}</EmptyHint>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function lspSeverityTone(severity: LspDiagnostic["severity"]): StatusTone {
  switch (severity) {
    case "error":
      return "danger"
    case "warning":
      return "warn"
    case "information":
      return "info"
    case "hint":
    case "unknown":
      return "muted"
  }
}

function lspSeverityRank(severity: LspDiagnostic["severity"]): number {
  switch (severity) {
    case "error":
      return 0
    case "warning":
      return 1
    case "information":
      return 2
    case "hint":
      return 3
    case "unknown":
      return 4
  }
}

function LspDiagnosticsSection({
  sessionId,
  incognito,
  turnActive,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
}) {
  const { t } = useTranslation()
  const { status, snapshot, loading, error, refresh } = useLspDiagnostics(sessionId, {
    incognito,
    turnActive,
  })
  const diagnostics = useMemo(
    () =>
      [...(snapshot?.diagnostics ?? [])].sort((a, b) => {
        const severity = lspSeverityRank(a.severity) - lspSeverityRank(b.severity)
        if (severity !== 0) return severity
        const file = (a.path ?? a.uri).localeCompare(b.path ?? b.uri)
        if (file !== 0) return file
        return a.range.startLine - b.range.startLine
      }),
    [snapshot?.diagnostics],
  )
  const visibleDiagnostics = diagnostics.slice(0, 6)
  const lspServers = status?.servers ?? []
  const activeServers = lspServers.filter((server) => server.active).length
  const availableServers = lspServers.filter((server) => server.available).length
  const count = diagnostics.length
  const meta =
    loading && !snapshot ? (
      <StatusPill label={t("workspace.lsp.loading", "读取中")} tone="info" loading />
    ) : snapshot && snapshot.errors > 0 ? (
      <StatusPill
        label={t("workspace.lsp.errors", "{{count}} 错误", { count: snapshot.errors })}
        tone="danger"
      />
    ) : snapshot && snapshot.warnings > 0 ? (
      <StatusPill
        label={t("workspace.lsp.warnings", "{{count}} 警告", { count: snapshot.warnings })}
        tone="warn"
      />
    ) : activeServers > 0 ? (
      <StatusPill label={t("workspace.lsp.ready", "已连接")} tone="good" />
    ) : (
      <StatusPill label={t("workspace.lsp.idle", "待启动")} tone="muted" />
    )

  return (
    <WorkspaceSection
      title={t("workspace.lsp.title", "语义诊断")}
      count={count}
      icon={CircleAlert}
      meta={meta}
      defaultExpanded={count > 0 || !!error}
    >
      <div className="space-y-2">
        <div className="grid grid-cols-3 gap-1.5">
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2 py-1.5">
            <div className="text-[10px] text-muted-foreground">
              {t("workspace.lsp.servers", "服务")}
            </div>
            <div className="text-xs font-medium tabular-nums text-foreground">
              {activeServers}/{availableServers}
            </div>
          </div>
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2 py-1.5">
            <div className="text-[10px] text-muted-foreground">
              {t("workspace.lsp.files", "文件")}
            </div>
            <div className="text-xs font-medium tabular-nums text-foreground">
              {snapshot?.files ?? 0}
            </div>
          </div>
          <button
            type="button"
            onClick={refresh}
            className="inline-flex items-center justify-center gap-1 rounded-md border border-border/50 bg-secondary/25 px-2 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            {t("workspace.lsp.refresh", "刷新")}
          </button>
        </div>

        {status?.workspaceRoot ? (
          <IconTip label={status.workspaceRoot}>
            <div className="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] text-muted-foreground">
              <FolderGit2 className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{basename(status.workspaceRoot)}</span>
            </div>
          </IconTip>
        ) : null}

        {error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : null}

        {visibleDiagnostics.length > 0 ? (
          <div className="space-y-1">
            {visibleDiagnostics.map((diagnostic, index) => {
              const path = diagnostic.path ?? diagnostic.uri
              const source = diagnostic.source ?? "lsp"
              return (
                <IconTip
                  key={`${diagnostic.uri}:${diagnostic.range.startLine}:${diagnostic.range.startColumn}:${index}`}
                  label={path}
                >
                  <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
                    <div className="flex min-w-0 items-center gap-2">
                      <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                      <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                        {basename(path)}
                      </span>
                      <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                        {diagnostic.range.startLine}:{diagnostic.range.startColumn}
                      </span>
                      <StatusPill
                        label={diagnostic.severity}
                        tone={lspSeverityTone(diagnostic.severity)}
                      />
                    </div>
                    <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
                      {diagnostic.message}
                    </div>
                    <div className="mt-1 pl-5 text-[10px] text-muted-foreground/65">{source}</div>
                  </div>
                </IconTip>
              )
            })}
            {diagnostics.length > visibleDiagnostics.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.lsp.more", "还有 {{count}} 条", {
                  count: diagnostics.length - visibleDiagnostics.length,
                })}
              </div>
            ) : null}
          </div>
        ) : error ? null : (
          <EmptyHint>
            {activeServers > 0
              ? t("workspace.lsp.emptyClean", "暂无诊断")
              : t("workspace.lsp.emptyIdle", "编辑或查询文件后启动")}
          </EmptyHint>
        )}

        {status && activeServers === 0 && availableServers > 0 ? (
          <div className="flex flex-wrap gap-1 px-1">
            {status.servers
              .filter((server) => server.available)
              .slice(0, 5)
              .map((server) => (
                <span
                  key={server.id}
                  className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-secondary/30 px-2 py-0.5 text-[10px] text-muted-foreground"
                >
                  <Server className="h-2.5 w-2.5" />
                  {server.id}
                </span>
              ))}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function reviewSeverityTone(severity: ReviewSeverity): StatusTone {
  switch (severity) {
    case "p0":
    case "p1":
      return "danger"
    case "p2":
      return "warn"
    case "p3":
      return "muted"
  }
}

function reviewVerdictTone(verdict: ReviewVerdict): StatusTone {
  switch (verdict) {
    case "confirmed":
      return "danger"
    case "plausible":
      return "warn"
    case "refuted":
      return "muted"
  }
}

function reviewStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status: ReviewFindingStatus,
): string {
  switch (status) {
    case "open":
      return t("workspace.review.statusOpen", "待处理")
    case "resolved":
      return t("workspace.review.statusResolved", "已修复")
    case "dismissed":
      return t("workspace.review.statusDismissed", "已忽略")
    case "false_positive":
      return t("workspace.review.statusFalsePositive", "误报")
  }
}

function reviewLineLabel(finding: ReviewFinding): string {
  if (!finding.startLine) return basename(finding.file)
  if (finding.endLine && finding.endLine > finding.startLine) {
    return `${basename(finding.file)}:${finding.startLine}-${finding.endLine}`
  }
  return `${basename(finding.file)}:${finding.startLine}`
}

function reviewStatsNumber(snapshot: ReviewRunSnapshot | null, key: string): number {
  const value = snapshot?.run.stats?.[key]
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

const REVIEW_PROFILE_OPTIONS = [
  { id: "correctness", label: "Correct" },
  { id: "security", label: "Security" },
  { id: "maintainability", label: "Maintain" },
  { id: "tests", label: "Tests" },
  { id: "concurrency", label: "Concurrency" },
  { id: "frontend", label: "Frontend" },
  { id: "accessibility", label: "A11y" },
  { id: "deep", label: "Deep" },
] as const

const DEFAULT_REVIEW_PROFILES = ["correctness", "security", "maintainability", "tests"]
const DOMAIN_WORKBENCH_EVIDENCE_LIMIT = 60

function eventBelongsToSession(payload: unknown, sessionId: string): boolean {
  if (typeof payload !== "object" || payload === null) return true
  const value = (payload as { sessionId?: unknown }).sessionId
  return typeof value !== "string" || value === sessionId
}

interface DomainTaskWorkbenchState {
  evidence: DomainEvidenceItem[]
  evidenceLoading: boolean
  evidenceError: string | null
  exportGuard: DomainArtifactExportGuardReport | null
  exportGuardLoading: boolean
  exportGuardError: string | null
  connectorGuard: DomainConnectorActionGuardReport | null
  connectorGuardLoading: boolean
  connectorGuardError: string | null
  operationalGate: DomainOperationalGateReport | null
  operationalGateLoading: boolean
  operationalGateError: string | null
  soakReport: DomainSoakReport | null
  soakReportLoading: boolean
  soakReportError: string | null
  refreshEvidence: () => Promise<DomainEvidenceItem[]>
  refreshExportGuard: () => Promise<DomainArtifactExportGuardReport | null>
  refreshConnectorGuard: () => Promise<DomainConnectorActionGuardReport | null>
  refreshOperationalGate: () => Promise<DomainOperationalGateReport | null>
  refreshSoakReport: () => Promise<DomainSoakReport | null>
  refreshAll: () => Promise<void>
}

function useDomainTaskWorkbench(
  sessionId: string | null | undefined,
  opts: { incognito?: boolean; turnActive?: boolean; disabled?: boolean } = {},
): DomainTaskWorkbenchState {
  const { incognito = false, turnActive = false, disabled = false } = opts
  const [evidence, setEvidence] = useState<DomainEvidenceItem[]>([])
  const [evidenceLoading, setEvidenceLoading] = useState(false)
  const [evidenceError, setEvidenceError] = useState<string | null>(null)
  const [exportGuard, setExportGuard] = useState<DomainArtifactExportGuardReport | null>(null)
  const [exportGuardLoading, setExportGuardLoading] = useState(false)
  const [exportGuardError, setExportGuardError] = useState<string | null>(null)
  const [connectorGuard, setConnectorGuard] =
    useState<DomainConnectorActionGuardReport | null>(null)
  const [connectorGuardLoading, setConnectorGuardLoading] = useState(false)
  const [connectorGuardError, setConnectorGuardError] = useState<string | null>(null)
  const [operationalGate, setOperationalGate] = useState<DomainOperationalGateReport | null>(null)
  const [operationalGateLoading, setOperationalGateLoading] = useState(false)
  const [operationalGateError, setOperationalGateError] = useState<string | null>(null)
  const [soakReport, setSoakReport] = useState<DomainSoakReport | null>(null)
  const [soakReportLoading, setSoakReportLoading] = useState(false)
  const [soakReportError, setSoakReportError] = useState<string | null>(null)

  const refreshEvidence = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setEvidence([])
      setEvidenceError(null)
      setEvidenceLoading(false)
      return []
    }
    setEvidenceLoading(true)
    setEvidenceError(null)
    try {
      const items = await getTransport().call<DomainEvidenceItem[]>("list_domain_evidence", {
        input: {
          sessionId,
          limit: DOMAIN_WORKBENCH_EVIDENCE_LIMIT,
        },
      })
      const safeItems = Array.isArray(items) ? items : []
      setEvidence(safeItems)
      return safeItems
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "useDomainTaskWorkbench",
        "Failed to load domain task evidence",
        e,
      )
      setEvidenceError(message)
      return []
    } finally {
      setEvidenceLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshExportGuard = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setExportGuard(null)
      setExportGuardError(null)
      setExportGuardLoading(false)
      return null
    }
    setExportGuardLoading(true)
    setExportGuardError(null)
    try {
      const report = await getTransport().call<DomainArtifactExportGuardReport>(
        "evaluate_domain_artifact_export_guard",
        {
          input: {
            sessionId,
            requireArtifactCreated: true,
            requireArtifactReviewed: true,
            maxSensitiveUnreviewed: 0,
            maxRedactionPending: 0,
          },
        },
      )
      setExportGuard(report)
      return report
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "useDomainTaskWorkbench",
        "Failed to evaluate artifact export guard",
        e,
      )
      setExportGuardError(message)
      return null
    } finally {
      setExportGuardLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshConnectorGuard = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setConnectorGuard(null)
      setConnectorGuardError(null)
      setConnectorGuardLoading(false)
      return null
    }
    setConnectorGuardLoading(true)
    setConnectorGuardError(null)
    try {
      const report = await getTransport().call<DomainConnectorActionGuardReport>(
        "evaluate_domain_connector_action_guard",
        {
          input: {
            sessionId,
            requireExplicitApproval: true,
            requireRollbackPlan: true,
            requireExportGuardForDelivery: true,
          },
        },
      )
      setConnectorGuard(report)
      return report
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "useDomainTaskWorkbench",
        "Failed to evaluate connector action guard",
        e,
      )
      setConnectorGuardError(message)
      return null
    } finally {
      setConnectorGuardLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshOperationalGate = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setOperationalGate(null)
      setOperationalGateError(null)
      setOperationalGateLoading(false)
      return null
    }
    setOperationalGateLoading(true)
    setOperationalGateError(null)
    try {
      const report = await getTransport().call<DomainOperationalGateReport>(
        "evaluate_domain_operational_gate",
        {
          input: {
            sessionId,
            windowDays: 14,
            minWorkflowRuns: 1,
            maxFailedWorkflowRuns: 0,
            maxBlockedWorkflowRuns: 0,
            maxCancelledWorkflowRuns: 0,
            maxActiveWorkflowRuns: 0,
            minLoopRuns: 0,
            maxFailedLoopRuns: 0,
            maxActiveCampaigns: 0,
            maxFailedCampaignItems: 0,
          },
        },
      )
      setOperationalGate(report)
      return report
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "useDomainTaskWorkbench",
        "Failed to evaluate domain operational gate",
        e,
      )
      setOperationalGateError(message)
      return null
    } finally {
      setOperationalGateLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshSoakReport = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setSoakReport(null)
      setSoakReportError(null)
      setSoakReportLoading(false)
      return null
    }
    setSoakReportLoading(true)
    setSoakReportError(null)
    try {
      const report = await getTransport().call<DomainSoakReport>(
        "generate_domain_soak_report",
        {
          input: {
            sessionId,
            windowDays: 14,
            maxItems: 8,
          },
        },
      )
      setSoakReport(report)
      return report
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useDomainTaskWorkbench", "Failed to generate domain soak report", e)
      setSoakReportError(message)
      return null
    } finally {
      setSoakReportLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshAll = useCallback(async () => {
    await Promise.all([
      refreshEvidence(),
      refreshExportGuard(),
      refreshConnectorGuard(),
      refreshOperationalGate(),
      refreshSoakReport(),
    ])
  }, [
    refreshConnectorGuard,
    refreshEvidence,
    refreshExportGuard,
    refreshOperationalGate,
    refreshSoakReport,
  ])

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) void refreshAll()
    })
    return () => {
      cancelled = true
    }
  }, [refreshAll])

  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    let cancelled = false
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive) {
      queueMicrotask(() => {
        if (!cancelled) void refreshAll()
      })
    }
    return () => {
      cancelled = true
    }
  }, [refreshAll, turnActive])

  useEffect(() => {
    if (disabled || !sessionId || incognito) return
    const maybeRefresh = (payload: unknown) => {
      if (!eventBelongsToSession(payload, sessionId)) return
      void refreshAll()
    }
    const unsubs = [
      getTransport().listen("domain_evidence:recorded", maybeRefresh),
      getTransport().listen("workflow:created", maybeRefresh),
      getTransport().listen("workflow:updated", maybeRefresh),
      getTransport().listen("loop:changed", maybeRefresh),
    ]
    return () => {
      unsubs.forEach((unsub) => unsub())
    }
  }, [disabled, incognito, refreshAll, sessionId])

  return {
    evidence,
    evidenceLoading,
    evidenceError,
    exportGuard,
    exportGuardLoading,
    exportGuardError,
    connectorGuard,
    connectorGuardLoading,
    connectorGuardError,
    operationalGate,
    operationalGateLoading,
    operationalGateError,
    soakReport,
    soakReportLoading,
    soakReportError,
    refreshEvidence,
    refreshExportGuard,
    refreshConnectorGuard,
    refreshOperationalGate,
    refreshSoakReport,
    refreshAll,
  }
}

function reviewStatsStringArray(snapshot: ReviewRunSnapshot | null, key: string): string[] {
  const value = snapshot?.run.stats?.[key]
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === "string" && item.length > 0)
}

function reviewStatsBoolean(snapshot: ReviewRunSnapshot | null, path: [string, string]): boolean {
  const root = snapshot?.run.stats?.[path[0]]
  if (!root || typeof root !== "object") return false
  const value = (root as Record<string, unknown>)[path[1]]
  return value === true
}

function ReviewSection({
  sessionId,
  incognito,
  turnActive,
  workingDir,
  reviewRunsState,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  reviewRunsState?: ReviewRunsState
}) {
  const { t } = useTranslation()
  const ownedReviewRunsState = useReviewRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(reviewRunsState),
  })
  const { runs, snapshot, loading, running, error, refresh, runReview, updateFindingStatus } =
    reviewRunsState ?? ownedReviewRunsState
  const findings = snapshot?.findings ?? []
  const openFindings = findings.filter((finding) => finding.status === "open")
  const visibleFindings = openFindings.slice(0, 6)
  const closedCount = findings.length - openFindings.length
  const latest = snapshot?.run ?? runs[0]
  const p0 = reviewStatsNumber(snapshot, "p0")
  const p1 = reviewStatsNumber(snapshot, "p1")
  const p2 = reviewStatsNumber(snapshot, "p2")
  const p3 = reviewStatsNumber(snapshot, "p3")
  const activeProfiles = reviewStatsStringArray(snapshot, "profiles")
  const warnings = reviewStatsStringArray(snapshot, "warnings")
  const ideContextPresent = reviewStatsBoolean(snapshot, ["ideContext", "present"])
  const llmReviewer =
    typeof latest?.stats?.llmReviewer === "string" ? latest.stats.llmReviewer : null
  const [selectedProfiles, setSelectedProfiles] = useState<string[]>(DEFAULT_REVIEW_PROFILES)
  const disabled =
    !sessionId || incognito || !workingDir || running || loading || latest?.state === "running"

  const blockingCount = openFindings.filter(
    (finding) => finding.severity === "p0" || finding.severity === "p1",
  ).length
  const meta =
    running || latest?.state === "running" ? (
      <StatusPill label={t("workspace.review.running", "审查中")} tone="info" loading />
    ) : latest?.state === "failed" ? (
      <StatusPill label={t("workspace.review.failed", "失败")} tone="danger" />
    ) : blockingCount > 0 ? (
      <StatusPill
        label={t("workspace.review.blocking", "{{count}} 阻塞", { count: blockingCount })}
        tone="danger"
      />
    ) : openFindings.length > 0 ? (
      <StatusPill
        label={t("workspace.review.findings", "{{count}} 问题", { count: openFindings.length })}
        tone="warn"
      />
    ) : latest?.state === "completed" ? (
      <StatusPill label={t("workspace.review.clean", "已通过")} tone="good" />
    ) : (
      <StatusPill label={t("workspace.review.idle", "待审查")} tone="muted" />
    )

  const handleRunReview = async () => {
    const next = await runReview({ profiles: selectedProfiles })
    if (next) {
      toast.success(
        next.findings.length > 0
          ? t("workspace.review.runDoneFindings", "审查完成，发现 {{count}} 条", {
              count: next.findings.length,
            })
          : t("workspace.review.runDoneClean", "审查完成，未发现问题"),
      )
    }
  }

  const toggleProfile = (profile: string) => {
    setSelectedProfiles((current) => {
      if (current.includes(profile)) {
        const next = current.filter((item) => item !== profile)
        return next.length > 0 ? next : DEFAULT_REVIEW_PROFILES
      }
      return [...current, profile]
    })
  }

  const handleFindingStatus = async (findingId: string, status: ReviewFindingStatus) => {
    const updated = await updateFindingStatus(findingId, status)
    if (updated) {
      toast.success(
        t("workspace.review.statusUpdated", "已标记为 {{status}}", {
          status: reviewStatusLabel(t, updated.status),
        }),
      )
    }
  }

  return (
    <WorkspaceSection
      title={t("workspace.review.title", "代码审查")}
      count={openFindings.length}
      icon={GitPullRequest}
      meta={meta}
      defaultExpanded={openFindings.length > 0 || !!error}
    >
      <div className="space-y-2">
        <div className="grid grid-cols-4 gap-1.5">
          {[
            ["P0", p0, "danger" as StatusTone],
            ["P1", p1, "danger" as StatusTone],
            ["P2", p2, "warn" as StatusTone],
            ["P3", p3, "muted" as StatusTone],
          ].map(([label, count, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{count as number}</div>
            </div>
          ))}
        </div>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={handleRunReview}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {running ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Search className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.review.run", "审查未提交改动")}</span>
          </button>
          <IconTip label={t("workspace.review.refresh", "刷新审查结果")}>
            <button
              type="button"
              onClick={refresh}
              disabled={loading || running}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
        </div>

        <div className="grid grid-cols-4 gap-1">
          {REVIEW_PROFILE_OPTIONS.map((profile) => {
            const active = selectedProfiles.includes(profile.id)
            return (
              <button
                key={profile.id}
                type="button"
                onClick={() => toggleProfile(profile.id)}
                disabled={running || loading}
                className={cn(
                  "h-7 min-w-0 rounded-md border px-1.5 text-[10px] font-medium transition-colors disabled:opacity-55",
                  active
                    ? "border-primary/40 bg-primary/10 text-primary"
                    : "border-border/50 bg-secondary/20 text-muted-foreground hover:bg-secondary/45 hover:text-foreground",
                )}
              >
                <span className="block truncate">{profile.label}</span>
              </button>
            )
          })}
        </div>

        {!workingDir ? (
          <EmptyHint>{t("workspace.review.noWorkspace", "选择工作目录后可审查本地改动")}</EmptyHint>
        ) : incognito ? (
          <EmptyHint>{t("workspace.review.incognito", "无痕会话不持久化审查结果")}</EmptyHint>
        ) : error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : latest ? (
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-2">
            <div className="flex min-w-0 items-center gap-2">
              <GitCommitHorizontal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                {latest.summary || t("workspace.review.summaryPending", "审查结果待生成")}
              </span>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {latest.id.slice(0, 10)}
              </span>
            </div>
            {closedCount > 0 ? (
              <div className="mt-1 pl-5 text-[10px] text-muted-foreground/65">
                {t("workspace.review.closedCount", "{{count}} 条已处理", { count: closedCount })}
              </div>
            ) : null}
            {activeProfiles.length > 0 || ideContextPresent || llmReviewer ? (
              <div className="mt-1 flex min-w-0 flex-wrap gap-1 pl-5">
                {activeProfiles.slice(0, 4).map((profile) => (
                  <StatusPill key={profile} label={profile} tone="muted" />
                ))}
                {ideContextPresent ? (
                  <StatusPill label={t("workspace.review.ideContext", "IDE context")} tone="info" />
                ) : null}
                {llmReviewer && llmReviewer !== "not_requested" ? (
                  <StatusPill
                    label={llmReviewer === "completed" ? "Deep reviewer" : "Deep skipped"}
                    tone={llmReviewer === "completed" ? "good" : "warn"}
                  />
                ) : null}
              </div>
            ) : null}
          </div>
        ) : (
          <EmptyHint>{t("workspace.review.empty", "还没有审查记录")}</EmptyHint>
        )}

        {visibleFindings.length > 0 ? (
          <div className="space-y-1">
            {visibleFindings.map((finding) => (
              <IconTip key={finding.id} label={finding.file}>
                <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                      {finding.title}
                    </span>
                    <StatusPill
                      label={finding.severity.toUpperCase()}
                      tone={reviewSeverityTone(finding.severity)}
                    />
                    <StatusPill label={finding.verdict} tone={reviewVerdictTone(finding.verdict)} />
                  </div>
                  <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
                    {finding.body}
                  </div>
                  <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5">
                    <span className="min-w-0 flex-1 truncate text-[10px] text-muted-foreground/70">
                      {reviewLineLabel(finding)} · {finding.category}
                    </span>
                    <IconTip label={t("workspace.review.markResolved", "标记已修复")}>
                      <button
                        type="button"
                        className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-emerald-600"
                        onClick={() => handleFindingStatus(finding.id, "resolved")}
                      >
                        <Check className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                    <IconTip label={t("workspace.review.markDismissed", "忽略")}>
                      <button
                        type="button"
                        className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground"
                        onClick={() => handleFindingStatus(finding.id, "dismissed")}
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                    <IconTip label={t("workspace.review.markFalsePositive", "标记误报")}>
                      <button
                        type="button"
                        className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-amber-600"
                        onClick={() => handleFindingStatus(finding.id, "false_positive")}
                      >
                        <ShieldAlert className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                  </div>
                </div>
              </IconTip>
            ))}
            {openFindings.length > visibleFindings.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.review.more", "还有 {{count}} 条", {
                  count: openFindings.length - visibleFindings.length,
                })}
              </div>
            ) : null}
          </div>
        ) : null}
        {warnings.length > 0 ? (
          <div className="px-2 text-[10px] text-muted-foreground/60">
            {warnings.slice(0, 2).join(" · ")}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function verificationStatsNumber(snapshot: VerificationRunSnapshot | null, key: string): number {
  const value = snapshot?.run.stats?.[key]
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

function verificationStepTone(state: VerificationStepState): StatusTone {
  switch (state) {
    case "passed":
      return "good"
    case "failed":
    case "timed_out":
      return "danger"
    case "running":
      return "info"
    case "skipped":
      return "warn"
    case "pending":
      return "muted"
  }
}

function verificationRiskTone(risk: VerificationRisk): StatusTone {
  switch (risk) {
    case "high":
      return "danger"
    case "medium":
      return "warn"
    case "low":
      return "good"
  }
}

function verificationStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: VerificationStepState,
): string {
  switch (state) {
    case "pending":
      return t("workspace.verification.stepPending", "待运行")
    case "running":
      return t("workspace.verification.stepRunning", "运行中")
    case "passed":
      return t("workspace.verification.stepPassed", "通过")
    case "failed":
      return t("workspace.verification.stepFailed", "失败")
    case "skipped":
      return t("workspace.verification.stepSkipped", "已跳过")
    case "timed_out":
      return t("workspace.verification.stepTimedOut", "超时")
  }
}

function verificationRiskLabel(
  t: ReturnType<typeof useTranslation>["t"],
  risk: VerificationRisk,
): string {
  switch (risk) {
    case "low":
      return t("workspace.verification.riskLow", "低风险")
    case "medium":
      return t("workspace.verification.riskMedium", "中风险")
    case "high":
      return t("workspace.verification.riskHigh", "需确认")
  }
}

function verificationDurationLabel(ms?: number | null): string | null {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return null
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)}s`
}

function VerificationStepRow({ step }: { step: VerificationStep }) {
  const { t } = useTranslation()
  const duration = verificationDurationLabel(step.durationMs)
  const output =
    step.outputPreview && (step.state === "failed" || step.state === "timed_out")
      ? truncateMiddle(step.outputPreview.replace(/\s+/g, " "), 220)
      : null
  return (
    <IconTip label={step.cwd}>
      <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5">
          {step.state === "running" ? (
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-blue-500" />
          ) : step.state === "passed" ? (
            <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
          ) : step.state === "failed" || step.state === "timed_out" ? (
            <CircleAlert className="h-3.5 w-3.5 shrink-0 text-destructive" />
          ) : (
            <Gauge className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          )}
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {step.title}
          </span>
          <StatusPill
            label={verificationStateLabel(t, step.state)}
            tone={verificationStepTone(step.state)}
            loading={step.state === "running"}
          />
          <StatusPill
            label={verificationRiskLabel(t, step.risk)}
            tone={verificationRiskTone(step.risk)}
          />
        </div>
        <div className="mt-1 min-w-0 truncate pl-5 font-mono text-[10px] text-muted-foreground/80">
          {step.command}
        </div>
        <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
          {step.reason}
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5 text-[10px] text-muted-foreground/65">
          <span className="truncate">{step.category}</span>
          {step.autoRun ? (
            <span>{t("workspace.verification.autoRun", "自动运行")}</span>
          ) : (
            <span>{t("workspace.verification.gated", "需手动确认")}</span>
          )}
          {duration ? <span>{duration}</span> : null}
          {typeof step.exitCode === "number" ? <span>exit {step.exitCode}</span> : null}
        </div>
        {output ? (
          <div className="mt-1 rounded border border-border/50 bg-background/60 px-2 py-1 font-mono text-[10px] leading-snug text-muted-foreground">
            {output}
          </div>
        ) : null}
      </div>
    </IconTip>
  )
}

function VerificationSection({
  sessionId,
  incognito,
  turnActive,
  workingDir,
  verificationRunsState,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  verificationRunsState?: VerificationRunsState
}) {
  const { t } = useTranslation()
  const ownedVerificationRunsState = useVerificationRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(verificationRunsState),
  })
  const {
    runs,
    snapshot,
    loading,
    planning,
    running,
    error,
    refresh,
    planVerification,
    runVerification,
  } = verificationRunsState ?? ownedVerificationRunsState
  const latest = snapshot?.run ?? runs[0]
  const steps = snapshot?.steps ?? []
  const visibleSteps = steps.slice(0, 6)
  const failed = verificationStatsNumber(snapshot, "failed")
  const passed = verificationStatsNumber(snapshot, "passed")
  const runnable = verificationStatsNumber(snapshot, "runnable")
  const gated = verificationStatsNumber(snapshot, "gated")
  const active = running || latest?.state === "running"
  const disabled = !sessionId || incognito || !workingDir || planning || active || loading

  const meta = active ? (
    <StatusPill label={t("workspace.verification.running", "验证中")} tone="info" loading />
  ) : latest?.state === "failed" || failed > 0 ? (
    <StatusPill label={t("workspace.verification.failed", "失败")} tone="danger" />
  ) : latest?.state === "completed" ? (
    <StatusPill label={t("workspace.verification.passed", "已验证")} tone="good" />
  ) : latest?.state === "planned" ? (
    <StatusPill label={t("workspace.verification.planned", "已推荐")} tone="info" />
  ) : (
    <StatusPill label={t("workspace.verification.idle", "待验证")} tone="muted" />
  )

  const handlePlan = async () => {
    const next = await planVerification()
    if (next) {
      toast.success(
        t("workspace.verification.planDone", "已推荐 {{count}} 条验证", {
          count: next.steps.length,
        }),
      )
    }
  }

  const handleRun = async () => {
    const next = await runVerification()
    if (next) {
      toast.success(t("workspace.verification.runStarted", "验证已开始"))
    }
  }

  return (
    <WorkspaceSection
      title={t("workspace.verification.title", "验证")}
      count={steps.length}
      icon={CheckCircle2}
      meta={meta}
      defaultExpanded={active || failed > 0 || !!error}
    >
      <div className="space-y-2">
        <div className="grid grid-cols-4 gap-1.5">
          {[
            [t("workspace.verification.runnable", "可跑"), runnable, "info" as StatusTone],
            [t("workspace.verification.passedShort", "通过"), passed, "good" as StatusTone],
            [t("workspace.verification.failedShort", "失败"), failed, "danger" as StatusTone],
            [t("workspace.verification.gatedShort", "门控"), gated, "warn" as StatusTone],
          ].map(([label, count, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{count as number}</div>
            </div>
          ))}
        </div>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={handlePlan}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {planning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Gauge className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.verification.plan", "推荐验证")}</span>
          </button>
          <button
            type="button"
            onClick={handleRun}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {active ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.verification.run", "运行推荐")}</span>
          </button>
          <IconTip label={t("workspace.verification.refresh", "刷新验证结果")}>
            <button
              type="button"
              onClick={refresh}
              disabled={loading || planning || active}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
        </div>

        {!workingDir ? (
          <EmptyHint>
            {t("workspace.verification.noWorkspace", "选择工作目录后可生成验证建议")}
          </EmptyHint>
        ) : incognito ? (
          <EmptyHint>{t("workspace.verification.incognito", "无痕会话不持久化验证结果")}</EmptyHint>
        ) : error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : latest ? (
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-2">
            <div className="flex min-w-0 items-center gap-2">
              <GitCommitHorizontal className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                {latest.summary || t("workspace.verification.summaryPending", "验证结果待生成")}
              </span>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {latest.id.slice(0, 10)}
              </span>
            </div>
          </div>
        ) : (
          <EmptyHint>{t("workspace.verification.empty", "还没有验证记录")}</EmptyHint>
        )}

        {visibleSteps.length > 0 ? (
          <div className="space-y-1">
            {visibleSteps.map((step) => (
              <VerificationStepRow key={step.id} step={step} />
            ))}
            {steps.length > visibleSteps.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.verification.more", "还有 {{count}} 条", {
                  count: steps.length - visibleSteps.length,
                })}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function domainQualityStatsNumber(snapshot: DomainQualityRunSnapshot | null, key: string): number {
  const value = snapshot?.run.stats?.[key]
  return typeof value === "number" && Number.isFinite(value) ? value : 0
}

function domainQualityStateTone(state?: DomainQualityRunState | null): StatusTone {
  switch (state) {
    case "completed":
      return "good"
    case "blocked":
    case "failed":
      return "danger"
    case "needs_user":
      return "warn"
    case "running":
      return "info"
    case "cancelled":
      return "muted"
    default:
      return "muted"
  }
}

function domainQualityStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state?: DomainQualityRunState | null,
): string {
  switch (state) {
    case "completed":
      return t("workspace.domainQuality.passed", "已通过")
    case "blocked":
      return t("workspace.domainQuality.blocked", "已阻塞")
    case "failed":
      return t("workspace.domainQuality.failed", "失败")
    case "needs_user":
      return t("workspace.domainQuality.needsUser", "需确认")
    case "running":
      return t("workspace.domainQuality.running", "复核中")
    case "cancelled":
      return t("workspace.domainQuality.cancelled", "已取消")
    default:
      return t("workspace.domainQuality.idle", "待复核")
  }
}

function domainQualityCheckTone(status: DomainQualityCheckStatus): StatusTone {
  switch (status) {
    case "passed":
      return "good"
    case "failed":
    case "blocked":
      return "danger"
    case "needs_user":
      return "warn"
    case "advisory":
      return "muted"
  }
}

function domainQualityCheckLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status: DomainQualityCheckStatus,
): string {
  switch (status) {
    case "passed":
      return t("workspace.domainQuality.checkPassed", "通过")
    case "failed":
      return t("workspace.domainQuality.checkFailed", "缺失")
    case "blocked":
      return t("workspace.domainQuality.checkBlocked", "阻塞")
    case "needs_user":
      return t("workspace.domainQuality.checkNeedsUser", "需确认")
    case "advisory":
      return t("workspace.domainQuality.checkAdvisory", "建议")
  }
}

function domainQualitySeverityTone(severity: DomainQualitySeverity): StatusTone {
  switch (severity) {
    case "p0":
    case "p1":
      return "danger"
    case "p2":
      return "warn"
    case "p3":
      return "muted"
  }
}

function domainLabel(domain?: string | null): string {
  return domain ? domain.replace(/_/g, " ") : "domain"
}

function domainQualityTemplateLabel(run: DomainQualityRunSnapshot["run"]): string | null {
  if (!run.templateId) return null
  return run.templateVersion ? `${run.templateId}@${run.templateVersion}` : run.templateId
}

function DomainQualityCheckRow({ check }: { check: DomainQualityCheck }) {
  const { t } = useTranslation()
  const icon =
    check.status === "passed" ? (
      <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
    ) : check.status === "failed" || check.status === "blocked" ? (
      <CircleAlert className="h-3.5 w-3.5 shrink-0 text-destructive" />
    ) : check.status === "needs_user" ? (
      <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-amber-600" />
    ) : (
      <Gauge className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
    )
  return (
    <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
      <div className="flex min-w-0 items-center gap-1.5">
        {icon}
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
          {check.title}
        </span>
        <StatusPill
          label={check.severity.toUpperCase()}
          tone={domainQualitySeverityTone(check.severity)}
        />
        <StatusPill
          label={domainQualityCheckLabel(t, check.status)}
          tone={domainQualityCheckTone(check.status)}
        />
      </div>
      <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
        {check.body}
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5 text-[10px] text-muted-foreground/65">
        <span className="truncate">{check.profile}</span>
        <span className="truncate">{check.checkType}</span>
        {check.evidenceType ? <span className="truncate">{check.evidenceType}</span> : null}
      </div>
    </div>
  )
}

function DomainQualitySection({
  sessionId,
  incognito,
  turnActive,
  domainQualityRunsState,
  domainWorkbenchState,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  domainQualityRunsState?: DomainQualityRunsState
  domainWorkbenchState?: DomainTaskWorkbenchState
}) {
  const { t } = useTranslation()
  const [learningRunId, setLearningRunId] = useState<string | null>(null)
  const ownedDomainQualityRunsState = useDomainQualityRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(domainQualityRunsState),
  })
  const ownedDomainWorkbenchState = useDomainTaskWorkbench(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(domainWorkbenchState),
  })
  const { runs, snapshot, loading, running, error, refresh, runDomainQuality } =
    domainQualityRunsState ?? ownedDomainQualityRunsState
  const {
    exportGuard,
    exportGuardLoading,
    exportGuardError,
    connectorGuard,
    connectorGuardLoading,
    connectorGuardError,
    refreshExportGuard,
    refreshConnectorGuard,
  } = domainWorkbenchState ?? ownedDomainWorkbenchState
  const latest = snapshot?.run ?? runs[0]
  const checks = snapshot?.checks ?? []
  const focusChecks = checks.filter((check) => check.status !== "passed")
  const visibleChecks = (focusChecks.length > 0 ? focusChecks : checks).slice(0, 6)
  const passed = domainQualityStatsNumber(snapshot, "passed")
  const failed = domainQualityStatsNumber(snapshot, "failed")
  const needsUser = domainQualityStatsNumber(snapshot, "needsUser")
  const advisory = domainQualityStatsNumber(snapshot, "advisory")
  const active = running || latest?.state === "running"
  const disabled = !sessionId || incognito || active || loading
  const learning =
    learningRunId !== null && latest !== undefined && learningRunId === latest.id
  const canGenerateLearning =
    !!sessionId &&
    !incognito &&
    !!latest &&
    !active &&
    latest.state !== "cancelled" &&
    !loading &&
    !learning
  const meta = active ? (
    <StatusPill label={t("workspace.domainQuality.running", "复核中")} tone="info" loading />
  ) : latest ? (
    <StatusPill
      label={domainQualityStateLabel(t, latest.state)}
      tone={domainQualityStateTone(latest.state)}
    />
  ) : (
    <StatusPill label={t("workspace.domainQuality.idle", "待复核")} tone="muted" />
  )

  const handleRun = async () => {
    const next = await runDomainQuality()
    if (next) {
      if (next.run.state === "completed") {
        toast.success(t("workspace.domainQuality.runDoneClean", "领域复核通过"))
      } else if (next.run.state === "needs_user") {
        toast.warning(t("workspace.domainQuality.runNeedsUser", "领域复核需要用户确认"))
      } else {
        toast.error(t("workspace.domainQuality.runBlocked", "领域复核发现阻塞项"))
      }
      void refreshExportGuard()
      void refreshConnectorGuard()
    }
  }

  const handleGenerateLearning = async () => {
    if (!sessionId || !latest || !canGenerateLearning) return
    setLearningRunId(latest.id)
    try {
      const result = await getTransport().call<GenerateCodingImprovementProposalsResult>(
        "generate_coding_improvement_proposals",
        {
          sessionId,
          windowDays: DOMAIN_LEARNING_WINDOW_DAYS,
          sourceType: "domain_quality",
          sourceId: latest.id,
        },
      )
      window.dispatchEvent(
        new CustomEvent(CODING_IMPROVEMENT_CHANGED_EVENT, {
          detail: { sessionId, sourceType: "domain_quality", sourceId: latest.id },
        }),
      )
      toast.success(
        result.inserted > 0
          ? t("workspace.domainQuality.learningGenerated", "已生成 {{count}} 个学习候选", {
              count: result.inserted,
            })
          : t("workspace.domainQuality.learningNoop", "学习候选已是最新"),
      )
    } catch (e) {
      logger.error(
        "ui",
        "DomainQualitySection::generateLearning",
        "Failed to generate domain learning proposals",
        e,
      )
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setLearningRunId(null)
    }
  }

  return (
    <WorkspaceSection
      title={t("workspace.domainQuality.title", "领域复核")}
      count={focusChecks.length}
      icon={ClipboardCheck}
      meta={meta}
      defaultExpanded={!!error || focusChecks.length > 0 || latest?.state === "needs_user"}
    >
      <div className="space-y-2">
        <div className="grid grid-cols-4 gap-1.5">
          {[
            [t("workspace.domainQuality.passedShort", "通过"), passed, "good" as StatusTone],
            [t("workspace.domainQuality.failedShort", "缺失"), failed, "danger" as StatusTone],
            [t("workspace.domainQuality.needsUserShort", "确认"), needsUser, "warn" as StatusTone],
            [t("workspace.domainQuality.advisoryShort", "建议"), advisory, "muted" as StatusTone],
          ].map(([label, count, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{count as number}</div>
            </div>
          ))}
        </div>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={handleRun}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {active ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ClipboardCheck className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.domainQuality.run", "运行领域复核")}</span>
          </button>
          <button
            type="button"
            onClick={handleGenerateLearning}
            disabled={!canGenerateLearning}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {learning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Brain className="h-3.5 w-3.5" />
            )}
            <span className="truncate">
              {t("workspace.domainQuality.generateLearning", "提炼经验")}
            </span>
          </button>
          <IconTip label={t("workspace.domainQuality.refresh", "刷新领域复核")}>
            <button
              type="button"
              onClick={refresh}
              disabled={loading || active}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
        </div>

        {!incognito ? (
          <>
            <DomainConnectorActionGuardPanel
              report={connectorGuard}
              loading={connectorGuardLoading}
              error={connectorGuardError}
              disabled={!sessionId || connectorGuardLoading}
              onRefresh={refreshConnectorGuard}
            />
            <DomainArtifactExportGuardPanel
              report={exportGuard}
              loading={exportGuardLoading}
              error={exportGuardError}
              disabled={!sessionId || exportGuardLoading}
              onRefresh={refreshExportGuard}
            />
          </>
        ) : null}

        {incognito ? (
          <EmptyHint>
            {t("workspace.domainQuality.incognito", "无痕会话不持久化领域复核")}
          </EmptyHint>
        ) : error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : latest ? (
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-2">
            <div className="flex min-w-0 items-center gap-2">
              <ClipboardCheck className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                {latest.summary || t("workspace.domainQuality.summaryPending", "复核结果待生成")}
              </span>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {latest.id.slice(0, 10)}
              </span>
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap gap-1 pl-5">
              <StatusPill label={domainLabel(latest.domain)} tone="info" />
              {domainQualityTemplateLabel(latest) ? (
                <StatusPill label={domainQualityTemplateLabel(latest)!} tone="muted" />
              ) : null}
            </div>
          </div>
        ) : (
          <EmptyHint>{t("workspace.domainQuality.empty", "还没有领域复核记录")}</EmptyHint>
        )}

        {visibleChecks.length > 0 ? (
          <div className="space-y-1">
            {visibleChecks.map((check) => (
              <DomainQualityCheckRow key={check.id} check={check} />
            ))}
            {checks.length > visibleChecks.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.domainQuality.more", "还有 {{count}} 条", {
                  count: checks.length - visibleChecks.length,
                })}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

function DomainOperationalGatePanel({
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  report: DomainOperationalGateReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainOperationalGateReport | null>
}) {
  const { t } = useTranslation()
  const issueChecks = (report?.checks ?? []).filter(
    (check) => check.status !== "passed" && check.severity !== "advisory",
  )
  const summary = report?.summary
  const clean = report?.status === "passed"

  return (
    <div className="rounded-md border border-border/55 bg-background/45 px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <Gauge className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium text-foreground/90">
            {t("workspace.domainOperationalGate.title", "运行稳定性")}
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {report
              ? t("workspace.domainOperationalGate.generated", "最近评估 {{time}}", {
                  time: formatMessageTime(report.generatedAt),
                })
              : t("workspace.domainOperationalGate.emptyHint", "检查工作流、Loop 和评测活动是否已排空")}
          </div>
        </div>
        <StatusPill
          label={domainOperationalGateLabel(t, report?.status, loading)}
          tone={domainOperationalGateTone(report?.status, loading)}
          loading={loading}
        />
        <IconTip label={t("workspace.domainOperationalGate.refresh", "刷新运行稳定性")}>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={disabled}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </IconTip>
      </div>

      {summary ? (
        <div className="mt-2 grid grid-cols-4 gap-1.5">
          {[
            [
              t("workspace.domainOperationalGate.workflows", "工作流"),
              `${summary.completedWorkflowRuns}/${summary.workflowRuns}`,
              summary.workflowRuns > 0 ? "good" : "warn",
            ],
            [
              t("workspace.domainOperationalGate.active", "运行中"),
              summary.activeWorkflowRuns,
              summary.activeWorkflowRuns > 0 ? "warn" : "muted",
            ],
            [
              t("workspace.domainOperationalGate.loops", "Loop"),
              `${summary.succeededLoopRuns}/${summary.loopRuns}`,
              summary.failedLoopRuns > 0 ? "danger" : summary.loopRuns > 0 ? "good" : "muted",
            ],
            [
              t("workspace.domainOperationalGate.campaigns", "评测"),
              summary.activeCampaigns,
              summary.activeCampaigns > 0 ? "warn" : "muted",
            ],
          ].map(([label, value, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{value as string | number}</div>
            </div>
          ))}
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t("workspace.domainOperationalGate.clean", "运行面已排空且没有失败残留。")}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 3).map((check) => (
            <div
              key={check.name}
              className={cn(
                "rounded-md px-2 py-1.5 text-[11px]",
                check.status === "failed"
                  ? "bg-destructive/10 text-destructive"
                  : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <CircleAlert className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">{check.name}</span>
                <span className="shrink-0 tabular-nums">{check.actual}</span>
              </div>
              <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
            </div>
          ))}
        </div>
      ) : !loading ? (
        <EmptyHint>{t("workspace.domainOperationalGate.empty", "还没有运行稳定性结果")}</EmptyHint>
      ) : null}
    </div>
  )
}

function DomainSoakReportPanel({
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  report: DomainSoakReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainSoakReport | null>
}) {
  const { t } = useTranslation()
  const summary = report?.summary
  const clean = report?.status === "passed"
  const maxDrain =
    summary?.maxWorkflowDrainSecs != null
      ? formatLoopDuration(Math.max(1, Math.round(summary.maxWorkflowDrainSecs)))
      : "-"

  return (
    <div className="rounded-md border border-border/55 bg-background/45 px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <Radio className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium text-foreground/90">
            {t("workspace.domainSoakReport.title", "长跑审计")}
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {report
              ? t("workspace.domainSoakReport.generated", "最近评估 {{time}}", {
                  time: formatMessageTime(report.generatedAt),
                })
              : t("workspace.domainSoakReport.emptyHint", "汇总最近长任务、Loop 和连接器链路事故")}
          </div>
        </div>
        <StatusPill
          label={domainSoakReportLabel(t, report?.status, loading)}
          tone={domainSoakReportTone(report?.status, loading)}
          loading={loading}
        />
        <IconTip label={t("workspace.domainSoakReport.refresh", "刷新长跑审计")}>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={disabled}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </IconTip>
      </div>

      {summary ? (
        <div className="mt-2 grid grid-cols-4 gap-1.5">
          {[
            [t("workspace.domainSoakReport.records", "样本"), summary.totalRecords, "info"],
            [
              t("workspace.domainSoakReport.critical", "事故"),
              summary.criticalIncidents,
              summary.criticalIncidents > 0 ? "danger" : "muted",
            ],
            [
              t("workspace.domainSoakReport.warning", "待排空"),
              summary.warningIncidents,
              summary.warningIncidents > 0 ? "warn" : "muted",
            ],
            [t("workspace.domainSoakReport.maxDrain", "最长"), maxDrain, "muted"],
          ].map(([label, value, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{value as string | number}</div>
            </div>
          ))}
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t("workspace.domainSoakReport.clean", "最近窗口没有长任务事故。")}
        </div>
      ) : report?.incidents?.length ? (
        <div className="mt-2 space-y-1">
          {report.incidents.slice(0, 2).map((incident) => (
            <div
              key={`${incident.source}:${incident.id}`}
              className={cn(
                "rounded-md px-2 py-1.5 text-[11px]",
                incident.severity === "critical"
                  ? "bg-destructive/10 text-destructive"
                  : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <CircleAlert className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">{incident.title}</span>
                <StatusPill
                  label={incident.source}
                  tone={incident.severity === "critical" ? "danger" : "warn"}
                />
              </div>
              <div className="mt-0.5 line-clamp-2 opacity-85">{incident.recommendation}</div>
            </div>
          ))}
        </div>
      ) : !loading ? (
        <EmptyHint>{t("workspace.domainSoakReport.empty", "还没有长跑审计结果")}</EmptyHint>
      ) : null}
    </div>
  )
}

function domainOperationalGateTone(
  status?: string | null,
  loading?: boolean,
): StatusTone {
  if (loading) return "info"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainOperationalGateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
): string {
  if (loading) return t("workspace.domainOperationalGate.loading", "评估中")
  if (status === "passed") return t("workspace.domainOperationalGate.passed", "稳定")
  if (status === "failed") return t("workspace.domainOperationalGate.failed", "阻塞")
  if (status === "insufficient_data") return t("workspace.domainOperationalGate.insufficient", "待排空")
  return t("workspace.domainOperationalGate.idle", "未评估")
}

function domainSoakReportTone(status?: string | null, loading?: boolean): StatusTone {
  if (loading) return "info"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainSoakReportLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
): string {
  if (loading) return t("workspace.domainSoakReport.loading", "评估中")
  if (status === "passed") return t("workspace.domainSoakReport.passed", "干净")
  if (status === "failed") return t("workspace.domainSoakReport.failed", "有事故")
  if (status === "insufficient_data") return t("workspace.domainSoakReport.insufficient", "样本不足")
  return t("workspace.domainSoakReport.idle", "未评估")
}

function DomainArtifactExportGuardPanel({
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  report: DomainArtifactExportGuardReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainArtifactExportGuardReport | null>
}) {
  const { t } = useTranslation()
  const issueChecks = (report?.checks ?? []).filter((check) => check.status !== "passed")
  const summary = report?.summary
  const clean = report?.status === "passed"

  return (
    <div className="rounded-md border border-border/55 bg-background/45 px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <Shield className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium text-foreground/90">
            {t("workspace.domainExportGuard.title", "交付守门")}
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {report
              ? t("workspace.domainExportGuard.generated", "最近评估 {{time}}", {
                  time: formatMessageTime(report.generatedAt),
                })
              : t("workspace.domainExportGuard.emptyHint", "检查最终产物、复核和脱敏状态")}
          </div>
        </div>
        <StatusPill
          label={domainArtifactExportGuardLabel(t, report?.status, loading)}
          tone={domainArtifactExportGuardTone(report?.status, loading)}
          loading={loading}
        />
        <IconTip label={t("workspace.domainExportGuard.refresh", "刷新交付守门")}>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={disabled}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </IconTip>
      </div>

      {summary ? (
        <div className="mt-2 grid grid-cols-4 gap-1.5">
          {[
            [t("workspace.domainExportGuard.artifact", "产物"), summary.artifactCreated, "info"],
            [t("workspace.domainExportGuard.reviewed", "复核"), summary.artifactReviewed, "good"],
            [t("workspace.domainExportGuard.sensitive", "敏感"), summary.sensitiveEvidence, summary.sensitiveEvidence > 0 ? "warn" : "muted"],
            [t("workspace.domainExportGuard.redaction", "待脱敏"), summary.redactionPending, summary.redactionPending > 0 ? "danger" : "muted"],
          ].map(([label, count, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{count as number}</div>
            </div>
          ))}
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t("workspace.domainExportGuard.clean", "最终交付证据通过，可以进入发送、分享或导出前的用户确认。")}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 3).map((check) => (
            <div
              key={check.name}
              className={cn(
                "rounded-md px-2 py-1.5 text-[11px]",
                check.status === "failed"
                  ? "bg-destructive/10 text-destructive"
                  : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <CircleAlert className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">{check.name}</span>
                <span className="shrink-0 tabular-nums">{check.actual}</span>
              </div>
              <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
            </div>
          ))}
        </div>
      ) : !loading ? (
        <EmptyHint>{t("workspace.domainExportGuard.empty", "还没有交付守门结果")}</EmptyHint>
      ) : null}

      {report?.evidenceRequiringReview.length ? (
        <div className="mt-2 space-y-1">
          {report.evidenceRequiringReview.slice(0, 2).map((item) => (
            <div
              key={item.id}
              className="min-w-0 rounded-md border border-amber-500/20 bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <ShieldAlert className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">{item.title}</span>
                <StatusPill label={item.reason} tone="warn" />
              </div>
              <div className="mt-0.5 truncate font-mono text-[10px] opacity-75">
                {item.accessScope} · {item.redactionStatus}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function DomainConnectorActionGuardPanel({
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  report: DomainConnectorActionGuardReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainConnectorActionGuardReport | null>
}) {
  const { t } = useTranslation()
  const issueChecks = (report?.checks ?? []).filter((check) => check.status !== "passed")
  const summary = report?.summary
  const clean = report?.status === "passed"

  return (
    <div className="rounded-md border border-border/55 bg-background/45 px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium text-foreground/90">
            {t("workspace.domainConnectorGuard.title", "外部动作守门")}
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {report
              ? t("workspace.domainConnectorGuard.generated", "最近评估 {{time}}", {
                  time: formatMessageTime(report.generatedAt),
                })
              : t("workspace.domainConnectorGuard.emptyHint", "检查连接器动作、用户批准和回滚提示")}
          </div>
        </div>
        <StatusPill
          label={domainConnectorActionGuardLabel(t, report?.status, loading)}
          tone={domainConnectorActionGuardTone(report?.status, loading)}
          loading={loading}
        />
        <IconTip label={t("workspace.domainConnectorGuard.refresh", "刷新外部动作守门")}>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={disabled}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </button>
        </IconTip>
      </div>

      {summary ? (
        <div className="mt-2 grid grid-cols-4 gap-1.5">
          {[
            [t("workspace.domainConnectorGuard.action", "动作"), summary.actionEvidence, "info"],
            [t("workspace.domainConnectorGuard.approval", "批准"), summary.approvalEvidence, summary.approvalEvidence > 0 ? "good" : "danger"],
            [t("workspace.domainConnectorGuard.rollback", "回滚"), summary.rollbackEvidence, summary.rollbackEvidence > 0 ? "good" : "warn"],
            [t("workspace.domainConnectorGuard.sensitive", "敏感"), summary.sensitiveEvidence, summary.sensitiveEvidence > 0 ? "warn" : "muted"],
          ].map(([label, count, tone]) => (
            <div
              key={label as string}
              className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone as StatusTone])}
            >
              <div className="truncate text-[10px]">{label as string}</div>
              <div className="text-xs font-semibold tabular-nums">{count as number}</div>
            </div>
          ))}
        </div>
      ) : null}

      {report?.connector || report?.action || report?.toolName ? (
        <div className="mt-2 flex min-w-0 flex-wrap gap-1">
          {report.connector ? <StatusPill label={report.connector} tone="info" /> : null}
          {report.action ? <StatusPill label={report.action} tone="muted" /> : null}
          {report.toolName ? <StatusPill label={report.toolName} tone="muted" /> : null}
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t("workspace.domainConnectorGuard.clean", "外部动作证据通过，真正执行前仍会逐次弹出确认。")}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 3).map((check) => (
            <div
              key={check.name}
              className={cn(
                "rounded-md px-2 py-1.5 text-[11px]",
                check.status === "failed"
                  ? "bg-destructive/10 text-destructive"
                  : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <CircleAlert className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">{check.name}</span>
                <span className="shrink-0 tabular-nums">{check.actual}</span>
              </div>
              <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
            </div>
          ))}
        </div>
      ) : !loading ? (
        <EmptyHint>{t("workspace.domainConnectorGuard.empty", "还没有外部动作守门结果")}</EmptyHint>
      ) : null}

      {report?.relatedEvidence.length ? (
        <div className="mt-2 space-y-1">
          {report.relatedEvidence.slice(0, 2).map((item) => (
            <div
              key={item.id}
              className="min-w-0 rounded-md border border-border/40 bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <Shield className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium text-foreground/80">
                  {item.title}
                </span>
                <StatusPill label={item.reason} tone="muted" />
              </div>
              <div className="mt-0.5 truncate font-mono text-[10px] opacity-75">
                {item.accessScope} · {item.redactionStatus}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function domainConnectorActionGuardTone(
  status?: string | null,
  loading?: boolean,
): StatusTone {
  if (loading) return "info"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainConnectorActionGuardLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
): string {
  if (loading) return t("workspace.domainConnectorGuard.loading", "评估中")
  if (status === "passed") return t("workspace.domainConnectorGuard.passed", "可执行")
  if (status === "failed") return t("workspace.domainConnectorGuard.failed", "阻塞")
  if (status === "insufficient_data") {
    return t("workspace.domainConnectorGuard.insufficient", "缺证据")
  }
  return t("workspace.domainConnectorGuard.idle", "未评估")
}

function domainArtifactExportGuardTone(
  status?: string | null,
  loading?: boolean,
): StatusTone {
  if (loading) return "info"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainArtifactExportGuardLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
): string {
  if (loading) return t("workspace.domainExportGuard.loading", "评估中")
  if (status === "passed") return t("workspace.domainExportGuard.passed", "可交付")
  if (status === "failed") return t("workspace.domainExportGuard.failed", "阻塞")
  if (status === "insufficient_data") {
    return t("workspace.domainExportGuard.insufficient", "缺证据")
  }
  return t("workspace.domainExportGuard.idle", "未评估")
}

function trendPercent(value?: number | null): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "—"
  return `${Math.round(value * 100)}%`
}

function codingTrendMetricTone(value?: number | null): StatusTone {
  if (typeof value !== "number" || !Number.isFinite(value)) return "muted"
  if (value >= 0.8) return "good"
  if (value >= 0.5) return "warn"
  return "danger"
}

function codingFailureTone(severity: string): StatusTone {
  switch (severity) {
    case "high":
      return "danger"
    case "medium":
      return "warn"
    default:
      return "muted"
  }
}

function codingProposalTone(status: string): StatusTone {
  switch (status) {
    case "applying":
    case "promoting":
      return "info"
    case "applied":
    case "promoted":
      return "good"
    case "failed":
    case "promotion_failed":
      return "danger"
    case "rejected":
      return "muted"
    default:
      return "info"
  }
}

function codingProposalKindLabel(t: ReturnType<typeof useTranslation>["t"], kind: string): string {
  switch (kind) {
    case "eval_candidate":
      return t("workspace.codingTrend.kindEval", "评测候选")
    case "workflow_template":
      return t("workspace.codingTrend.kindWorkflow", "工作流模板")
    case "guidance_candidate":
      return t("workspace.codingTrend.kindGuidance", "规则候选")
    case "skill_candidate":
      return t("workspace.codingTrend.kindSkill", "Skill 候选")
    case "domain_workflow_template":
      return t("workspace.codingTrend.kindDomainWorkflow", "领域工作流")
    case "domain_guidance":
      return t("workspace.codingTrend.kindDomainGuidance", "领域规则")
    case "domain_review_profile":
      return t("workspace.codingTrend.kindDomainReview", "领域复核")
    case "domain_eval_case":
      return t("workspace.codingTrend.kindDomainEval", "领域评测")
    case "connector_usage_pattern":
      return t("workspace.codingTrend.kindConnectorPattern", "连接器模式")
    default:
      return kind
  }
}

function codingProposalStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status: string,
): string {
  switch (status) {
    case "applying":
      return t("workspace.codingTrend.applying", "应用中")
    case "applied":
      return t("workspace.codingTrend.applied", "已应用")
    case "promoting":
      return t("workspace.codingTrend.promoting", "晋升中")
    case "promoted":
      return t("workspace.codingTrend.promoted", "已晋升")
    case "failed":
      return t("workspace.codingTrend.applyFailed", "应用失败")
    case "promotion_failed":
      return t("workspace.codingTrend.promotionFailed", "晋升失败")
    case "rejected":
      return t("workspace.codingTrend.rejected", "已拒绝")
    default:
      return t("workspace.codingTrend.draft", "草案")
  }
}

function CodingTrendMetric({
  label,
  value,
  tone,
}: {
  label: string
  value: string | number
  tone: StatusTone
}) {
  return (
    <div className={cn("rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone])}>
      <div className="truncate text-[10px]">{label}</div>
      <div className="text-xs font-semibold tabular-nums">{value}</div>
    </div>
  )
}

function CodingFailureRow({ failure }: { failure: CodingFailureBucket }) {
  return (
    <IconTip label={failure.examples.join("\n") || failure.label}>
      <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <CircleAlert className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {failure.label}
          </span>
          <StatusPill label={String(failure.count)} tone={codingFailureTone(failure.severity)} />
        </div>
        {failure.examples[0] ? (
          <div className="mt-1 truncate pl-5 text-[11px] text-muted-foreground">
            {failure.examples[0]}
          </div>
        ) : null}
      </div>
    </IconTip>
  )
}

function CodingRetroRow({ retro }: { retro: CodingWorkflowRetro }) {
  const { t } = useTranslation()
  const topRecommendation = retro.recommendations[0]
  const severeSignal = retro.signals.find((signal) => signal.severity === "high")
  return (
    <IconTip label={topRecommendation?.rationale ?? retro.summary}>
      <div className="rounded-md border border-border/50 bg-background/55 px-2.5 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <Brain className="h-3.5 w-3.5 shrink-0 text-sky-500" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {retro.summary}
          </span>
          <StatusPill
            label={retro.runState}
            tone={retro.runState === "completed" ? "good" : severeSignal ? "danger" : "warn"}
          />
        </div>
        {topRecommendation ? (
          <div className="mt-1 truncate pl-5 text-[11px] text-muted-foreground">
            {t("workspace.codingTrend.retroRecommendation", "建议")} · {topRecommendation.title}
          </div>
        ) : null}
      </div>
    </IconTip>
  )
}

function CodingProposalDetail({
  proposal,
  actionPlan,
  promotionPlan,
  previewing,
  applying,
  previewingPromotion,
  promoting,
  importingDomainEvalCase,
  updating,
  onPreview,
  onApply,
  onPreviewPromotion,
  onPromote,
  onImportDomainEvalCase,
  onReject,
}: {
  proposal: CodingImprovementProposal
  actionPlan: CodingImprovementActionPlan | null
  promotionPlan: CodingImprovementPromotionPlan | null
  previewing?: boolean
  applying?: boolean
  previewingPromotion?: boolean
  promoting?: boolean
  importingDomainEvalCase?: boolean
  updating?: boolean
  onPreview: (proposalId: string) => void
  onApply: (proposalId: string) => void
  onPreviewPromotion: (proposalId: string) => void
  onPromote: (proposalId: string) => void
  onImportDomainEvalCase: (proposalId: string) => void
  onReject: (proposalId: string) => void
}) {
  const { t } = useTranslation()
  const plan = actionPlan?.proposal.id === proposal.id ? actionPlan : null
  const promotion = promotionPlan?.proposal.id === proposal.id ? promotionPlan : null
  const action = proposal.action
  const promotionRecord = proposal.promotion
  const disabled = proposal.status !== "draft" || previewing || applying || updating
  const canPromote =
    (proposal.status === "applied" || proposal.status === "promotion_failed") &&
    !previewingPromotion &&
    !promoting
  const canImportDomainEvalCase =
    proposal.kind === "domain_eval_case" && proposal.status === "promoted" && !importingDomainEvalCase
  return (
    <div className="mt-2 space-y-2 border-t border-border/50 pt-2 pl-5">
      <div className="flex min-w-0 items-center gap-1.5">
        <button
          type="button"
          disabled={previewing}
          onClick={() => onPreview(proposal.id)}
          className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-background/60 px-2 py-1.5 text-[11px] font-medium text-foreground transition-colors hover:bg-secondary/50 disabled:cursor-not-allowed disabled:opacity-55"
        >
          {previewing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Eye className="h-3.5 w-3.5" />
          )}
          <span className="truncate">{t("workspace.codingTrend.previewAction", "预览")}</span>
        </button>
        <button
          type="button"
          disabled={disabled || !plan}
          onClick={() => onApply(proposal.id)}
          className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-emerald-500/35 bg-emerald-500/10 px-2 py-1.5 text-[11px] font-medium text-emerald-700 transition-colors hover:bg-emerald-500/15 disabled:cursor-not-allowed disabled:opacity-55 dark:text-emerald-300"
        >
          {applying ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Check className="h-3.5 w-3.5" />
          )}
          <span className="truncate">{t("workspace.codingTrend.applyAction", "应用")}</span>
        </button>
        <IconTip label={t("workspace.codingTrend.reject", "拒绝")}>
          <button
            type="button"
            disabled={proposal.status !== "draft" || updating}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-background/60 text-muted-foreground transition-colors hover:bg-secondary/50 hover:text-destructive disabled:opacity-45"
            onClick={() => onReject(proposal.id)}
            aria-label={t("workspace.codingTrend.reject", "拒绝")}
          >
            {updating ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <X className="h-3.5 w-3.5" />
            )}
          </button>
        </IconTip>
      </div>

      {plan ? (
        <div className="space-y-1.5">
          <div className="text-[11px] leading-snug text-muted-foreground">{plan.summary}</div>
          {plan.steps.map((step) => (
            <div
              key={`${step.action}:${step.targetPath}`}
              className="rounded-md border border-border/50 bg-background/60 px-2 py-1.5"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
                  {step.label}
                </span>
                <StatusPill
                  label={
                    step.targetExists
                      ? t("workspace.codingTrend.targetExists", "已存在")
                      : t("workspace.codingTrend.newTarget", "新建")
                  }
                  tone={step.targetExists ? "warn" : "good"}
                />
              </div>
              <div className="mt-1 truncate pl-5 text-[10px] text-muted-foreground/70">
                {step.targetPath}
              </div>
              {step.contentPreview ? (
                <pre className="mt-1.5 max-h-40 overflow-auto rounded border border-border/40 bg-muted/35 p-2 text-[10px] leading-snug text-muted-foreground">
                  {step.contentPreview}
                </pre>
              ) : null}
            </div>
          ))}
        </div>
      ) : (
        <div className="rounded-md border border-border/50 bg-background/50 px-2 py-1.5 text-[11px] text-muted-foreground">
          {t("workspace.codingTrend.previewEmpty", "打开预览后再应用")}
        </div>
      )}

      {action?.artifacts?.length ? (
        <div className="space-y-1">
          {action.artifacts.map((artifact) => (
            <div
              key={`${artifact.kind}:${artifact.path}`}
              className="flex min-w-0 items-center gap-1.5 rounded-md border border-emerald-500/25 bg-emerald-500/10 px-2 py-1.5 text-[11px]"
            >
              <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600 dark:text-emerald-300" />
              <span className="min-w-0 flex-1 truncate text-emerald-700 dark:text-emerald-200">
                {artifact.path}
              </span>
            </div>
          ))}
        </div>
      ) : action?.error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {action.error}
        </div>
      ) : null}

      {action?.applied ||
      proposal.status === "promotion_failed" ||
      proposal.status === "promoted" ? (
        <div className="space-y-2 rounded-md border border-border/50 bg-background/50 p-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <Sparkles className="h-3.5 w-3.5 shrink-0 text-sky-500" />
            <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
              {t("workspace.codingTrend.promotionTitle", "晋升为正式能力")}
            </span>
            {proposal.status === "promoted" ? (
              <StatusPill label={t("workspace.codingTrend.promoted", "已晋升")} tone="good" />
            ) : proposal.status === "promotion_failed" ? (
              <StatusPill
                label={t("workspace.codingTrend.promotionFailed", "晋升失败")}
                tone="danger"
              />
            ) : null}
          </div>
          <div className="flex min-w-0 items-center gap-1.5">
            <button
              type="button"
              disabled={!canPromote && proposal.status !== "promoted"}
              onClick={() => onPreviewPromotion(proposal.id)}
              className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-background/60 px-2 py-1.5 text-[11px] font-medium text-foreground transition-colors hover:bg-secondary/50 disabled:cursor-not-allowed disabled:opacity-55"
            >
              {previewingPromotion ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Eye className="h-3.5 w-3.5" />
              )}
              <span className="truncate">
                {t("workspace.codingTrend.previewPromotion", "预览晋升")}
              </span>
            </button>
            <button
              type="button"
              disabled={!canPromote || !promotion}
              onClick={() => onPromote(proposal.id)}
              className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-sky-500/35 bg-sky-500/10 px-2 py-1.5 text-[11px] font-medium text-sky-700 transition-colors hover:bg-sky-500/15 disabled:cursor-not-allowed disabled:opacity-55 dark:text-sky-300"
            >
              {promoting ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.codingTrend.promoteAction", "晋升")}</span>
            </button>
          </div>

          {promotion ? (
            <div className="space-y-1.5">
              <div className="text-[11px] leading-snug text-muted-foreground">
                {promotion.summary}
              </div>
              {promotion.steps.map((step) => (
                <div
                  key={`${step.action}:${step.targetPath}`}
                  className="rounded-md border border-border/50 bg-background/60 px-2 py-1.5"
                >
                  <div className="flex min-w-0 items-center gap-1.5">
                    <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
                      {step.label}
                    </span>
                    <StatusPill
                      label={
                        step.targetExists
                          ? t("workspace.codingTrend.targetExists", "已存在")
                          : t("workspace.codingTrend.newTarget", "新建")
                      }
                      tone={step.targetExists ? "warn" : "good"}
                    />
                  </div>
                  <div className="mt-1 truncate pl-5 text-[10px] text-muted-foreground/70">
                    {step.targetPath}
                  </div>
                  {step.contentPreview ? (
                    <pre className="mt-1.5 max-h-32 overflow-auto rounded border border-border/40 bg-muted/35 p-2 text-[10px] leading-snug text-muted-foreground">
                      {step.contentPreview}
                    </pre>
                  ) : null}
                </div>
              ))}
            </div>
          ) : proposal.status === "applied" || proposal.status === "promotion_failed" ? (
            <div className="rounded-md border border-border/50 bg-background/50 px-2 py-1.5 text-[11px] text-muted-foreground">
              {t("workspace.codingTrend.promotionPreviewEmpty", "预览晋升后再执行")}
            </div>
          ) : null}

          {promotionRecord?.artifacts?.length ? (
            <div className="space-y-1">
              {promotionRecord.artifacts.map((artifact) => (
                <div
                  key={`${artifact.kind}:${artifact.path}`}
                  className="flex min-w-0 items-center gap-1.5 rounded-md border border-sky-500/25 bg-sky-500/10 px-2 py-1.5 text-[11px]"
                >
                  <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-sky-600 dark:text-sky-300" />
                  <span className="min-w-0 flex-1 truncate text-sky-700 dark:text-sky-200">
                    {artifact.path}
                  </span>
                </div>
              ))}
            </div>
          ) : promotionRecord?.error ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
              {promotionRecord.error}
            </div>
          ) : null}

          {proposal.kind === "domain_eval_case" && proposal.status === "promoted" ? (
            <button
              type="button"
              disabled={!canImportDomainEvalCase}
              onClick={() => onImportDomainEvalCase(proposal.id)}
              className="inline-flex w-full min-w-0 items-center justify-center gap-1.5 rounded-md border border-violet-500/35 bg-violet-500/10 px-2 py-1.5 text-[11px] font-medium text-violet-700 transition-colors hover:bg-violet-500/15 disabled:cursor-not-allowed disabled:opacity-55 dark:text-violet-300"
            >
              {importingDomainEvalCase ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Database className="h-3.5 w-3.5" />
              )}
              <span className="truncate">
                {t("workspace.codingTrend.importDomainEval", "导入评测")}
              </span>
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function CodingProposalRow({
  proposal,
  selected,
  actionPlan,
  promotionPlan,
  previewing,
  applying,
  previewingPromotion,
  promoting,
  importingDomainEvalCase,
  updating,
  onToggle,
  onPreview,
  onApply,
  onPreviewPromotion,
  onPromote,
  onImportDomainEvalCase,
  onReject,
}: {
  proposal: CodingImprovementProposal
  selected?: boolean
  actionPlan: CodingImprovementActionPlan | null
  promotionPlan: CodingImprovementPromotionPlan | null
  previewing?: boolean
  applying?: boolean
  previewingPromotion?: boolean
  promoting?: boolean
  importingDomainEvalCase?: boolean
  updating?: boolean
  onToggle: (proposalId: string) => void
  onPreview: (proposalId: string) => void
  onApply: (proposalId: string) => void
  onPreviewPromotion: (proposalId: string) => void
  onPromote: (proposalId: string) => void
  onImportDomainEvalCase: (proposalId: string) => void
  onReject: (proposalId: string) => void
}) {
  const { t } = useTranslation()
  return (
    <IconTip label={proposal.body}>
      <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-1.5">
        <button
          type="button"
          className="flex w-full min-w-0 items-center gap-1.5 text-left"
          onClick={() => onToggle(proposal.id)}
        >
          <Lightbulb className="h-3.5 w-3.5 shrink-0 text-amber-500" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
            {proposal.title}
          </span>
          <StatusPill
            label={codingProposalKindLabel(t, proposal.kind)}
            tone={codingProposalTone(proposal.status)}
          />
          {selected ? (
            <ChevronUp className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          )}
        </button>
        <div className="mt-1 line-clamp-2 pl-5 text-[11px] leading-snug text-muted-foreground">
          {proposal.body}
        </div>
        <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5">
          <span className="min-w-0 flex-1 truncate text-[10px] text-muted-foreground/65">
            {codingProposalStatusLabel(t, proposal.status)} ·{" "}
            {formatMessageTime(proposal.updatedAt)}
          </span>
          <IconTip label={t("workspace.codingTrend.previewAction", "预览")}>
            <button
              type="button"
              disabled={previewing}
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-foreground disabled:opacity-45"
              onClick={(event) => {
                event.stopPropagation()
                onPreview(proposal.id)
              }}
              aria-label={t("workspace.codingTrend.previewAction", "预览")}
            >
              {previewing ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Eye className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
          <IconTip label={t("workspace.codingTrend.applyAction", "应用")}>
            <button
              type="button"
              disabled={
                proposal.status !== "draft" ||
                applying ||
                previewing ||
                actionPlan?.proposal.id !== proposal.id
              }
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-emerald-600 disabled:opacity-45"
              onClick={(event) => {
                event.stopPropagation()
                onApply(proposal.id)
              }}
              aria-label={t("workspace.codingTrend.applyAction", "应用")}
            >
              {applying ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Check className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
          <IconTip label={t("workspace.codingTrend.promoteAction", "晋升")}>
            <button
              type="button"
              disabled={
                !(proposal.status === "applied" || proposal.status === "promotion_failed") ||
                promoting ||
                previewingPromotion ||
                promotionPlan?.proposal.id !== proposal.id
              }
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-sky-600 disabled:opacity-45"
              onClick={(event) => {
                event.stopPropagation()
                onPromote(proposal.id)
              }}
              aria-label={t("workspace.codingTrend.promoteAction", "晋升")}
            >
              {promoting ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
          {proposal.kind === "domain_eval_case" && proposal.status === "promoted" ? (
            <IconTip label={t("workspace.codingTrend.importDomainEval", "导入评测")}>
              <button
                type="button"
                disabled={importingDomainEvalCase}
                className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary/60 hover:text-violet-600 disabled:opacity-45"
                onClick={(event) => {
                  event.stopPropagation()
                  onImportDomainEvalCase(proposal.id)
                }}
                aria-label={t("workspace.codingTrend.importDomainEval", "导入评测")}
              >
                {importingDomainEvalCase ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Database className="h-3.5 w-3.5" />
                )}
              </button>
            </IconTip>
          ) : null}
        </div>
        {selected ? (
          <CodingProposalDetail
            proposal={proposal}
            actionPlan={actionPlan}
            promotionPlan={promotionPlan}
            previewing={previewing}
            applying={applying}
            previewingPromotion={previewingPromotion}
            promoting={promoting}
            importingDomainEvalCase={importingDomainEvalCase}
            updating={updating}
            onPreview={onPreview}
            onApply={onApply}
            onPreviewPromotion={onPreviewPromotion}
            onPromote={onPromote}
            onImportDomainEvalCase={onImportDomainEvalCase}
            onReject={onReject}
          />
        ) : null}
      </div>
    </IconTip>
  )
}

function isCodingTrendReport(value: unknown): value is CodingTrendReport {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    "overview" in value &&
    "eval" in value &&
    "repairLoop" in value &&
    "review" in value &&
    "verification" in value &&
    "retro" in value
  )
}

function codingTrendTopReviewCategory(report: CodingTrendReport): CodingMetricBucket | null {
  return [...report.review.byCategory].sort((a, b) => b.count - a.count)[0] ?? null
}

function CodingTrendSection({
  sessionId,
  incognito,
  turnActive,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
}) {
  const { t } = useTranslation()
  const {
    report,
    loading,
    generating,
    distilling,
    updatingProposalId,
    previewingProposalId,
    applyingProposalId,
    previewingPromotionId,
    promotingProposalId,
    importingDomainEvalCaseId,
    actionPlan,
    promotionPlan,
    error,
    refresh,
    generateProposals,
    distillProposals,
    updateProposalStatus,
    previewProposalAction,
    applyProposal,
    previewProposalPromotion,
    promoteProposal,
    importDomainEvalCase,
  } = useCodingTrendReport(sessionId, { incognito, turnActive })
  const [selectedProposalId, setSelectedProposalId] = useState<string | null>(null)
  const trendReport = isCodingTrendReport(report) ? report : null
  const failures = trendReport?.failures ?? []
  const visibleFailures = failures.slice(0, 4)
  const proposals = trendReport?.proposals ?? []
  const visibleProposals = proposals.slice(0, 4)
  const draftCount = proposals.filter((proposal) => proposal.status === "draft").length
  const promotionCount = proposals.filter(
    (proposal) => proposal.status === "applied" || proposal.status === "promotion_failed",
  ).length
  const visibleRetros = (trendReport?.retros ?? []).slice(0, 3)
  const topCategory = trendReport ? codingTrendTopReviewCategory(trendReport) : null
  const disabled = !sessionId || incognito || loading || generating || distilling

  const meta =
    loading && !trendReport ? (
      <StatusPill label={t("workspace.codingTrend.loading", "读取中")} tone="info" loading />
    ) : error ? (
      <StatusPill label={t("workspace.codingTrend.failed", "失败")} tone="danger" />
    ) : failures.some((failure) => failure.severity === "high") ? (
      <StatusPill
        label={t("workspace.codingTrend.blockers", "{{count}} 阻塞", { count: failures.length })}
        tone="danger"
      />
    ) : draftCount > 0 ? (
      <StatusPill
        label={t("workspace.codingTrend.proposals", "{{count}} 草案", { count: draftCount })}
        tone="info"
      />
    ) : promotionCount > 0 ? (
      <StatusPill
        label={t("workspace.codingTrend.promotable", "{{count}} 待晋升", { count: promotionCount })}
        tone="info"
      />
    ) : trendReport ? (
      <StatusPill label={t("workspace.codingTrend.ready", "已汇总")} tone="good" />
    ) : (
      <StatusPill label={t("workspace.codingTrend.idle", "待汇总")} tone="muted" />
    )

  const handleGenerate = async () => {
    const result = await generateProposals()
    if (result) {
      toast.success(
        result.inserted > 0
          ? t("workspace.codingTrend.generated", "已生成 {{count}} 个候选", {
              count: result.inserted,
            })
          : t("workspace.codingTrend.generatedNoop", "候选已是最新"),
      )
    }
  }

  const handleDistill = async () => {
    const result = await distillProposals()
    if (result) {
      toast.success(
        result.inserted > 0
          ? t("workspace.codingTrend.distilled", "已提炼 {{count}} 个候选", {
              count: result.inserted,
            })
          : t("workspace.codingTrend.distilledNoop", "提炼候选已是最新"),
      )
    }
  }

  const handleToggleProposal = async (proposalId: string) => {
    setSelectedProposalId((current) => (current === proposalId ? null : proposalId))
    if (actionPlan?.proposal.id !== proposalId) {
      await previewProposalAction(proposalId)
    }
  }

  const handlePreviewProposal = async (proposalId: string) => {
    setSelectedProposalId(proposalId)
    const plan = await previewProposalAction(proposalId)
    if (plan) {
      toast.success(t("workspace.codingTrend.previewReady", "预览已生成"))
    }
  }

  const handleApplyProposal = async (proposalId: string) => {
    setSelectedProposalId(proposalId)
    if (actionPlan?.proposal.id !== proposalId) {
      const plan = await previewProposalAction(proposalId)
      if (!plan) return
    }
    const result = await applyProposal(proposalId)
    if (result?.applied) {
      toast.success(
        t("workspace.codingTrend.appliedToast", "已应用 {{count}} 个草稿产物", {
          count: result.artifacts.length,
        }),
      )
    } else if (result?.error) {
      toast.error(result.error)
    }
  }

  const handlePreviewPromotion = async (proposalId: string) => {
    setSelectedProposalId(proposalId)
    const plan = await previewProposalPromotion(proposalId)
    if (plan) {
      toast.success(t("workspace.codingTrend.promotionPreviewReady", "晋升预览已生成"))
    }
  }

  const handlePromoteProposal = async (proposalId: string) => {
    setSelectedProposalId(proposalId)
    if (promotionPlan?.proposal.id !== proposalId) {
      const plan = await previewProposalPromotion(proposalId)
      if (!plan) return
    }
    const result = await promoteProposal(proposalId)
    if (result?.promoted) {
      toast.success(
        t("workspace.codingTrend.promotedToast", "已晋升 {{count}} 个正式产物", {
          count: result.artifacts.length,
        }),
      )
    } else if (result?.error) {
      toast.error(result.error)
    }
  }

  const handleImportDomainEvalCase = async (proposalId: string) => {
    setSelectedProposalId(proposalId)
    const result = await importDomainEvalCase(proposalId)
    if (result?.imported) {
      toast.success(
        t("workspace.codingTrend.importedDomainEval", "已导入领域评测：{{title}}", {
          title: result.task.title,
        }),
      )
    } else if (result) {
      toast.success(
        t("workspace.codingTrend.domainEvalAlreadyImported", "领域评测已在任务库中"),
      )
    }
  }

  const handleRejectProposal = async (proposalId: string) => {
    const proposal = await updateProposalStatus(proposalId, "rejected")
    if (proposal) {
      toast.success(
        t("workspace.codingTrend.statusUpdated", "候选已更新为 {{status}}", {
          status: codingProposalStatusLabel(t, proposal.status),
        }),
      )
    }
  }

  return (
    <WorkspaceSection
      title={t("workspace.codingTrend.title", "质量趋势")}
      count={failures.length + draftCount + promotionCount + visibleRetros.length}
      icon={BarChart3}
      meta={meta}
      defaultExpanded={failures.length > 0 || draftCount > 0 || promotionCount > 0 || !!error}
    >
      <div className="space-y-2">
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={handleGenerate}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {generating ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Sparkles className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.codingTrend.generate", "生成改进候选")}</span>
          </button>
          <button
            type="button"
            onClick={handleDistill}
            disabled={disabled}
            className="inline-flex min-w-0 flex-1 items-center justify-center gap-1.5 rounded-md border border-border/60 bg-secondary/35 px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {distilling ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Brain className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.codingTrend.distill", "提炼候选")}</span>
          </button>
          <IconTip label={t("workspace.codingTrend.refresh", "刷新质量趋势")}>
            <button
              type="button"
              onClick={refresh}
              disabled={loading || generating || distilling}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
              aria-label={t("workspace.codingTrend.refresh", "刷新质量趋势")}
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RefreshCw className="h-3.5 w-3.5" />
              )}
            </button>
          </IconTip>
        </div>

        {trendReport ? (
          <div className="grid grid-cols-4 gap-1.5">
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricGoal", "Goal")}
              value={trendPercent(trendReport.overview.goalCompletionRate)}
              tone={codingTrendMetricTone(trendReport.overview.goalCompletionRate)}
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricWorkflow", "Workflow")}
              value={trendPercent(trendReport.overview.workflowCompletionRate)}
              tone={codingTrendMetricTone(trendReport.overview.workflowCompletionRate)}
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricEval", "Eval")}
              value={trendPercent(trendReport.eval.successRate)}
              tone={codingTrendMetricTone(trendReport.eval.successRate)}
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricRepair", "Repair")}
              value={trendPercent(trendReport.repairLoop.successRate)}
              tone={codingTrendMetricTone(trendReport.repairLoop.successRate)}
            />
          </div>
        ) : null}

        {trendReport ? (
          <div className="grid grid-cols-4 gap-1.5">
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricReview", "Review")}
              value={trendReport.review.blockingFindings}
              tone={trendReport.review.blockingFindings > 0 ? "danger" : "good"}
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricVerify", "Verify")}
              value={trendReport.verification.failedSteps + trendReport.verification.timedOutSteps}
              tone={
                trendReport.verification.failedSteps + trendReport.verification.timedOutSteps > 0
                  ? "danger"
                  : "good"
              }
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricFailures", "Blockers")}
              value={failures.length}
              tone={failures.length > 0 ? "warn" : "good"}
            />
            <CodingTrendMetric
              label={t("workspace.codingTrend.metricLearning", "Learning")}
              value={trendReport.retro.recommendations}
              tone={trendReport.retro.recommendations > 0 ? "info" : "muted"}
            />
          </div>
        ) : null}

        {incognito ? (
          <EmptyHint>{t("workspace.codingTrend.incognito", "无痕会话不持久化质量趋势")}</EmptyHint>
        ) : error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : !trendReport ? (
          <EmptyHint>{t("workspace.codingTrend.empty", "还没有质量趋势记录")}</EmptyHint>
        ) : (
          <div className="rounded-md border border-border/50 bg-secondary/25 px-2.5 py-2">
            <div className="flex min-w-0 items-center gap-2">
              <BarChart3 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
                {trendReport.scope === "project"
                  ? t("workspace.codingTrend.projectScope", "项目近 {{days}} 天", {
                      days: trendReport.windowDays,
                    })
                  : t("workspace.codingTrend.sessionScope", "会话近 {{days}} 天", {
                      days: trendReport.windowDays,
                    })}
              </span>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {formatMessageTime(trendReport.generatedAt)}
              </span>
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap gap-1 pl-5">
              <StatusPill
                label={t("workspace.codingTrend.sessions", "{{count}} 会话", {
                  count: trendReport.overview.sessions,
                })}
                tone="muted"
              />
              <StatusPill
                label={t("workspace.codingTrend.workflows", "{{count}} runs", {
                  count: trendReport.overview.workflowRuns,
                })}
                tone="muted"
              />
              <StatusPill
                label={t("workspace.codingTrend.retros", "{{count}} retros", {
                  count: trendReport.retro.total,
                })}
                tone={trendReport.retro.recommendations > 0 ? "info" : "muted"}
              />
              {topCategory ? <StatusPill label={topCategory.label} tone="warn" /> : null}
            </div>
          </div>
        )}

        {visibleRetros.length > 0 ? (
          <div className="space-y-1">
            {visibleRetros.map((retro) => (
              <CodingRetroRow key={retro.id} retro={retro} />
            ))}
          </div>
        ) : null}

        {visibleFailures.length > 0 ? (
          <div className="space-y-1">
            {visibleFailures.map((failure) => (
              <CodingFailureRow key={failure.category} failure={failure} />
            ))}
          </div>
        ) : null}

        {visibleProposals.length > 0 ? (
          <div className="space-y-1">
            {visibleProposals.map((proposal) => (
              <CodingProposalRow
                key={proposal.id}
                proposal={proposal}
                selected={selectedProposalId === proposal.id}
                actionPlan={actionPlan}
                promotionPlan={promotionPlan}
                previewing={previewingProposalId === proposal.id}
                applying={applyingProposalId === proposal.id}
                previewingPromotion={previewingPromotionId === proposal.id}
                promoting={promotingProposalId === proposal.id}
                importingDomainEvalCase={importingDomainEvalCaseId === proposal.id}
                updating={updatingProposalId === proposal.id}
                onToggle={handleToggleProposal}
                onPreview={handlePreviewProposal}
                onApply={handleApplyProposal}
                onPreviewPromotion={handlePreviewPromotion}
                onPromote={handlePromoteProposal}
                onImportDomainEvalCase={handleImportDomainEvalCase}
                onReject={handleRejectProposal}
              />
            ))}
            {proposals.length > visibleProposals.length ? (
              <div className="px-2 pt-0.5 text-center text-[11px] text-muted-foreground/60">
                {t("workspace.codingTrend.moreProposals", "还有 {{count}} 个候选", {
                  count: proposals.length - visibleProposals.length,
                })}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </WorkspaceSection>
  )
}

/**
 * R4 工作台区块:复用独立「后台任务」面板的本会话任务行能力
 * (展开 / 实时输出 / 取消 / 子会话入口),但保留工作台内的紧凑展示上限。
 */
const WORKSPACE_JOBS_PREVIEW = 6

function BackgroundJobsSection({
  jobs,
  jobExpansionOverrides,
  onJobExpandedChange,
  onOpenPanel,
  onViewSubagentSession,
}: {
  jobs: BackgroundJobSnapshot[]
  jobExpansionOverrides?: Record<string, boolean>
  onJobExpandedChange?: (jobId: string, expanded: boolean) => void
  onOpenPanel?: () => void
  onViewSubagentSession?: (sessionId: string) => void
}) {
  const { t } = useTranslation()
  const activeCount = jobs.filter(isBackgroundJobActive).length

  return (
    <WorkspaceSection
      title={t("backgroundJobs.panelTitle", "后台任务")}
      count={activeCount}
      icon={Layers}
    >
      {jobs.length > 0 ? (
        <div className="space-y-1">
          <SessionBackgroundJobsList
            jobs={jobs}
            jobExpansionOverrides={jobExpansionOverrides}
            onJobExpandedChange={onJobExpandedChange}
            onViewSubagentSession={onViewSubagentSession}
            limit={WORKSPACE_JOBS_PREVIEW}
          />
          {onOpenPanel && (
            <button
              type="button"
              onClick={onOpenPanel}
              className="w-full rounded-md px-2 py-1 text-center text-[11px] text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground"
            >
              {t("backgroundJobs.openFull", "查看全部")}
            </button>
          )}
        </div>
      ) : (
        <EmptyHint>{t("backgroundJobs.empty", "暂无后台任务")}</EmptyHint>
      )}
    </WorkspaceSection>
  )
}

const WORKFLOW_RUN_PREVIEW = 6
const WORKFLOW_EVENT_PREVIEW = 4
const WORKFLOW_OVERVIEW_EVENT_PREVIEW = 5
const WORKFLOW_OP_PREVIEW = 6
const WORKFLOW_FOCUS_OP_PREVIEW = 4
const DOMAIN_LEARNING_WINDOW_DAYS = 30
const CODING_IMPROVEMENT_CHANGED_EVENT = "hope-agent:coding-improvement-changed"
const GOAL_DOMAIN_FREE_VALUE = "__free_goal_domain__"

function domainTemplateOptionValue(template: Pick<DomainWorkflowTemplate, "id" | "version">) {
  return `${template.id}@${template.version}`
}

function findDomainTemplateByValue(
  templates: DomainWorkflowTemplate[],
  value: string | null | undefined,
) {
  if (!value || value === GOAL_DOMAIN_FREE_VALUE) return null
  return (
    templates.find((template) => domainTemplateOptionValue(template) === value) ??
    templates.find((template) => template.id === value) ??
    null
  )
}

function goalDomainTemplateValue(
  goal: Pick<Goal, "workflowTemplateId" | "workflowTemplateVersion">,
) {
  if (!goal.workflowTemplateId) return GOAL_DOMAIN_FREE_VALUE
  return goal.workflowTemplateVersion
    ? `${goal.workflowTemplateId}@${goal.workflowTemplateVersion}`
    : goal.workflowTemplateId
}

type ExecutionMode = "off" | "guarded" | "deep" | "autonomous"
type WorkflowAutonomyMode = "off" | "on" | "ultracode"
type WorkflowDetailTab = "trace" | "validation" | "agents"

const WORKFLOW_MODE_CHANGED_EVENT = "hope-agent:workflow-mode-changed"
const WORKFLOW_KIND_DEFAULT = "general.workflow"
const WORKFLOW_SCRIPT_TEMPLATE = `export default async function main(workflow) {
  const observe = await workflow.task.create({
    title: "Understand target",
    label: "observe",
  });

  await workflow.trace({
    label: "observe",
    payload: { summary: "Manual workflow started" },
  });
  await workflow.task.update({ task: observe, status: "completed" });

  await workflow.finish({
    summary: "Manual workflow completed",
    evidence: [],
    verification: [],
    residualRisk: [],
  });
}
`

function workflowJsLiteral(value: string): string {
  return JSON.stringify(value.trim()).replace(/</g, "\\u003C")
}

function buildGoalDrivenWorkflowScript(objective: string): string {
  const target = objective.trim() || "Complete the requested task."
  const targetLiteral = workflowJsLiteral(target)
  const implementationTask = workflowJsLiteral(`Complete this target end-to-end:

${target}

Work in the current session context. First inspect the relevant files, notes, sources, or artifacts that are available. If this is a coding task, follow the repository AGENTS.md instructions, make the smallest coherent change, and run targeted validation only; do not run the full pre-push suite unless explicitly requested. If this is not a coding task, produce concrete evidence, checks performed, deliverables, and residual risk.`)

  return `export default async function main(workflow) {
  // Budget: owner create request sets maxScriptSecs/maxOps/maxOutputTokens by execution mode.
  const observe = await workflow.task.create({
    title: "Understand target",
    label: "observe",
  });

  await workflow.trace({
    label: "target",
    payload: {
      target: ${targetLiteral},
      source: "goal-driven-workflow",
    },
  });
  const files = await workflow.fileSearch({
    query: ${targetLiteral},
    limit: 12,
    label: "target-files",
  });
  await workflow.task.update({ task: observe, status: "completed" });

  const implement = await workflow.task.create({
    title: "Complete target",
    label: "implement",
  });
  const worker = await workflow.spawnAgent({
    task: ${implementationTask},
    label: "complete-target",
  });
  const result = await workflow.waitAll([worker], {
    timeout: 120,
    label: "wait-target",
  });
  await workflow.task.update({ task: implement, status: "completed" });

  const diff = await workflow.diff({ label: "final-diff" });

  await workflow.finish({
    summary: "Goal-driven workflow finished",
    target: ${targetLiteral},
    searchedFiles: files.matches ?? [],
    result,
    diff,
    verification: ["See worker result for checks performed"],
    residualRisk: [],
  });
}
`
}

function workflowBudgetForMode(mode: ExecutionMode): Record<string, number> {
  switch (mode) {
    case "autonomous":
      return { maxScriptSecs: 300, maxOps: 32, maxOutputTokens: 24000 }
    case "deep":
      return { maxScriptSecs: 240, maxOps: 28, maxOutputTokens: 16000 }
    case "guarded":
      return { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 }
    case "off":
      return { maxScriptSecs: 90, maxOps: 16, maxOutputTokens: 6000 }
  }
}

const WORKFLOW_MODE_OPTIONS: Array<{ mode: ExecutionMode; icon: LucideIcon }> = [
  { mode: "off", icon: X },
  { mode: "guarded", icon: Shield },
  { mode: "deep", icon: Brain },
  { mode: "autonomous", icon: Bot },
]

type WorkflowRunCommand =
  | "run_workflow_run"
  | "approve_workflow_run"
  | "pause_workflow_run"
  | "resume_workflow_run"
  | "cancel_workflow_run"

interface WorkflowRunActionSpec {
  command: WorkflowRunCommand
  label: string
  success: string
  icon: LucideIcon
  danger?: boolean
  primary?: boolean
}

interface WorkflowDraftOrigin {
  type: "repair"
  runId: string
  runKind: string
  runState: WorkflowRunState
}

function workflowRunStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: WorkflowRunState,
): string {
  switch (state) {
    case "draft":
      return t("workspace.workflow.stateDraft", "待启动")
    case "awaiting_approval":
      return t("workspace.workflow.stateAwaitingApproval", "待批准")
    case "running":
      return t("workspace.workflow.stateRunning", "运行中")
    case "awaiting_user":
      return t("workspace.workflow.stateAwaitingUser", "待用户")
    case "paused":
      return t("workspace.workflow.statePaused", "已暂停")
    case "recovering":
      return t("workspace.workflow.stateRecovering", "恢复中")
    case "completed":
      return t("workspace.workflow.stateCompleted", "已完成")
    case "failed":
      return t("workspace.workflow.stateFailed", "失败")
    case "cancelled":
      return t("workspace.workflow.stateCancelled", "已取消")
    case "blocked":
      return t("workspace.workflow.stateBlocked", "已阻塞")
  }
}

function workflowRunTone(state: WorkflowRunState): StatusTone {
  switch (state) {
    case "completed":
      return "good"
    case "awaiting_approval":
    case "awaiting_user":
    case "paused":
      return "warn"
    case "failed":
    case "blocked":
      return "danger"
    case "running":
    case "recovering":
      return "info"
    case "draft":
    case "cancelled":
      return "muted"
  }
}

function workflowRunIsLive(state: WorkflowRunState): boolean {
  return (
    state === "running" || state === "awaiting_user" || state === "paused" || state === "recovering"
  )
}

function normalizeExecutionMode(value: unknown): ExecutionMode {
  const raw = typeof value === "string" ? value : stringField(asRecord(value), "mode")
  return raw === "guarded" || raw === "deep" || raw === "autonomous" ? raw : "off"
}

function normalizeWorkflowAutonomyMode(value: unknown): WorkflowAutonomyMode {
  const raw = typeof value === "string" ? value : stringField(asRecord(value), "mode")
  return raw === "on" || raw === "ultracode" ? raw : "off"
}

function compactJson(value: unknown, fallback: string): string {
  if (value == null) return fallback
  if (typeof value === "string") return value
  if (typeof value === "number" || typeof value === "boolean") return String(value)
  try {
    return JSON.stringify(value)
  } catch {
    return fallback
  }
}

function prettyJson(value: unknown, fallback: string): string {
  if (value == null) return fallback
  if (typeof value === "string") return value
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return fallback
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function stringField(
  record: Record<string, unknown> | null | undefined,
  key: string,
): string | null {
  const value = record?.[key]
  return typeof value === "string" && value.length > 0 ? value : null
}

function numberField(
  record: Record<string, unknown> | null | undefined,
  key: string,
): number | null {
  const value = record?.[key]
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function compactCount(value: number): string {
  if (!Number.isFinite(value)) return "0"
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
  if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}k`
  return String(Math.round(value))
}

function workflowOutputBudget(
  run: WorkflowRun,
  events: WorkflowEvent[] = [],
): { spent: number; limit: number; exhausted: boolean } | null {
  const budget = asRecord(run.budget)
  const limit = numberField(budget, "maxOutputTokens") ?? numberField(budget, "max_output_tokens")
  const latestBudgetPayload = events
    .filter((event) => event.eventType === "budget_usage")
    .map((event) => asRecord(event.payload))
    .filter((payload): payload is Record<string, unknown> => Boolean(payload))
    .at(-1)
  const spent = numberField(latestBudgetPayload, "spentOutputTokens") ?? 0
  const exhausted = boolField(latestBudgetPayload, "exhausted") ?? false
  return typeof limit === "number" && limit > 0 ? { spent, limit, exhausted } : null
}

function boolField(
  record: Record<string, unknown> | null | undefined,
  key: string,
): boolean | null {
  const value = record?.[key]
  return typeof value === "boolean" ? value : null
}

function arrayField(record: Record<string, unknown> | null | undefined, key: string): unknown[] {
  const value = record?.[key]
  return Array.isArray(value) ? value : []
}

function recordArrayField(
  record: Record<string, unknown> | null | undefined,
  key: string,
): Record<string, unknown>[] {
  return arrayField(record, key)
    .map(asRecord)
    .filter((item): item is Record<string, unknown> => !!item)
}

function truncateMiddle(value: string, max = 96): string {
  if (value.length <= max) return value
  const head = Math.max(8, Math.floor((max - 1) * 0.58))
  const tail = Math.max(8, max - head - 1)
  return `${value.slice(0, head)}…${value.slice(-tail)}`
}

function workflowOpSummary(t: ReturnType<typeof useTranslation>["t"], ops: WorkflowOp[]): string {
  if (ops.length === 0) return t("workspace.workflow.noOps", "暂无 op")
  const completed = ops.filter((op) => op.state === "completed").length
  const failed = ops.filter((op) => op.state === "failed").length
  if (failed > 0) {
    return t("workspace.workflow.opSummaryFailed", "{{completed}}/{{total}} · {{failed}} 失败", {
      completed,
      total: ops.length,
      failed,
    })
  }
  return t("workspace.workflow.opSummary", "{{completed}}/{{total}} 完成", {
    completed,
    total: ops.length,
  })
}

function executionModeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  mode: ExecutionMode,
): string {
  switch (mode) {
    case "off":
      return t("workspace.workflow.executionOff", "关闭")
    case "guarded":
      return t("workspace.workflow.executionGuarded", "守护")
    case "deep":
      return t("workspace.workflow.executionDeep", "深入")
    case "autonomous":
      return t("workspace.workflow.executionAutonomous", "自主")
  }
}

function executionModeHint(t: ReturnType<typeof useTranslation>["t"], mode: ExecutionMode): string {
  switch (mode) {
    case "off":
      return t("workspace.workflow.executionOffHint", "普通对话")
    case "guarded":
      return t("workspace.workflow.executionGuardedHint", "编辑后验证")
    case "deep":
      return t("workspace.workflow.executionDeepHint", "更强排查")
    case "autonomous":
      return t("workspace.workflow.executionAutonomousHint", "长任务持续")
  }
}

function workflowAutonomyModeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  mode: WorkflowAutonomyMode,
): string {
  switch (mode) {
    case "off":
      return t("workspace.workflow.modeOff", "关闭")
    case "on":
      return t("workspace.workflow.modeOn", "开启")
    case "ultracode":
      return t("workspace.workflow.modeUltracode", "Ultracode")
  }
}

function workflowAutonomyModeHint(
  t: ReturnType<typeof useTranslation>["t"],
  mode: WorkflowAutonomyMode,
): string {
  switch (mode) {
    case "off":
      return t("workspace.workflow.modeOffHint", "不开放自主编排")
    case "on":
      return t("workspace.workflow.modeOnHint", "模型按需编排")
    case "ultracode":
      return t("workspace.workflow.modeUltracodeHint", "全面验证长任务")
  }
}

function workflowRunHasHardProblem(run: WorkflowRun): boolean {
  return run.state === "failed" || run.state === "blocked"
}

function loopScheduleNeedsAttention(schedule: LoopSchedule): boolean {
  return schedule.state === "blocked"
}

function activeLoopSchedules(schedules: LoopSchedule[]): LoopSchedule[] {
  return schedules.filter(
    (schedule) =>
      schedule.state === "active" || schedule.state === "paused" || schedule.state === "blocked",
  )
}

function readinessStatusTone(args: {
  incognito?: boolean
  activeGoal: Goal | null
  workflowMode: WorkflowAutonomyMode
  executionMode: ExecutionMode
  workflowProblems: number
  loopProblems: number
  operationalStatus?: string | null
  soakStatus?: string | null
}): StatusTone {
  if (args.incognito) return "muted"
  if (
    args.workflowProblems > 0 ||
    args.loopProblems > 0 ||
    args.operationalStatus === "failed" ||
    args.soakStatus === "failed"
  ) {
    return "danger"
  }
  if (!args.activeGoal || args.workflowMode === "off" || args.executionMode === "off") {
    return "warn"
  }
  if (args.operationalStatus === "insufficient_data" || args.soakStatus === "insufficient_data") {
    return "info"
  }
  return "good"
}

function readinessStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  tone: StatusTone,
  loading: boolean,
): string {
  if (loading) return t("workspace.autonomousReadiness.loading", "同步中")
  if (tone === "danger") return t("workspace.autonomousReadiness.blocked", "需处理")
  if (tone === "warn") return t("workspace.autonomousReadiness.needsSetup", "待配置")
  if (tone === "info") return t("workspace.autonomousReadiness.observing", "观察中")
  if (tone === "good") return t("workspace.autonomousReadiness.ready", "自主就绪")
  return t("workspace.autonomousReadiness.idle", "未开始")
}

function readinessMetricTone(ready: boolean, warning = false): StatusTone {
  if (warning) return "warn"
  return ready ? "good" : "muted"
}

function autonomousReadinessNextSteps(
  t: ReturnType<typeof useTranslation>["t"],
  args: {
    incognito?: boolean
    activeGoal: Goal | null
    workflowMode: WorkflowAutonomyMode
    executionMode: ExecutionMode
    workflowProblems: number
    loopProblems: number
    workflowLoopCount: number
    operationalGate: DomainOperationalGateReport | null
    soakReport: DomainSoakReport | null
  },
): string[] {
  if (args.incognito) {
    return [t("workspace.autonomousReadiness.nextIncognito", "无痕会话不持久化 Goal、Loop 或 Workflow。")]
  }
  const steps: string[] = []
  if (!args.activeGoal) {
    steps.push(t("workspace.autonomousReadiness.nextGoal", "先创建目标，明确最终结果和完成标准。"))
  }
  if (args.workflowMode === "off") {
    steps.push(t("workspace.autonomousReadiness.nextWorkflowMode", "开启工作流模式，让模型可按需动态编排。"))
  }
  if (args.executionMode === "off") {
    steps.push(t("workspace.autonomousReadiness.nextExecutionMode", "选择守护、深入或自主执行模式。"))
  }
  if (args.workflowProblems > 0 || args.loopProblems > 0) {
    steps.push(t("workspace.autonomousReadiness.nextProblems", "处理失败或阻塞的 Workflow / Loop。"))
  }
  if (args.activeGoal?.workflowTemplateId && args.workflowLoopCount === 0) {
    steps.push(t("workspace.autonomousReadiness.nextWorkflowLoop", "可创建工作流 Loop，让目标按周期持续推进。"))
  } else if (args.activeGoal && !args.activeGoal.workflowTemplateId) {
    steps.push(t("workspace.autonomousReadiness.nextTemplate", "为目标绑定领域模板后，可启用工作流 Loop。"))
  }
  if (args.operationalGate?.status && args.operationalGate.status !== "passed") {
    steps.push(
      args.operationalGate.recommendedNextSteps[0] ??
        t("workspace.autonomousReadiness.nextOperational", "等待运行排空或补齐运行样本。"),
    )
  }
  if (args.soakReport?.status && args.soakReport.status !== "passed") {
    steps.push(
      args.soakReport.recommendedNextSteps[0] ??
        t("workspace.autonomousReadiness.nextSoak", "查看长跑审计里的事故与待排空项。"),
    )
  }
  return steps.length > 0
    ? steps.slice(0, 4)
    : [t("workspace.autonomousReadiness.nextReady", "自主推进基础已就绪；运行中仍可随时暂停、恢复或取消。")]
}

type AutonomousReadinessQuickAction = {
  key: string
  label: string
  icon: LucideIcon
  disabled?: boolean
  loading?: boolean
  onClick: () => void
}

function AutonomousReadinessCard({
  activeGoal,
  workflowMode,
  workflowModeLoading,
  workflowModeSaving,
  executionMode,
  executionModeLoading,
  executionModeSaving,
  workflowRunsState,
  loopSchedulesState,
  domainWorkbenchState,
  incognito,
  canCreateLoop,
  onChangeWorkflowMode,
  onChangeExecutionMode,
  onOpenGoalCreate,
  onOpenGoalEdit,
  onOpenLoopCreate,
  onOpenWorkflowProblem,
  onOpenLoopProblem,
  onOpenOperationalGate,
  onOpenSoakReport,
}: {
  activeGoal: Goal | null
  workflowMode: WorkflowAutonomyMode
  workflowModeLoading?: boolean
  workflowModeSaving?: WorkflowAutonomyMode | null
  executionMode: ExecutionMode
  executionModeLoading?: boolean
  executionModeSaving?: ExecutionMode | null
  workflowRunsState: WorkflowRunsState
  loopSchedulesState: LoopSchedulesState
  domainWorkbenchState: DomainTaskWorkbenchState
  incognito?: boolean
  canCreateLoop?: boolean
  onChangeWorkflowMode?: (mode: WorkflowAutonomyMode) => void
  onChangeExecutionMode?: (mode: ExecutionMode) => void
  onOpenGoalCreate?: () => void
  onOpenGoalEdit?: () => void
  onOpenLoopCreate?: () => void
  onOpenWorkflowProblem?: (runId: string) => void
  onOpenLoopProblem?: (loopId: string) => void
  onOpenOperationalGate?: () => void
  onOpenSoakReport?: () => void
}) {
  const { t } = useTranslation()
  const problemWorkflow = workflowRunsState.runs.find(workflowRunHasHardProblem) ?? null
  const problemLoops = loopSchedulesState.schedules.filter(loopScheduleNeedsAttention)
  const problemLoop = problemLoops[0] ?? null
  const workflowProblems = workflowRunsState.runs.filter(workflowRunHasHardProblem).length
  const activeWorkflows = workflowRunsState.activeCount
  const liveLoops = activeLoopSchedules(loopSchedulesState.schedules)
  const workflowLoopCount = loopSchedulesState.schedules.filter(
    (schedule) => schedule.executionStrategy === "workflow",
  ).length
  const loopProblems = problemLoops.length
  const loading =
    workflowModeLoading ||
    executionModeLoading ||
    workflowRunsState.loading ||
    loopSchedulesState.loading ||
    domainWorkbenchState.operationalGateLoading ||
    domainWorkbenchState.soakReportLoading
  const tone = readinessStatusTone({
    incognito,
    activeGoal,
    workflowMode,
    executionMode,
    workflowProblems,
    loopProblems,
    operationalStatus: domainWorkbenchState.operationalGate?.status,
    soakStatus: domainWorkbenchState.soakReport?.status,
  })
  const nextSteps = autonomousReadinessNextSteps(t, {
    incognito,
    activeGoal,
    workflowMode,
    executionMode,
    workflowProblems,
    loopProblems,
    workflowLoopCount,
    operationalGate: domainWorkbenchState.operationalGate,
    soakReport: domainWorkbenchState.soakReport,
  })
  const goalReady = Boolean(activeGoal)
  const workflowReady = workflowMode !== "off"
  const executionReady = executionMode !== "off"
  const runHealthReady = workflowProblems === 0 && loopProblems === 0
  const runHealthWarning =
    domainWorkbenchState.operationalGate?.status === "failed" ||
    domainWorkbenchState.soakReport?.status === "failed"
  const quickActions: AutonomousReadinessQuickAction[] = []
  if (!incognito && problemWorkflow && onOpenWorkflowProblem) {
    quickActions.push({
      key: "workflow-problem",
      label: t("workspace.autonomousReadiness.actionViewWorkflow", "查看工作流"),
      icon: Eye,
      onClick: () => onOpenWorkflowProblem(problemWorkflow.id),
    })
  }
  if (!incognito && problemLoop && onOpenLoopProblem) {
    quickActions.push({
      key: "loop-problem",
      label: t("workspace.autonomousReadiness.actionViewLoop", "查看 Loop"),
      icon: Radio,
      onClick: () => onOpenLoopProblem(problemLoop.id),
    })
  }
  if (
    !incognito &&
    domainWorkbenchState.operationalGate?.status &&
    domainWorkbenchState.operationalGate.status !== "passed" &&
    onOpenOperationalGate
  ) {
    quickActions.push({
      key: "operational-gate",
      label: t("workspace.autonomousReadiness.actionViewOperational", "查看稳定性"),
      icon: Gauge,
      onClick: onOpenOperationalGate,
    })
  }
  if (
    !incognito &&
    domainWorkbenchState.soakReport?.status &&
    domainWorkbenchState.soakReport.status !== "passed" &&
    onOpenSoakReport
  ) {
    quickActions.push({
      key: "soak-report",
      label: t("workspace.autonomousReadiness.actionViewSoak", "查看长跑"),
      icon: Clock,
      onClick: onOpenSoakReport,
    })
  }
  if (!incognito && !activeGoal && onOpenGoalCreate) {
    quickActions.push({
      key: "goal",
      label: t("workspace.autonomousReadiness.actionCreateGoal", "创建目标"),
      icon: Sparkles,
      onClick: onOpenGoalCreate,
    })
  }
  if (!incognito && activeGoal && !activeGoal.workflowTemplateId && onOpenGoalEdit) {
    quickActions.push({
      key: "template",
      label: t("workspace.autonomousReadiness.actionBindTemplate", "绑定模板"),
      icon: ClipboardCheck,
      onClick: onOpenGoalEdit,
    })
  }
  if (!incognito && workflowMode === "off" && onChangeWorkflowMode) {
    quickActions.push({
      key: "workflow-mode",
      label: t("workspace.autonomousReadiness.actionEnableWorkflow", "开启编排"),
      icon: GitPullRequest,
      disabled: Boolean(workflowModeLoading || workflowModeSaving),
      loading: workflowModeSaving === "on",
      onClick: () => onChangeWorkflowMode("on"),
    })
  }
  if (!incognito && executionMode === "off" && onChangeExecutionMode) {
    quickActions.push({
      key: "execution-mode",
      label: t("workspace.autonomousReadiness.actionGuardedMode", "设为守护"),
      icon: Shield,
      disabled: Boolean(executionModeLoading || executionModeSaving),
      loading: executionModeSaving === "guarded",
      onClick: () => onChangeExecutionMode("guarded"),
    })
  }
  if (
    !incognito &&
    activeGoal?.workflowTemplateId &&
    workflowLoopCount === 0 &&
    onOpenLoopCreate
  ) {
    quickActions.push({
      key: "workflow-loop",
      label: t("workspace.autonomousReadiness.actionCreateLoop", "新建 Loop"),
      icon: Radio,
      disabled: !canCreateLoop,
      onClick: onOpenLoopCreate,
    })
  }

  return (
    <div className={cn("rounded-md border p-2", STATUS_TONE_CLASS[tone])}>
      <div className="flex min-w-0 items-center gap-2">
        <Bot className="h-3.5 w-3.5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium">
            {t("workspace.autonomousReadiness.title", "自主推进就绪")}
          </div>
          <div className="truncate text-[10px] opacity-75">
            {activeGoal?.objective ??
              t("workspace.autonomousReadiness.emptyGoal", "还没有目标")}
          </div>
        </div>
        <StatusPill label={readinessStatusLabel(t, tone, Boolean(loading))} tone={tone} loading={loading} />
      </div>

      <div className="mt-2 grid grid-cols-4 gap-1.5">
        <div
          className={cn(
            "rounded-md border px-2 py-1.5",
            STATUS_TONE_CLASS[readinessMetricTone(goalReady)],
          )}
        >
          <div className="truncate text-[10px]">{t("workspace.autonomousReadiness.goal", "目标")}</div>
          <div className="truncate text-xs font-semibold">
            {goalReady
              ? activeGoal?.workflowTemplateId
                ? t("workspace.autonomousReadiness.goalTemplate", "模板")
                : t("workspace.autonomousReadiness.goalSet", "已设置")
              : t("workspace.autonomousReadiness.goalMissing", "缺失")}
          </div>
        </div>
        <div
          className={cn(
            "rounded-md border px-2 py-1.5",
            STATUS_TONE_CLASS[readinessMetricTone(workflowReady)],
          )}
        >
          <div className="truncate text-[10px]">
            {t("workspace.autonomousReadiness.workflow", "编排")}
          </div>
          <div className="truncate text-xs font-semibold">
            {workflowAutonomyModeLabel(t, workflowMode)}
          </div>
        </div>
        <div
          className={cn(
            "rounded-md border px-2 py-1.5",
            STATUS_TONE_CLASS[readinessMetricTone(executionReady)],
          )}
        >
          <div className="truncate text-[10px]">
            {t("workspace.autonomousReadiness.execution", "强度")}
          </div>
          <div className="truncate text-xs font-semibold">
            {executionModeLabel(t, executionMode)}
          </div>
        </div>
        <div
          className={cn(
            "rounded-md border px-2 py-1.5",
            STATUS_TONE_CLASS[readinessMetricTone(runHealthReady, runHealthWarning)],
          )}
        >
          <div className="truncate text-[10px]">
            {t("workspace.autonomousReadiness.health", "运行")}
          </div>
          <div className="truncate text-xs font-semibold">
            {workflowProblems > 0 || loopProblems > 0
              ? t("workspace.autonomousReadiness.healthProblems", "{{count}} 个问题", {
                  count: workflowProblems + loopProblems,
                })
              : activeWorkflows > 0 || liveLoops.length > 0
                ? t("workspace.autonomousReadiness.healthActive", "{{count}} 活跃", {
                    count: activeWorkflows + liveLoops.length,
                  })
                : t("workspace.autonomousReadiness.healthClean", "干净")}
          </div>
        </div>
      </div>

      <div className="mt-2 space-y-1">
        {nextSteps.map((step, index) => (
          <div key={`${index}:${step}`} className="line-clamp-2 text-[11px] leading-snug opacity-85">
            {step}
          </div>
        ))}
      </div>

      {quickActions.length > 0 ? (
        <div className="mt-2 grid grid-cols-3 gap-1.5">
          {quickActions.slice(0, 3).map((action) => {
            const Icon = action.icon
            return (
              <Button
                key={action.key}
                type="button"
                variant="outline"
                size="sm"
                className="h-7 min-w-0 gap-1 px-2 text-[11px]"
                disabled={action.disabled}
                onClick={action.onClick}
              >
                {action.loading ? (
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                ) : (
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                )}
                <span className="truncate">{action.label}</span>
              </Button>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}

function workflowRunActionSpecs(
  t: ReturnType<typeof useTranslation>["t"],
  state: WorkflowRunState,
): WorkflowRunActionSpec[] {
  const cancel: WorkflowRunActionSpec = {
    command: "cancel_workflow_run",
    label: t("workspace.workflow.cancel", "取消"),
    success: t("workspace.workflow.cancelled", "已取消工作流"),
    icon: X,
    danger: true,
  }

  switch (state) {
    case "draft":
      return [
        {
          command: "run_workflow_run",
          label: t("workspace.workflow.run", "运行"),
          success: t("workspace.workflow.runStarted", "已请求启动工作流"),
          icon: Play,
          primary: true,
        },
        cancel,
      ]
    case "awaiting_approval":
      return [
        {
          command: "approve_workflow_run",
          label: t("workspace.workflow.approve", "批准"),
          success: t("workspace.workflow.approved", "已批准并请求继续工作流"),
          icon: Check,
          primary: true,
        },
        cancel,
      ]
    case "running":
      return [
        {
          command: "pause_workflow_run",
          label: t("workspace.workflow.pause", "暂停"),
          success: t("workspace.workflow.paused", "已暂停工作流"),
          icon: Pause,
        },
        cancel,
      ]
    case "paused":
      return [
        {
          command: "resume_workflow_run",
          label: t("workspace.workflow.resume", "恢复"),
          success: t("workspace.workflow.resumed", "已请求恢复工作流"),
          icon: Play,
          primary: true,
        },
        cancel,
      ]
    case "awaiting_user":
    case "recovering":
      return [cancel]
    case "completed":
    case "failed":
    case "cancelled":
    case "blocked":
      return []
  }
}

function workflowPermissionSummaryText(
  t: ReturnType<typeof useTranslation>["t"],
  summary: Record<string, unknown> | null | undefined,
): string {
  const parts: string[] = []
  const total = numberField(summary, "total")
  const allow = numberField(summary, "allow")
  const ask = numberField(summary, "ask")
  const dynamic = numberField(summary, "dynamic")
  const deny = numberField(summary, "deny")
  const strict = numberField(summary, "strict")
  if (typeof total === "number") {
    parts.push(t("workspace.workflow.permissionTotal", "{{count}} 个调用", { count: total }))
  }
  if (typeof ask === "number" && ask > 0) {
    parts.push(t("workspace.workflow.permissionAsk", "{{count}} 个需批准", { count: ask }))
  }
  if (typeof dynamic === "number" && dynamic > 0) {
    parts.push(
      t("workspace.workflow.permissionDynamic", "{{count}} 个动态调用", { count: dynamic }),
    )
  }
  if (typeof deny === "number" && deny > 0) {
    parts.push(t("workspace.workflow.permissionDeny", "{{count}} 个被拒绝", { count: deny }))
  }
  if (typeof strict === "number" && strict > 0) {
    parts.push(t("workspace.workflow.permissionStrict", "{{count}} 个 strict", { count: strict }))
  }
  if (parts.length === 0 && typeof allow === "number") {
    parts.push(t("workspace.workflow.permissionAllow", "{{count}} 个可自动执行", { count: allow }))
  }
  return parts.join(" · ")
}

function workflowPermissionPreview(snapshot: WorkflowRunSnapshot | null): {
  summary: Record<string, unknown>
  calls: Record<string, unknown>[]
  truncated: boolean
} | null {
  const events = snapshot?.events ?? []
  const permissionEvents = events.filter(
    (event) =>
      event.eventType === "script_permission_preview" ||
      event.eventType === "script_permission_preview_blocked" ||
      event.eventType === "script_permission_approval_required",
  )
  if (permissionEvents.length === 0) return null

  const latestPayload = asRecord(permissionEvents.at(-1)?.payload)
  const callsPayload =
    [...permissionEvents]
      .reverse()
      .map((event) => asRecord(event.payload))
      .find((payload) => recordArrayField(payload, "calls").length > 0) ?? latestPayload
  const summary = asRecord(latestPayload?.summary) ?? asRecord(callsPayload?.summary)
  const calls = recordArrayField(callsPayload, "calls")
  const truncated = boolField(callsPayload, "truncated") ?? false

  return summary || calls.length > 0 ? { summary: summary ?? {}, calls, truncated } : null
}

function workflowApprovalAuditEvents(snapshot: WorkflowRunSnapshot | null): WorkflowEvent[] {
  return (snapshot?.events ?? []).filter((event) => {
    if (
      event.eventType === "script_permission_preview" ||
      event.eventType === "script_permission_preview_blocked" ||
      event.eventType === "script_permission_approval_required"
    ) {
      return true
    }
    if (event.eventType !== "run_state_changed") return false
    const payload = asRecord(event.payload)
    const reason = stringField(payload, "reason")
    const from = stringField(payload, "from")
    const to = stringField(payload, "to")
    return (
      reason === "approval_granted" ||
      reason === "permission_preview" ||
      reason === "permission_preview_denied" ||
      (reason === "cancel_requested" && from === "awaiting_approval") ||
      to === "awaiting_approval"
    )
  })
}

function workflowApprovalAuditTitle(
  t: ReturnType<typeof useTranslation>["t"],
  event: WorkflowEvent,
): string {
  const payload = asRecord(event.payload)
  const reason = stringField(payload, "reason")
  switch (event.eventType) {
    case "script_permission_preview":
      return t("workspace.workflow.approvalAuditPreview", "权限预检")
    case "script_permission_preview_blocked":
      return t("workspace.workflow.approvalAuditBlocked", "预检阻塞")
    case "script_permission_approval_required":
      return t("workspace.workflow.approvalAuditRequired", "等待批准")
    case "run_state_changed":
      if (reason === "approval_granted") {
        return t("workspace.workflow.approvalAuditGranted", "已批准")
      }
      if (reason === "permission_preview") {
        return t("workspace.workflow.approvalAuditEntered", "进入待批准")
      }
      if (reason === "permission_preview_denied") {
        return t("workspace.workflow.approvalAuditDenied", "预检拒绝")
      }
      if (reason === "cancel_requested") {
        return t("workspace.workflow.approvalAuditCancelled", "审批已取消")
      }
      return workflowEventTitle(t, event)
    default:
      return workflowEventTitle(t, event)
  }
}

function workflowApprovalAuditTone(event: WorkflowEvent): StatusTone {
  const payload = asRecord(event.payload)
  const reason = stringField(payload, "reason")
  const to = stringField(payload, "to")
  if (
    event.eventType === "script_permission_preview_blocked" ||
    reason === "permission_preview_denied"
  ) {
    return "danger"
  }
  if (reason === "approval_granted") return "good"
  if (
    event.eventType === "script_permission_approval_required" ||
    reason === "permission_preview" ||
    to === "awaiting_approval"
  ) {
    return "warn"
  }
  if (reason === "cancel_requested") return "muted"
  return "info"
}

function workflowApprovalAuditStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  tone: StatusTone,
): string {
  switch (tone) {
    case "good":
      return t("workspace.workflow.approvalAuditStatusGranted", "已通过")
    case "warn":
      return t("workspace.workflow.approvalAuditStatusWaiting", "待处理")
    case "danger":
      return t("workspace.workflow.approvalAuditStatusBlocked", "已阻塞")
    case "info":
      return t("workspace.workflow.approvalAuditStatusRecorded", "已记录")
    case "muted":
      return t("workspace.workflow.approvalAuditStatusClosed", "已关闭")
  }
}

function workflowPermissionDecisionLabel(
  t: ReturnType<typeof useTranslation>["t"],
  call: Record<string, unknown>,
): string {
  const decision = stringField(call, "decision")
  if (boolField(call, "dynamic")) return t("workspace.workflow.permissionDecisionDynamic", "动态")
  if (boolField(call, "strict")) return t("workspace.workflow.permissionDecisionStrict", "Strict")
  switch (decision) {
    case "allow":
      return t("workspace.workflow.permissionDecisionAllow", "自动")
    case "ask":
      return t("workspace.workflow.permissionDecisionAsk", "需批准")
    case "deny":
      return t("workspace.workflow.permissionDecisionDeny", "拒绝")
    default:
      return decision ?? t("workspace.workflow.permissionDecisionUnknown", "未知")
  }
}

function workflowPermissionDecisionTone(call: Record<string, unknown>): StatusTone {
  const decision = stringField(call, "decision")
  if (decision === "deny") return "danger"
  if (boolField(call, "strict") || decision === "ask") return "warn"
  if (boolField(call, "dynamic")) return "info"
  if (decision === "allow") return "good"
  return "muted"
}

function workflowPermissionCallTitle(call: Record<string, unknown>): string {
  return (
    stringField(call, "label") ??
    stringField(call, "toolName") ??
    stringField(call, "tool_name") ??
    stringField(call, "api") ??
    "workflow"
  )
}

function workflowPermissionCallDetail(
  t: ReturnType<typeof useTranslation>["t"],
  call: Record<string, unknown>,
): string {
  const api = stringField(call, "api")
  const toolName = stringField(call, "toolName") ?? stringField(call, "tool_name")
  const line = numberField(call, "line")
  const reason = stringField(call, "reason")
  const detail = [
    typeof line === "number"
      ? t("workspace.workflow.permissionLine", "line {{line}}", { line })
      : null,
    api && toolName && api !== toolName ? `${api} · ${toolName}` : (api ?? toolName),
    reason,
  ].filter(Boolean)
  return detail.join(" · ")
}

function workflowOpHasValidationFailure(op: WorkflowOp): boolean {
  if (op.opType !== "validate") return false
  const output = asRecord(op.output)
  return op.state === "failed" || boolField(output, "ok") === false
}

function workflowOpNeedsAttention(op: WorkflowOp): boolean {
  return op.state === "failed" || op.state === "started" || workflowOpHasValidationFailure(op)
}

function workflowOpTone(op: WorkflowOp): StatusTone {
  if (workflowOpHasValidationFailure(op) || op.state === "failed") return "danger"
  if (op.state === "completed") return "good"
  if (op.state === "started" || op.state === "pending") return "info"
  return "muted"
}

function workflowOpTitle(op: WorkflowOp): string {
  const input = asRecord(op.input)
  const output = asRecord(op.output)
  return (
    stringField(input, "label") ??
    stringField(output, "label") ??
    stringField(input, "name") ??
    stringField(output, "name") ??
    op.opKey
  )
}

function workflowOpDetail(op: WorkflowOp): string {
  const input = asRecord(op.input)
  const output = asRecord(op.output)
  const error = asRecord(op.error)
  return (
    stringField(error, "message") ??
    stringField(output, "summary") ??
    stringField(output, "status") ??
    stringField(input, "query") ??
    op.opType
  )
}

function workflowOpDetailTab(op: WorkflowOp): WorkflowDetailTab {
  if (op.opType === "validate") return "validation"
  if (op.opType === "spawnAgent") return "agents"
  return "trace"
}

function workflowDetailTabLabel(
  t: ReturnType<typeof useTranslation>["t"],
  tab: WorkflowDetailTab,
): string {
  switch (tab) {
    case "trace":
      return t("workspace.workflow.tabTrace", "Trace")
    case "validation":
      return t("workspace.workflow.tabValidation", "Validation")
    case "agents":
      return t("workspace.workflow.tabAgents", "Agents")
  }
}

function workflowValidationResultLines(op: WorkflowOp): string[] {
  const output = asRecord(op.output)
  const results = recordArrayField(output, "results")
  return results.slice(0, 6).map((result) => {
    const command = stringField(result, "command") ?? "validation command"
    const ok = boolField(result, "ok")
    const exitCode = numberField(result, "exitCode")
    const cwd = stringField(result, "cwd")
    const resultOutput = stringField(result, "output")
    return [
      `- ${command}`,
      ok === null ? null : `ok=${ok}`,
      typeof exitCode === "number" ? `exit=${exitCode}` : null,
      cwd ? `cwd=${cwd}` : null,
      resultOutput ? `output=${truncateMiddle(resultOutput, 260)}` : null,
    ]
      .filter(Boolean)
      .join(" | ")
  })
}

function buildWorkflowRepairPrompt(
  run: WorkflowRun,
  snapshot: WorkflowRunSnapshot | null,
): string | null {
  const ops = snapshot?.ops ?? []
  const events = snapshot?.events ?? []
  const failedOp =
    [...ops].reverse().find((op) => op.state === "failed") ??
    [...ops].reverse().find(workflowOpHasValidationFailure)
  const validationOps = ops.filter(workflowOpHasValidationFailure)
  const isRecoverableFailure =
    run.state === "failed" ||
    run.state === "blocked" ||
    !!run.blockedReason ||
    !!failedOp ||
    validationOps.length > 0

  if (!isRecoverableFailure) return null

  const lines = [
    "请基于下面的工作流失败上下文继续修复。先定位根因，必要时调整代码或工作流脚本，然后运行最小验证。",
    "",
    "## Run",
    `- id: ${run.id}`,
    `- kind: ${run.kind}`,
    `- state: ${run.state}`,
    `- executionMode: ${run.executionMode}`,
    `- scriptHash: ${run.scriptHash}`,
  ]

  if (run.blockedReason) {
    lines.push(`- blockedReason: ${run.blockedReason}`)
  }

  if (failedOp) {
    const failedInput = compactJson(failedOp.input, "")
    const failedOutput = compactJson(failedOp.output, "")
    const failedError = compactJson(failedOp.error, "")
    lines.push(
      "",
      "## Failed Op",
      `- key: ${failedOp.opKey}`,
      `- type: ${failedOp.opType}`,
      `- state: ${failedOp.state}`,
    )
    if (failedInput) lines.push(`- input: ${truncateMiddle(failedInput, 360)}`)
    if (failedOutput) lines.push(`- output: ${truncateMiddle(failedOutput, 360)}`)
    if (failedError) lines.push(`- error: ${truncateMiddle(failedError, 360)}`)
  }

  if (validationOps.length > 0) {
    lines.push("", "## Validation")
    for (const op of validationOps.slice(0, 3)) {
      const output = asRecord(op.output)
      const summary = stringField(output, "summary")
      lines.push(`- op: ${op.opKey}${summary ? ` | ${summary}` : ""}`)
      lines.push(...workflowValidationResultLines(op))
    }
  }

  if (events.length > 0) {
    lines.push("", "## Recent Events")
    for (const event of events.slice(-5)) {
      lines.push(
        `- #${event.seq} ${event.eventType}: ${truncateMiddle(compactJson(event.payload, ""), 240)}`,
      )
    }
  }

  return lines.join("\n")
}

function workflowInitialDetailTab(
  snapshot: WorkflowRunSnapshot,
): "trace" | "validation" | "agents" {
  if (snapshot.ops.some(workflowOpHasValidationFailure)) return "validation"
  if (snapshot.ops.some((op) => op.opType === "spawnAgent")) return "agents"
  return "trace"
}

function workflowEventNeedsAttention(event: WorkflowEvent): boolean {
  const payload = asRecord(event.payload)
  const to = stringField(payload, "to")
  return (
    event.eventType === "op_failed" ||
    event.eventType === "script_permission_preview_blocked" ||
    event.eventType === "script_permission_approval_required" ||
    event.eventType === "budget_usage" ||
    event.eventType === "guarded_repair_validation_failed" ||
    event.eventType === "run_derived_from" ||
    event.eventType === "run_derived_child_created" ||
    to === "failed" ||
    to === "blocked" ||
    to === "awaiting_approval"
  )
}

const WORKFLOW_OVERVIEW_EVENT_TYPES = new Set([
  "run_created",
  "run_state_changed",
  "run_control_action",
  "run_runtime_launch",
  "run_runtime_result",
  "run_recovery_claimed",
  "run_worktree_attached",
  "script_permission_preview",
  "script_permission_preview_blocked",
  "script_permission_approval_required",
  "budget_usage",
  "guarded_repair_validation_failed",
  "guarded_repair_validation_passed",
  "run_derived_from",
  "run_derived_child_created",
])

function workflowOverviewEvents(events: WorkflowEvent[]): WorkflowEvent[] {
  const important = events
    .filter(
      (event) =>
        workflowEventNeedsAttention(event) || WORKFLOW_OVERVIEW_EVENT_TYPES.has(event.eventType),
    )
    .slice(-WORKFLOW_OVERVIEW_EVENT_PREVIEW)
  if (important.length > 0) return important
  return events
    .filter((event) => event.eventType !== "trace")
    .slice(-Math.min(3, WORKFLOW_OVERVIEW_EVENT_PREVIEW))
}

function workflowEventTone(event: WorkflowEvent): StatusTone {
  const payload = asRecord(event.payload)
  const to = stringField(payload, "to")
  if (event.eventType === "run_control_action") {
    const action = stringField(payload, "action")
    if (action === "approve" || action === "resume") return "good"
    if (action === "pause") return "warn"
    if (action === "cancel") return "muted"
    return "info"
  }
  if (event.eventType === "run_runtime_launch") {
    return boolField(payload, "accepted") === false ? "warn" : "info"
  }
  if (event.eventType === "run_runtime_result") {
    const status = stringField(payload, "status")
    const finalState = stringField(payload, "finalState")
    if (status === "error" || finalState === "failed" || finalState === "blocked") return "danger"
    if (status === "rejected" || status === "skipped" || finalState === "awaiting_approval") {
      return "warn"
    }
    if (finalState === "completed") return "good"
    return "info"
  }
  if (
    event.eventType === "op_failed" ||
    event.eventType === "guarded_repair_validation_failed" ||
    event.eventType === "script_permission_preview_blocked" ||
    to === "failed" ||
    to === "blocked"
  ) {
    return "danger"
  }
  if (
    event.eventType === "script_permission_approval_required" ||
    event.eventType === "budget_usage" ||
    to === "awaiting_approval" ||
    to === "awaiting_user"
  ) {
    return "warn"
  }
  if (
    event.eventType === "op_completed" ||
    event.eventType === "guarded_repair_validation_passed" ||
    to === "completed"
  ) {
    return "good"
  }
  if (
    event.eventType === "run_recovery_claimed" ||
    event.eventType === "run_worktree_attached" ||
    event.eventType === "run_derived_from" ||
    event.eventType === "run_derived_child_created" ||
    event.eventType === "op_started" ||
    event.eventType === "script_permission_preview" ||
    to === "running" ||
    to === "recovering"
  ) {
    return "info"
  }
  return "muted"
}

function workflowEventTitle(
  t: ReturnType<typeof useTranslation>["t"],
  event: WorkflowEvent,
): string {
  const payload = asRecord(event.payload)
  switch (event.eventType) {
    case "run_created":
      return t("workspace.workflow.eventRunCreated", "工作流已创建")
    case "run_state_changed":
      return t("workspace.workflow.eventRunStateChanged", "状态已更新")
    case "run_control_action":
      return t("workspace.workflow.eventRunControlAction", "控制动作")
    case "run_runtime_launch":
      return t("workspace.workflow.eventRunRuntimeLaunch", "启动请求")
    case "run_runtime_result":
      return t("workspace.workflow.eventRunRuntimeResult", "启动结果")
    case "run_recovery_claimed":
      return t("workspace.workflow.eventRecoveryClaimed", "恢复接管")
    case "run_worktree_attached":
      return t("workspace.workflow.eventWorktreeAttached", "运行位置已绑定")
    case "script_permission_preview":
      return t("workspace.workflow.eventPermissionPreview", "权限预览")
    case "script_permission_preview_blocked":
      return t("workspace.workflow.eventPermissionBlocked", "权限预览阻塞")
    case "script_permission_approval_required":
      return t("workspace.workflow.eventApprovalRequired", "需要批准")
    case "budget_usage":
      return t("workspace.workflow.eventBudgetUsage", "预算用量")
    case "run_derived_from":
      return t("workspace.workflow.eventRunDerivedFrom", "派生来源")
    case "run_derived_child_created":
      return t("workspace.workflow.eventRunDerivedChildCreated", "已生成派生运行")
    case "op_started":
      return t("workspace.workflow.eventOpStarted", "步骤开始")
    case "op_completed":
      return t("workspace.workflow.eventOpCompleted", "步骤完成")
    case "op_failed":
      return t("workspace.workflow.eventOpFailed", "步骤失败")
    case "guarded_repair_validation_failed":
      return t("workspace.workflow.eventRepairValidationFailed", "修复验证失败")
    case "guarded_repair_validation_passed":
      return t("workspace.workflow.eventRepairValidationPassed", "修复验证通过")
    case "trace":
      return stringField(payload, "label") ?? t("workspace.workflow.eventTrace", "Trace")
    default:
      return event.eventType
  }
}

function workflowEventDetail(
  t: ReturnType<typeof useTranslation>["t"],
  event: WorkflowEvent,
): string {
  const payload = asRecord(event.payload)
  switch (event.eventType) {
    case "run_created": {
      const kind = stringField(payload, "kind")
      const state = stringField(payload, "state")
      return [kind, state].filter(Boolean).join(" · ")
    }
    case "run_state_changed": {
      const from = stringField(payload, "from")
      const to = stringField(payload, "to")
      const reason = stringField(payload, "reason")
      return [from && to ? `${from} → ${to}` : null, reason].filter(Boolean).join(" · ")
    }
    case "run_control_action": {
      const action = stringField(payload, "action")
      const resultState = stringField(payload, "resultState")
      const reason = stringField(payload, "reason")
      return [action, resultState, reason].filter(Boolean).join(" · ")
    }
    case "run_runtime_launch": {
      const accepted = boolField(payload, "accepted")
      const owner = stringField(payload, "owner")
      const reason = stringField(payload, "reason")
      const acceptedLabel =
        accepted === null
          ? null
          : accepted
            ? t("workspace.workflow.launchAccepted", "已接收")
            : t("workspace.workflow.launchRejected", "未接收")
      return [acceptedLabel, owner, reason].filter(Boolean).join(" · ")
    }
    case "run_runtime_result": {
      const status = stringField(payload, "status")
      const finalState = stringField(payload, "finalState")
      const reason = stringField(payload, "reason")
      const error = stringField(payload, "error")
      const detail = [status, finalState, reason, error ? truncateMiddle(error, 72) : null]
      return detail.filter(Boolean).join(" · ")
    }
    case "script_permission_preview":
    case "script_permission_preview_blocked":
    case "script_permission_approval_required":
      return workflowPermissionSummaryText(t, asRecord(payload?.summary))
    case "run_worktree_attached": {
      const worktreeId = stringField(payload, "worktreeId")
      const path = stringField(payload, "path")
      const state = stringField(payload, "state")
      return [worktreeId, path ? basename(path) : null, state].filter(Boolean).join(" · ")
    }
    case "budget_usage": {
      const spent = numberField(payload, "spentOutputTokens")
      const limit = numberField(payload, "maxOutputTokens")
      const exhausted = boolField(payload, "exhausted")
      const reason = stringField(payload, "reason")
      const usage =
        typeof spent === "number" && typeof limit === "number"
          ? `${compactCount(spent)}/${compactCount(limit)}`
          : null
      return [usage, exhausted ? t("workspace.workflow.budgetExhausted", "已达上限") : null, reason]
        .filter(Boolean)
        .join(" · ")
    }
    case "run_derived_from":
    case "run_derived_child_created": {
      const parentRunId = stringField(payload, "parentRunId")
      const childRunId = stringField(payload, "childRunId")
      const origin = stringField(payload, "origin")
      return [
        parentRunId ? `parent ${parentRunId}` : null,
        childRunId ? `child ${childRunId}` : null,
        origin,
      ]
        .filter(Boolean)
        .join(" · ")
    }
    case "op_started":
    case "op_completed":
    case "op_failed": {
      const opKey = stringField(payload, "opKey")
      const opType = stringField(payload, "opType")
      const state = stringField(payload, "state")
      return [opKey, opType, state].filter(Boolean).join(" · ")
    }
    case "guarded_repair_validation_failed":
    case "guarded_repair_validation_passed": {
      const summary = stringField(payload, "summary")
      const failed = numberField(payload, "failed")
      const total = numberField(payload, "total")
      const stopReason = stringField(payload, "stopReason")
      const count =
        typeof failed === "number" && typeof total === "number"
          ? t("workspace.workflow.validationCount", "{{failed}}/{{total}} failed", {
              failed,
              total,
            })
          : typeof total === "number"
            ? t("workspace.workflow.validationTotal", "{{total}} total", { total })
            : null
      return [summary, count, stopReason].filter(Boolean).join(" · ")
    }
    case "trace":
      return truncateMiddle(compactJson(payload?.payload, event.eventType), 120)
    default:
      return truncateMiddle(compactJson(event.payload, event.eventType), 120)
  }
}

function loopStateLabel(t: ReturnType<typeof useTranslation>["t"], state: LoopState): string {
  switch (state) {
    case "active":
      return t("workspace.loop.stateActive", "运行中")
    case "paused":
      return t("workspace.loop.statePaused", "已暂停")
    case "completed":
      return t("workspace.loop.stateCompleted", "已完成")
    case "cancelled":
      return t("workspace.loop.stateCancelled", "已停止")
    case "blocked":
      return t("workspace.loop.stateBlocked", "已阻塞")
  }
}

function loopStateTone(state: LoopState): StatusTone {
  switch (state) {
    case "active":
      return "info"
    case "paused":
      return "warn"
    case "completed":
      return "good"
    case "blocked":
      return "danger"
    case "cancelled":
      return "muted"
  }
}

function loopRunStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: LoopRunState,
): string {
  switch (state) {
    case "running":
      return t("workspace.loop.runRunning", "运行中")
    case "queued":
      return t("workspace.loop.runQueued", "已排队")
    case "injected":
      return t("workspace.loop.runInjected", "已注入")
    case "succeeded":
      return t("workspace.loop.runSucceeded", "成功")
    case "empty":
      return t("workspace.loop.runEmpty", "无输出")
    case "failed":
      return t("workspace.loop.runFailed", "失败")
    case "cancelled":
      return t("workspace.loop.runCancelled", "已取消")
    case "skipped":
      return t("workspace.loop.runSkipped", "已跳过")
  }
}

function loopRunStateTone(state: LoopRunState): StatusTone {
  switch (state) {
    case "succeeded":
      return "good"
    case "failed":
    case "cancelled":
      return "danger"
    case "running":
    case "queued":
    case "injected":
      return "info"
    case "empty":
    case "skipped":
      return "muted"
  }
}

function isLoopTerminal(state: LoopState): boolean {
  return state === "completed" || state === "cancelled"
}

function loopTriggerSummary(kind: LoopTriggerKind, spec: Record<string, unknown>): string {
  if (kind === "interval") {
    const secs = typeof spec.intervalSecs === "number" ? spec.intervalSecs : null
    return secs ? `every ${formatLoopDuration(secs)}` : "interval"
  }
  if (kind === "condition") {
    const condition = typeof spec.condition === "string" ? spec.condition : "condition"
    return `until ${condition.length > 48 ? `${condition.slice(0, 48)}...` : condition}`
  }
  if (kind === "cron") return "cron"
  return "event"
}

function formatLoopDuration(secs: number): string {
  if (secs % 86_400 === 0) return `${secs / 86_400}d`
  if (secs % 3600 === 0) return `${secs / 3600}h`
  if (secs % 60 === 0) return `${secs / 60}m`
  return `${secs}s`
}

function parseLoopDurationSecs(input: string): number | null {
  const trimmed = input.trim().toLowerCase()
  const match = trimmed.match(/^(\d+)\s*(s|sec|secs|m|min|mins|h|hr|hrs|d|day|days)?$/)
  if (!match) return null
  const value = Number(match[1])
  if (!Number.isFinite(value) || value <= 0) return null
  const unit = match[2] ?? "s"
  const multiplier =
    unit === "d" || unit === "day" || unit === "days"
      ? 86_400
      : unit === "h" || unit === "hr" || unit === "hrs"
        ? 3600
        : unit === "m" || unit === "min" || unit === "mins"
          ? 60
          : 1
  return value * multiplier
}

function parseOptionalPositiveInt(input: string): number | null {
  const trimmed = input.trim()
  if (!trimmed) return null
  const value = Number(trimmed)
  return Number.isInteger(value) && value > 0 ? value : null
}

interface WorkflowFocusTarget {
  runId: string
  nonce: number
}

function workflowRunSortTime(run: WorkflowRun): number {
  const time = Date.parse(run.updatedAt || run.createdAt || "")
  return Number.isFinite(time) ? time : 0
}

function loopRunTrace(run: LoopRun): Record<string, unknown> {
  return typeof run.trace === "object" && run.trace !== null && !Array.isArray(run.trace)
    ? (run.trace as Record<string, unknown>)
    : {}
}

function loopRunTraceString(run: LoopRun, field: string): string | null {
  const value = loopRunTrace(run)[field]
  return typeof value === "string" && value.trim() ? value : null
}

function loopRunTemplateLabel(run: LoopRun): string | null {
  const templateId = loopRunTraceString(run, "templateId")
  if (!templateId) return null
  const version = loopRunTraceString(run, "templateVersion")
  return version ? `${templateId}@${version}` : templateId
}

function LoopRunHistory({
  snapshot,
  loading,
  error,
  onSelectWorkflowRun,
}: {
  snapshot: LoopSnapshot | null
  loading: boolean
  error: string | null
  onSelectWorkflowRun?: (runId: string) => void
}) {
  const { t } = useTranslation()
  if (loading) {
    return (
      <div className="mt-2 flex items-center gap-2 rounded-md bg-secondary/20 px-2 py-2 text-[11px] text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        {t("workspace.loop.historyLoading", "加载 Loop 运行记录")}
      </div>
    )
  }
  if (error) {
    return (
      <div className="mt-2 rounded-md border border-destructive/25 bg-destructive/5 px-2 py-1.5 text-[11px] text-destructive">
        {error}
      </div>
    )
  }
  if (!snapshot) return null
  if (snapshot.runs.length === 0) {
    return (
      <div className="mt-2 rounded-md bg-secondary/20 px-2 py-1.5 text-[11px] text-muted-foreground">
        {t("workspace.loop.historyEmpty", "还没有触发记录")}
      </div>
    )
  }
  return (
    <div className="mt-2 space-y-1 rounded-md border border-border/60 bg-secondary/15 p-1.5">
      <div className="flex items-center gap-1.5 px-0.5 text-[10px] font-medium text-muted-foreground">
        <Clock className="h-3 w-3" />
        {t("workspace.loop.historyTitle", "最近运行")}
      </div>
      {snapshot.runs.slice(0, 5).map((run) => {
        const workflowRunId = loopRunTraceString(run, "workflowRunId")
        const template = loopRunTemplateLabel(run)
        const summary = run.error || run.resultSummary
        return (
          <div
            key={run.id}
            className="rounded-md border border-border/45 bg-background/65 px-2 py-1.5"
          >
            <div className="flex min-w-0 items-center gap-2">
              <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                #{run.seq}
              </span>
              <StatusPill
                label={loopRunStateLabel(t, run.state)}
                tone={loopRunStateTone(run.state)}
                loading={run.state === "running" || run.state === "queued"}
              />
              <span className="min-w-0 flex-1 truncate text-[10px] text-muted-foreground">
                {formatMessageTime(run.finishedAt ?? run.startedAt)}
              </span>
              {workflowRunId && onSelectWorkflowRun ? (
                <IconTip label={t("workspace.loop.viewWorkflow", "查看工作流")}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0"
                    aria-label={t("workspace.loop.viewWorkflow", "查看工作流")}
                    onClick={() => onSelectWorkflowRun(workflowRunId)}
                  >
                    <Eye className="h-3 w-3" />
                  </Button>
                </IconTip>
              ) : null}
            </div>
            {workflowRunId || template ? (
              <div className="mt-1 flex min-w-0 flex-wrap gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
                {workflowRunId ? (
                  <span className="min-w-0 truncate">
                    {t("workspace.loop.workflowRun", "工作流")}{" "}
                    <span className="font-mono">{truncateMiddle(workflowRunId, 18)}</span>
                  </span>
                ) : null}
                {template ? (
                  <span className="min-w-0 truncate">
                    {t("workspace.loop.template", "模板")}{" "}
                    <span className="font-mono">{template}</span>
                  </span>
                ) : null}
              </div>
            ) : null}
            {summary ? (
              <p
                className={cn(
                  "mt-1 line-clamp-2 text-[10px]",
                  run.error ? "text-destructive" : "text-muted-foreground",
                )}
              >
                {summary}
              </p>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

function LoopSchedulesSection({
  sessionId,
  incognito,
  turnActive,
  workflowRuns = [],
  onSelectWorkflowRun,
  loopSchedulesState,
  goalState,
  createRequest,
  inspectRequest,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workflowRuns?: WorkflowRun[]
  onSelectWorkflowRun?: (runId: string) => void
  loopSchedulesState?: LoopSchedulesState
  goalState: GoalStateSnapshot
  createRequest?: number
  inspectRequest?: { loopId: string; nonce: number } | null
}) {
  const { t } = useTranslation()
  const ownedLoopSchedulesState = useLoopSchedules(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(loopSchedulesState),
  })
  const { schedules, activeCount, loading, error, refresh } =
    loopSchedulesState ?? ownedLoopSchedulesState
  const [actionId, setActionId] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [createSaving, setCreateSaving] = useState(false)
  const [draftKind, setDraftKind] = useState<"interval" | "condition">("interval")
  const [draftExecutionStrategy, setDraftExecutionStrategy] =
    useState<LoopExecutionStrategy>("continue")
  const [draftInterval, setDraftInterval] = useState("10m")
  const [draftCondition, setDraftCondition] = useState("")
  const [draftPrompt, setDraftPrompt] = useState("")
  const [draftMaxRuns, setDraftMaxRuns] = useState("")
  const [draftMaxRuntime, setDraftMaxRuntime] = useState("")
  const [draftTokens, setDraftTokens] = useState("")
  const [detailLoopId, setDetailLoopId] = useState<string | null>(null)
  const [detailSnapshot, setDetailSnapshot] = useState<LoopSnapshot | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)
  const detailReqRef = useRef(0)
  const lastCreateRequestRef = useRef(createRequest ?? 0)
  const lastInspectRequestRef = useRef(inspectRequest?.nonce ?? 0)
  const activeGoal = goalState.snapshot?.goal ?? null
  const canUseWorkflowLoop =
    draftKind === "interval" && Boolean(activeGoal?.workflowTemplateId)
  const workflowRunsByLoop = useMemo(() => {
    const byLoop = new Map<string, WorkflowRun[]>()
    for (const run of workflowRuns) {
      const origin = run.origin?.trim()
      if (!origin?.startsWith("loop:")) continue
      const loopId = origin.slice("loop:".length)
      if (!loopId) continue
      const list = byLoop.get(loopId)
      if (list) {
        list.push(run)
      } else {
        byLoop.set(loopId, [run])
      }
    }
    for (const list of byLoop.values()) {
      list.sort((a, b) => workflowRunSortTime(b) - workflowRunSortTime(a))
    }
    return byLoop
  }, [workflowRuns])

  useEffect(() => {
    if (!canUseWorkflowLoop && draftExecutionStrategy === "workflow") {
      setDraftExecutionStrategy("continue")
    }
  }, [canUseWorkflowLoop, draftExecutionStrategy])

  useEffect(() => {
    const nextRequest = createRequest ?? 0
    if (nextRequest === lastCreateRequestRef.current) return
    lastCreateRequestRef.current = nextRequest
    if (!sessionId || incognito) return
    setCreateOpen(true)
    setDraftKind("interval")
    if (activeGoal?.workflowTemplateId) {
      setDraftExecutionStrategy("workflow")
    }
  }, [activeGoal?.workflowTemplateId, createRequest, incognito, sessionId])

  const loadLoopDetail = useCallback((loopId: string) => {
    const req = ++detailReqRef.current
    setDetailLoopId(loopId)
    setDetailLoading(true)
    setDetailError(null)
    setDetailSnapshot(null)
    getTransport()
      .call<LoopSnapshot | null>("get_loop_schedule", { loopId })
      .then((snapshot) => {
        if (detailReqRef.current !== req) return
        setDetailSnapshot(snapshot)
        setDetailLoading(false)
      })
      .catch((e) => {
        if (detailReqRef.current !== req) return
        logger.error("ui", "LoopSchedulesSection::loadDetail", "Loop detail load failed", e)
        setDetailSnapshot(null)
        setDetailError(e instanceof Error ? e.message : String(e))
        setDetailLoading(false)
      })
  }, [])

  useEffect(() => {
    const nextRequest = inspectRequest?.nonce ?? 0
    if (nextRequest === lastInspectRequestRef.current) return
    lastInspectRequestRef.current = nextRequest
    if (!inspectRequest?.loopId || !sessionId || incognito) return
    setCreateOpen(false)
    loadLoopDetail(inspectRequest.loopId)
  }, [incognito, inspectRequest, loadLoopDetail, sessionId])

  const toggleLoopDetail = useCallback(
    (loopId: string) => {
      if (detailLoopId === loopId) {
        detailReqRef.current += 1
        setDetailLoopId(null)
        setDetailSnapshot(null)
        setDetailLoading(false)
        setDetailError(null)
        return
      }
      loadLoopDetail(loopId)
    },
    [detailLoopId, loadLoopDetail],
  )

  const runAction = useCallback(
    async (loop: LoopSchedule, action: "pause" | "resume" | "stop") => {
      setActionId(`${loop.id}:${action}`)
      try {
        await getTransport().call(`${action}_loop_schedule`, { loopId: loop.id })
        refresh()
        if (detailLoopId === loop.id) {
          loadLoopDetail(loop.id)
        }
      } catch (e) {
        logger.error("ui", "LoopSchedulesSection::action", "Loop action failed", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setActionId(null)
      }
    },
    [detailLoopId, loadLoopDetail, refresh],
  )

  const createLoop = useCallback(async () => {
    if (!sessionId) {
      toast.error(t("workspace.loop.sessionRequired", "先选择一个会话"))
      return
    }
    const intervalSecs = parseLoopDurationSecs(draftInterval)
    if (!intervalSecs) {
      toast.error(t("workspace.loop.intervalInvalid", "请输入有效间隔，例如 10m"))
      return
    }
    const condition = draftCondition.trim()
    if (draftKind === "condition" && !condition) {
      toast.error(t("workspace.loop.conditionRequired", "请输入停止条件"))
      return
    }
    const prompt = draftPrompt.trim()
    if (draftKind === "interval" && !prompt && !activeGoal) {
      toast.error(
        t("workspace.loop.promptOrGoalRequired", "请输入 prompt，或先创建一个 active goal"),
      )
      return
    }
    if (draftExecutionStrategy === "workflow" && !canUseWorkflowLoop) {
      toast.error(
        t(
          "workspace.loop.workflowRequiresGoalTemplate",
          "创建工作流的 Loop 需要当前 Goal 已选择任务领域模板",
        ),
      )
      return
    }
    const maxRuntimeSecs = draftMaxRuntime.trim() ? parseLoopDurationSecs(draftMaxRuntime) : null
    if (draftMaxRuntime.trim() && !maxRuntimeSecs) {
      toast.error(t("workspace.loop.maxRuntimeInvalid", "请输入有效最长运行时间，例如 2h"))
      return
    }
    const maxRuns = parseOptionalPositiveInt(draftMaxRuns)
    if (draftMaxRuns.trim() && !maxRuns) {
      toast.error(t("workspace.loop.maxRunsInvalid", "最大次数必须是正整数"))
      return
    }
    const tokenBudget = parseOptionalPositiveInt(draftTokens)
    if (draftTokens.trim() && !tokenBudget) {
      toast.error(t("workspace.loop.tokensInvalid", "Token 预算必须是正整数"))
      return
    }
    const defaultConditionPrompt = t(
      "workspace.loop.defaultConditionPrompt",
      "Continue until this condition is true: {{condition}}. Check the condition first, stop when it is satisfied, otherwise take the next useful step.",
      { condition },
    )
    setCreateSaving(true)
    try {
      await getTransport().call("create_loop_schedule", {
        sessionId,
        prompt: draftKind === "condition" && !prompt ? defaultConditionPrompt : prompt,
        triggerKind: draftKind,
        triggerSpec: draftKind === "condition" ? { condition, intervalSecs } : { intervalSecs },
        executionStrategy: draftExecutionStrategy,
        maxRuns,
        maxRuntimeSecs,
        tokenBudget,
      })
      toast.success(t("workspace.loop.created", "Loop 已创建"))
      setDraftPrompt("")
      setDraftCondition("")
      setCreateOpen(false)
      refresh()
    } catch (e) {
      logger.error("ui", "LoopSchedulesSection::create", "Loop create failed", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setCreateSaving(false)
    }
  }, [
    activeGoal,
    canUseWorkflowLoop,
    draftCondition,
    draftExecutionStrategy,
    draftInterval,
    draftKind,
    draftMaxRuns,
    draftMaxRuntime,
    draftPrompt,
    draftTokens,
    refresh,
    sessionId,
    t,
  ])

  const canCreate = Boolean(sessionId) && !incognito

  return (
    <WorkspaceSection
      title={t("workspace.loop.title", "Loop")}
      count={schedules.length}
      icon={Radio}
      expandSignal={(createRequest ?? 0) + (inspectRequest?.nonce ?? 0)}
      meta={
        activeCount > 0 ? (
          <StatusPill
            label={t("workspace.loop.activeCount", "{{count}} active", { count: activeCount })}
            tone="info"
          />
        ) : undefined
      }
      defaultExpanded={activeCount > 0 || schedules.length > 0}
    >
      <div className="mb-2 space-y-2">
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0 text-[11px] text-muted-foreground">
            {activeGoal
              ? t("workspace.loop.boundGoal", "绑定当前 Goal")
              : t("workspace.loop.promptMode", "按 prompt 重复触发")}
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-7 gap-1 px-2 text-[11px]"
            disabled={!canCreate}
            onClick={() => setCreateOpen((v) => !v)}
          >
            <Plus className="h-3.5 w-3.5" />
            {createOpen ? t("workspace.loop.closeCreate", "收起") : t("workspace.loop.new", "新建")}
          </Button>
        </div>
        {createOpen ? (
          <div className="space-y-2 rounded-md border border-border/70 bg-background/70 p-2">
            <div className="grid grid-cols-2 gap-1 rounded-md bg-secondary/40 p-1">
              {(["interval", "condition"] as const).map((kind) => (
                <Button
                  key={kind}
                  type="button"
                  variant={draftKind === kind ? "secondary" : "ghost"}
                  size="sm"
                  className="h-7 text-[11px]"
                  onClick={() => setDraftKind(kind)}
                >
                  {kind === "interval"
                    ? t("workspace.loop.kindInterval", "Every")
                    : t("workspace.loop.kindCondition", "Until")}
                </Button>
              ))}
            </div>
            <div className="grid grid-cols-2 gap-1 rounded-md bg-secondary/40 p-1">
              {(["continue", "workflow"] as const).map((strategy) => {
                const disabled = strategy === "workflow" && !canUseWorkflowLoop
                return (
                  <Button
                    key={strategy}
                    type="button"
                    variant={draftExecutionStrategy === strategy ? "secondary" : "ghost"}
                    size="sm"
                    className="h-7 text-[11px]"
                    disabled={disabled}
                    onClick={() => setDraftExecutionStrategy(strategy)}
                  >
                    {strategy === "workflow"
                      ? t("workspace.loop.strategyWorkflow", "创建工作流")
                      : t("workspace.loop.strategyContinue", "继续会话")}
                  </Button>
                )
              })}
            </div>
            {draftExecutionStrategy === "workflow" && activeGoal?.workflowTemplateId ? (
              <div className="rounded-md border border-primary/20 bg-primary/5 px-2 py-1.5 text-[11px] text-muted-foreground">
                {t(
                  "workspace.loop.strategyWorkflowHint",
                  "每次触发都会按当前 Goal 的领域模板创建可观察 Workflow run。",
                )}{" "}
                <span className="font-medium text-foreground/80">
                  {activeGoal.workflowTemplateVersion
                    ? `${activeGoal.workflowTemplateId}@${activeGoal.workflowTemplateVersion}`
                    : activeGoal.workflowTemplateId}
                </span>
              </div>
            ) : !canUseWorkflowLoop ? (
              <div className="rounded-md bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground">
                {t(
                  "workspace.loop.strategyWorkflowDisabled",
                  "选择带任务领域模板的 active Goal 后，可让 Loop 直接创建 Workflow run。",
                )}
              </div>
            ) : null}
            <div className="grid grid-cols-2 gap-2">
              <Input
                value={draftInterval}
                onChange={(e) => setDraftInterval(e.target.value)}
                placeholder="10m"
                className="h-8 text-xs"
                aria-label={t("workspace.loop.interval", "间隔")}
              />
              <Input
                value={draftMaxRuns}
                onChange={(e) => setDraftMaxRuns(e.target.value)}
                placeholder={t("workspace.loop.maxRunsPlaceholder", "max runs")}
                className="h-8 text-xs"
                aria-label={t("workspace.loop.maxRunsLabel", "最大次数")}
              />
            </div>
            {draftKind === "condition" ? (
              <Input
                value={draftCondition}
                onChange={(e) => setDraftCondition(e.target.value)}
                placeholder={t("workspace.loop.conditionPlaceholder", "CI is green")}
                className="h-8 text-xs"
                aria-label={t("workspace.loop.condition", "停止条件")}
              />
            ) : null}
            <Textarea
              value={draftPrompt}
              onChange={(e) => setDraftPrompt(e.target.value)}
              placeholder={
                draftKind === "condition"
                  ? t(
                      "workspace.loop.promptOptionalPlaceholder",
                      "每次触发要做什么；留空则只检查条件并推进下一步",
                    )
                  : activeGoal
                    ? t("workspace.loop.promptGoalPlaceholder", "留空则继续当前 active goal")
                    : t(
                        "workspace.loop.promptPlaceholder",
                        "check CI and continue fixing if failing",
                      )
              }
              className="min-h-[64px] resize-none text-xs"
              aria-label={t("workspace.loop.prompt", "Prompt")}
            />
            <div className="grid grid-cols-2 gap-2">
              <Input
                value={draftMaxRuntime}
                onChange={(e) => setDraftMaxRuntime(e.target.value)}
                placeholder={t("workspace.loop.maxRuntimePlaceholder", "max runtime")}
                className="h-8 text-xs"
                aria-label={t("workspace.loop.maxRuntimeLabel", "最长运行时间")}
              />
              <Input
                value={draftTokens}
                onChange={(e) => setDraftTokens(e.target.value)}
                placeholder={t("workspace.loop.tokensPlaceholder", "tokens")}
                className="h-8 text-xs"
                aria-label={t("workspace.loop.tokensLabel", "Token 预算")}
              />
            </div>
            <div className="flex justify-end">
              <Button
                type="button"
                size="sm"
                className="h-7 gap-1 px-2 text-[11px]"
                disabled={createSaving}
                onClick={() => void createLoop()}
              >
                {createSaving ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Radio className="h-3.5 w-3.5" />
                )}
                {t("workspace.loop.create", "创建 Loop")}
              </Button>
            </div>
          </div>
        ) : null}
      </div>
      {loading && schedules.length === 0 ? (
        <div className="flex items-center justify-center py-4 text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
        </div>
      ) : error ? (
        <EmptyHint>{error}</EmptyHint>
      ) : schedules.length === 0 ? (
        <EmptyHint>
          {incognito
            ? t("workspace.loop.incognitoEmpty", "无痕会话不保存 loop")
            : t("workspace.loop.empty", "暂无 loop")}
        </EmptyHint>
      ) : (
        <div className="space-y-2">
          {schedules.slice(0, 5).map((loop) => {
            const isBusy = actionId?.startsWith(`${loop.id}:`)
            const derivedWorkflowRuns = workflowRunsByLoop.get(loop.id) ?? []
            const latestWorkflowRun = derivedWorkflowRuns[0]
            const detailOpen = detailLoopId === loop.id
            return (
              <div
                key={loop.id}
                className="rounded-md border border-border/70 bg-background/70 px-2.5 py-2"
              >
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <StatusPill
                        label={loopStateLabel(t, loop.state)}
                        tone={loopStateTone(loop.state)}
                      />
                      {loop.executionStrategy === "workflow" ? (
                        <StatusPill
                          label={t("workspace.loop.strategyWorkflowShort", "Workflow")}
                          tone="info"
                        />
                      ) : null}
                      <span className="truncate text-xs font-medium text-foreground">
                        {loopTriggerSummary(loop.triggerKind, loop.triggerSpec)}
                      </span>
                    </div>
                    <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">{loop.prompt}</p>
                    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
                      <span>
                        {t("workspace.loop.runs", "运行")} {loop.runCount}/{loop.maxRuns ?? "∞"}
                      </span>
                      {loop.maxRuntimeSecs ? (
                        <span>
                          {t("workspace.loop.maxRuntime", "最长")}{" "}
                          {formatLoopDuration(loop.maxRuntimeSecs)}
                        </span>
                      ) : null}
                      <span>{formatMessageTime(loop.updatedAt)}</span>
                    </div>
                    {loop.blockedReason ? (
                      <p className="mt-1 text-[10px] text-destructive">{loop.blockedReason}</p>
                    ) : null}
                    {loop.executionStrategy === "workflow" ? (
                      latestWorkflowRun ? (
                        <div className="mt-2 flex min-w-0 items-center gap-2 rounded-md bg-secondary/25 px-2 py-1.5">
                          <GitPullRequest className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                          <div className="min-w-0 flex-1">
                            <div className="flex min-w-0 items-center gap-1.5">
                              <span className="truncate text-[11px] font-medium text-foreground/85">
                                {latestWorkflowRun.kind}
                              </span>
                              {derivedWorkflowRuns.length > 1 ? (
                                <span className="shrink-0 text-[10px] text-muted-foreground">
                                  +{derivedWorkflowRuns.length - 1}
                                </span>
                              ) : null}
                            </div>
                            <div className="truncate text-[10px] text-muted-foreground">
                              {latestWorkflowRun.id}
                              <span className="px-1 text-muted-foreground/50">·</span>
                              {formatMessageTime(latestWorkflowRun.updatedAt)}
                            </div>
                          </div>
                          <StatusPill
                            label={workflowRunStateLabel(t, latestWorkflowRun.state)}
                            tone={workflowRunTone(latestWorkflowRun.state)}
                            loading={
                              latestWorkflowRun.state === "running" ||
                              latestWorkflowRun.state === "recovering"
                            }
                          />
                          {onSelectWorkflowRun ? (
                            <IconTip label={t("workspace.loop.viewWorkflow", "查看工作流")}>
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon"
                                className="h-7 w-7 shrink-0"
                                aria-label={t("workspace.loop.viewWorkflow", "查看工作流")}
                                onClick={() => onSelectWorkflowRun(latestWorkflowRun.id)}
                              >
                                <Eye className="h-3.5 w-3.5" />
                              </Button>
                            </IconTip>
                          ) : null}
                        </div>
                      ) : (
                        <div className="mt-2 flex items-center gap-2 rounded-md bg-secondary/20 px-2 py-1.5 text-[10px] text-muted-foreground">
                          <GitPullRequest className="h-3.5 w-3.5 shrink-0" />
                          <span className="truncate">
                            {t("workspace.loop.workflowPending", "等待下一次触发创建 Workflow run")}
                          </span>
                        </div>
                      )
                    ) : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <IconTip label={t("workspace.loop.history", "运行记录")}>
                      <Button
                        variant={detailOpen ? "secondary" : "ghost"}
                        size="icon"
                        className="h-7 w-7"
                        aria-label={t("workspace.loop.history", "运行记录")}
                        onClick={() => toggleLoopDetail(loop.id)}
                      >
                        {detailOpen && detailLoading ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Clock className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </IconTip>
                    {loop.state === "active" ? (
                      <IconTip label={t("workspace.loop.pause", "暂停")}>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7"
                          disabled={isBusy}
                          onClick={() => void runAction(loop, "pause")}
                        >
                          <Pause className="h-3.5 w-3.5" />
                        </Button>
                      </IconTip>
                    ) : loop.state === "paused" || loop.state === "blocked" ? (
                      <IconTip label={t("workspace.loop.resume", "恢复")}>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7"
                          disabled={isBusy}
                          onClick={() => void runAction(loop, "resume")}
                        >
                          <Play className="h-3.5 w-3.5" />
                        </Button>
                      </IconTip>
                    ) : null}
                    {!isLoopTerminal(loop.state) ? (
                      <IconTip label={t("workspace.loop.stop", "停止")}>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7 text-muted-foreground hover:text-destructive"
                          disabled={isBusy}
                          onClick={() => void runAction(loop, "stop")}
                        >
                          <X className="h-3.5 w-3.5" />
                        </Button>
                      </IconTip>
                    ) : null}
                  </div>
                </div>
                {detailOpen ? (
                  <LoopRunHistory
                    snapshot={detailSnapshot}
                    loading={detailLoading}
                    error={detailError}
                    onSelectWorkflowRun={onSelectWorkflowRun}
                  />
                ) : null}
              </div>
            )
          })}
          {schedules.length > 5 ? (
            <p className="px-1 text-[10px] text-muted-foreground">
              {t("workspace.loop.more", "还有 {{count}} 个 loop，可用 /loop status 查看", {
                count: schedules.length - 5,
              })}
            </p>
          ) : null}
        </div>
      )}
    </WorkspaceSection>
  )
}

function WorkflowRunsSection({
  sessionId,
  incognito,
  turnActive,
  workingDir,
  onEnsureSession,
  onViewSubagentSession,
  onOpenLoopCreate,
  onOpenLoopProblem,
  onOpenDomainWorkbench,
  workflowRunsState,
  loopSchedulesState,
  goalState,
  domainWorkbenchState,
  focusedRunTarget,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  onEnsureSession?: () => Promise<string | null>
  onViewSubagentSession?: (sessionId: string) => void
  onOpenLoopCreate?: () => void
  onOpenLoopProblem?: (loopId: string) => void
  onOpenDomainWorkbench?: (target: "operational" | "soak") => void
  workflowRunsState?: WorkflowRunsState
  loopSchedulesState: LoopSchedulesState
  goalState: GoalStateSnapshot
  domainWorkbenchState: DomainTaskWorkbenchState
  focusedRunTarget?: WorkflowFocusTarget | null
}) {
  const { t } = useTranslation()
  const ownedWorkflowRuns = useWorkflowRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(workflowRunsState),
  })
  const { runs, activeCount, loading, error, refresh } = workflowRunsState ?? ownedWorkflowRuns
  const managedWorktreesState = useManagedWorktrees(sessionId, { incognito, turnActive })
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [snapshot, setSnapshot] = useState<WorkflowRunSnapshot | null>(null)
  const [snapshotLoading, setSnapshotLoading] = useState(false)
  const [actionKey, setActionKey] = useState<string | null>(null)
  const [workflowMode, setWorkflowMode] = useState<WorkflowAutonomyMode>("off")
  const [workflowModeLoading, setWorkflowModeLoading] = useState(false)
  const [workflowModeSaving, setWorkflowModeSaving] = useState<WorkflowAutonomyMode | null>(null)
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("off")
  const [executionModeLoading, setExecutionModeLoading] = useState(false)
  const [executionModeSaving, setExecutionModeSaving] = useState<ExecutionMode | null>(null)
  const [detailTab, setDetailTab] = useState<WorkflowDetailTab>("trace")
  const [createOpen, setCreateOpen] = useState(false)
  const [createSaving, setCreateSaving] = useState(false)
  const [draftPreview, setDraftPreview] = useState<WorkflowScriptPreview | null>(null)
  const [draftPreviewLoading, setDraftPreviewLoading] = useState(false)
  const [draftPreviewError, setDraftPreviewError] = useState<string | null>(null)
  const [draftKind, setDraftKind] = useState(WORKFLOW_KIND_DEFAULT)
  const [draftMode, setDraftMode] = useState<ExecutionMode>("guarded")
  const [draftRunImmediately, setDraftRunImmediately] = useState(false)
  const [draftWorktreeMode, setDraftWorktreeMode] = useState("session")
  const [draftObjective, setDraftObjective] = useState("")
  const [draftScript, setDraftScript] = useState(WORKFLOW_SCRIPT_TEMPLATE)
  const [draftOrigin, setDraftOrigin] = useState<WorkflowDraftOrigin | null>(null)
  const [domainTemplates, setDomainTemplates] = useState<DomainWorkflowTemplate[]>([])
  const [domainTemplatesLoading, setDomainTemplatesLoading] = useState(false)
  const [domainTemplatesError, setDomainTemplatesError] = useState<string | null>(null)
  const [selectedDomainTemplateId, setSelectedDomainTemplateId] = useState("")
  const [selectedDomainTaskType, setSelectedDomainTaskType] = useState("")
  const [domainDraft, setDomainDraft] = useState<DomainWorkflowDraft | null>(null)
  const [domainDraftLoading, setDomainDraftLoading] = useState(false)
  const [showAllRuns, setShowAllRuns] = useState(false)
  const [pendingCancelRun, setPendingCancelRun] = useState<WorkflowRun | null>(null)
  const [goalActionKey, setGoalActionKey] = useState<string | null>(null)
  const [goalCreateOpen, setGoalCreateOpen] = useState(false)
  const [goalObjective, setGoalObjective] = useState("")
  const [goalCriteria, setGoalCriteria] = useState("")
  const [goalTemplateId, setGoalTemplateId] = useState(GOAL_DOMAIN_FREE_VALUE)
  const [goalTaskType, setGoalTaskType] = useState("")
  const [goalSaving, setGoalSaving] = useState(false)
  const [goalEditRequest, setGoalEditRequest] = useState(0)
  const snapshotReqRef = useRef(0)
  const workflowModeReqRef = useRef(0)
  const executionModeReqRef = useRef(0)
  const previewReqRef = useRef(0)
  const domainTemplatesReqRef = useRef(0)
  const domainTemplatesRequestedRef = useRef(false)
  const domainDraftReqRef = useRef(0)
  const autoDetailTabRunRef = useRef<string | null>(null)
  const ensureSessionRef = useRef<Promise<string | null> | null>(null)

  const selectedRun = runs.find((run) => run.id === selectedRunId) ?? null
  const visibleRuns = showAllRuns ? runs : runs.slice(0, WORKFLOW_RUN_PREVIEW)
  const canMaterializeSession = Boolean(sessionId || onEnsureSession)
  const activeGoal = goalState.snapshot?.goal ?? null
  const selectedDomainTemplate = findDomainTemplateByValue(
    domainTemplates,
    selectedDomainTemplateId,
  )
  const selectedGoalTemplate =
    goalTemplateId === GOAL_DOMAIN_FREE_VALUE
      ? null
      : findDomainTemplateByValue(domainTemplates, goalTemplateId)
  const draftWorktrees = managedWorktreesState.worktrees.filter(
    (worktree) => worktree.state !== "archived" && worktree.pathExists,
  )
  const normalizedDraftWorktreeMode =
    draftWorktreeMode === "new" && !workingDir ? "session" : draftWorktreeMode

  useEffect(() => {
    if (draftWorktreeMode === "new" || draftWorktreeMode === "session") return
    if (draftOrigin?.type === "repair") return
    const referencedByRun = runs.some((run) => run.worktreeId === draftWorktreeMode)
    const listedWorktree = draftWorktrees.some((worktree) => worktree.id === draftWorktreeMode)
    if (!referencedByRun && !listedWorktree) {
      setDraftWorktreeMode(workingDir ? "new" : "session")
    }
  }, [draftOrigin?.type, draftWorktreeMode, draftWorktrees, runs, workingDir])

  const ensureWorkflowSession = useCallback(async () => {
    if (sessionId) return sessionId
    if (!onEnsureSession) {
      toast.error(t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建工作流"))
      return null
    }
    if (!ensureSessionRef.current) {
      ensureSessionRef.current = onEnsureSession().finally(() => {
        ensureSessionRef.current = null
      })
    }
    const nextSessionId = await ensureSessionRef.current
    if (!nextSessionId) {
      toast.error(t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建工作流"))
    }
    return nextSessionId
  }, [onEnsureSession, sessionId, t])

  const loadDomainWorkflowTemplates = useCallback(() => {
    if (incognito) {
      domainTemplatesReqRef.current += 1
      domainTemplatesRequestedRef.current = false
      setDomainTemplates([])
      setDomainTemplatesLoading(false)
      setDomainTemplatesError(null)
      return
    }
    domainTemplatesRequestedRef.current = true
    const req = ++domainTemplatesReqRef.current
    setDomainTemplatesLoading(true)
    setDomainTemplatesError(null)
    getTransport()
      .call<DomainWorkflowTemplate[]>("list_domain_workflow_templates", { limit: 24 })
      .then((next) => {
        if (domainTemplatesReqRef.current !== req) return
        setDomainTemplates(Array.isArray(next) ? next.filter((template) => template.enabled) : [])
        setDomainTemplatesLoading(false)
      })
      .catch((e) => {
        if (domainTemplatesReqRef.current !== req) return
        logger.error(
          "ui",
          "WorkflowRunsSection::loadDomainWorkflowTemplates",
          "Failed to load domain workflow templates",
          e,
        )
        setDomainTemplates([])
        setDomainTemplatesError(e instanceof Error ? e.message : String(e))
        setDomainTemplatesLoading(false)
      })
  }, [incognito])

  useEffect(() => {
    if (
      (!createOpen && !goalCreateOpen && !activeGoal) ||
      incognito ||
      domainTemplatesRequestedRef.current
    ) {
      return
    }
    loadDomainWorkflowTemplates()
  }, [activeGoal, createOpen, goalCreateOpen, incognito, loadDomainWorkflowTemplates])

  useEffect(() => {
    if (domainTemplates.length === 0) {
      if (selectedDomainTemplateId) setSelectedDomainTemplateId("")
      if (selectedDomainTaskType) setSelectedDomainTaskType("")
      return
    }
    const selected = findDomainTemplateByValue(domainTemplates, selectedDomainTemplateId)
    if (!selected) {
      const first = domainTemplates[0]
      setSelectedDomainTemplateId(domainTemplateOptionValue(first))
      setSelectedDomainTaskType(first.taskTypes[0] ?? "")
      return
    }
    if (selected.taskTypes.length === 0) {
      if (selectedDomainTaskType) setSelectedDomainTaskType("")
      return
    }
    if (!selected.taskTypes.includes(selectedDomainTaskType)) {
      setSelectedDomainTaskType(selected.taskTypes[0] ?? "")
    }
  }, [domainTemplates, selectedDomainTaskType, selectedDomainTemplateId])

  useEffect(() => {
    if (goalTemplateId === GOAL_DOMAIN_FREE_VALUE) {
      if (goalTaskType) setGoalTaskType("")
      return
    }
    const selected = findDomainTemplateByValue(domainTemplates, goalTemplateId)
    if (!selected) {
      setGoalTemplateId(GOAL_DOMAIN_FREE_VALUE)
      setGoalTaskType("")
      return
    }
    if (selected.taskTypes.length === 0) {
      if (goalTaskType) setGoalTaskType("")
      return
    }
    if (!selected.taskTypes.includes(goalTaskType)) {
      setGoalTaskType(selected.taskTypes[0] ?? "")
    }
  }, [domainTemplates, goalTaskType, goalTemplateId])

  useEffect(() => {
    if (!activeGoal?.workflowTemplateId || domainTemplates.length === 0 || domainDraft) return
    const template = findDomainTemplateByValue(domainTemplates, goalDomainTemplateValue(activeGoal))
    if (!template) return
    const templateValue = domainTemplateOptionValue(template)
    if (selectedDomainTemplateId === templateValue) return
    setSelectedDomainTemplateId(templateValue)
    setSelectedDomainTaskType(activeGoal.workflowTaskType || template.taskTypes[0] || "")
    setDraftKind(`domain:${template.domain}`)
    setDraftMode(normalizeExecutionMode(template.defaultMode))
  }, [
    activeGoal?.workflowTaskType,
    activeGoal?.workflowTemplateId,
    activeGoal?.workflowTemplateVersion,
    domainDraft,
    domainTemplates,
    selectedDomainTemplateId,
  ])

  useEffect(() => {
    if (runs.length === 0) {
      setSelectedRunId(null)
      setSnapshot(null)
      autoDetailTabRunRef.current = null
      return
    }
    if (selectedRunId && runs.some((run) => run.id === selectedRunId)) return
    const live = runs.find(
      (run) => workflowRunIsLive(run.state) || run.state === "awaiting_approval",
    )
    setSelectedRunId((live ?? runs[0]).id)
  }, [runs, selectedRunId])

  useEffect(() => {
    if (!focusedRunTarget?.runId) return
    if (!runs.some((run) => run.id === focusedRunTarget.runId)) return
    setSelectedRunId(focusedRunTarget.runId)
    if (!runs.slice(0, WORKFLOW_RUN_PREVIEW).some((run) => run.id === focusedRunTarget.runId)) {
      setShowAllRuns(true)
    }
  }, [focusedRunTarget?.nonce, focusedRunTarget?.runId, runs])

  useEffect(() => {
    if (runs.length <= WORKFLOW_RUN_PREVIEW && showAllRuns) {
      setShowAllRuns(false)
    }
  }, [runs.length, showAllRuns])

  const loadSnapshot = useCallback((runId: string) => {
    const req = ++snapshotReqRef.current
    setSnapshotLoading(true)
    getTransport()
      .call<WorkflowRunSnapshot | null>("get_workflow_run", { runId })
      .then((next) => {
        if (snapshotReqRef.current !== req) return
        const validNext =
          next &&
          typeof next === "object" &&
          typeof (next as { run?: { id?: unknown } }).run?.id === "string"
            ? next
            : null
        setSnapshot(validNext)
        if (validNext && autoDetailTabRunRef.current !== validNext.run.id) {
          setDetailTab(workflowInitialDetailTab(validNext))
          autoDetailTabRunRef.current = validNext.run.id
        }
        setSnapshotLoading(false)
      })
      .catch((e) => {
        if (snapshotReqRef.current !== req) return
        logger.error(
          "ui",
          "WorkflowRunsSection::loadSnapshot",
          "Failed to load workflow snapshot",
          e,
        )
        setSnapshot(null)
        setSnapshotLoading(false)
      })
  }, [])

  const focusWorkflowRun = useCallback(
    (runId: string) => {
      setSelectedRunId(runId)
      if (!runs.slice(0, WORKFLOW_RUN_PREVIEW).some((run) => run.id === runId)) {
        setShowAllRuns(true)
      }
      loadSnapshot(runId)
    },
    [loadSnapshot, runs],
  )

  const loadWorkflowMode = useCallback(() => {
    if (!sessionId || incognito) {
      workflowModeReqRef.current += 1
      setWorkflowMode("off")
      setWorkflowModeLoading(false)
      setWorkflowModeSaving(null)
      return
    }
    const req = ++workflowModeReqRef.current
    setWorkflowModeLoading(true)
    getTransport()
      .call<unknown>("get_workflow_mode", { sessionId })
      .then((next) => {
        if (workflowModeReqRef.current !== req) return
        setWorkflowMode(normalizeWorkflowAutonomyMode(next))
        setWorkflowModeLoading(false)
      })
      .catch((e) => {
        if (workflowModeReqRef.current !== req) return
        logger.error(
          "ui",
          "WorkflowRunsSection::loadWorkflowMode",
          "Failed to load workflow mode",
          e,
        )
        setWorkflowModeLoading(false)
      })
  }, [incognito, sessionId])

  const loadExecutionMode = useCallback(() => {
    if (!sessionId || incognito) {
      executionModeReqRef.current += 1
      setExecutionMode("off")
      setExecutionModeLoading(false)
      setExecutionModeSaving(null)
      return
    }
    const req = ++executionModeReqRef.current
    setExecutionModeLoading(true)
    getTransport()
      .call<unknown>("get_execution_mode", { sessionId })
      .then((next) => {
        if (executionModeReqRef.current !== req) return
        setExecutionMode(normalizeExecutionMode(next))
        setExecutionModeLoading(false)
      })
      .catch((e) => {
        if (executionModeReqRef.current !== req) return
        logger.error(
          "ui",
          "WorkflowRunsSection::loadExecutionMode",
          "Failed to load execution mode",
          e,
        )
        setExecutionModeLoading(false)
      })
  }, [incognito, sessionId])

  useEffect(() => {
    if (!selectedRunId || incognito) {
      snapshotReqRef.current += 1
      setSnapshot(null)
      setSnapshotLoading(false)
      return
    }
    loadSnapshot(selectedRunId)
  }, [incognito, loadSnapshot, selectedRun?.state, selectedRun?.updatedAt, selectedRunId])

  useEffect(() => {
    loadWorkflowMode()
  }, [loadWorkflowMode])

  useEffect(() => {
    const onWorkflowModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ sessionId?: string | null; mode?: unknown }>).detail
      if (!detail || detail.sessionId !== sessionId) return
      setWorkflowMode(normalizeWorkflowAutonomyMode(detail.mode))
      setWorkflowModeLoading(false)
      setWorkflowModeSaving(null)
    }
    window.addEventListener(WORKFLOW_MODE_CHANGED_EVENT, onWorkflowModeChanged)
    return () => window.removeEventListener(WORKFLOW_MODE_CHANGED_EVENT, onWorkflowModeChanged)
  }, [sessionId])

  useEffect(() => {
    loadExecutionMode()
  }, [loadExecutionMode])

  const updateWorkflowMode = useCallback(
    async (nextMode: WorkflowAutonomyMode) => {
      if (incognito || nextMode === workflowMode || workflowModeSaving) return
      const targetSessionId = sessionId ?? (await ensureWorkflowSession())
      if (!targetSessionId) return
      setWorkflowModeSaving(nextMode)
      try {
        const next = await getTransport().call<unknown>("set_workflow_mode", {
          sessionId: targetSessionId,
          mode: nextMode,
        })
        const saved = normalizeWorkflowAutonomyMode(next)
        setWorkflowMode(saved)
        window.dispatchEvent(
          new CustomEvent(WORKFLOW_MODE_CHANGED_EVENT, {
            detail: { sessionId: targetSessionId, mode: saved },
          }),
        )
        toast.success(
          t("workspace.workflow.modeSaved", "工作流模式已切换为 {{mode}}，模型下一轮会感知", {
            mode: workflowAutonomyModeLabel(t, saved),
          }),
        )
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::updateWorkflowMode",
          "Failed to update workflow mode",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setWorkflowModeSaving(null)
      }
    },
    [ensureWorkflowSession, incognito, sessionId, t, workflowMode, workflowModeSaving],
  )

  const updateExecutionMode = useCallback(
    async (nextMode: ExecutionMode) => {
      if (incognito || nextMode === executionMode || executionModeSaving) return
      const targetSessionId = sessionId ?? (await ensureWorkflowSession())
      if (!targetSessionId) return
      setExecutionModeSaving(nextMode)
      try {
        const next = await getTransport().call<unknown>("set_execution_mode", {
          sessionId: targetSessionId,
          mode: nextMode,
        })
        const saved = normalizeExecutionMode(next)
        setExecutionMode(saved)
        toast.success(
          t("workspace.workflow.executionModeSaved", "Execution mode 已切换为 {{mode}}", {
            mode: executionModeLabel(t, saved),
          }),
        )
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::updateExecutionMode",
          "Failed to update execution mode",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setExecutionModeSaving(null)
      }
    },
    [ensureWorkflowSession, incognito, executionMode, executionModeSaving, sessionId, t],
  )

  const createGoalFromDraft = useCallback(async () => {
    if (incognito) return
    const objective = goalObjective.trim()
    if (!objective) {
      toast.error(t("workspace.goal.objectiveRequired", "请输入目标"))
      return
    }
    const targetSessionId = await ensureWorkflowSession()
    if (!targetSessionId) return
    setGoalSaving(true)
    try {
      const snapshot = await getTransport().call<GoalSnapshot>("create_goal", {
        sessionId: targetSessionId,
        objective,
        completionCriteria: goalCriteria.trim(),
        domain: selectedGoalTemplate?.domain ?? undefined,
        workflowTemplateId: selectedGoalTemplate?.id ?? undefined,
        workflowTemplateVersion: selectedGoalTemplate?.version ?? undefined,
        workflowTaskType: selectedGoalTemplate
          ? goalTaskType || selectedGoalTemplate.taskTypes[0] || undefined
          : undefined,
      })
      goalState.setSnapshot(snapshot)
      if (selectedGoalTemplate) {
        setSelectedDomainTemplateId(domainTemplateOptionValue(selectedGoalTemplate))
        setSelectedDomainTaskType(goalTaskType || selectedGoalTemplate.taskTypes[0] || "")
        setDraftKind(`domain:${selectedGoalTemplate.domain}`)
        setDraftMode(normalizeExecutionMode(selectedGoalTemplate.defaultMode))
      }
      setGoalObjective("")
      setGoalCriteria("")
      setGoalTemplateId(GOAL_DOMAIN_FREE_VALUE)
      setGoalTaskType("")
      setGoalCreateOpen(false)
      toast.success(t("workspace.goal.created", "已创建 Goal"))
    } catch (e) {
      logger.error("ui", "WorkflowRunsSection::createGoal", "Failed to create goal", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setGoalSaving(false)
    }
  }, [
    ensureWorkflowSession,
    goalCriteria,
    goalObjective,
    goalState,
    goalTaskType,
    incognito,
    selectedGoalTemplate,
    t,
  ])

  const runGoalAction = useCallback(
    async (command: "pause_goal" | "resume_goal" | "clear_goal" | "evaluate_goal") => {
      if (!activeGoal) return
      const key = `${command}:${activeGoal.id}`
      setGoalActionKey(key)
      try {
        const snapshot = await getTransport().call<GoalSnapshot>(command, { goalId: activeGoal.id })
        goalState.setSnapshot(command === "clear_goal" ? null : snapshot)
        if (command === "clear_goal") {
          toast.success(t("workspace.goal.cleared", "Goal 已清除"))
        } else if (command === "evaluate_goal") {
          toast.success(t("workspace.goal.evaluated", "Goal audit 已更新"))
        } else {
          toast.success(t("workspace.goal.updated", "Goal 状态已更新"))
        }
        goalState.refresh()
      } catch (e) {
        logger.error("ui", "WorkflowRunsSection::goalAction", `Goal action failed: ${command}`, e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setGoalActionKey(null)
      }
    },
    [activeGoal, goalState, t],
  )

  const updateActiveGoal = useCallback(
    async (
      objective: string,
      completionCriteria: string,
      domainSelection?: {
        template: DomainWorkflowTemplate | null
        taskType: string
      },
    ) => {
      if (!activeGoal) return false
      const trimmedObjective = objective.trim()
      if (!trimmedObjective) {
        toast.error(t("workspace.goal.objectiveRequired", "请输入目标"))
        return false
      }
      const key = `update_goal:${activeGoal.id}`
      setGoalActionKey(key)
      try {
        const payload: Record<string, unknown> = {
          goalId: activeGoal.id,
          objective: trimmedObjective,
          completionCriteria: completionCriteria.trim(),
        }
        if (domainSelection) {
          payload.domain = domainSelection.template?.domain ?? ""
          payload.workflowTemplateId = domainSelection.template?.id ?? ""
          payload.workflowTemplateVersion = domainSelection.template?.version ?? ""
          payload.workflowTaskType =
            domainSelection.taskType || domainSelection.template?.taskTypes[0] || ""
        }
        const snapshot = await getTransport().call<GoalSnapshot>("update_goal", payload)
        goalState.setSnapshot(snapshot)
        if (domainSelection?.template) {
          setSelectedDomainTemplateId(domainTemplateOptionValue(domainSelection.template))
          setSelectedDomainTaskType(
            domainSelection.taskType || domainSelection.template.taskTypes[0] || "",
          )
          setDraftKind(`domain:${domainSelection.template.domain}`)
          setDraftMode(normalizeExecutionMode(domainSelection.template.defaultMode))
        }
        toast.success(t("workspace.goal.updated", "Goal 状态已更新"))
        goalState.refresh()
        return true
      } catch (e) {
        logger.error("ui", "WorkflowRunsSection::updateGoal", "Failed to update goal", e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      } finally {
        setGoalActionKey(null)
      }
    },
    [activeGoal, goalState, t],
  )

  const clearDraftPreview = useCallback(() => {
    previewReqRef.current += 1
    setDraftPreview(null)
    setDraftPreviewError(null)
    setDraftPreviewLoading(false)
  }, [])

  const clearDomainDraft = useCallback(() => {
    domainDraftReqRef.current += 1
    setDomainDraft(null)
    setDomainDraftLoading(false)
  }, [])

  const selectDomainTemplate = useCallback(
    (templateValue: string) => {
      const template = findDomainTemplateByValue(domainTemplates, templateValue)
      setSelectedDomainTemplateId(templateValue)
      setSelectedDomainTaskType(template?.taskTypes[0] ?? "")
      if (template) {
        setDraftKind(`domain:${template.domain}`)
        setDraftMode(normalizeExecutionMode(template.defaultMode))
      }
      clearDomainDraft()
      clearDraftPreview()
    },
    [clearDomainDraft, clearDraftPreview, domainTemplates],
  )

  const selectDomainTaskType = useCallback(
    (taskType: string) => {
      setSelectedDomainTaskType(taskType)
      clearDomainDraft()
      clearDraftPreview()
    },
    [clearDomainDraft, clearDraftPreview],
  )

  const previewWorkflowScriptSource = useCallback(
    async (
      scriptSource: string,
      mode: ExecutionMode,
      toastMessages: { passed?: string; blocked?: string } = {},
    ) => {
      if (incognito) return null
      const script = scriptSource.trim()
      if (!script) {
        toast.error(t("workspace.workflow.scriptRequired", "请输入工作流脚本"))
        return null
      }
      const targetSessionId = await ensureWorkflowSession()
      if (!targetSessionId) return null
      const req = ++previewReqRef.current
      setDraftPreview(null)
      setDraftPreviewLoading(true)
      setDraftPreviewError(null)
      try {
        const preview = await getTransport().call<WorkflowScriptPreview>(
          "preview_workflow_script",
          {
            sessionId: targetSessionId,
            scriptSource: script,
            executionMode: mode,
          },
        )
        if (previewReqRef.current !== req) return null
        setDraftPreview(preview)
        if (preview.canCreate) {
          toast.success(toastMessages.passed ?? t("workspace.workflow.previewPassed", "预检通过"))
        } else {
          toast.error(toastMessages.blocked ?? t("workspace.workflow.previewBlocked", "预检未通过"))
        }
        return preview
      } catch (e) {
        if (previewReqRef.current !== req) return null
        logger.error(
          "ui",
          "WorkflowRunsSection::previewWorkflowDraft",
          "Failed to preview workflow script",
          e,
        )
        setDraftPreview(null)
        setDraftPreviewError(e instanceof Error ? e.message : String(e))
        toast.error(e instanceof Error ? e.message : String(e))
        return null
      } finally {
        if (previewReqRef.current === req) setDraftPreviewLoading(false)
      }
    },
    [ensureWorkflowSession, incognito, t],
  )

  const generateDomainWorkflowDraft = useCallback(async () => {
    if (incognito) return
    if (!selectedDomainTemplate) {
      toast.error(t("workspace.workflow.domainTemplateRequired", "请选择领域模板"))
      return
    }
    const objective = draftObjective.trim()
    if (!objective && !activeGoal) {
      toast.error(t("workspace.workflow.domainObjectiveRequired", "请输入目标，或先创建 Goal"))
      return
    }
    const targetSessionId = await ensureWorkflowSession()
    if (!targetSessionId) return

    const req = ++domainDraftReqRef.current
    previewReqRef.current += 1
    setDomainDraftLoading(true)
    setDomainDraft(null)
    setDraftPreview(null)
    setDraftPreviewError(null)
    setDraftPreviewLoading(false)
    try {
      const draft = await getTransport().call<DomainWorkflowDraft>("preview_domain_workflow", {
        templateId: selectedDomainTemplate.id,
        version: selectedDomainTemplate.version,
        sessionId: targetSessionId,
        goalId: activeGoal?.id ?? undefined,
        taskType: selectedDomainTaskType || undefined,
        objective: objective || undefined,
        modeOverride: draftMode,
      })
      if (domainDraftReqRef.current !== req) return
      const preview = draft.scriptPreview as unknown as WorkflowScriptPreview
      setDomainDraft(draft)
      setDraftKind(draft.workflowKind)
      setDraftMode(normalizeExecutionMode(draft.executionMode))
      setDraftScript(draft.scriptSource)
      setDraftPreview(preview)
      setDraftPreviewError(null)
      setDraftRunImmediately(Boolean(workingDir))
      setDraftOrigin(null)
      if (preview.canCreate) {
        toast.success(t("workspace.workflow.domainDraftReady", "领域工作流草稿已生成并通过预检"))
      } else {
        toast.error(
          t("workspace.workflow.domainDraftBlocked", "领域工作流草稿已生成，但预检未通过"),
        )
      }
    } catch (e) {
      if (domainDraftReqRef.current !== req) return
      logger.error(
        "ui",
        "WorkflowRunsSection::generateDomainWorkflowDraft",
        "Failed to generate domain workflow draft",
        e,
      )
      setDomainDraft(null)
      setDraftPreview(null)
      setDraftPreviewError(e instanceof Error ? e.message : String(e))
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      if (domainDraftReqRef.current === req) setDomainDraftLoading(false)
    }
  }, [
    activeGoal,
    draftMode,
    draftObjective,
    ensureWorkflowSession,
    incognito,
    selectedDomainTaskType,
    selectedDomainTemplate,
    t,
    workingDir,
  ])

  const generateGoalDrivenDraft = useCallback(() => {
    const objective = draftObjective.trim()
    if (!objective) {
      toast.error(t("workspace.workflow.objectiveRequired", "请输入要完成的目标"))
      return
    }
    setDraftKind(WORKFLOW_KIND_DEFAULT)
    setDraftScript(buildGoalDrivenWorkflowScript(objective))
    setDraftRunImmediately(Boolean(workingDir))
    setDraftOrigin(null)
    clearDomainDraft()
    clearDraftPreview()
    if (workingDir) {
      toast.success(t("workspace.workflow.objectiveDraftReady", "已生成目标驱动工作流草稿"))
    } else {
      toast.warning(
        t(
          "workspace.workflow.objectiveDraftNeedsWorkspace",
          "已生成草稿；设置工作目录后再运行更稳妥",
        ),
      )
    }
  }, [clearDomainDraft, clearDraftPreview, draftObjective, t, workingDir])

  const generateRepairDraft = useCallback(
    (repairPrompt: string, run: WorkflowRun) => {
      const sourceMode = normalizeExecutionMode(run.executionMode)
      const nextMode = sourceMode === "off" ? "guarded" : sourceMode
      const objective = `继续修复失败的工作流运行 ${run.id}。

${repairPrompt}`
      setCreateOpen(true)
      setDraftKind(WORKFLOW_KIND_DEFAULT)
      setDraftMode(nextMode)
      setDraftObjective(objective)
      setDraftOrigin({
        type: "repair",
        runId: run.id,
        runKind: run.kind,
        runState: run.state,
      })
      clearDomainDraft()
      setDraftWorktreeMode(run.worktreeId ?? "session")
      const script = buildGoalDrivenWorkflowScript(objective)
      setDraftScript(script)
      setDraftRunImmediately(Boolean(workingDir || run.worktreeId))
      if (workingDir || run.worktreeId) {
        toast.success(t("workspace.workflow.repairDraftReady", "已生成修复工作流草稿"))
      } else {
        toast.warning(
          t(
            "workspace.workflow.repairDraftNeedsWorkspace",
            "已生成修复草稿；设置工作目录后再运行更稳妥",
          ),
        )
      }
      void previewWorkflowScriptSource(script, nextMode, {
        passed: t("workspace.workflow.repairDraftPreviewPassed", "修复草稿预检通过"),
        blocked: t("workspace.workflow.repairDraftPreviewBlocked", "修复草稿预检未通过"),
      })
    },
    [clearDomainDraft, previewWorkflowScriptSource, t, workingDir],
  )

  const previewWorkflowDraft = useCallback(async () => {
    await previewWorkflowScriptSource(draftScript, draftMode)
  }, [draftMode, draftScript, previewWorkflowScriptSource])

  const createWorkflow = useCallback(async () => {
    if (incognito) return
    const script = draftScript.trim()
    if (!script) {
      toast.error(t("workspace.workflow.scriptRequired", "请输入工作流脚本"))
      return
    }
    if (!draftPreview?.canCreate) {
      toast.error(t("workspace.workflow.previewRequired", "请先完成预检并修复阻塞项"))
      return
    }
    const targetSessionId = await ensureWorkflowSession()
    if (!targetSessionId) return
    setCreateSaving(true)
    try {
      let worktreeId: string | undefined
      if (normalizedDraftWorktreeMode === "new") {
        if (!workingDir) {
          toast.error(
            t("workspace.workflow.worktreeNeedsWorkspace", "先设置工作目录再创建隔离工作树"),
          )
          return
        }
        const worktree = await getTransport().call<ManagedWorktree>("create_managed_worktree", {
          sessionId: targetSessionId,
          sourceWorkingDir: workingDir,
          label: draftKind.trim() || WORKFLOW_KIND_DEFAULT,
          purpose: "workflow",
        })
        worktreeId = worktree.id
        managedWorktreesState.refresh()
      } else if (normalizedDraftWorktreeMode !== "session") {
        worktreeId = normalizedDraftWorktreeMode
      }
      const runImmediatelyForCreate = Boolean(workingDir || worktreeId) && draftRunImmediately
      const createArgs: Record<string, unknown> = {
        sessionId: targetSessionId,
        kind: draftKind.trim() || WORKFLOW_KIND_DEFAULT,
        executionMode: draftMode,
        scriptSource: script,
        budget: workflowBudgetForMode(draftMode),
        parentRunId: draftOrigin?.type === "repair" ? draftOrigin.runId : undefined,
        origin: draftOrigin?.type === "repair" ? "repair" : undefined,
        goalId: activeGoal?.id ?? undefined,
        runImmediately: runImmediatelyForCreate,
      }
      if (worktreeId) createArgs.worktreeId = worktreeId
      const run = await getTransport().call<WorkflowRun>("create_workflow_run", createArgs)
      setSelectedRunId(run.id)
      loadSnapshot(run.id)
      refresh()
      toast.success(
        draftOrigin?.type === "repair"
          ? runImmediatelyForCreate
            ? t("workspace.workflow.repairCreatedAndStarted", "已创建修复工作流并请求启动")
            : t("workspace.workflow.repairCreated", "已创建修复工作流")
          : runImmediatelyForCreate
            ? t("workspace.workflow.createdAndStarted", "已创建工作流并请求启动")
            : t("workspace.workflow.created", "已创建工作流"),
      )
      setCreateOpen(false)
      setDraftOrigin(null)
      clearDomainDraft()
      clearDraftPreview()
    } catch (e) {
      logger.error("ui", "WorkflowRunsSection::createWorkflow", "Failed to create workflow", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setCreateSaving(false)
    }
  }, [
    clearDraftPreview,
    clearDomainDraft,
    draftKind,
    draftMode,
    draftOrigin?.runId,
    draftOrigin?.type,
    draftPreview?.canCreate,
    draftRunImmediately,
    draftScript,
    ensureWorkflowSession,
    activeGoal?.id,
    incognito,
    loadSnapshot,
    managedWorktreesState,
    normalizedDraftWorktreeMode,
    refresh,
    t,
    workingDir,
  ])

  const runAction = useCallback(
    async (run: WorkflowRun, command: string, label: string) => {
      const key = `${command}:${run.id}`
      setActionKey(key)
      try {
        await getTransport().call<WorkflowRun>(command, { runId: run.id })
        toast.success(label)
        refresh()
        if (selectedRunId === run.id) {
          loadSnapshot(run.id)
        }
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::runAction",
          `Workflow action failed: ${command}`,
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setActionKey(null)
      }
    },
    [loadSnapshot, refresh, selectedRunId],
  )

  const requestRunAction = useCallback(
    (run: WorkflowRun, action: WorkflowRunActionSpec) => {
      if (action.command === "cancel_workflow_run") {
        setPendingCancelRun(run)
        return
      }
      void runAction(run, action.command, action.success)
    },
    [runAction],
  )

  const renderActions = (run: WorkflowRun) => {
    const actions = workflowRunActionSpecs(t, run.state)
    if (actions.length === 0) return null
    return (
      <div className="flex shrink-0 items-center gap-1">
        {actions.map((action) => {
          const Icon = action.icon
          const key = `${action.command}:${run.id}`
          return (
            <IconTip key={action.command} label={action.label}>
              <button
                type="button"
                className={cn(
                  "inline-flex h-6 w-6 items-center justify-center rounded-md border border-border/50 text-muted-foreground transition-colors hover:bg-secondary/65 hover:text-foreground disabled:opacity-50",
                  action.danger && "hover:border-destructive/50 hover:text-destructive",
                )}
                disabled={!!actionKey}
                onClick={(e) => {
                  e.stopPropagation()
                  requestRunAction(run, action)
                }}
                aria-label={action.label}
              >
                {actionKey === key ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <Icon className="h-3 w-3" />
                )}
              </button>
            </IconTip>
          )
        })}
      </div>
    )
  }

  const renderDetailActions = (run: WorkflowRun) => {
    const actions = workflowRunActionSpecs(t, run.state)
    if (actions.length === 0) return null
    return (
      <div className="grid grid-cols-2 gap-1.5">
        {actions.map((action) => {
          const Icon = action.icon
          const key = `${action.command}:${run.id}`
          const busy = actionKey === key
          return (
            <Button
              key={action.command}
              type="button"
              size="sm"
              variant={action.primary ? "default" : "outline"}
              className={cn(
                "h-8 min-w-0 gap-1.5 text-xs",
                action.danger && "border-destructive/35 text-destructive hover:text-destructive",
                actions.length === 1 && "col-span-2",
              )}
              disabled={!!actionKey}
              onClick={() => requestRunAction(run, action)}
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Icon className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{action.label}</span>
            </Button>
          )
        })}
      </div>
    )
  }

  const pendingCancelCurrentRun = pendingCancelRun
    ? (runs.find((run) => run.id === pendingCancelRun.id) ?? pendingCancelRun)
    : null
  const pendingCancelAction = pendingCancelCurrentRun
    ? workflowRunActionSpecs(t, pendingCancelCurrentRun.state).find(
        (action) => action.command === "cancel_workflow_run",
      )
    : null
  const pendingCancelKey = pendingCancelCurrentRun
    ? `cancel_workflow_run:${pendingCancelCurrentRun.id}`
    : null

  const latestEvent = snapshot?.events.at(-1)
  const detailRun = snapshot?.run ?? selectedRun
  const validationCount = snapshot?.ops.filter((op) => op.opType === "validate").length ?? 0
  const agentCount = snapshot?.ops.filter((op) => op.opType === "spawnAgent").length ?? 0

  return (
    <>
      <WorkspaceSection
        title={t("workspace.workflow.title", "工作流")}
        count={activeCount}
        icon={GitPullRequest}
        meta={
          loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null
        }
      >
        {incognito ? (
          <EmptyHint>{t("workspace.workflow.incognito", "无痕会话不持久化工作流")}</EmptyHint>
        ) : (
          <div className="space-y-2">
            <AutonomousReadinessCard
              activeGoal={activeGoal}
              workflowMode={workflowMode}
              workflowModeLoading={workflowModeLoading}
              workflowModeSaving={workflowModeSaving}
              executionMode={executionMode}
              executionModeLoading={executionModeLoading}
              executionModeSaving={executionModeSaving}
              workflowRunsState={workflowRunsState ?? ownedWorkflowRuns}
              loopSchedulesState={loopSchedulesState}
              domainWorkbenchState={domainWorkbenchState}
              incognito={incognito}
              canCreateLoop={Boolean(sessionId) && !incognito}
              onChangeWorkflowMode={(mode) => void updateWorkflowMode(mode)}
              onChangeExecutionMode={(mode) => void updateExecutionMode(mode)}
              onOpenGoalCreate={() => setGoalCreateOpen(true)}
              onOpenGoalEdit={() => setGoalEditRequest((value) => value + 1)}
              onOpenLoopCreate={onOpenLoopCreate}
              onOpenWorkflowProblem={focusWorkflowRun}
              onOpenLoopProblem={onOpenLoopProblem}
              onOpenOperationalGate={() => onOpenDomainWorkbench?.("operational")}
              onOpenSoakReport={() => onOpenDomainWorkbench?.("soak")}
            />
            <WorkflowAutonomyModeControl
              mode={workflowMode}
              loading={workflowModeLoading}
              saving={workflowModeSaving}
              onChange={(mode) => void updateWorkflowMode(mode)}
            />
            <WorkflowExecutionModeControl
              mode={executionMode}
              loading={executionModeLoading}
              saving={executionModeSaving}
              onChange={(mode) => void updateExecutionMode(mode)}
            />
            <GoalControlStrip
              snapshot={goalState.snapshot}
              loading={goalState.loading}
              error={goalState.error}
              createOpen={goalCreateOpen}
              objective={goalObjective}
              criteria={goalCriteria}
              domainTemplates={domainTemplates}
              domainTemplatesLoading={domainTemplatesLoading}
              domainTemplatesError={domainTemplatesError}
              selectedTemplate={selectedGoalTemplate}
              selectedTaskType={goalTaskType}
              saving={goalSaving}
              actionKey={goalActionKey}
              disabled={!canMaterializeSession}
              editRequest={goalEditRequest}
              onCreateOpenChange={setGoalCreateOpen}
              onObjectiveChange={setGoalObjective}
              onCriteriaChange={setGoalCriteria}
              onReloadDomainTemplates={loadDomainWorkflowTemplates}
              onTemplateChange={setGoalTemplateId}
              onTaskTypeChange={setGoalTaskType}
              onCreate={() => void createGoalFromDraft()}
              onPause={() => void runGoalAction("pause_goal")}
              onResume={() => void runGoalAction("resume_goal")}
              onClear={() => void runGoalAction("clear_goal")}
              onEvaluate={() => void runGoalAction("evaluate_goal")}
              onUpdate={updateActiveGoal}
            />
            <WorkflowCreateComposer
              open={createOpen}
              disabled={!canMaterializeSession}
              disabledReason={
                !canMaterializeSession
                  ? t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建工作流")
                  : !sessionId
                    ? t(
                        "workspace.workflow.sessionAutoCreateHint",
                        "预检时会自动创建并切换到一个新会话",
                      )
                    : null
              }
              workspaceReady={!!workingDir}
              saving={createSaving}
              preview={draftPreview}
              previewLoading={draftPreviewLoading}
              previewError={draftPreviewError}
              kind={draftKind}
              mode={draftMode}
              objective={draftObjective}
              script={draftScript}
              draftOrigin={draftOrigin}
              linkedGoal={activeGoal}
              domainTemplates={domainTemplates}
              domainTemplatesLoading={domainTemplatesLoading}
              domainTemplatesError={domainTemplatesError}
              selectedDomainTemplate={selectedDomainTemplate}
              selectedDomainTaskType={selectedDomainTaskType}
              domainDraft={domainDraft}
              domainDraftLoading={domainDraftLoading}
              runImmediately={draftRunImmediately}
              worktrees={draftWorktrees}
              worktreeMode={normalizedDraftWorktreeMode}
              worktreeLoading={managedWorktreesState.loading}
              onOpenChange={setCreateOpen}
              onKindChange={setDraftKind}
              onModeChange={(mode) => {
                setDraftMode(mode)
                clearDomainDraft()
                clearDraftPreview()
              }}
              onScriptChange={(script) => {
                setDraftScript(script)
                clearDomainDraft()
                clearDraftPreview()
              }}
              onObjectiveChange={(objective) => {
                setDraftObjective(objective)
                clearDomainDraft()
                clearDraftPreview()
              }}
              onClearDraftOrigin={() => setDraftOrigin(null)}
              onReloadDomainTemplates={loadDomainWorkflowTemplates}
              onDomainTemplateChange={selectDomainTemplate}
              onDomainTaskTypeChange={selectDomainTaskType}
              onGenerateDomainDraft={() => void generateDomainWorkflowDraft()}
              onRunImmediatelyChange={setDraftRunImmediately}
              onWorktreeModeChange={setDraftWorktreeMode}
              onGenerateGoalDraft={generateGoalDrivenDraft}
              onPreview={() => void previewWorkflowDraft()}
              onSubmit={() => void createWorkflow()}
            />

            {error ? (
              <EmptyHint>{error}</EmptyHint>
            ) : runs.length === 0 ? (
              <WorkflowEmptyState
                mode={executionMode}
                workspaceReady={!!workingDir}
                disabled={!canMaterializeSession}
                onCreate={() => setCreateOpen(true)}
              />
            ) : (
              <>
                <div className="space-y-1">
                  {visibleRuns.map((run) => {
                    const selected = run.id === selectedRunId
                    const rowBudget = workflowOutputBudget(
                      run,
                      selected ? (snapshot?.events ?? []) : [],
                    )
                    return (
                      <div
                        key={run.id}
                        className={cn(
                          "flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-secondary/45",
                          selected && "bg-secondary/45",
                        )}
                      >
                        <button
                          type="button"
                          className="flex min-w-0 flex-1 items-center gap-2 text-left"
                          onClick={() => setSelectedRunId(run.id)}
                        >
                          <GitPullRequest className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                          <span className="min-w-0 flex-1 truncate text-xs text-foreground/90">
                            {run.kind}
                            <span className="px-1 text-muted-foreground/50">·</span>
                            {run.executionMode}
                            {run.worktreeId ? (
                              <>
                                <span className="px-1 text-muted-foreground/50">·</span>
                                <span className="text-muted-foreground">
                                  {t("workspace.workflow.worktreeBadge", "worktree")}
                                </span>
                              </>
                            ) : null}
                            {rowBudget ? (
                              <>
                                <span className="px-1 text-muted-foreground/50">·</span>
                                <span className="text-muted-foreground">
                                  {t("workspace.workflow.outputBudget", "输出预算")}
                                </span>
                                <span className="pl-1 font-mono text-muted-foreground">
                                  {rowBudget.spent > 0
                                    ? `${compactCount(rowBudget.spent)}/${compactCount(rowBudget.limit)}`
                                    : compactCount(rowBudget.limit)}
                                </span>
                              </>
                            ) : null}
                          </span>
                          <StatusPill
                            label={workflowRunStateLabel(t, run.state)}
                            tone={workflowRunTone(run.state)}
                            loading={run.state === "running" || run.state === "recovering"}
                          />
                        </button>
                        {renderActions(run)}
                      </div>
                    )
                  })}
                  {runs.length > WORKFLOW_RUN_PREVIEW ? (
                    <button
                      type="button"
                      className="flex w-full items-center justify-between rounded-md px-2 py-1 text-[10px] text-muted-foreground/70 transition-colors hover:bg-secondary/45 hover:text-foreground"
                      aria-expanded={showAllRuns}
                      onClick={() => setShowAllRuns((value) => !value)}
                    >
                      <span>
                        {showAllRuns
                          ? t("workspace.workflow.collapseRuns", "收起历史运行")
                          : t("workspace.workflow.moreRuns", "另有 {{count}} 个历史运行", {
                              count: runs.length - WORKFLOW_RUN_PREVIEW,
                            })}
                      </span>
                      {showAllRuns ? (
                        <ChevronUp className="h-3 w-3" />
                      ) : (
                        <ChevronDown className="h-3 w-3" />
                      )}
                    </button>
                  ) : null}
                </div>

                {detailRun ? (
                  <div className="space-y-1.5 border-t border-border/60 pt-2">
                    <WorkflowRunOverview
                      run={detailRun}
                      snapshot={snapshot}
                      latestEvent={latestEvent}
                      worktree={
                        detailRun.worktreeId
                          ? (managedWorktreesState.worktrees.find(
                              (worktree) => worktree.id === detailRun.worktreeId,
                            ) ?? null)
                          : null
                      }
                      actions={renderDetailActions(detailRun)}
                      onSelectDetailTab={setDetailTab}
                      onCreateRepairDraft={generateRepairDraft}
                    />

                    {snapshotLoading ? (
                      <div className="flex items-center justify-center gap-2 py-2 text-xs text-muted-foreground">
                        <Loader2 className="h-3 w-3 animate-spin" />
                        {t("workspace.workflow.loadingTrace", "加载 trace")}
                      </div>
                    ) : snapshot ? (
                      <div className="space-y-1.5">
                        <Tabs
                          value={detailTab}
                          onValueChange={(value) => setDetailTab(value as WorkflowDetailTab)}
                          className="space-y-1.5"
                        >
                          <TabsList className="grid h-8 w-full grid-cols-3">
                            <TabsTrigger value="trace" className="text-[11px]">
                              {t("workspace.workflow.tabTrace", "Trace")}
                            </TabsTrigger>
                            <TabsTrigger value="validation" className="text-[11px]">
                              {t("workspace.workflow.tabValidation", "Validation")}
                              {validationCount > 0 ? (
                                <span className="ml-1 text-[10px] text-muted-foreground">
                                  {validationCount}
                                </span>
                              ) : null}
                            </TabsTrigger>
                            <TabsTrigger value="agents" className="text-[11px]">
                              {t("workspace.workflow.tabAgents", "Agents")}
                              {agentCount > 0 ? (
                                <span className="ml-1 text-[10px] text-muted-foreground">
                                  {agentCount}
                                </span>
                              ) : null}
                            </TabsTrigger>
                          </TabsList>

                          <TabsContent value="trace" className="mt-0">
                            <WorkflowTraceTimeline snapshot={snapshot} />
                          </TabsContent>

                          <TabsContent value="validation" className="mt-0">
                            <WorkflowValidationTab snapshot={snapshot} />
                          </TabsContent>

                          <TabsContent value="agents" className="mt-0">
                            <WorkflowAgentsTab
                              snapshot={snapshot}
                              onViewSubagentSession={onViewSubagentSession}
                            />
                          </TabsContent>
                        </Tabs>
                      </div>
                    ) : (
                      <EmptyHint>{t("workspace.workflow.emptyTrace", "暂无 trace")}</EmptyHint>
                    )}
                  </div>
                ) : null}
              </>
            )}
          </div>
        )}
      </WorkspaceSection>
      <AlertDialog
        open={!!pendingCancelRun}
        onOpenChange={(open) => {
          if (!open) setPendingCancelRun(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("workspace.workflow.cancelConfirmTitle", "取消这个工作流运行？")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                "workspace.workflow.cancelConfirmBody",
                "会停止这个运行，并尽量取消它拥有的后台任务、验证命令和子 Agent；已有 trace 会保留，方便之后复盘或生成修复草稿。",
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {pendingCancelCurrentRun ? (
            <div className="rounded-md border border-border/55 bg-secondary/25 px-2.5 py-2 text-xs">
              <div className="flex min-w-0 items-center gap-2">
                <GitPullRequest className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">
                  {pendingCancelCurrentRun.kind}
                </span>
                <StatusPill
                  label={workflowRunStateLabel(t, pendingCancelCurrentRun.state)}
                  tone={workflowRunTone(pendingCancelCurrentRun.state)}
                />
              </div>
              <div className="mt-1 truncate text-[11px] text-muted-foreground">
                {pendingCancelCurrentRun.id}
              </div>
            </div>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pendingCancelKey ? actionKey === pendingCancelKey : false}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={
                !pendingCancelCurrentRun ||
                !pendingCancelAction ||
                (pendingCancelKey ? actionKey === pendingCancelKey : false)
              }
              onClick={(event) => {
                event.preventDefault()
                if (!pendingCancelCurrentRun || !pendingCancelAction) return
                const run = pendingCancelCurrentRun
                const action = pendingCancelAction
                setPendingCancelRun(null)
                void runAction(run, action.command, action.success)
              }}
            >
              {pendingCancelKey && actionKey === pendingCancelKey ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : null}
              {t("workspace.workflow.cancelConfirmAction", "确认取消")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function WorkflowEmptyState({
  mode,
  workspaceReady,
  disabled,
  onCreate,
}: {
  mode: ExecutionMode
  workspaceReady: boolean
  disabled?: boolean
  onCreate: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-md border border-dashed border-border/70 bg-secondary/15 p-2">
      <div className="flex min-w-0 items-center gap-2">
        <Sparkles className="h-4 w-4 shrink-0 text-primary" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
          {t("workspace.workflow.emptyTitle", "准备开始工作流运行")}
        </span>
        <StatusPill label={executionModeLabel(t, mode)} tone={mode === "off" ? "muted" : "info"} />
      </div>
      <div className="mt-2 grid grid-cols-2 gap-1 text-[10px]">
        <WorkflowMetric
          label={t("workspace.workflow.emptyMode", "模式")}
          value={executionModeLabel(t, mode)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.emptyWorkspace", "工作目录")}
          value={
            workspaceReady
              ? t("workspace.workflow.emptyWorkspaceReady", "已设置")
              : t("workspace.workflow.emptyWorkspaceDraftOnly", "草稿")
          }
        />
      </div>
      <Button
        type="button"
        size="sm"
        className="mt-2 h-8 w-full gap-1.5 text-xs"
        disabled={disabled}
        onClick={onCreate}
      >
        <Plus className="h-3.5 w-3.5" />
        <span className="truncate">{t("workspace.workflow.emptyCreate", "开始工作流运行")}</span>
      </Button>
    </div>
  )
}

function WorkflowCreateComposer({
  open,
  disabled,
  disabledReason,
  workspaceReady,
  saving,
  preview,
  previewLoading,
  previewError,
  kind,
  mode,
  objective,
  script,
  draftOrigin,
  linkedGoal,
  domainTemplates,
  domainTemplatesLoading,
  domainTemplatesError,
  selectedDomainTemplate,
  selectedDomainTaskType,
  domainDraft,
  domainDraftLoading,
  runImmediately,
  worktrees,
  worktreeMode,
  worktreeLoading,
  onOpenChange,
  onKindChange,
  onModeChange,
  onObjectiveChange,
  onScriptChange,
  onClearDraftOrigin,
  onReloadDomainTemplates,
  onDomainTemplateChange,
  onDomainTaskTypeChange,
  onGenerateDomainDraft,
  onRunImmediatelyChange,
  onWorktreeModeChange,
  onGenerateGoalDraft,
  onPreview,
  onSubmit,
}: {
  open: boolean
  disabled?: boolean
  disabledReason?: string | null
  workspaceReady?: boolean
  saving?: boolean
  preview: WorkflowScriptPreview | null
  previewLoading?: boolean
  previewError?: string | null
  kind: string
  mode: ExecutionMode
  objective: string
  script: string
  draftOrigin?: WorkflowDraftOrigin | null
  linkedGoal?: Goal | null
  domainTemplates: DomainWorkflowTemplate[]
  domainTemplatesLoading?: boolean
  domainTemplatesError?: string | null
  selectedDomainTemplate?: DomainWorkflowTemplate | null
  selectedDomainTaskType: string
  domainDraft?: DomainWorkflowDraft | null
  domainDraftLoading?: boolean
  runImmediately: boolean
  worktrees: ManagedWorktree[]
  worktreeMode: string
  worktreeLoading?: boolean
  onOpenChange: (open: boolean) => void
  onKindChange: (kind: string) => void
  onModeChange: (mode: ExecutionMode) => void
  onObjectiveChange: (objective: string) => void
  onScriptChange: (script: string) => void
  onClearDraftOrigin: () => void
  onReloadDomainTemplates: () => void
  onDomainTemplateChange: (templateId: string) => void
  onDomainTaskTypeChange: (taskType: string) => void
  onGenerateDomainDraft: () => void
  onRunImmediatelyChange: (checked: boolean) => void
  onWorktreeModeChange: (mode: string) => void
  onGenerateGoalDraft: () => void
  onPreview: () => void
  onSubmit: () => void
}) {
  const { t } = useTranslation()
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const worktreeBacked = worktreeMode !== "session"
  const effectiveRunImmediately = Boolean(workspaceReady || worktreeBacked) && runImmediately
  const canPreview =
    !disabled && !saving && !previewLoading && !domainDraftLoading && script.trim().length > 0
  const canSubmit =
    !disabled &&
    !saving &&
    !previewLoading &&
    !domainDraftLoading &&
    script.trim().length > 0 &&
    preview?.canCreate === true
  const canGenerate =
    !disabled && !saving && !previewLoading && !domainDraftLoading && objective.trim().length > 0
  const selectedDomainTask =
    selectedDomainTemplate?.taskTypes.find((taskType) => taskType === selectedDomainTaskType) ??
    selectedDomainTemplate?.taskTypes[0] ??
    ""
  const canGenerateDomain =
    !disabled &&
    !saving &&
    !previewLoading &&
    !domainDraftLoading &&
    Boolean(selectedDomainTemplate) &&
    (objective.trim().length > 0 || Boolean(linkedGoal))
  const repairOrigin = draftOrigin?.type === "repair" ? draftOrigin : null

  useEffect(() => {
    if (!open) setAdvancedOpen(false)
  }, [open])

  return (
    <div className="rounded-md border border-border/55 bg-background/35">
      <button
        type="button"
        className="flex w-full min-w-0 items-center gap-2 px-2 py-1.5 text-left text-xs transition-colors hover:bg-secondary/45 disabled:opacity-60"
        disabled={disabled}
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
      >
        <Plus className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">
          {t("workspace.workflow.createTitle", "新建工作流")}
        </span>
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
            open && "rotate-90",
          )}
        />
      </button>
      {disabledReason ? (
        <div className="border-t border-border/60 px-2 py-1.5 text-[11px] text-muted-foreground">
          {disabledReason}
        </div>
      ) : null}
      <AnimatedCollapse open={open}>
        <form
          className="space-y-2 border-t border-border/60 p-2"
          onSubmit={(event) => {
            event.preventDefault()
            if (canSubmit) onSubmit()
          }}
        >
          {repairOrigin ? (
            <div className="rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300">
              <div className="flex min-w-0 items-center gap-1.5 font-medium">
                <GitPullRequest className="h-3.5 w-3.5 shrink-0" />
                <span className="min-w-0 flex-1 truncate">
                  {t("workspace.workflow.repairDraftOrigin", "修复自 {{id}}", {
                    id: repairOrigin.runId,
                  })}
                </span>
                <StatusPill
                  label={workflowRunStateLabel(t, repairOrigin.runState)}
                  tone={workflowRunTone(repairOrigin.runState)}
                />
              </div>
              <div className="mt-0.5 truncate opacity-80">
                {linkedGoal
                  ? t(
                      "workspace.workflow.repairDraftGoalDetail",
                      "将创建同一 Goal 下的修复运行，不会覆盖原运行 · {{kind}}",
                      {
                        kind: repairOrigin.runKind,
                      },
                    )
                  : t(
                      "workspace.workflow.repairDraftOriginDetail",
                      "将创建新的修复运行，不会覆盖原运行 · {{kind}}",
                      {
                        kind: repairOrigin.runKind,
                      },
                    )}
              </div>
            </div>
          ) : null}

          <div className="space-y-1.5 rounded-md border border-primary/20 bg-primary/5 p-2">
            <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
              <Sparkles className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="truncate">
                {t("workspace.workflow.objectiveTitle", "从目标开始")}
              </span>
            </div>
            <Textarea
              value={objective}
              disabled={saving || previewLoading}
              onChange={(event) => onObjectiveChange(event.target.value)}
              placeholder={t(
                "workspace.workflow.objectivePlaceholder",
                "例如：调研多份发布说明并总结风险",
              )}
              className="min-h-20 resize-y text-xs"
              aria-label={t("workspace.workflow.objectiveTitle", "从目标开始")}
            />
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 w-full gap-1.5 text-xs"
              disabled={!canGenerate}
              onClick={onGenerateGoalDraft}
            >
              <Sparkles className="h-3.5 w-3.5" />
              <span className="truncate">
                {t("workspace.workflow.generateObjectiveDraft", "生成可预检草稿")}
              </span>
            </Button>
            {!workspaceReady ? (
              <div className="rounded-md bg-background/55 px-2 py-1.5 text-[11px] text-muted-foreground">
                {t(
                  "workspace.workflow.workspaceRequiredHint",
                  "当前会话未设置工作目录；目标草稿会先创建为待启动，设置目录后再运行。",
                )}
              </div>
            ) : null}
          </div>

          <div className="space-y-1.5 rounded-md border border-border/55 bg-secondary/15 p-2">
            <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
              <Layers className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1 truncate">
                {t("workspace.workflow.domainTemplateTitle", "领域模板")}
              </span>
              {domainTemplatesLoading ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
            </div>
            {domainTemplatesError ? (
              <div className="rounded-md border border-destructive/20 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
                <div className="flex min-w-0 items-center gap-1.5">
                  <CircleAlert className="h-3.5 w-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">
                    {truncateMiddle(domainTemplatesError, 120)}
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="h-6 px-2 text-[10px]"
                    disabled={saving || domainTemplatesLoading}
                    onClick={onReloadDomainTemplates}
                  >
                    <RefreshCw className="mr-1 h-3 w-3" />
                    {t("workspace.workflow.domainTemplateRetry", "重试")}
                  </Button>
                </div>
              </div>
            ) : domainTemplates.length === 0 && !domainTemplatesLoading ? (
              <div className="rounded-md bg-background/55 px-2 py-1.5 text-[11px] text-muted-foreground">
                {t("workspace.workflow.domainTemplateEmpty", "暂无可用领域模板")}
              </div>
            ) : (
              <>
                <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
                  <Select
                    value={
                      selectedDomainTemplate
                        ? domainTemplateOptionValue(selectedDomainTemplate)
                        : ""
                    }
                    onValueChange={onDomainTemplateChange}
                    disabled={
                      saving || previewLoading || domainDraftLoading || domainTemplates.length === 0
                    }
                  >
                    <SelectTrigger className="h-8 bg-background/60 text-xs">
                      <SelectValue
                        placeholder={t("workspace.workflow.domainTemplatePlaceholder", "选择模板")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {domainTemplates.map((template) => (
                        <SelectItem
                          key={`${template.id}:${template.version}`}
                          value={domainTemplateOptionValue(template)}
                        >
                          {template.title}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Select
                    value={selectedDomainTask}
                    onValueChange={onDomainTaskTypeChange}
                    disabled={
                      saving ||
                      previewLoading ||
                      domainDraftLoading ||
                      !selectedDomainTemplate ||
                      selectedDomainTemplate.taskTypes.length <= 1
                    }
                  >
                    <SelectTrigger className="h-8 bg-background/60 text-xs">
                      <SelectValue
                        placeholder={t("workspace.workflow.domainTaskTypePlaceholder", "任务类型")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {(selectedDomainTemplate?.taskTypes ?? []).map((taskType) => (
                        <SelectItem key={taskType} value={taskType}>
                          {taskType}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {selectedDomainTemplate ? (
                  <div className="space-y-1 rounded-md bg-background/55 px-2 py-1.5 text-[11px]">
                    <div className="flex min-w-0 flex-wrap items-center gap-1">
                      <StatusPill label={selectedDomainTemplate.domain} tone="info" />
                      <StatusPill
                        label={executionModeLabel(
                          t,
                          normalizeExecutionMode(selectedDomainTemplate.defaultMode),
                        )}
                        tone="muted"
                      />
                      <StatusPill
                        label={t("workspace.workflow.domainEvidenceCount", "{{count}} 证据", {
                          count: selectedDomainTemplate.requiredEvidence.length,
                        })}
                        tone="good"
                      />
                      {selectedDomainTemplate.approvalGates.length > 0 ? (
                        <StatusPill
                          label={t("workspace.workflow.domainGateCount", "{{count}} 审批", {
                            count: selectedDomainTemplate.approvalGates.length,
                          })}
                          tone="warn"
                        />
                      ) : null}
                    </div>
                    <p className="line-clamp-2 text-muted-foreground">
                      {selectedDomainTemplate.outputContract}
                    </p>
                  </div>
                ) : null}

                {domainDraft ? <WorkflowDomainDraftSummary draft={domainDraft} /> : null}

                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="h-8 w-full gap-1.5 text-xs"
                  disabled={!canGenerateDomain}
                  onClick={onGenerateDomainDraft}
                >
                  {domainDraftLoading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Layers className="h-3.5 w-3.5" />
                  )}
                  <span className="truncate">
                    {linkedGoal && objective.trim().length === 0
                      ? t(
                          "workspace.workflow.generateDomainDraftFromGoal",
                          "用当前 Goal 生成领域草稿",
                        )
                      : t("workspace.workflow.generateDomainDraft", "生成领域草稿")}
                  </span>
                </Button>
              </>
            )}
          </div>

          <div className="space-y-1">
            <label className="block text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.createKind", "Kind")}
            </label>
            <Input
              value={kind}
              disabled={saving || previewLoading}
              onChange={(event) => onKindChange(event.target.value)}
              className="h-8 text-xs"
              aria-label={t("workspace.workflow.createKind", "Kind")}
            />
          </div>

          <div className="space-y-1">
            <div className="text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.createMode", "Execution mode")}
            </div>
            <div className="grid grid-cols-2 gap-1">
              {WORKFLOW_MODE_OPTIONS.map((option) => {
                const Icon = option.icon
                const selected = option.mode === mode
                return (
                  <button
                    key={option.mode}
                    type="button"
                    className={cn(
                      "flex min-h-8 min-w-0 items-center gap-1.5 rounded-md border px-2 text-left text-[11px] transition-colors disabled:opacity-60",
                      selected
                        ? "border-primary/55 bg-primary/10 text-foreground"
                        : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
                    )}
                    disabled={saving || previewLoading}
                    aria-pressed={selected}
                    onClick={() => onModeChange(option.mode)}
                  >
                    <Icon className="h-3.5 w-3.5 shrink-0" />
                    <span className="min-w-0 truncate">{executionModeLabel(t, option.mode)}</span>
                  </button>
                )
              })}
            </div>
          </div>

          <div className="space-y-1">
            <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
              <FolderGit2 className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">
                {t("workspace.workflow.worktreeTarget", "运行位置")}
              </span>
              {worktreeLoading ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
            </div>
            <div className="grid grid-cols-2 gap-1">
              <button
                type="button"
                className={cn(
                  "flex min-h-8 min-w-0 items-center gap-1.5 rounded-md border px-2 text-left text-[11px] transition-colors disabled:opacity-60",
                  worktreeMode === "session"
                    ? "border-primary/55 bg-primary/10 text-foreground"
                    : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
                )}
                disabled={saving || previewLoading}
                aria-pressed={worktreeMode === "session"}
                onClick={() => onWorktreeModeChange("session")}
              >
                <FolderOpen className="h-3.5 w-3.5 shrink-0" />
                <span className="min-w-0 truncate">
                  {t("workspace.workflow.worktreeSession", "当前目录")}
                </span>
              </button>
              <button
                type="button"
                className={cn(
                  "flex min-h-8 min-w-0 items-center gap-1.5 rounded-md border px-2 text-left text-[11px] transition-colors disabled:opacity-60",
                  worktreeMode === "new"
                    ? "border-primary/55 bg-primary/10 text-foreground"
                    : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
                )}
                disabled={saving || previewLoading || !workspaceReady}
                aria-pressed={worktreeMode === "new"}
                onClick={() => onWorktreeModeChange("new")}
              >
                <Plus className="h-3.5 w-3.5 shrink-0" />
                <span className="min-w-0 truncate">
                  {t("workspace.workflow.worktreeNew", "新隔离工作树")}
                </span>
              </button>
            </div>
            {worktrees.length > 0 ? (
              <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
                {worktrees.slice(0, 4).map((worktree) => (
                  <button
                    key={worktree.id}
                    type="button"
                    className={cn(
                      "flex min-h-8 min-w-0 items-center gap-1.5 rounded-md border px-2 text-left text-[11px] transition-colors disabled:opacity-60",
                      worktreeMode === worktree.id
                        ? "border-primary/55 bg-primary/10 text-foreground"
                        : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
                    )}
                    disabled={saving || previewLoading}
                    aria-pressed={worktreeMode === worktree.id}
                    title={worktree.path}
                    onClick={() => onWorktreeModeChange(worktree.id)}
                  >
                    <FolderGit2 className="h-3.5 w-3.5 shrink-0" />
                    <span className="min-w-0 flex-1 truncate">
                      {worktree.label || basename(worktree.path)}
                    </span>
                    <StatusPill
                      label={managedWorktreeStateLabel(t, worktree.state)}
                      tone={managedWorktreeStateTone(worktree.state)}
                    />
                  </button>
                ))}
              </div>
            ) : null}
          </div>

          <div className="overflow-hidden rounded-md border border-border/55 bg-secondary/15">
            <button
              type="button"
              className="flex w-full min-w-0 items-center gap-2 px-2 py-1.5 text-left text-[11px] transition-colors hover:bg-secondary/45"
              aria-expanded={advancedOpen}
              onClick={() => setAdvancedOpen((value) => !value)}
            >
              <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate font-medium text-foreground/85">
                {t("workspace.workflow.advancedScript", "高级脚本")}
              </span>
              <span className="hidden min-w-0 max-w-[9rem] truncate text-[10px] text-muted-foreground sm:inline">
                {t("workspace.workflow.advancedScriptHint", "需要时再编辑 workflow.js")}
              </span>
              <ChevronRight
                className={cn(
                  "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
                  advancedOpen && "rotate-90",
                )}
              />
            </button>
            <AnimatedCollapse open={advancedOpen}>
              <div className="space-y-1 border-t border-border/60 p-2">
                <label className="block text-[10px] font-medium text-muted-foreground">
                  {t("workspace.workflow.createScript", "Script")}
                </label>
                <Textarea
                  value={script}
                  disabled={saving || previewLoading}
                  onChange={(event) => onScriptChange(event.target.value)}
                  placeholder={t(
                    "workspace.workflow.createScriptPlaceholder",
                    "Paste or edit workflow.js",
                  )}
                  className="min-h-44 resize-y font-mono text-[11px]"
                  aria-label={t("workspace.workflow.createScript", "Script")}
                  spellCheck={false}
                />
              </div>
            </AnimatedCollapse>
          </div>

          <div className="flex items-center justify-between gap-2 rounded-md bg-secondary/25 px-2 py-1.5">
            <label className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/85">
              {t("workspace.workflow.runImmediately", "创建后立即运行")}
            </label>
            <Switch
              checked={effectiveRunImmediately}
              disabled={saving || previewLoading || (!workspaceReady && !worktreeBacked)}
              onCheckedChange={onRunImmediatelyChange}
            />
          </div>

          <WorkflowScriptPreviewPanel
            preview={preview}
            loading={previewLoading}
            error={previewError}
          />

          <div className="flex gap-1.5">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 flex-1 gap-1.5 text-xs"
              disabled={saving || previewLoading || domainDraftLoading}
              onClick={() => {
                onKindChange(WORKFLOW_KIND_DEFAULT)
                onModeChange("guarded")
                onObjectiveChange("")
                onScriptChange(WORKFLOW_SCRIPT_TEMPLATE)
                onClearDraftOrigin()
              }}
            >
              <Copy className="h-3.5 w-3.5" />
              {t("workspace.workflow.resetTemplate", "恢复模板")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 flex-1 gap-1.5 text-xs"
              disabled={!canPreview}
              onClick={onPreview}
            >
              {previewLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <CheckCircle2 className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.workflow.preview", "预检")}</span>
            </Button>
            <Button
              type="submit"
              size="sm"
              className="h-8 flex-1 gap-1.5 text-xs"
              disabled={!canSubmit}
            >
              {saving ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Play className="h-3.5 w-3.5" />
              )}
              <span className="truncate">
                {repairOrigin
                  ? effectiveRunImmediately
                    ? t("workspace.workflow.createRepairAndRun", "创建并运行修复")
                    : t("workspace.workflow.createRepair", "创建修复运行")
                  : effectiveRunImmediately
                    ? t("workspace.workflow.createAndRun", "创建并运行")
                    : t("workspace.workflow.create", "创建")}
              </span>
            </Button>
          </div>
        </form>
      </AnimatedCollapse>
    </div>
  )
}

function WorkflowDomainDraftSummary({ draft }: { draft: DomainWorkflowDraft }) {
  const { t } = useTranslation()
  const evidence = draft.requiredEvidence.slice(0, 4)
  const gates = draft.approvalGates.slice(0, 3)
  const rules = draft.verificationPolicy.slice(0, 3)
  const warnings = draft.warnings.slice(0, 3)

  return (
    <div className="space-y-1.5 rounded-md border border-primary/20 bg-primary/5 p-2 text-[11px]">
      <div className="flex min-w-0 items-center gap-1.5">
        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-primary" />
        <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">
          {t("workspace.workflow.domainDraftSummary", "草稿来自 {{title}}", {
            title: draft.template.title,
          })}
        </span>
        <StatusPill label={draft.workflowKind} tone="info" />
      </div>

      {evidence.length > 0 ? (
        <WorkflowDomainDraftRow
          icon={ClipboardCheck}
          label={t("workspace.workflow.domainDraftEvidence", "证据")}
          values={evidence.map(domainEvidenceRequirementLabel)}
        />
      ) : null}
      {gates.length > 0 ? (
        <WorkflowDomainDraftRow
          icon={ShieldAlert}
          label={t("workspace.workflow.domainDraftGates", "审批")}
          values={gates.map(domainApprovalGateLabel)}
        />
      ) : null}
      {rules.length > 0 ? (
        <WorkflowDomainDraftRow
          icon={CheckCircle2}
          label={t("workspace.workflow.domainDraftVerification", "验证")}
          values={rules.map(domainVerificationRuleLabel)}
        />
      ) : null}
      {warnings.length > 0 ? (
        <div className="space-y-1 rounded-md bg-background/45 px-2 py-1.5 text-amber-700 dark:text-amber-300">
          {warnings.map((warning) => (
            <div key={warning} className="flex min-w-0 items-center gap-1.5">
              <CircleAlert className="h-3 w-3 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{warning}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function WorkflowDomainDraftRow({
  icon: Icon,
  label,
  values,
}: {
  icon: LucideIcon
  label: string
  values: string[]
}) {
  return (
    <div className="flex min-w-0 items-start gap-1.5 rounded-md bg-background/45 px-2 py-1.5">
      <Icon className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
      <span className="shrink-0 text-[10px] font-medium text-muted-foreground">{label}</span>
      <span className="min-w-0 flex-1 truncate text-foreground/85">{values.join(" · ")}</span>
    </div>
  )
}

function domainEvidenceRequirementLabel(requirement: DomainEvidenceRequirement): string {
  const count = requirement.minCount ? ` x${requirement.minCount}` : ""
  const optional = requirement.required ? "" : " optional"
  return `${requirement.title}${count}${optional}`
}

function domainApprovalGateLabel(gate: DomainApprovalGate): string {
  return gate.required ? gate.action : `${gate.action} optional`
}

function domainVerificationRuleLabel(rule: DomainVerificationRule): string {
  return `${rule.rule}:${rule.severity}`
}

function WorkflowScriptPreviewPanel({
  preview,
  loading,
  error,
}: {
  preview: WorkflowScriptPreview | null
  loading?: boolean
  error?: string | null
}) {
  const { t } = useTranslation()

  if (loading) {
    return (
      <div className="rounded-md border border-border/55 bg-secondary/20 px-2 py-1.5 text-[11px] text-muted-foreground">
        <div className="flex min-w-0 items-center gap-1.5">
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
          <span className="truncate">{t("workspace.workflow.previewLoading", "正在预检脚本")}</span>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="rounded-md border border-destructive/25 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
        <div className="flex min-w-0 items-center gap-1.5 font-medium">
          <CircleAlert className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{t("workspace.workflow.previewError", "预检失败")}</span>
        </div>
        <div className="mt-0.5 truncate opacity-85">{truncateMiddle(error, 140)}</div>
      </div>
    )
  }

  if (!preview) {
    return (
      <div className="rounded-md border border-border/55 bg-secondary/20 px-2 py-1.5 text-[11px] text-muted-foreground">
        <div className="flex min-w-0 items-center gap-1.5">
          <Shield className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">
            {t("workspace.workflow.previewBeforeCreate", "创建前先预检脚本和授权清单")}
          </span>
        </div>
      </div>
    )
  }

  const issues = preview.gate?.issues ?? []
  const errorCount = issues.filter((issue) => issue.severity === "error").length
  const warningCount = issues.filter((issue) => issue.severity === "warning").length
  const visibleIssues = issues.slice(0, 4)
  const tone = preview.canCreate ? "good" : "danger"

  return (
    <div
      className={cn(
        "space-y-1.5 rounded-md border p-2 text-[11px]",
        preview.canCreate
          ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : "border-destructive/25 bg-destructive/10 text-destructive",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        {preview.canCreate ? (
          <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <CircleAlert className="h-3.5 w-3.5 shrink-0" />
        )}
        <span className="min-w-0 flex-1 truncate font-medium">
          {preview.canCreate
            ? t("workspace.workflow.previewPassed", "预检通过")
            : t("workspace.workflow.previewBlocked", "预检未通过")}
        </span>
        <StatusPill
          label={
            preview.requiresApproval
              ? t("workspace.workflow.previewNeedsApproval", "需审批")
              : t("workspace.workflow.previewNoApproval", "可直接创建")
          }
          tone={preview.requiresApproval ? "warn" : tone}
        />
      </div>

      <div className="grid grid-cols-3 gap-1 text-[10px]">
        <WorkflowMetric
          label={t("workspace.workflow.gateMetricErrors", "错误")}
          value={String(errorCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.gateMetricWarnings", "警告")}
          value={String(warningCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.gateMetricDenied", "拒绝")}
          value={preview.hasDenials ? "1" : "0"}
        />
      </div>

      {visibleIssues.length > 0 ? (
        <div className="space-y-1">
          {visibleIssues.map((issue) => (
            <WorkflowGateIssueRow key={`${issue.severity}:${issue.code}`} issue={issue} />
          ))}
          {issues.length > visibleIssues.length ? (
            <div className="px-1 text-[10px] opacity-75">
              {t("workspace.workflow.gateMoreIssues", "另有 {{count}} 个 gate 提示", {
                count: issues.length - visibleIssues.length,
              })}
            </div>
          ) : null}
        </div>
      ) : (
        <div className="rounded-md bg-background/40 px-2 py-1 text-[10px] opacity-85">
          {t("workspace.workflow.noGateIssues", "Script Gate 未发现阻塞项")}
        </div>
      )}

      <WorkflowPermissionPreviewCard preview={preview.permission} />
    </div>
  )
}

function WorkflowGateIssueRow({ issue }: { issue: WorkflowGateIssue }) {
  const { t } = useTranslation()
  const isError = issue.severity === "error"
  return (
    <div className="rounded-md bg-background/45 px-2 py-1.5">
      <div className="flex min-w-0 items-center gap-1.5">
        {isError ? (
          <CircleAlert className="h-3 w-3 shrink-0 text-destructive" />
        ) : (
          <ShieldAlert className="h-3 w-3 shrink-0 text-amber-500" />
        )}
        <span className="min-w-0 flex-1 truncate font-medium text-foreground/85">
          {truncateMiddle(issue.message, 110)}
        </span>
        <StatusPill
          label={
            isError
              ? t("workspace.workflow.gateSeverityError", "错误")
              : t("workspace.workflow.gateSeverityWarning", "警告")
          }
          tone={isError ? "danger" : "warn"}
        />
      </div>
      <div className="mt-0.5 truncate pl-4 text-[10px] text-muted-foreground/80">
        {issue.suggestion}
      </div>
    </div>
  )
}

function WorkflowAutonomyModeControl({
  mode,
  loading,
  saving,
  onChange,
}: {
  mode: WorkflowAutonomyMode
  loading?: boolean
  saving?: WorkflowAutonomyMode | null
  onChange: (mode: WorkflowAutonomyMode) => void
}) {
  const { t } = useTranslation()
  const options: Array<{ mode: WorkflowAutonomyMode; icon: LucideIcon }> = [
    { mode: "off", icon: X },
    { mode: "on", icon: GitPullRequest },
    { mode: "ultracode", icon: Sparkles },
  ]
  const busy = loading || !!saving

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <GitPullRequest className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
          {t("workspace.workflow.workflowMode", "工作流模式")}
        </span>
        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
      </div>
      <div className="grid grid-cols-3 gap-1">
        {options.map((option) => {
          const Icon = option.icon
          const selected = option.mode === mode
          const isSaving = saving === option.mode
          return (
            <button
              key={option.mode}
              type="button"
              className={cn(
                "min-h-12 rounded-md border px-2 py-1.5 text-left transition-colors disabled:opacity-60",
                selected
                  ? "border-primary/55 bg-primary/10 text-foreground"
                  : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
              )}
              disabled={busy}
              onClick={() => onChange(option.mode)}
              aria-pressed={selected}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                {isSaving ? (
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                ) : (
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                )}
                <span className="truncate text-[11px] font-medium">
                  {workflowAutonomyModeLabel(t, option.mode)}
                </span>
              </span>
              <span className="mt-0.5 block truncate text-[10px] opacity-70">
                {workflowAutonomyModeHint(t, option.mode)}
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}

function WorkflowExecutionModeControl({
  mode,
  loading,
  saving,
  onChange,
}: {
  mode: ExecutionMode
  loading?: boolean
  saving?: ExecutionMode | null
  onChange: (mode: ExecutionMode) => void
}) {
  const { t } = useTranslation()
  const options: Array<{ mode: ExecutionMode; icon: LucideIcon }> = [
    { mode: "off", icon: X },
    { mode: "guarded", icon: Shield },
    { mode: "deep", icon: Brain },
    { mode: "autonomous", icon: Bot },
  ]
  const busy = loading || !!saving

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <Gauge className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/90">
          {t("workspace.workflow.executionMode", "Execution Mode")}
        </span>
        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null}
      </div>
      <div className="grid grid-cols-2 gap-1">
        {options.map((option) => {
          const Icon = option.icon
          const selected = option.mode === mode
          const isSaving = saving === option.mode
          return (
            <button
              key={option.mode}
              type="button"
              className={cn(
                "min-h-12 rounded-md border px-2 py-1.5 text-left transition-colors disabled:opacity-60",
                selected
                  ? "border-primary/55 bg-primary/10 text-foreground"
                  : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
              )}
              disabled={busy}
              onClick={() => onChange(option.mode)}
              aria-pressed={selected}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                {isSaving ? (
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                ) : (
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                )}
                <span className="truncate text-[11px] font-medium">
                  {executionModeLabel(t, option.mode)}
                </span>
              </span>
              <span className="mt-0.5 block truncate text-[10px] opacity-70">
                {executionModeHint(t, option.mode)}
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}

function goalStateLabel(t: ReturnType<typeof useTranslation>["t"], state: GoalState): string {
  switch (state) {
    case "active":
      return t("workspace.goal.stateActive", "推进中")
    case "paused":
      return t("workspace.goal.statePaused", "已暂停")
    case "evaluating":
      return t("workspace.goal.stateEvaluating", "评估中")
    case "completed":
      return t("workspace.goal.stateCompleted", "已完成")
    case "failed":
      return t("workspace.goal.stateFailed", "失败")
    case "cancelled":
      return t("workspace.goal.stateCancelled", "已取消")
    case "blocked":
      return t("workspace.goal.stateBlocked", "已阻塞")
  }
}

function goalStateTone(state: GoalState): StatusTone {
  switch (state) {
    case "completed":
      return "good"
    case "paused":
    case "evaluating":
      return "warn"
    case "failed":
    case "blocked":
      return "danger"
    case "active":
      return "info"
    case "cancelled":
      return "muted"
  }
}

function goalIsTerminal(state: GoalState): boolean {
  return state === "completed" || state === "failed" || state === "cancelled"
}

function goalCriterionStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status: GoalCriterionStatus,
): string {
  switch (status) {
    case "satisfied":
      return t("workspace.goal.criterionSatisfied", "已满足")
    case "missing":
      return t("workspace.goal.criterionMissing", "缺证据")
    case "blocked":
      return t("workspace.goal.criterionBlocked", "被阻塞")
  }
}

function goalCriterionStatusTone(status: GoalCriterionStatus): StatusTone {
  switch (status) {
    case "satisfied":
      return "good"
    case "missing":
      return "warn"
    case "blocked":
      return "danger"
  }
}

function goalEvidenceTone(relation: string): StatusTone {
  if (
    relation.includes("failed") ||
    relation.includes("blocked") ||
    relation.includes("cancelled")
  ) {
    return "danger"
  }
  if (relation.includes("passed") || relation.includes("completed")) {
    return "good"
  }
  if (
    relation.includes("diff") ||
    relation.includes("file") ||
    relation.includes("artifact") ||
    relation.includes("worktree")
  ) {
    return "info"
  }
  return "muted"
}

interface GoalDomainEvidence {
  item: GoalEvidenceItem
  domain: string
  evidenceType: string
  source: Record<string, unknown> | null
  sourceLabel: string | null
  confidence: number | null
  accessScope: string | null
  redactionStatus: string | null
  connectorLabel: string | null
  needsExportReview: boolean
  workflowRunId: string | null
  workflowOpKey: string | null
}

function goalDomainEvidenceItems(evidence: GoalEvidenceItem[]): GoalDomainEvidence[] {
  return evidence
    .filter((item) => item.sourceType === "domain_evidence")
    .map(goalDomainEvidenceFromItem)
    .filter((item): item is GoalDomainEvidence => Boolean(item))
}

function goalDomainEvidenceFromItem(item: GoalEvidenceItem): GoalDomainEvidence | null {
  const metadata = asRecord(item.metadata)
  const source = asRecord(metadata?.source)
  const workflow = asRecord(source?.workflow)
  const accessScope = stringField(metadata, "accessScope")
  const redactionStatus = stringField(metadata, "redactionStatus")
  return {
    item,
    domain: stringField(metadata, "domain") ?? stringField(source, "domain") ?? "-",
    evidenceType: item.relation,
    source,
    sourceLabel: domainEvidenceSourceLabel(source),
    confidence: numberField(metadata, "confidence"),
    accessScope,
    redactionStatus,
    connectorLabel: domainEvidenceConnectorLabel(source),
    needsExportReview: domainEvidenceNeedsExportReview(accessScope, redactionStatus),
    workflowRunId: stringField(workflow, "runId"),
    workflowOpKey: stringField(workflow, "opKey"),
  }
}

function domainEvidenceSourceLabel(source: Record<string, unknown> | null): string | null {
  if (!source) return null
  return (
    stringField(source, "uri") ??
    stringField(source, "url") ??
    stringField(source, "path") ??
    stringField(source, "dataset") ??
    stringField(source, "sheet") ??
    stringField(source, "range") ??
    stringField(source, "threadId") ??
    stringField(source, "eventId") ??
    stringField(source, "title")
  )
}

function domainEvidenceConnectorLabel(source: Record<string, unknown> | null): string | null {
  if (!source) return null
  const connector =
    stringField(source, "connector") ??
    stringField(source, "connectorName") ??
    stringField(source, "provider") ??
    stringField(source, "app") ??
    stringField(source, "sourceConnector")
  const account =
    stringField(source, "account") ??
    stringField(source, "accountId") ??
    stringField(source, "email") ??
    stringField(source, "calendarId") ??
    stringField(source, "driveId")
  if (connector && account) return `${connector} · ${account}`
  return connector ?? account ?? null
}

function domainEvidenceNeedsExportReview(
  accessScope: string | null,
  redactionStatus: string | null,
): boolean {
  return (
    accessScope === "private" ||
    accessScope === "connector" ||
    redactionStatus === "sensitive" ||
    redactionStatus === "pending" ||
    redactionStatus === "redacted"
  )
}

function domainEvidenceRedactionTone(redactionStatus: string | null): StatusTone {
  if (redactionStatus === "sensitive") return "danger"
  if (redactionStatus === "pending" || redactionStatus === "redacted") return "warn"
  if (redactionStatus === "none") return "good"
  return "muted"
}

function domainEvidenceConfidenceLabel(confidence: number | null): string {
  if (typeof confidence !== "number" || !Number.isFinite(confidence)) return "-"
  return `${Math.round(confidence * 100)}%`
}

interface GoalWorktreeEvidence {
  item: GoalEvidenceItem
  worktreeId: string
  runId: string | null
  label: string | null
  state: ManagedWorktree["state"]
  path: string
  pathExists: boolean
  baseRef: string | null
  baseBranch: string | null
  baseSha: string | null
  dirtySnapshot: ManagedWorktree["dirtySnapshot"]
  handedOffAt: string | null
  summary: string | null
}

function goalWorktreeEvidenceItems(evidence: GoalEvidenceItem[]): GoalWorktreeEvidence[] {
  return evidence
    .filter((item) => item.relation === "worktree_attached")
    .map(goalWorktreeEvidenceFromItem)
    .filter((item): item is GoalWorktreeEvidence => Boolean(item))
}

function goalWorktreeEvidenceFromItem(item: GoalEvidenceItem): GoalWorktreeEvidence | null {
  const metadata = asRecord(item.metadata)
  const worktreeId = stringField(metadata, "worktreeId") ?? item.sourceId
  const path = stringField(metadata, "path") ?? item.sourceId
  if (!worktreeId || !path) return null
  return {
    item,
    worktreeId,
    runId: stringField(metadata, "runId"),
    label: stringField(metadata, "label"),
    state: parseManagedWorktreeState(stringField(metadata, "state")),
    path,
    pathExists: boolField(metadata, "pathExists") ?? true,
    baseRef: stringField(metadata, "baseRef"),
    baseBranch: stringField(metadata, "baseBranch"),
    baseSha: stringField(metadata, "baseSha"),
    dirtySnapshot: goalWorktreeDirtySnapshotFromMetadata(asRecord(metadata?.dirtySnapshot)),
    handedOffAt: stringField(metadata, "handedOffAt"),
    summary: stringField(metadata, "summary") ?? item.summary ?? null,
  }
}

function parseManagedWorktreeState(value: string | null): ManagedWorktree["state"] {
  return value === "archived" || value === "handoff" ? value : "active"
}

function goalWorktreeDirtySnapshotFromMetadata(
  record: Record<string, unknown> | null,
): ManagedWorktree["dirtySnapshot"] {
  if (!record) return null
  return {
    clean: boolField(record, "clean") ?? false,
    stagedFiles: numberField(record, "stagedFiles") ?? 0,
    unstagedFiles: numberField(record, "unstagedFiles") ?? 0,
    untrackedFiles: numberField(record, "untrackedFiles") ?? 0,
    conflictedFiles: numberField(record, "conflictedFiles") ?? 0,
    changedFiles: numberField(record, "changedFiles") ?? 0,
  }
}

function goalWorktreeBaseLabel(worktree: GoalWorktreeEvidence): string {
  return worktreeBaseLabel(worktree.baseBranch, worktree.baseRef, worktree.baseSha)
}

function worktreeBaseLabel(
  baseBranch: string | null | undefined,
  baseRef: string | null | undefined,
  baseSha: string | null | undefined,
): string {
  if (baseBranch && baseSha) {
    return `${baseBranch} · ${baseSha.slice(0, 8)}`
  }
  return baseBranch ?? baseRef ?? baseSha?.slice(0, 8) ?? "-"
}

function goalWorktreeDirtyLabel(
  t: ReturnType<typeof useTranslation>["t"],
  worktree: GoalWorktreeEvidence,
): string {
  if (!worktree.pathExists) return t("workspace.worktree.pathMissing", "路径已清理")
  const dirty = worktree.dirtySnapshot
  if (!dirty) return t("workspace.goal.worktreeNoSnapshot", "未记录变更快照")
  if (dirty.clean) return t("workspace.worktree.clean", "无本地变更")
  return t("workspace.worktree.changed", "{{count}} 个变更", { count: dirty.changedFiles })
}

function goalBudgetTone(budget: GoalBudgetSnapshot): StatusTone {
  if (budget.exhausted) return "danger"
  if (budget.warning) return "warn"
  return "info"
}

function goalBudgetRatioText(ratio?: number | null): string {
  if (typeof ratio !== "number" || !Number.isFinite(ratio)) return "-"
  return `${Math.round(ratio * 100)}%`
}

function formatDurationCompact(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0s"
  if (seconds < 60) return `${Math.round(seconds)}s`
  const minutes = seconds / 60
  if (minutes < 60) return `${Math.round(minutes)}m`
  const hours = minutes / 60
  if (hours < 48) return `${hours.toFixed(hours < 10 ? 1 : 0)}h`
  return `${Math.round(hours / 24)}d`
}

function GoalDomainTemplatePicker({
  templates,
  loading,
  error,
  selectedTemplate,
  selectedTaskType,
  disabled,
  onReload,
  onTemplateChange,
  onTaskTypeChange,
}: {
  templates: DomainWorkflowTemplate[]
  loading?: boolean
  error?: string | null
  selectedTemplate?: DomainWorkflowTemplate | null
  selectedTaskType: string
  disabled?: boolean
  onReload: () => void
  onTemplateChange: (templateId: string) => void
  onTaskTypeChange: (taskType: string) => void
}) {
  const { t } = useTranslation()
  const selectedTask =
    selectedTemplate?.taskTypes.find((taskType) => taskType === selectedTaskType) ??
    selectedTemplate?.taskTypes[0] ??
    ""

  return (
    <div className="space-y-1 rounded-md border border-border/50 bg-background/45 p-2">
      <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
        <Layers className="h-3.5 w-3.5 shrink-0 text-primary" />
        <span className="min-w-0 flex-1 truncate">
          {t("workspace.goal.domainTemplate", "任务领域")}
        </span>
        {loading ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
      </div>
      {error ? (
        <div className="flex min-w-0 items-center gap-1.5 rounded-md bg-destructive/10 px-2 py-1 text-[11px] text-destructive">
          <CircleAlert className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 truncate">{truncateMiddle(error, 100)}</span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-6 px-2 text-[10px]"
            disabled={disabled || loading}
            onClick={onReload}
          >
            <RefreshCw className="mr-1 h-3 w-3" />
            {t("workspace.goal.domainRetry", "重试")}
          </Button>
        </div>
      ) : null}
      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
        <Select
          value={
            selectedTemplate ? domainTemplateOptionValue(selectedTemplate) : GOAL_DOMAIN_FREE_VALUE
          }
          onValueChange={onTemplateChange}
          disabled={disabled || loading}
        >
          <SelectTrigger className="h-8 bg-background/60 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={GOAL_DOMAIN_FREE_VALUE}>
              {t("workspace.goal.domainFree", "自由任务")}
            </SelectItem>
            {templates.map((template) => (
              <SelectItem
                key={`${template.id}:${template.version}`}
                value={domainTemplateOptionValue(template)}
              >
                {template.title}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select
          value={selectedTask}
          onValueChange={onTaskTypeChange}
          disabled={disabled || !selectedTemplate || selectedTemplate.taskTypes.length <= 1}
        >
          <SelectTrigger className="h-8 bg-background/60 text-xs">
            <SelectValue placeholder={t("workspace.goal.domainTaskTypePlaceholder", "任务类型")} />
          </SelectTrigger>
          <SelectContent>
            {(selectedTemplate?.taskTypes ?? []).map((taskType) => (
              <SelectItem key={taskType} value={taskType}>
                {taskType}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {selectedTemplate ? (
        <div className="flex min-w-0 flex-wrap items-center gap-1 pt-0.5">
          <StatusPill label={selectedTemplate.domain} tone="info" />
          <StatusPill
            label={t("workspace.goal.domainEvidenceCount", "{{count}} 证据", {
              count: selectedTemplate.requiredEvidence.length,
            })}
            tone="good"
          />
          {selectedTemplate.approvalGates.length > 0 ? (
            <StatusPill
              label={t("workspace.goal.domainGateCount", "{{count}} 审批", {
                count: selectedTemplate.approvalGates.length,
              })}
              tone="warn"
            />
          ) : null}
        </div>
      ) : (
        <p className="text-[11px] text-muted-foreground">
          {t("workspace.goal.domainFreeHint", "不绑定领域模板，模型按目标自由判断。")}
        </p>
      )}
    </div>
  )
}

function GoalControlStrip({
  snapshot,
  loading,
  error,
  createOpen,
  objective,
  criteria,
  domainTemplates,
  domainTemplatesLoading,
  domainTemplatesError,
  selectedTemplate,
  selectedTaskType,
  saving,
  actionKey,
  disabled,
  editRequest,
  onCreateOpenChange,
  onObjectiveChange,
  onCriteriaChange,
  onReloadDomainTemplates,
  onTemplateChange,
  onTaskTypeChange,
  onCreate,
  onPause,
  onResume,
  onClear,
  onEvaluate,
  onUpdate,
}: {
  snapshot: GoalSnapshot | null
  loading?: boolean
  error?: string | null
  createOpen: boolean
  objective: string
  criteria: string
  domainTemplates: DomainWorkflowTemplate[]
  domainTemplatesLoading?: boolean
  domainTemplatesError?: string | null
  selectedTemplate?: DomainWorkflowTemplate | null
  selectedTaskType: string
  saving?: boolean
  actionKey?: string | null
  disabled?: boolean
  editRequest?: number
  onCreateOpenChange: (open: boolean) => void
  onObjectiveChange: (value: string) => void
  onCriteriaChange: (value: string) => void
  onReloadDomainTemplates: () => void
  onTemplateChange: (templateId: string) => void
  onTaskTypeChange: (taskType: string) => void
  onCreate: () => void
  onPause: () => void
  onResume: () => void
  onClear: () => void
  onEvaluate: () => void
  onUpdate: (
    objective: string,
    completionCriteria: string,
    domainSelection?: { template: DomainWorkflowTemplate | null; taskType: string },
  ) => Promise<boolean>
}) {
  const { t } = useTranslation()
  const [detailOpen, setDetailOpen] = useState(false)
  const [editOpen, setEditOpen] = useState(false)
  const [editObjective, setEditObjective] = useState("")
  const [editCriteria, setEditCriteria] = useState("")
  const [editTemplateId, setEditTemplateId] = useState(GOAL_DOMAIN_FREE_VALUE)
  const [editTaskType, setEditTaskType] = useState("")
  const lastEditRequestRef = useRef(editRequest ?? 0)
  const goal = snapshot?.goal ?? null
  const audit = asRecord(goal?.finalEvidence)
  const auditEvidence = recordArrayField(audit, "evidence")
  const criteriaAudit = Array.isArray(snapshot?.criteria) ? snapshot.criteria : []
  const evidenceItems = Array.isArray(snapshot?.evidence) ? snapshot.evidence : []
  const timelineItems = Array.isArray(snapshot?.timeline) ? snapshot.timeline : []
  const workflowRuns = Array.isArray(snapshot?.workflowRuns) ? snapshot.workflowRuns : []
  const tasks = Array.isArray(snapshot?.tasks) ? snapshot.tasks : []
  const achieved = arrayField(audit, "achieved").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const missing = arrayField(audit, "missing").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const blockers = arrayField(audit, "blockers").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const workflowCount = workflowRuns.length
  const taskCount = tasks.length
  const taskDone = tasks.filter((task) => task.status === "completed").length
  const evidenceCount = evidenceItems.length || auditEvidence.length
  const isBusy = saving || Boolean(actionKey)
  const canCreate = !disabled && !saving && objective.trim().length > 0
  const activeGoalTemplate =
    goal?.workflowTemplateId && domainTemplates.length > 0
      ? findDomainTemplateByValue(domainTemplates, goalDomainTemplateValue(goal))
      : null
  const editTemplate =
    editTemplateId === GOAL_DOMAIN_FREE_VALUE
      ? null
      : findDomainTemplateByValue(domainTemplates, editTemplateId)
  const createTaskType =
    selectedTemplate?.taskTypes.find((taskType) => taskType === selectedTaskType) ??
    selectedTemplate?.taskTypes[0] ??
    ""
  const editSelectedTaskType =
    editTemplate?.taskTypes.find((taskType) => taskType === editTaskType) ??
    editTemplate?.taskTypes[0] ??
    ""

  useEffect(() => {
    setEditObjective(goal?.objective ?? "")
    setEditCriteria(goal?.completionCriteria ?? "")
    setEditTemplateId(goal ? goalDomainTemplateValue(goal) : GOAL_DOMAIN_FREE_VALUE)
    setEditTaskType(goal?.workflowTaskType ?? "")
    setEditOpen(false)
  }, [
    goal?.id,
    goal?.objective,
    goal?.completionCriteria,
    goal?.workflowTemplateId,
    goal?.workflowTemplateVersion,
    goal?.workflowTaskType,
  ])

  useEffect(() => {
    const nextRequest = editRequest ?? 0
    if (nextRequest === lastEditRequestRef.current) return
    lastEditRequestRef.current = nextRequest
    if (!goal) return
    setDetailOpen(true)
    setEditOpen(true)
  }, [editRequest, goal])

  useEffect(() => {
    if (editTemplateId === GOAL_DOMAIN_FREE_VALUE) {
      if (editTaskType) setEditTaskType("")
      return
    }
    const template = findDomainTemplateByValue(domainTemplates, editTemplateId)
    if (!template) return
    if (template.taskTypes.length === 0) {
      if (editTaskType) setEditTaskType("")
      return
    }
    if (!template.taskTypes.includes(editTaskType)) {
      setEditTaskType(template.taskTypes[0] ?? "")
    }
  }, [domainTemplates, editTaskType, editTemplateId])

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20">
      <button
        type="button"
        className="flex w-full min-w-0 items-center gap-2 px-2 py-1.5 text-left text-xs transition-colors hover:bg-secondary/45 disabled:opacity-60"
        disabled={disabled && !goal}
        aria-expanded={goal ? detailOpen : createOpen}
        onClick={() => {
          if (goal) {
            setDetailOpen((open) => !open)
          } else {
            onCreateOpenChange(!createOpen)
          }
        }}
      >
        <Sparkles className="h-3.5 w-3.5 shrink-0 text-primary" />
        <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">
          {goal
            ? truncateMiddle(goal.objective.replace(/\s+/g, " "), 96)
            : t("workspace.goal.title", "Goal")}
        </span>
        {loading ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
        ) : null}
        {goal ? (
          <StatusPill
            label={goalStateLabel(t, goal.state)}
            tone={goalStateTone(goal.state)}
            loading={goal.state === "evaluating"}
          />
        ) : (
          <StatusPill label={t("workspace.goal.noActive", "未设置")} tone="muted" />
        )}
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
            (goal ? detailOpen : createOpen) && "rotate-90",
          )}
        />
      </button>

      {error ? (
        <div className="border-t border-border/60 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : null}

      {!goal ? (
        <AnimatedCollapse open={createOpen}>
          <form
            className="space-y-2 border-t border-border/60 p-2"
            onSubmit={(event) => {
              event.preventDefault()
              if (canCreate) onCreate()
            }}
          >
            <div className="space-y-1">
              <label className="block text-[10px] font-medium text-muted-foreground">
                {t("workspace.goal.objective", "目标")}
              </label>
              <Textarea
                value={objective}
                disabled={saving}
                onChange={(event) => onObjectiveChange(event.target.value)}
                placeholder={t(
                  "workspace.goal.objectivePlaceholder",
                  "例如：完整实现 Goal 模式，并通过针对性检查",
                )}
                className="min-h-16 resize-y text-xs"
              />
            </div>
            <div className="space-y-1">
              <label className="block text-[10px] font-medium text-muted-foreground">
                {t("workspace.goal.criteria", "完成标准")}
              </label>
              <Textarea
                value={criteria}
                disabled={saving}
                onChange={(event) => onCriteriaChange(event.target.value)}
                placeholder={t(
                  "workspace.goal.criteriaPlaceholder",
                  "每行一个标准：功能完成、证据充分、风险可解释",
                )}
                className="min-h-16 resize-y text-xs"
              />
            </div>
            <GoalDomainTemplatePicker
              templates={domainTemplates}
              loading={domainTemplatesLoading}
              error={domainTemplatesError}
              selectedTemplate={selectedTemplate}
              selectedTaskType={createTaskType}
              disabled={saving}
              onReload={onReloadDomainTemplates}
              onTemplateChange={onTemplateChange}
              onTaskTypeChange={onTaskTypeChange}
            />
            <Button
              type="submit"
              size="sm"
              className="h-8 w-full gap-1.5 text-xs"
              disabled={!canCreate}
            >
              {saving ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.goal.create", "创建 Goal")}</span>
            </Button>
          </form>
        </AnimatedCollapse>
      ) : (
        <div className="space-y-2 border-t border-border/60 p-2">
          {goal.completionCriteria.trim() ? (
            <div className="rounded-md bg-background/45 px-2 py-1.5 text-[11px] text-muted-foreground">
              <span className="font-medium text-foreground/80">
                {t("workspace.goal.criteria", "完成标准")}
              </span>
              <span className="px-1 text-muted-foreground/45">·</span>
              <span>{truncateMiddle(goal.completionCriteria.replace(/\s+/g, " "), 180)}</span>
            </div>
          ) : null}

          {goal.domain || goal.workflowTemplateId ? (
            <div className="flex min-w-0 flex-wrap items-center gap-1 rounded-md bg-background/45 px-2 py-1.5 text-[11px] text-muted-foreground">
              {goal.domain ? <StatusPill label={goal.domain} tone="info" /> : null}
              {goal.workflowTemplateId ? (
                <StatusPill
                  label={
                    goal.workflowTemplateVersion
                      ? `${goal.workflowTemplateId}@${goal.workflowTemplateVersion}`
                      : goal.workflowTemplateId
                  }
                  tone="muted"
                />
              ) : null}
              {goal.workflowTaskType ? (
                <StatusPill label={goal.workflowTaskType} tone="good" />
              ) : null}
              {activeGoalTemplate ? (
                <span className="min-w-0 truncate pl-1">{activeGoalTemplate.outputContract}</span>
              ) : null}
            </div>
          ) : null}

          <div className="grid grid-cols-3 gap-1 text-[10px]">
            <WorkflowMetric
              label={t("workspace.goal.metricWorkflows", "Workflows")}
              value={workflowCount.toString()}
            />
            <WorkflowMetric
              label={t("workspace.goal.metricTasks", "Tasks")}
              value={`${taskDone}/${taskCount}`}
            />
            <WorkflowMetric
              label={t("workspace.goal.metricEvidence", "Evidence")}
              value={evidenceCount.toString()}
            />
          </div>

          {goal.finalSummary ? (
            <div
              className={cn(
                "rounded-md border px-2 py-1.5 text-[11px]",
                goal.state === "completed"
                  ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                  : "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5 font-medium">
                {goal.state === "completed" ? (
                  <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
                ) : (
                  <CircleAlert className="h-3.5 w-3.5 shrink-0" />
                )}
                <span className="truncate">{goal.finalSummary}</span>
              </div>
              {goal.blockedReason ? (
                <div className="mt-0.5 truncate opacity-80">{goal.blockedReason}</div>
              ) : null}
            </div>
          ) : (
            <div className="rounded-md bg-background/45 px-2 py-1.5 text-[11px] text-muted-foreground">
              {t(
                "workspace.goal.noAudit",
                "还没有 final audit；workflow 完成后会自动评估，也可以手动评估。",
              )}
            </div>
          )}

          <AnimatedCollapse open={editOpen}>
            <div className="space-y-2 rounded-md border border-border/55 bg-background/45 p-2">
              <div className="space-y-1">
                <label className="block text-[10px] font-medium text-muted-foreground">
                  {t("workspace.goal.objective", "目标")}
                </label>
                <Textarea
                  value={editObjective}
                  disabled={isBusy}
                  onChange={(event) => setEditObjective(event.target.value)}
                  className="min-h-14 resize-y text-xs"
                />
              </div>
              <div className="space-y-1">
                <label className="block text-[10px] font-medium text-muted-foreground">
                  {t("workspace.goal.criteria", "完成标准")}
                </label>
                <Textarea
                  value={editCriteria}
                  disabled={isBusy}
                  onChange={(event) => setEditCriteria(event.target.value)}
                  className="min-h-14 resize-y text-xs"
                />
              </div>
              <GoalDomainTemplatePicker
                templates={domainTemplates}
                loading={domainTemplatesLoading}
                error={domainTemplatesError}
                selectedTemplate={editTemplate}
                selectedTaskType={editSelectedTaskType}
                disabled={isBusy}
                onReload={onReloadDomainTemplates}
                onTemplateChange={setEditTemplateId}
                onTaskTypeChange={setEditTaskType}
              />
              <div className="flex justify-end gap-1.5">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="h-7 px-2 text-xs"
                  disabled={isBusy}
                  onClick={() => {
                    setEditOpen(false)
                    setEditObjective(goal.objective)
                    setEditCriteria(goal.completionCriteria)
                    setEditTemplateId(goalDomainTemplateValue(goal))
                    setEditTaskType(goal.workflowTaskType ?? "")
                  }}
                >
                  {t("common.cancel", "取消")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  className="h-7 px-2 text-xs"
                  disabled={isBusy || !editObjective.trim()}
                  onClick={() => {
                    const domainSelection =
                      editTemplateId === GOAL_DOMAIN_FREE_VALUE
                        ? { template: null, taskType: "" }
                        : editTemplate
                          ? { template: editTemplate, taskType: editSelectedTaskType }
                          : undefined
                    void onUpdate(editObjective, editCriteria, domainSelection).then((ok) => {
                      if (ok) setEditOpen(false)
                    })
                  }}
                >
                  {actionKey?.startsWith("update_goal") ? (
                    <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                  ) : null}
                  {t("common.save", "保存")}
                </Button>
              </div>
            </div>
          </AnimatedCollapse>

          {blockers.length > 0 || missing.length > 0 || achieved.length > 0 ? (
            <div className="space-y-1 text-[10px]">
              {blockers.slice(0, 2).map((item) => (
                <div
                  key={`blocker:${item}`}
                  className="truncate rounded-md bg-destructive/10 px-2 py-1 text-destructive"
                >
                  {item}
                </div>
              ))}
              {missing.slice(0, 2).map((item) => (
                <div
                  key={`missing:${item}`}
                  className="truncate rounded-md bg-amber-500/10 px-2 py-1 text-amber-700 dark:text-amber-300"
                >
                  {item}
                </div>
              ))}
              {blockers.length === 0 && missing.length === 0
                ? achieved.slice(0, 2).map((item) => (
                    <div
                      key={`achieved:${item}`}
                      className="truncate rounded-md bg-emerald-500/10 px-2 py-1 text-emerald-700 dark:text-emerald-300"
                    >
                      {item}
                    </div>
                  ))
                : null}
            </div>
          ) : null}

          <GoalDetailSection
            open={detailOpen}
            snapshot={snapshot!}
            criteria={criteriaAudit}
            evidence={evidenceItems}
            timeline={timelineItems}
          />

          <div className="grid grid-cols-4 gap-1.5">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 min-w-0 gap-1.5 text-xs"
              disabled={isBusy || goalIsTerminal(goal.state)}
              onClick={() => setEditOpen((open) => !open)}
            >
              <Pencil className="h-3.5 w-3.5" />
              <span className="truncate">{t("workspace.goal.edit", "编辑")}</span>
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 min-w-0 gap-1.5 text-xs"
              disabled={isBusy || goalIsTerminal(goal.state) || goal.state === "evaluating"}
              onClick={onEvaluate}
            >
              {actionKey?.startsWith("evaluate_goal") ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <CheckCircle2 className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.goal.evaluate", "评估")}</span>
            </Button>
            {goal.state === "paused" || goal.state === "blocked" ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                disabled={isBusy || goalIsTerminal(goal.state)}
                onClick={onResume}
              >
                {actionKey?.startsWith("resume_goal") ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Play className="h-3.5 w-3.5" />
                )}
                <span className="truncate">{t("workspace.goal.resume", "恢复")}</span>
              </Button>
            ) : (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                disabled={isBusy || goalIsTerminal(goal.state) || goal.state === "evaluating"}
                onClick={onPause}
              >
                {actionKey?.startsWith("pause_goal") ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Pause className="h-3.5 w-3.5" />
                )}
                <span className="truncate">{t("workspace.goal.pause", "暂停")}</span>
              </Button>
            )}
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 min-w-0 gap-1.5 border-destructive/35 text-xs text-destructive hover:text-destructive"
              disabled={isBusy || goalIsTerminal(goal.state)}
              onClick={onClear}
            >
              {actionKey?.startsWith("clear_goal") ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <X className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.goal.clear", "清除")}</span>
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

function GoalDetailSection({
  open,
  snapshot,
  criteria,
  evidence,
  timeline,
}: {
  open: boolean
  snapshot: GoalSnapshot
  criteria: GoalCriterionAudit[]
  evidence: GoalEvidenceItem[]
  timeline: GoalTimelineItem[]
}) {
  const { t } = useTranslation()
  const runs = snapshot.workflowRuns
  const tasks = snapshot.tasks
  const latestTimeline = timeline.slice(-8).reverse()
  const latestEvidence = evidence.slice(-8).reverse()
  const worktreeEvidence = goalWorktreeEvidenceItems(evidence).slice(-4).reverse()
  const domainEvidence = goalDomainEvidenceItems(evidence).slice(-6).reverse()
  const audit = asRecord(snapshot.goal.finalEvidence)
  const nextEvidence = recordArrayField(audit, "nextEvidenceNeeded").slice(0, 6)
  const budget = snapshot.budget

  return (
    <AnimatedCollapse open={open}>
      <div className="space-y-2 rounded-md border border-border/55 bg-background/45 p-2">
        {budget ? <GoalBudgetCard budget={budget} /> : null}

        {worktreeEvidence.length > 0 ? (
          <GoalDetailBlock
            title={t("workspace.goal.detailWorktrees", "Worktrees")}
            count={worktreeEvidence.length}
          >
            <div className="space-y-1">
              {worktreeEvidence.map((worktree) => (
                <IconTip
                  key={worktree.item.id}
                  label={compactJson(worktree.item.metadata, worktree.path)}
                >
                  <div className="min-w-0 rounded-md border border-blue-500/20 bg-blue-500/10 px-2 py-1.5 text-blue-700 dark:text-blue-300">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <FolderGit2 className="h-3.5 w-3.5 shrink-0" />
                      <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
                        {worktree.label || basename(worktree.path)}
                      </span>
                      <StatusPill
                        label={managedWorktreeStateLabel(t, worktree.state)}
                        tone={managedWorktreeStateTone(worktree.state)}
                      />
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] opacity-85">
                      <span className="min-w-0 flex-1 truncate font-mono">
                        {truncateMiddle(worktree.path, 120)}
                      </span>
                      {!worktree.pathExists ? (
                        <StatusPill
                          label={t("workspace.worktree.pathMissing", "路径已清理")}
                          tone="warn"
                        />
                      ) : null}
                    </div>
                    <div className="mt-1 grid grid-cols-3 gap-1 text-[10px]">
                      <GoalWorktreeMetric
                        label={t("workspace.goal.worktreeBase", "Base")}
                        value={goalWorktreeBaseLabel(worktree)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.worktreeDirty", "Changes")}
                        value={goalWorktreeDirtyLabel(t, worktree)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.worktreeHandoff", "Handoff")}
                        value={
                          worktree.handedOffAt
                            ? formatMessageTime(worktree.handedOffAt)
                            : worktree.runId
                              ? truncateMiddle(worktree.runId, 18)
                              : "-"
                        }
                      />
                    </div>
                    {worktree.summary ? (
                      <div className="mt-1 truncate text-[10px] opacity-80">{worktree.summary}</div>
                    ) : null}
                  </div>
                </IconTip>
              ))}
            </div>
          </GoalDetailBlock>
        ) : null}

        {domainEvidence.length > 0 ? (
          <GoalDetailBlock
            title={t("workspace.goal.detailDomainEvidence", "领域证据")}
            count={domainEvidence.length}
          >
            <div className="space-y-1">
              {domainEvidence.map((evidenceItem) => (
                <IconTip
                  key={evidenceItem.item.id}
                  label={compactJson(evidenceItem.item.metadata, evidenceItem.item.id)}
                >
                  <div className="min-w-0 rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2 py-1.5 text-emerald-700 dark:text-emerald-300">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <BookText className="h-3.5 w-3.5 shrink-0" />
                      <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
                        {evidenceItem.item.title}
                      </span>
                      <StatusPill label={evidenceItem.evidenceType} tone="good" />
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] opacity-85">
                      <span className="shrink-0">{evidenceItem.domain}</span>
                      {evidenceItem.sourceLabel ? (
                        <>
                          <span className="text-emerald-700/45 dark:text-emerald-300/45">·</span>
                          <span className="min-w-0 flex-1 truncate font-mono">
                            {truncateMiddle(evidenceItem.sourceLabel, 120)}
                          </span>
                        </>
                      ) : null}
                    </div>
                    {evidenceItem.item.summary ? (
                      <div className="mt-1 truncate text-[10px] opacity-80">
                        {evidenceItem.item.summary}
                      </div>
                    ) : null}
                    {evidenceItem.connectorLabel ||
                    evidenceItem.redactionStatus ||
                    evidenceItem.needsExportReview ? (
                      <div className="mt-1 flex min-w-0 flex-wrap gap-1">
                        {evidenceItem.connectorLabel ? (
                          <StatusPill label={evidenceItem.connectorLabel} tone="info" />
                        ) : null}
                        {evidenceItem.redactionStatus ? (
                          <StatusPill
                            label={evidenceItem.redactionStatus}
                            tone={domainEvidenceRedactionTone(evidenceItem.redactionStatus)}
                          />
                        ) : null}
                        {evidenceItem.needsExportReview ? (
                          <StatusPill
                            label={t("workspace.goal.domainExportReview", "导出前复核")}
                            tone="warn"
                          />
                        ) : null}
                      </div>
                    ) : null}
                    <div className="mt-1 grid grid-cols-3 gap-1 text-[10px]">
                      <GoalWorktreeMetric
                        label={t("workspace.goal.domainConfidence", "Confidence")}
                        value={domainEvidenceConfidenceLabel(evidenceItem.confidence)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.domainAccess", "Access")}
                        value={evidenceItem.accessScope ?? "-"}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.domainWorkflow", "Workflow")}
                        value={
                          evidenceItem.workflowOpKey
                            ? truncateMiddle(evidenceItem.workflowOpKey, 28)
                            : evidenceItem.workflowRunId
                              ? truncateMiddle(evidenceItem.workflowRunId, 18)
                              : (evidenceItem.redactionStatus ?? "-")
                        }
                      />
                    </div>
                  </div>
                </IconTip>
              ))}
            </div>
          </GoalDetailBlock>
        ) : null}

        <GoalDetailBlock title={t("workspace.goal.detailCriteria", "标准")} count={criteria.length}>
          {criteria.length > 0 ? (
            <div className="space-y-1">
              {criteria.map((criterion) => (
                <IconTip key={criterion.id} label={criterion.reason ?? criterion.text}>
                  <div className="min-w-0 rounded-md bg-secondary/30 px-2 py-1.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/90">
                        {criterion.text}
                      </span>
                      <StatusPill
                        label={goalCriterionStatusLabel(t, criterion.status)}
                        tone={goalCriterionStatusTone(criterion.status)}
                      />
                    </div>
                    {criterion.evidenceIds.length > 0 ? (
                      <div className="mt-1 truncate font-mono text-[10px] text-muted-foreground">
                        {criterion.evidenceIds.slice(0, 3).join(" · ")}
                      </div>
                    ) : null}
                  </div>
                </IconTip>
              ))}
            </div>
          ) : (
            <div className="rounded-md bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground">
              {t("workspace.goal.detailNoCriteria", "没有拆分出的完成标准")}
            </div>
          )}
        </GoalDetailBlock>

        {nextEvidence.length > 0 ? (
          <GoalDetailBlock
            title={t("workspace.goal.detailNextEvidence", "下一步证据")}
            count={nextEvidence.length}
          >
            <div className="space-y-1">
              {nextEvidence.map((item, index) => {
                const kind = stringField(item, "kind") ?? `item-${index + 1}`
                const reason = stringField(item, "reason") ?? compactJson(item, kind)
                return (
                  <IconTip key={`${kind}:${index}`} label={compactJson(item, reason)}>
                    <div className="min-w-0 rounded-md bg-amber-500/10 px-2 py-1.5 text-amber-700 dark:text-amber-300">
                      <div className="flex min-w-0 items-center gap-1.5">
                        <CircleAlert className="h-3.5 w-3.5 shrink-0" />
                        <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
                          {kind}
                        </span>
                      </div>
                      <div className="mt-1 truncate text-[10px] opacity-85">{reason}</div>
                    </div>
                  </IconTip>
                )
              })}
            </div>
          </GoalDetailBlock>
        ) : null}

        <GoalDetailBlock title={t("workspace.goal.detailEvidence", "证据")} count={evidence.length}>
          {latestEvidence.length > 0 ? (
            <div className="space-y-1">
              {latestEvidence.map((item) => (
                <IconTip key={item.id} label={compactJson(item.metadata, item.id)}>
                  <div className="min-w-0 rounded-md bg-secondary/30 px-2 py-1.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/90">
                        {item.title}
                      </span>
                      <StatusPill label={item.relation} tone={goalEvidenceTone(item.relation)} />
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
                      <span className="truncate">{item.sourceType}</span>
                      <span className="text-muted-foreground/45">·</span>
                      <span className="min-w-0 flex-1 truncate font-mono">{item.sourceId}</span>
                      <span className="shrink-0">{formatMessageTime(item.createdAt)}</span>
                    </div>
                    {item.summary ? (
                      <div className="mt-1 truncate text-[10px] text-muted-foreground/85">
                        {item.summary}
                      </div>
                    ) : null}
                  </div>
                </IconTip>
              ))}
            </div>
          ) : (
            <div className="rounded-md bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground">
              {t("workspace.goal.detailNoEvidence", "还没有 workflow / validation / diff 证据")}
            </div>
          )}
        </GoalDetailBlock>

        <GoalDetailBlock
          title={t("workspace.goal.detailTimeline", "时间线")}
          count={timeline.length}
        >
          {latestTimeline.length > 0 ? (
            <div className="space-y-1">
              {latestTimeline.map((item) => (
                <IconTip key={item.id} label={compactJson(item.metadata, item.title)}>
                  <div className="min-w-0 rounded-md bg-secondary/25 px-2 py-1.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/90">
                        {item.title}
                      </span>
                      {item.status ? (
                        <StatusPill label={item.status} tone={goalEvidenceTone(item.status)} />
                      ) : null}
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
                      <span className="shrink-0">{formatMessageTime(item.createdAt)}</span>
                      <span className="text-muted-foreground/45">·</span>
                      <span className="truncate">{item.kind}</span>
                      {item.summary ? (
                        <>
                          <span className="text-muted-foreground/45">·</span>
                          <span className="min-w-0 flex-1 truncate">{item.summary}</span>
                        </>
                      ) : null}
                    </div>
                  </div>
                </IconTip>
              ))}
            </div>
          ) : (
            <div className="rounded-md bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground">
              {t("workspace.goal.detailNoTimeline", "暂无时间线事件")}
            </div>
          )}
        </GoalDetailBlock>

        <div className="grid grid-cols-2 gap-1.5 text-[10px]">
          <GoalLinkedSummary
            title={t("workspace.goal.detailWorkflows", "Workflows")}
            value={`${runs.filter((run) => run.state === "completed").length}/${runs.length}`}
            detail={
              runs[0]
                ? `${runs[0].kind} · ${workflowRunStateLabel(t, runs[0].state)}`
                : t("workspace.goal.detailNoWorkflow", "暂无 workflow")
            }
          />
          <GoalLinkedSummary
            title={t("workspace.goal.detailTasks", "Tasks")}
            value={`${tasks.filter((task) => task.status === "completed").length}/${tasks.length}`}
            detail={tasks[0]?.content ?? t("workspace.goal.detailNoTask", "暂无 task")}
          />
        </div>
      </div>
    </AnimatedCollapse>
  )
}

function GoalBudgetCard({ budget }: { budget: GoalBudgetSnapshot }) {
  const { t } = useTranslation()
  const tone = goalBudgetTone(budget)
  const statusLabel = budget.exhausted
    ? t("workspace.goal.budgetExhausted", "预算耗尽")
    : budget.warning
      ? t("workspace.goal.budgetWarning", "接近上限")
      : t("workspace.goal.budgetOk", "预算正常")

  return (
    <div className={cn("space-y-1 rounded-md border px-2 py-1.5", STATUS_TONE_CLASS[tone])}>
      <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium">
        <Gauge className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{t("workspace.goal.detailBudget", "预算")}</span>
        <span className="shrink-0">{statusLabel}</span>
      </div>
      <div className="grid grid-cols-3 gap-1 text-[10px]">
        <GoalBudgetMetric
          label={t("workspace.goal.budgetTokens", "Tokens")}
          value={
            budget.tokenLimit
              ? `${compactCount(budget.tokensUsed)}/${compactCount(budget.tokenLimit)}`
              : compactCount(budget.tokensUsed)
          }
          ratio={goalBudgetRatioText(budget.tokenRatio)}
        />
        <GoalBudgetMetric
          label={t("workspace.goal.budgetTime", "Time")}
          value={
            budget.timeLimitSecs
              ? `${formatDurationCompact(budget.elapsedSecs)}/${formatDurationCompact(budget.timeLimitSecs)}`
              : formatDurationCompact(budget.elapsedSecs)
          }
          ratio={goalBudgetRatioText(budget.timeRatio)}
        />
        <GoalBudgetMetric
          label={t("workspace.goal.budgetTurns", "Turns")}
          value={
            budget.turnLimit
              ? `${compactCount(budget.turnsUsed)}/${compactCount(budget.turnLimit)}`
              : compactCount(budget.turnsUsed)
          }
          ratio={goalBudgetRatioText(budget.turnRatio)}
        />
      </div>
    </div>
  )
}

function GoalBudgetMetric({
  label,
  value,
  ratio,
}: {
  label: string
  value: string
  ratio: string
}) {
  return (
    <div className="min-w-0 rounded-md bg-background/45 px-1.5 py-1 text-center">
      <div className="truncate font-medium">{value}</div>
      <div className="truncate opacity-75">{label}</div>
      <div className="truncate font-mono opacity-65">{ratio}</div>
    </div>
  )
}

function GoalWorktreeMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md bg-background/45 px-1.5 py-1">
      <div className="truncate font-medium">{value}</div>
      <div className="truncate opacity-70">{label}</div>
    </div>
  )
}

interface WorkflowRunWorktreeInfo {
  label: string | null
  state: ManagedWorktree["state"]
  path: string
  pathExists: boolean
  baseLabel: string | null
  dirtyLabel: string | null
  source: "managed" | "trace"
}

function workflowRunWorktreeInfo(
  t: ReturnType<typeof useTranslation>["t"],
  run: WorkflowRun,
  snapshot: WorkflowRunSnapshot | null,
  worktree?: ManagedWorktree | null,
): WorkflowRunWorktreeInfo | null {
  const worktreeId = run.worktreeId
  if (!worktreeId) return null
  if (worktree) {
    return {
      label: worktree.label ?? null,
      state: worktree.state,
      path: worktree.path,
      pathExists: worktree.pathExists,
      baseLabel: worktreeBaseLabel(worktree.baseBranch, worktree.baseRef, worktree.baseSha),
      dirtyLabel: worktreeDirtySummary(t, worktree),
      source: "managed",
    }
  }
  const event = snapshot?.events
    .slice()
    .reverse()
    .find((event) => event.eventType === "run_worktree_attached")
  const payload = asRecord(event?.payload)
  const path = stringField(payload, "path") ?? worktreeId
  return {
    label: null,
    state: parseManagedWorktreeState(stringField(payload, "state")),
    path,
    pathExists: true,
    baseLabel: null,
    dirtyLabel: null,
    source: "trace",
  }
}

function GoalDetailBlock({
  title,
  count,
  children,
}: {
  title: string
  count: number
  children: ReactNode
}) {
  return (
    <div className="space-y-1">
      <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
        <span className="min-w-0 flex-1 truncate">{title}</span>
        <span className="shrink-0 font-mono">{count}</span>
      </div>
      {children}
    </div>
  )
}

function GoalLinkedSummary({
  title,
  value,
  detail,
}: {
  title: string
  value: string
  detail: string
}) {
  return (
    <div className="min-w-0 rounded-md bg-secondary/30 px-2 py-1.5">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="truncate text-muted-foreground">{title}</span>
        <span className="shrink-0 font-mono text-foreground/85">{value}</span>
      </div>
      <div className="mt-1 truncate text-muted-foreground/75">{detail}</div>
    </div>
  )
}

function WorkflowRunOverview({
  run,
  snapshot,
  latestEvent,
  worktree,
  actions,
  onSelectDetailTab,
  onCreateRepairDraft,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  latestEvent?: WorkflowEvent
  worktree?: ManagedWorktree | null
  actions?: ReactNode
  onSelectDetailTab?: (tab: WorkflowDetailTab) => void
  onCreateRepairDraft?: (repairPrompt: string, run: WorkflowRun) => void
}) {
  const { t } = useTranslation()
  const ops = snapshot?.ops ?? []
  const worktreeInfo = workflowRunWorktreeInfo(t, run, snapshot, worktree)
  const completed = ops.filter((op) => op.state === "completed").length
  const failed = ops.filter((op) => op.state === "failed").length
  const validationCount = ops.filter((op) => op.opType === "validate").length
  const agentCount = ops.filter((op) => op.opType === "spawnAgent").length
  const derivedChildEvents = (snapshot?.events ?? []).filter(
    (event) => event.eventType === "run_derived_child_created",
  )
  const budget = workflowOutputBudget(run, snapshot?.events ?? [])
  const total = ops.length
  const progress =
    total > 0 ? Math.round((completed / total) * 100) : run.state === "completed" ? 100 : 0
  const progressTone =
    failed > 0 || run.state === "failed" || run.state === "blocked"
      ? "bg-destructive"
      : run.state === "completed"
        ? "bg-emerald-500"
        : "bg-blue-500"

  return (
    <div className="space-y-2 rounded-md border border-border/55 bg-background/45 p-2">
      <div className="flex min-w-0 items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 truncate text-xs font-medium text-foreground/90">
              {run.kind}
            </span>
            <StatusPill
              label={workflowRunStateLabel(t, run.state)}
              tone={workflowRunTone(run.state)}
              loading={run.state === "running" || run.state === "recovering"}
            />
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
            <span className="truncate">
              {executionModeLabel(t, normalizeExecutionMode(run.executionMode))}
            </span>
            <span className="text-muted-foreground/45">·</span>
            <span className="truncate">
              {t("workspace.workflow.updated", "更新")} {formatMessageTime(run.updatedAt)}
            </span>
            <span className="text-muted-foreground/45">·</span>
            <span className="shrink-0 font-mono">{run.scriptHash.slice(0, 7)}</span>
          </div>
        </div>
        {latestEvent ? (
          <IconTip label={compactJson(latestEvent.payload, latestEvent.eventType)}>
            <span className="inline-flex max-w-[6.5rem] shrink-0 items-center gap-1 rounded-md bg-secondary/55 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              <Clock className="h-2.5 w-2.5 shrink-0" />
              <span className="truncate">
                #{latestEvent.seq} {workflowEventTitle(t, latestEvent)}
              </span>
            </span>
          </IconTip>
        ) : null}
      </div>

      <div className="space-y-1">
        <div className="h-1.5 overflow-hidden rounded-full bg-secondary">
          <div
            className={cn("h-full rounded-full transition-all", progressTone)}
            style={{ width: `${Math.max(failed > 0 ? 8 : 0, progress)}%` }}
          />
        </div>
        <div className="grid grid-cols-4 gap-1 text-[10px]">
          <WorkflowMetric
            label={t("workspace.workflow.metricOps", "Ops")}
            value={total.toString()}
          />
          <WorkflowMetric
            label={t("workspace.workflow.metricDone", "完成")}
            value={`${completed}/${total || 0}`}
          />
          <WorkflowMetric
            label={t("workspace.workflow.metricValidate", "验证")}
            value={validationCount.toString()}
          />
          <WorkflowMetric
            label={t("workspace.workflow.metricAgents", "Agents")}
            value={agentCount.toString()}
          />
        </div>
        {budget ? (
          <div
            className={cn(
              "flex min-w-0 items-center justify-between gap-2 rounded-md border px-2 py-1 text-[10px]",
              budget.exhausted
                ? "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300"
                : "border-border/55 bg-secondary/35 text-muted-foreground",
            )}
          >
            <span className="truncate">{t("workspace.workflow.outputBudget", "输出预算")}</span>
            <span className="shrink-0 font-mono">
              {compactCount(budget.spent)}/{compactCount(budget.limit)}
            </span>
          </div>
        ) : null}
      </div>

      {run.parentRunId || derivedChildEvents.length > 0 ? (
        <div className="space-y-1 rounded-md border border-blue-500/20 bg-blue-500/10 px-2 py-1.5 text-[11px] text-blue-700 dark:text-blue-300">
          {run.parentRunId ? (
            <div className="flex min-w-0 items-center gap-1.5">
              <GitBranch className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">
                {run.origin === "repair"
                  ? t("workspace.workflow.derivedFromRepair", "修复自 {{id}}", {
                      id: run.parentRunId,
                    })
                  : t("workspace.workflow.derivedFrom", "派生自 {{id}}", { id: run.parentRunId })}
              </span>
            </div>
          ) : null}
          {derivedChildEvents.slice(-2).map((event) => {
            const payload = asRecord(event.payload)
            const childRunId = stringField(payload, "childRunId")
            const origin = stringField(payload, "origin")
            if (!childRunId) return null
            return (
              <div key={event.id} className="flex min-w-0 items-center gap-1.5">
                <GitBranch className="h-3.5 w-3.5 shrink-0" />
                <span className="min-w-0 flex-1 truncate">
                  {origin === "repair"
                    ? t("workspace.workflow.derivedChildRepair", "已生成修复运行 {{id}}", {
                        id: childRunId,
                      })
                    : t("workspace.workflow.derivedChild", "已生成派生运行 {{id}}", {
                        id: childRunId,
                      })}
                </span>
              </div>
            )
          })}
        </div>
      ) : null}

      <WorkflowRunFocusCard run={run} snapshot={snapshot} onSelectDetailTab={onSelectDetailTab} />
      {worktreeInfo ? <WorkflowRunWorktreeCard info={worktreeInfo} /> : null}
      <WorkflowRunTimelineCard snapshot={snapshot} />
      <WorkflowApprovalPreview snapshot={snapshot} />
      <WorkflowApprovalAudit snapshot={snapshot} />
      <WorkflowRecoveryHint
        run={run}
        snapshot={snapshot}
        onCreateRepairDraft={onCreateRepairDraft}
      />
      {actions ? <div>{actions}</div> : null}
    </div>
  )
}

function WorkflowMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md bg-secondary/35 px-1.5 py-1 text-center">
      <div className="truncate font-medium text-foreground/85">{value}</div>
      <div className="truncate text-muted-foreground/70">{label}</div>
    </div>
  )
}

const WORKFLOW_TIMELINE_DOT_CLASS: Record<StatusTone, string> = {
  muted: "bg-muted-foreground/45",
  good: "bg-emerald-500",
  warn: "bg-amber-500",
  danger: "bg-destructive",
  info: "bg-blue-500",
}

function WorkflowRunTimelineCard({ snapshot }: { snapshot: WorkflowRunSnapshot | null }) {
  const { t } = useTranslation()
  const events = workflowOverviewEvents(snapshot?.events ?? [])
  if (events.length === 0) return null

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <Clock className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
          {t("workspace.workflow.runTimeline", "运行时间线")}
        </span>
        <span className="shrink-0 text-[10px] text-muted-foreground/70">
          {t("workspace.workflow.recentEventCount", "最近 {{count}} 条", {
            count: events.length,
          })}
        </span>
      </div>
      <div className="space-y-1">
        {events.map((event) => (
          <WorkflowRunTimelineRow key={event.id} event={event} />
        ))}
      </div>
    </div>
  )
}

function WorkflowRunTimelineRow({ event }: { event: WorkflowEvent }) {
  const { t } = useTranslation()
  const title = workflowEventTitle(t, event)
  const detail = workflowEventDetail(t, event)
  const tone = workflowEventTone(event)
  return (
    <IconTip label={compactJson(event.payload, event.eventType)}>
      <div className="flex min-w-0 items-start gap-2 rounded-md px-1.5 py-1 text-[11px] hover:bg-background/45">
        <span
          className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", WORKFLOW_TIMELINE_DOT_CLASS[tone])}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 font-mono text-[10px] text-muted-foreground/70">
              #{event.seq}
            </span>
            <span className="min-w-0 flex-1 truncate text-foreground/85">{title}</span>
            <span className="max-w-[34%] shrink-0 truncate text-[10px] text-muted-foreground/65">
              {formatMessageTime(event.createdAt)}
            </span>
          </div>
          {detail ? (
            <div className="mt-0.5 truncate text-[10px] text-muted-foreground/75">{detail}</div>
          ) : null}
        </div>
      </div>
    </IconTip>
  )
}

function WorkflowRunWorktreeCard({ info }: { info: WorkflowRunWorktreeInfo }) {
  const { t } = useTranslation()
  return (
    <IconTip label={info.path}>
      <div className="space-y-1 rounded-md border border-blue-500/20 bg-blue-500/10 px-2 py-1.5 text-blue-700 dark:text-blue-300">
        <div className="flex min-w-0 items-center gap-1.5">
          <FolderGit2 className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
            {t("workspace.workflow.worktreeRuntime", "运行位置")} ·{" "}
            {info.label || basename(info.path)}
          </span>
          <StatusPill
            label={managedWorktreeStateLabel(t, info.state)}
            tone={managedWorktreeStateTone(info.state)}
          />
        </div>
        <div className="flex min-w-0 items-center gap-1.5 text-[10px] opacity-85">
          <span className="min-w-0 flex-1 truncate font-mono">
            {truncateMiddle(info.path, 120)}
          </span>
          {!info.pathExists ? (
            <StatusPill label={t("workspace.worktree.pathMissing", "路径已清理")} tone="warn" />
          ) : null}
        </div>
        <div className="grid grid-cols-3 gap-1 text-[10px]">
          <GoalWorktreeMetric
            label={t("workspace.workflow.worktreeBase", "Base")}
            value={info.baseLabel ?? "-"}
          />
          <GoalWorktreeMetric
            label={t("workspace.workflow.worktreeChanges", "Changes")}
            value={info.dirtyLabel ?? "-"}
          />
          <GoalWorktreeMetric
            label={t("workspace.workflow.worktreeSource", "Source")}
            value={
              info.source === "managed"
                ? t("workspace.workflow.worktreeSourceManaged", "Managed")
                : t("workspace.workflow.worktreeSourceTrace", "Trace")
            }
          />
        </div>
      </div>
    </IconTip>
  )
}

function WorkflowRunFocusCard({
  run,
  snapshot,
  onSelectDetailTab,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  onSelectDetailTab?: (tab: WorkflowDetailTab) => void
}) {
  const { t } = useTranslation()
  const ops = snapshot?.ops ?? []
  const events = snapshot?.events ?? []
  const activeOp = [...ops].reverse().find((op) => op.state === "started")
  const pendingOp = ops.find((op) => op.state === "pending")
  const validationFailureOp = [...ops].reverse().find(workflowOpHasValidationFailure)
  const failedOp = [...ops].reverse().find((op) => op.state === "failed") ?? validationFailureOp
  const focusOp = activeOp ?? pendingOp
  const permissionPreview = workflowPermissionPreview(snapshot)
  const waitEvent = [...events]
    .reverse()
    .find((event) => event.eventType.includes("user") || event.eventType.includes("ask"))
  const latestEvent = events.at(-1)
  const completed = ops.filter((op) => op.state === "completed").length
  const total = ops.length
  const failedError = asRecord(failedOp?.error)
  const failedMessage =
    stringField(failedError, "message") ??
    (failedOp ? workflowOpDetail(failedOp) : null) ??
    run.blockedReason

  let title: string
  let body: string
  let tone: "muted" | "good" | "warn" | "danger" | "info" = workflowRunTone(run.state)
  let Icon: LucideIcon = Radio
  let targetTab: WorkflowDetailTab | null = null

  if (run.state === "draft") {
    title = t("workspace.workflow.focusDraftTitle", "当前焦点：草稿待启动")
    body = t("workspace.workflow.focusDraftBody", "脚本已保存，运行前仍会保留 trace 与审批记录。")
    Icon = Play
  } else if (run.state === "awaiting_approval") {
    title = t("workspace.workflow.focusApprovalTitle", "当前焦点：等待授权")
    body =
      workflowPermissionSummaryText(t, permissionPreview?.summary) ||
      t("workspace.workflow.focusApprovalBody", "有调用需要确认，批准后 run 会继续。")
    tone = "warn"
    Icon = ShieldAlert
    targetTab = "trace"
  } else if (run.state === "awaiting_user") {
    title = t("workspace.workflow.focusUserTitle", "当前焦点：等待用户回复")
    body =
      (waitEvent ? workflowEventDetail(t, waitEvent) || workflowEventTitle(t, waitEvent) : null) ??
      t("workspace.workflow.focusUserBody", "当前运行正在等待会话里的用户输入或外部确认。")
    tone = "warn"
    Icon = MessageCircle
    targetTab = "trace"
  } else if (run.state === "running" || run.state === "recovering") {
    if (focusOp) {
      const opTitle = truncateMiddle(workflowOpTitle(focusOp), 56)
      title =
        run.state === "recovering"
          ? t("workspace.workflow.focusRecoveringOpTitle", "当前焦点：恢复 {{op}}", { op: opTitle })
          : activeOp
            ? t("workspace.workflow.focusRunningOpTitle", "当前焦点：正在执行 {{op}}", {
                op: opTitle,
              })
            : t("workspace.workflow.focusPendingOpTitle", "当前焦点：准备执行 {{op}}", {
                op: opTitle,
              })
      body = `${focusOp.opType} · ${truncateMiddle(workflowOpDetail(focusOp), 100)}`
      targetTab = workflowOpDetailTab(focusOp)
    } else {
      title =
        run.state === "recovering"
          ? t("workspace.workflow.focusRecoveringTitle", "当前焦点：恢复中")
          : t("workspace.workflow.focusRunningTitle", "当前焦点：运行中")
      body = latestEvent
        ? `${workflowEventTitle(t, latestEvent)} · ${workflowEventDetail(t, latestEvent) || `#${latestEvent.seq}`}`
        : t("workspace.workflow.focusRunningBody", "正在等待下一条运行信号。")
      targetTab = "trace"
    }
    tone = "info"
    Icon = run.state === "recovering" ? Clock : Radio
  } else if (run.state === "paused") {
    title = t("workspace.workflow.focusPausedTitle", "当前焦点：已暂停")
    body = focusOp
      ? t("workspace.workflow.focusPausedBodyWithOp", "暂停在 {{op}}，恢复后会继续该 run。", {
          op: truncateMiddle(workflowOpTitle(focusOp), 64),
        })
      : t("workspace.workflow.focusPausedBody", "恢复后会从当前 trace 继续，取消则保留已有记录。")
    tone = "warn"
    Icon = Pause
    targetTab = focusOp ? workflowOpDetailTab(focusOp) : "trace"
  } else if (run.state === "blocked") {
    title = t("workspace.workflow.focusBlockedTitle", "当前焦点：阻塞原因")
    body = truncateMiddle(
      run.blockedReason ?? t("workspace.workflow.blockedFallback", "需要人工处理"),
      140,
    )
    tone = "danger"
    Icon = CircleAlert
    targetTab = validationFailureOp ? "validation" : "trace"
  } else if (run.state === "failed") {
    title = validationFailureOp
      ? t("workspace.workflow.focusValidationFailedTitle", "当前焦点：验证失败")
      : t("workspace.workflow.focusFailedTitle", "当前焦点：步骤失败")
    body = truncateMiddle(
      failedMessage ??
        t("workspace.workflow.nextFailedBody", "查看 Trace 与 Validation，基于失败步骤继续修复。"),
      140,
    )
    tone = "danger"
    Icon = CircleAlert
    targetTab = validationFailureOp
      ? "validation"
      : failedOp
        ? workflowOpDetailTab(failedOp)
        : "trace"
  } else if (run.state === "completed") {
    title = t("workspace.workflow.focusCompletedTitle", "当前焦点：已完成")
    body =
      total > 0
        ? t(
            "workspace.workflow.focusCompletedBody",
            "{{completed}}/{{total}} 个步骤完成，验证和产物已保留。",
            {
              completed,
              total,
            },
          )
        : t("workspace.workflow.focusCompletedBodyNoOps", "运行已完成，trace 已保留。")
    tone = "good"
    Icon = CheckCircle2
    targetTab = "trace"
  } else {
    title = t("workspace.workflow.focusCancelledTitle", "当前焦点：已取消")
    body = t("workspace.workflow.focusCancelledBody", "运行已停止，已有 trace 可用于复盘。")
    tone = "muted"
    Icon = X
    targetTab = "trace"
  }

  const tabLabel = targetTab ? workflowDetailTabLabel(t, targetTab) : null

  return (
    <div
      className={cn(
        "rounded-md border px-2 py-1.5 text-[11px]",
        tone === "danger"
          ? "border-destructive/25 bg-destructive/10 text-destructive"
          : tone === "warn"
            ? "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300"
            : tone === "good"
              ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
              : tone === "info"
                ? "border-blue-500/25 bg-blue-500/10 text-blue-700 dark:text-blue-300"
                : "border-border/55 bg-secondary/20 text-muted-foreground",
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5 font-medium">
        <Icon className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{title}</span>
        {targetTab && onSelectDetailTab && tabLabel ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-6 min-w-0 shrink-0 gap-1 border-current/25 bg-background/45 px-1.5 text-[10px] hover:bg-background/70"
            onClick={() => onSelectDetailTab(targetTab)}
          >
            <Eye className="h-3 w-3" />
            <span className="truncate">
              {t("workspace.workflow.focusOpenTab", "查看 {{tab}}", { tab: tabLabel })}
            </span>
          </Button>
        ) : null}
      </div>
      <div className="mt-0.5 truncate opacity-85">{body}</div>
    </div>
  )
}

function WorkflowApprovalAudit({ snapshot }: { snapshot: WorkflowRunSnapshot | null }) {
  const { t } = useTranslation()
  const events = workflowApprovalAuditEvents(snapshot).slice(-6)
  if (events.length === 0) return null

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <ClipboardCheck className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
          {t("workspace.workflow.approvalAudit", "审批审计")}
        </span>
        <span className="shrink-0 text-[10px] text-muted-foreground/70">
          {t("workspace.workflow.approvalAuditCount", "{{count}} 条", { count: events.length })}
        </span>
      </div>
      <div className="space-y-1">
        {events.map((event) => (
          <WorkflowApprovalAuditRow key={event.id} event={event} />
        ))}
      </div>
    </div>
  )
}

function WorkflowApprovalAuditRow({ event }: { event: WorkflowEvent }) {
  const { t } = useTranslation()
  const tone = workflowApprovalAuditTone(event)
  const title = workflowApprovalAuditTitle(t, event)
  const detail = workflowEventDetail(t, event)
  return (
    <IconTip label={compactJson(event.payload, event.eventType)}>
      <div className="flex min-w-0 items-start gap-2 rounded-md px-1.5 py-1 text-[11px] hover:bg-background/45">
        <span
          className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", WORKFLOW_TIMELINE_DOT_CLASS[tone])}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 font-mono text-[10px] text-muted-foreground/70">
              #{event.seq}
            </span>
            <span className="min-w-0 flex-1 truncate text-foreground/85">{title}</span>
            <StatusPill label={workflowApprovalAuditStatusLabel(t, tone)} tone={tone} />
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground/75">
            {detail ? <span className="min-w-0 flex-1 truncate">{detail}</span> : null}
            <span className="shrink-0">{formatMessageTime(event.createdAt)}</span>
          </div>
        </div>
      </div>
    </IconTip>
  )
}

function WorkflowApprovalPreview({ snapshot }: { snapshot: WorkflowRunSnapshot | null }) {
  const preview = workflowPermissionPreview(snapshot)
  if (!preview) return null
  return <WorkflowPermissionPreviewCard preview={preview} />
}

function WorkflowPermissionPreviewCard({ preview }: { preview: WorkflowPermissionPreview }) {
  const { t } = useTranslation()
  const { summary, calls, truncated } = preview
  const total = numberField(summary, "total")
  const allow = numberField(summary, "allow")
  const ask = numberField(summary, "ask")
  const dynamic = numberField(summary, "dynamic")
  const deny = numberField(summary, "deny")
  const strict = numberField(summary, "strict")
  const visibleCalls = calls.slice(0, 5)

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <Shield className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
          {t("workspace.workflow.permissionChecklist", "授权清单")}
        </span>
        {truncated ? (
          <StatusPill label={t("workspace.workflow.truncated", "已截断")} tone="warn" />
        ) : null}
      </div>

      <div className="grid grid-cols-3 gap-1 text-[10px]">
        {typeof total === "number" ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricTotal", "调用")}
            value={String(total)}
          />
        ) : null}
        {typeof ask === "number" ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricAsk", "需批准")}
            value={String(ask)}
          />
        ) : null}
        {typeof strict === "number" ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricStrict", "Strict")}
            value={String(strict)}
          />
        ) : null}
        {typeof dynamic === "number" ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricDynamic", "动态")}
            value={String(dynamic)}
          />
        ) : null}
        {typeof deny === "number" && deny > 0 ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricDeny", "拒绝")}
            value={String(deny)}
          />
        ) : null}
        {typeof allow === "number" ? (
          <WorkflowMetric
            label={t("workspace.workflow.permissionMetricAllow", "自动")}
            value={String(allow)}
          />
        ) : null}
      </div>

      {visibleCalls.length > 0 ? (
        <div className="mt-1.5 space-y-1">
          {visibleCalls.map((call, index) => (
            <WorkflowPermissionCallRow
              key={`${workflowPermissionCallTitle(call)}:${index}`}
              call={call}
            />
          ))}
          {calls.length > visibleCalls.length ? (
            <div className="px-1 text-[10px] text-muted-foreground/70">
              {t("workspace.workflow.permissionMoreCalls", "另有 {{count}} 个调用", {
                count: calls.length - visibleCalls.length,
              })}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function WorkflowPermissionCallRow({ call }: { call: Record<string, unknown> }) {
  const { t } = useTranslation()
  const title = workflowPermissionCallTitle(call)
  const detail = workflowPermissionCallDetail(t, call)
  const args = call.args
  const argsPreview = args == null ? null : truncateMiddle(compactJson(args, ""), 110)
  return (
    <IconTip label={compactJson(call, title)}>
      <div className="rounded-md bg-background/40 px-2 py-1.5 text-[11px]">
        <div className="flex min-w-0 items-center gap-2">
          <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate font-medium text-foreground/85">
            {truncateMiddle(title, 88)}
          </span>
          <StatusPill
            label={workflowPermissionDecisionLabel(t, call)}
            tone={workflowPermissionDecisionTone(call)}
          />
        </div>
        {detail ? (
          <div className="mt-0.5 truncate pl-5 text-[10px] text-muted-foreground/80">{detail}</div>
        ) : null}
        {argsPreview ? (
          <div className="mt-0.5 truncate pl-5 font-mono text-[10px] text-muted-foreground/65">
            {argsPreview}
          </div>
        ) : null}
      </div>
    </IconTip>
  )
}

function WorkflowRecoveryHint({
  run,
  snapshot,
  onCreateRepairDraft,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  onCreateRepairDraft?: (repairPrompt: string, run: WorkflowRun) => void
}) {
  const { t } = useTranslation()
  const failedOp = [...(snapshot?.ops ?? [])].reverse().find((op) => op.state === "failed")
  const failedError = asRecord(failedOp?.error)
  const failedMessage = stringField(failedError, "message")
  const blockedReason = run.blockedReason
  const hasValidationFailure = (snapshot?.ops ?? []).some(workflowOpHasValidationFailure)
  const repairPrompt = buildWorkflowRepairPrompt(run, snapshot)

  let title: string | null = null
  let body: string | null = null
  let tone: "warn" | "danger" | "info" = "info"

  if (run.state === "awaiting_approval") {
    title = t("workspace.workflow.nextApproveTitle", "下一步：确认授权")
    body = t(
      "workspace.workflow.nextApproveBody",
      "检查上面的授权清单，确认后批准；不符合预期就取消。",
    )
    tone = "warn"
  } else if (run.state === "paused") {
    title = t("workspace.workflow.nextPausedTitle", "下一步：恢复或取消")
    body = t(
      "workspace.workflow.nextPausedBody",
      "当前运行已暂停，可恢复继续执行，也可取消并保留 trace。",
    )
    tone = "warn"
  } else if (run.state === "blocked") {
    title = t("workspace.workflow.nextBlockedTitle", "下一步：处理阻塞")
    body =
      blockedReason === "script_hash_mismatch"
        ? t(
            "workspace.workflow.nextBlockedScriptHash",
            "脚本内容已变化；请基于当前目标生成新的工作流。",
          )
        : truncateMiddle(
            blockedReason ?? t("workspace.workflow.blockedFallback", "需要人工处理"),
            140,
          )
    tone = "danger"
  } else if (run.state === "failed" || failedOp || hasValidationFailure) {
    title = hasValidationFailure
      ? t("workspace.workflow.nextValidationTitle", "下一步：修复验证失败")
      : t("workspace.workflow.nextFailedTitle", "下一步：定位失败步骤")
    body =
      failedMessage ??
      failedOp?.opKey ??
      t("workspace.workflow.nextFailedBody", "查看 Trace 与 Validation，基于失败步骤继续修复。")
    tone = "danger"
  }

  if (!title || !body) return null

  const copyRepairPrompt = async () => {
    if (!repairPrompt) return
    try {
      await navigator.clipboard.writeText(repairPrompt)
      toast.success(t("workspace.workflow.repairPromptCopied", "已复制修复提示"))
    } catch (e) {
      logger.error(
        "ui",
        "WorkflowRecoveryHint::copyRepairPrompt",
        "Copy workflow repair prompt failed",
        e,
      )
      toast.error(t("workspace.workflow.repairPromptCopyFailed", "复制修复提示失败"))
    }
  }

  return (
    <div
      className={cn(
        "rounded-md border px-2 py-1.5 text-[11px]",
        tone === "danger"
          ? "border-destructive/25 bg-destructive/10 text-destructive"
          : tone === "warn"
            ? "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300"
            : "border-blue-500/25 bg-blue-500/10 text-blue-700 dark:text-blue-300",
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5 font-medium">
        {tone === "danger" ? (
          <CircleAlert className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
        )}
        <span className="truncate">{title}</span>
      </div>
      <div className="mt-0.5 truncate opacity-85">{body}</div>
      {repairPrompt ? (
        <div className="mt-1.5 grid grid-cols-2 gap-1.5">
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-7 min-w-0 gap-1.5 border-current/25 bg-background/45 text-[11px] hover:bg-background/70"
            onClick={() => onCreateRepairDraft?.(repairPrompt, run)}
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span className="truncate">
              {t("workspace.workflow.createRepairDraft", "生成修复草稿")}
            </span>
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-7 min-w-0 gap-1.5 border-current/25 bg-background/45 text-[11px] hover:bg-background/70"
            onClick={() => void copyRepairPrompt()}
          >
            <Copy className="h-3.5 w-3.5" />
            <span className="truncate">
              {t("workspace.workflow.copyRepairPrompt", "复制修复提示")}
            </span>
          </Button>
        </div>
      ) : null}
    </div>
  )
}

function WorkflowTraceTimeline({ snapshot }: { snapshot: WorkflowRunSnapshot }) {
  const { t } = useTranslation()
  const indexedOps = snapshot.ops.map((op, index) => ({ op, index: index + 1 }))
  const focusOps = indexedOps
    .filter(({ op }) => workflowOpNeedsAttention(op))
    .slice(-WORKFLOW_FOCUS_OP_PREVIEW)
  const previewOps = indexedOps.slice(0, WORKFLOW_OP_PREVIEW)
  const importantEvents = snapshot.events
    .filter(workflowEventNeedsAttention)
    .slice(-WORKFLOW_EVENT_PREVIEW)
  const importantEventIds = new Set(importantEvents.map((event) => event.id))
  const recentEvents = snapshot.events
    .slice(-WORKFLOW_EVENT_PREVIEW)
    .filter((event) => !importantEventIds.has(event.id))
  if (snapshot.ops.length === 0 && snapshot.events.length === 0) {
    return <EmptyHint>{t("workspace.workflow.emptyTrace", "暂无 trace")}</EmptyHint>
  }

  return (
    <div className="space-y-2">
      {focusOps.length > 0 ? (
        <div className="space-y-1">
          <div className="flex min-w-0 items-center justify-between gap-2 px-1 text-[10px] font-medium uppercase tracking-normal text-muted-foreground/70">
            <span className="truncate">{t("workspace.workflow.focusOps", "关注步骤")}</span>
            <span className="shrink-0 tabular-nums">
              {t("workspace.workflow.focusOpsCount", "{{count}} 个", { count: focusOps.length })}
            </span>
          </div>
          <div className="space-y-1">
            {focusOps.map(({ op, index }) => (
              <WorkflowOpRow key={`focus:${op.id}`} op={op} index={index} />
            ))}
          </div>
        </div>
      ) : null}

      {previewOps.length > 0 ? (
        <div className="space-y-1">
          <div className="px-1 text-[10px] font-medium uppercase tracking-normal text-muted-foreground/70">
            {workflowOpSummary(t, snapshot.ops)}
          </div>
          <div className="space-y-1">
            {previewOps.map(({ op, index }) => (
              <WorkflowOpRow key={op.id} op={op} index={index} />
            ))}
          </div>
          {snapshot.ops.length > previewOps.length ? (
            <div className="px-2 text-[10px] text-muted-foreground/60">
              {t(
                "workspace.workflow.opPreviewTruncated",
                "先显示前 {{shown}}/{{total}} 个步骤；失败和运行中的步骤会在关注步骤中置顶。",
                {
                  shown: previewOps.length,
                  total: snapshot.ops.length,
                },
              )}
            </div>
          ) : null}
        </div>
      ) : null}

      {importantEvents.length > 0 ? (
        <div className="space-y-1">
          <div className="px-1 text-[10px] font-medium uppercase tracking-normal text-muted-foreground/70">
            {t("workspace.workflow.keySignals", "关键信号")}
          </div>
          <div className="space-y-1">
            {importantEvents.map((event) => (
              <WorkflowEventRow key={`important:${event.id}`} event={event} />
            ))}
          </div>
        </div>
      ) : null}

      {recentEvents.length > 0 ? (
        <div className="space-y-1">
          <div className="px-1 text-[10px] font-medium uppercase tracking-normal text-muted-foreground/70">
            {t("workspace.workflow.recentSignals", "最近信号")}
          </div>
          <div className="space-y-1">
            {recentEvents.map((event) => (
              <WorkflowEventRow key={event.id} event={event} />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  )
}

function WorkflowOpRow({ op, index }: { op: WorkflowOp; index: number }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const tone = workflowOpTone(op)
  const title = workflowOpTitle(op)
  const detail = workflowOpDetail(op)
  const payload = op.output ?? op.error ?? op.input
  const payloadText = prettyJson(payload, t("workspace.workflow.noDetails", "暂无详情"))
  const Icon =
    op.opType === "validate"
      ? CheckCircle2
      : op.opType === "spawnAgent"
        ? Bot
        : op.opType === "fileSearch"
          ? Search
          : op.opType === "tool"
            ? Cpu
            : Radio

  const copyDetails = async () => {
    try {
      await navigator.clipboard.writeText(payloadText)
      toast.success(t("workspace.workflow.detailsCopied", "已复制详情"))
    } catch (e) {
      logger.error("ui", "WorkflowOpRow::copyDetails", "Copy workflow op details failed", e)
      toast.error(t("workspace.workflow.detailsCopyFailed", "复制详情失败"))
    }
  }

  return (
    <div className="rounded-md hover:bg-secondary/35">
      <IconTip label={compactJson(payload, op.opKey)}>
        <div className="flex min-w-0 gap-2 px-2 py-1.5 text-xs">
          <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-secondary/65 text-[10px] text-muted-foreground">
            {index}
          </div>
          <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
                {truncateMiddle(title, 88)}
              </span>
              <StatusPill label={op.state} tone={tone} loading={op.state === "started"} />
              <button
                type="button"
                className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
                aria-label={
                  expanded
                    ? t("workspace.workflow.collapseStepDetails", "收起步骤详情")
                    : t("workspace.workflow.expandStepDetails", "展开步骤详情")
                }
                aria-expanded={expanded}
                onClick={() => setExpanded((value) => !value)}
              >
                <ChevronDown
                  className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
                />
              </button>
            </div>
            <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground">
              <span className="truncate font-mono">{op.opKey}</span>
              <span className="shrink-0 text-muted-foreground/45">·</span>
              <span className="shrink-0">{op.opType}</span>
            </div>
            <div className="mt-0.5 truncate text-[11px] text-muted-foreground/80">
              {truncateMiddle(detail, 120)}
            </div>
          </div>
        </div>
      </IconTip>
      <AnimatedCollapse open={expanded}>
        <div className="mx-2 mb-1.5 rounded-md border border-border/55 bg-background/65 p-2">
          <div className="mb-1.5 flex min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.stepDetails", "步骤详情")}
            </span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-6 shrink-0 gap-1 px-1.5 text-[10px]"
              onClick={() => void copyDetails()}
            >
              <Copy className="h-3 w-3" />
              <span>{t("common.copy", "复制")}</span>
            </Button>
          </div>
          <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-secondary/30 p-2 font-mono text-[10px] leading-relaxed text-muted-foreground">
            {payloadText}
          </pre>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function WorkflowEventRow({ event }: { event: WorkflowEvent }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const label = truncateMiddle(compactJson(event.payload, event.eventType), 120)
  const title = workflowEventTitle(t, event)
  const detail = workflowEventDetail(t, event)
  const payloadText = prettyJson(event.payload, t("workspace.workflow.noDetails", "暂无详情"))

  const copyDetails = async () => {
    try {
      await navigator.clipboard.writeText(payloadText)
      toast.success(t("workspace.workflow.detailsCopied", "已复制详情"))
    } catch (e) {
      logger.error("ui", "WorkflowEventRow::copyDetails", "Copy workflow event details failed", e)
      toast.error(t("workspace.workflow.detailsCopyFailed", "复制详情失败"))
    }
  }

  return (
    <div className="rounded-md hover:bg-secondary/35">
      <IconTip label={label}>
        <div className="flex min-w-0 items-start gap-2 px-2 py-1.5 text-[11px] text-muted-foreground">
          <Clock className="h-3 w-3 shrink-0" />
          <span className="shrink-0 font-mono">#{event.seq}</span>
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-foreground/85">{title}</span>
              <span className="max-w-[38%] shrink-0 truncate">
                {formatMessageTime(event.createdAt)}
              </span>
              <button
                type="button"
                className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
                aria-label={
                  expanded
                    ? t("workspace.workflow.collapseEventDetails", "收起事件详情")
                    : t("workspace.workflow.expandEventDetails", "展开事件详情")
                }
                aria-expanded={expanded}
                onClick={() => setExpanded((value) => !value)}
              >
                <ChevronDown
                  className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
                />
              </button>
            </div>
            {detail ? (
              <div className="mt-0.5 truncate text-[10px] text-muted-foreground/75">{detail}</div>
            ) : null}
          </div>
        </div>
      </IconTip>
      <AnimatedCollapse open={expanded}>
        <div className="mx-2 mb-1.5 rounded-md border border-border/55 bg-background/65 p-2">
          <div className="mb-1.5 flex min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.eventDetails", "事件详情")}
            </span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-6 shrink-0 gap-1 px-1.5 text-[10px]"
              onClick={() => void copyDetails()}
            >
              <Copy className="h-3 w-3" />
              <span>{t("common.copy", "复制")}</span>
            </Button>
          </div>
          <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-secondary/30 p-2 font-mono text-[10px] leading-relaxed text-muted-foreground">
            {payloadText}
          </pre>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function WorkflowValidationTab({ snapshot }: { snapshot: WorkflowRunSnapshot }) {
  const { t } = useTranslation()
  const validationOps = snapshot.ops.filter((op) => op.opType === "validate")
  if (validationOps.length === 0) {
    return <EmptyHint>{t("workspace.workflow.noValidation", "暂无验证记录")}</EmptyHint>
  }
  const passedCount = validationOps.filter(
    (op) => boolField(asRecord(op.output), "ok") === true,
  ).length
  const failedCount = validationOps.filter(workflowOpHasValidationFailure).length
  const runningCount = validationOps.filter((op) => op.state === "started").length

  const repairEventsByOp = new Map<string, WorkflowEvent>()
  for (const event of snapshot.events) {
    if (
      event.eventType !== "guarded_repair_validation_failed" &&
      event.eventType !== "guarded_repair_validation_passed"
    ) {
      continue
    }
    const payload = asRecord(event.payload)
    const opKey = stringField(payload, "opKey")
    if (opKey) repairEventsByOp.set(opKey, event)
  }

  return (
    <div className="space-y-1.5">
      <div className="grid grid-cols-4 gap-1 text-[10px]">
        <WorkflowMetric
          label={t("workspace.workflow.validationMetricTotal", "验证")}
          value={String(validationOps.length)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.validationMetricPassed", "通过")}
          value={String(passedCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.validationMetricFailed", "失败")}
          value={String(failedCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.validationMetricRunning", "运行中")}
          value={String(runningCount)}
        />
      </div>
      {validationOps.map((op) => {
        const output = asRecord(op.output)
        const error = asRecord(op.error)
        const repairEvent = repairEventsByOp.get(op.opKey)
        const repairPayload = asRecord(repairEvent?.payload)
        const ok = boolField(output, "ok")
        const results = recordArrayField(output, "results")
        const stopReason = stringField(repairPayload, "stopReason")
        const summary =
          stringField(output, "summary") ??
          stringField(repairPayload, "summary") ??
          stringField(error, "message") ??
          op.state
        const failed = numberField(repairPayload, "failed")
        const total = numberField(repairPayload, "total") ?? results.length
        const visibleResults = results.slice(0, 4)
        const tone =
          stopReason || ok === false || op.state === "failed"
            ? "danger"
            : ok === true
              ? "good"
              : "info"

        return (
          <IconTip key={op.id} label={compactJson(op.output ?? op.error ?? op.input, op.opKey)}>
            <div className="rounded-md px-2 py-1.5 text-xs hover:bg-secondary/35">
              <div className="flex min-w-0 items-center gap-2">
                {tone === "good" ? (
                  <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                ) : tone === "danger" ? (
                  <CircleAlert className="h-3.5 w-3.5 shrink-0 text-destructive" />
                ) : (
                  <Radio className="h-3.5 w-3.5 shrink-0 text-blue-500" />
                )}
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/85">
                  {op.opKey}
                </span>
                <StatusPill
                  label={
                    ok === true
                      ? t("workspace.workflow.validationPassed", "通过")
                      : ok === false
                        ? t("workspace.workflow.validationFailed", "失败")
                        : op.state
                  }
                  tone={tone}
                  loading={op.state === "started"}
                />
              </div>
              <div className="mt-1 min-w-0 space-y-0.5 pl-5 text-[11px] text-muted-foreground">
                <div className="truncate">{summary}</div>
                {typeof failed === "number" || total > 0 ? (
                  <div className="tabular-nums">
                    {t("workspace.workflow.validationCount", "{{failed}}/{{total}} failed", {
                      failed: failed ?? (ok === false ? 1 : 0),
                      total,
                    })}
                  </div>
                ) : null}
                {visibleResults.length > 0 ? (
                  <div className="space-y-1 pt-0.5">
                    {visibleResults.map((result, index) => (
                      <WorkflowValidationResultRow key={`${op.id}:${index}`} result={result} />
                    ))}
                    {results.length > visibleResults.length ? (
                      <div className="text-[10px] text-muted-foreground/70">
                        {t(
                          "workspace.workflow.validationMoreCommands",
                          "另有 {{count}} 条验证命令",
                          {
                            count: results.length - visibleResults.length,
                          },
                        )}
                      </div>
                    ) : null}
                  </div>
                ) : null}
                {stopReason ? (
                  <div className="truncate text-destructive">
                    {t("workspace.workflow.stopReason", "停止")} · {stopReason}
                  </div>
                ) : null}
              </div>
            </div>
          </IconTip>
        )
      })}
    </div>
  )
}

function WorkflowValidationResultRow({ result }: { result: Record<string, unknown> }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const command =
    stringField(result, "command") ?? t("workspace.workflow.validationCommand", "验证命令")
  const cwd = stringField(result, "cwd")
  const jobStatus = stringField(result, "jobStatus")
  const ok = boolField(result, "ok")
  const exitCode = numberField(result, "exitCode")
  const output = stringField(result, "output")
  const detailsText = output ?? prettyJson(result, command)
  const tone: "good" | "danger" | "info" =
    ok === true
      ? "good"
      : ok === false || (typeof exitCode === "number" && exitCode !== 0)
        ? "danger"
        : "info"

  const copyDetails = async () => {
    try {
      await navigator.clipboard.writeText(detailsText)
      toast.success(t("workspace.workflow.detailsCopied", "已复制详情"))
    } catch (e) {
      logger.error(
        "ui",
        "WorkflowValidationResultRow::copyDetails",
        "Copy workflow validation output failed",
        e,
      )
      toast.error(t("workspace.workflow.detailsCopyFailed", "复制详情失败"))
    }
  }

  return (
    <IconTip label={compactJson(result, command)}>
      <div className="rounded-md bg-background/45 px-2 py-1">
        <div className="flex min-w-0 items-center gap-2">
          {tone === "good" ? (
            <CheckCircle2 className="h-3 w-3 shrink-0 text-emerald-500" />
          ) : tone === "danger" ? (
            <CircleAlert className="h-3 w-3 shrink-0 text-destructive" />
          ) : (
            <Radio className="h-3 w-3 shrink-0 text-blue-500" />
          )}
          <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-foreground/80">
            {command}
          </span>
          {typeof exitCode === "number" ? (
            <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
              exit {exitCode}
            </span>
          ) : null}
          <button
            type="button"
            className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
            aria-label={
              expanded
                ? t("workspace.workflow.collapseValidationOutput", "收起验证输出")
                : t("workspace.workflow.expandValidationOutput", "展开验证输出")
            }
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            <ChevronDown
              className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
            />
          </button>
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 pl-5 text-[10px] text-muted-foreground/75">
          {jobStatus ? <span className="shrink-0">{jobStatus}</span> : null}
          {jobStatus && cwd ? <span className="text-muted-foreground/45">·</span> : null}
          {cwd ? <span className="truncate">{cwd}</span> : null}
        </div>
        {output ? (
          <div className="mt-0.5 truncate pl-5 font-mono text-[10px] text-muted-foreground/65">
            {truncateMiddle(output, 130)}
          </div>
        ) : null}
        <AnimatedCollapse open={expanded}>
          <div className="mt-1 rounded-md border border-border/55 bg-background/65 p-2">
            <div className="mb-1.5 flex min-w-0 items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-[10px] font-medium text-muted-foreground">
                {t("workspace.workflow.validationOutput", "验证输出")}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-6 shrink-0 gap-1 px-1.5 text-[10px]"
                onClick={() => void copyDetails()}
              >
                <Copy className="h-3 w-3" />
                <span>{t("common.copy", "复制")}</span>
              </Button>
            </div>
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-secondary/30 p-2 font-mono text-[10px] leading-relaxed text-muted-foreground">
              {detailsText}
            </pre>
          </div>
        </AnimatedCollapse>
      </div>
    </IconTip>
  )
}

function workflowAgentStatusInfo(op: WorkflowOp): { status: string; tone: StatusTone } {
  const output = asRecord(op.output)
  const status = stringField(output, "status") ?? op.state
  const tone: StatusTone =
    status === "completed" || status === "success"
      ? "good"
      : status === "failed" || status === "cancelled" || op.state === "failed"
        ? "danger"
        : status === "queued" ||
            status === "running" ||
            status === "spawned" ||
            op.state === "started"
          ? "info"
          : "muted"
  return { status, tone }
}

function WorkflowAgentsTab({
  snapshot,
  onViewSubagentSession,
}: {
  snapshot: WorkflowRunSnapshot
  onViewSubagentSession?: (sessionId: string) => void
}) {
  const { t } = useTranslation()
  const agentOps = snapshot.ops.filter((op) => op.opType === "spawnAgent")
  if (agentOps.length === 0) {
    return <EmptyHint>{t("workspace.workflow.noAgents", "暂无子 Agent 记录")}</EmptyHint>
  }
  const agentStatusInfos = agentOps.map(workflowAgentStatusInfo)
  const completedCount = agentStatusInfos.filter((info) => info.tone === "good").length
  const failedCount = agentStatusInfos.filter((info) => info.tone === "danger").length
  const runningCount = agentStatusInfos.filter((info) => info.tone === "info").length

  return (
    <div className="space-y-1.5">
      <div className="grid grid-cols-4 gap-1 text-[10px]">
        <WorkflowMetric
          label={t("workspace.workflow.agentMetricTotal", "Agents")}
          value={String(agentOps.length)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.agentMetricDone", "完成")}
          value={String(completedCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.agentMetricRunning", "运行中")}
          value={String(runningCount)}
        />
        <WorkflowMetric
          label={t("workspace.workflow.agentMetricFailed", "失败")}
          value={String(failedCount)}
        />
      </div>
      {agentOps.map((op, index) => {
        const output = asRecord(op.output)
        const input = asRecord(op.input)
        const runId =
          stringField(output, "runId") ?? stringField(output, "run_id") ?? op.childHandle ?? null
        const sessionId = stringField(output, "sessionId")
        const label = stringField(output, "label") ?? stringField(input, "label")
        const task = stringField(output, "task")
        const { status, tone } = agentStatusInfos[index]

        return (
          <IconTip key={op.id} label={compactJson(op.output ?? op.input, op.opKey)}>
            <div className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-secondary/35">
              <Bot className="h-3.5 w-3.5 shrink-0 text-blue-500" />
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 truncate font-mono text-[11px] text-foreground/85">
                    {label ?? runId ?? op.opKey}
                  </span>
                  <StatusPill
                    label={status}
                    tone={tone}
                    loading={status === "running" || op.state === "started"}
                  />
                </div>
                <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                  {task ? `${task} · ` : ""}
                  {runId ? truncateMiddle(runId, 72) : op.opKey}
                </div>
              </div>
              {sessionId && onViewSubagentSession ? (
                <IconTip label={t("workspace.workflow.openAgentSession", "打开子会话")}>
                  <button
                    type="button"
                    className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md border border-border/50 text-muted-foreground transition-colors hover:bg-secondary/65 hover:text-foreground"
                    onClick={(e) => {
                      e.stopPropagation()
                      onViewSubagentSession(sessionId)
                    }}
                    aria-label={t("workspace.workflow.openAgentSession", "打开子会话")}
                  >
                    <Eye className="h-3 w-3" />
                  </button>
                </IconTip>
              ) : null}
            </div>
          </IconTip>
        )
      })}
    </div>
  )
}

/**
 * 右侧「工作台」面板:把本会话的任务进度、碰到的文件、引用来源聚合到一处。
 * 文件 / 来源走 useWorkspaceArtifacts —— 后端读时聚合全会话历史 + 当前轮 live tail
 * 内存合并;输出 / 来源两段各自定高内部滚动,滚到底自动增量渲染(无按钮)。
 */
export default function WorkspacePanel({
  taskSnapshot,
  taskExecutionState = "idle",
  messages,
  contextUsageOverride,
  onOpenDiff,
  onPreviewFile,
  sessionId,
  sessionMeta,
  project,
  effectiveWorkingDir,
  workingDirSource,
  permissionMode = "default",
  planState = "off",
  activeModel,
  agentName,
  reasoningEffort,
  availableModels,
  currentAgentId,
  compacting = false,
  onCompactContext,
  onCommandAction,
  onViewSystemPrompt,
  systemPromptLoading,
  incognito = false,
  turnActive = false,
  workflowRunsState,
  backgroundJobs = [],
  backgroundJobExpansionOverrides,
  onBackgroundJobExpandedChange,
  onOpenBackgroundJobs,
  onViewSubagentSession,
  onEnsureSession,
  onClose,
}: WorkspacePanelProps) {
  const { t } = useTranslation()
  const { files, sources, filesTruncated, sourcesTruncated } = useWorkspaceArtifacts(
    sessionId,
    messages,
    { incognito, turnActive },
  )
  const ownedWorkflowRunsState = useWorkflowRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(workflowRunsState),
  })
  const sharedWorkflowRunsState = workflowRunsState ?? ownedWorkflowRunsState
  const loopSchedulesState = useLoopSchedules(sessionId, { incognito, turnActive })
  const goalState = useGoal(sessionId, { incognito })
  const [loopCreateRequest, setLoopCreateRequest] = useState(0)
  const [loopInspectRequest, setLoopInspectRequest] = useState<{
    loopId: string
    nonce: number
  } | null>(null)
  const reviewRunsState = useReviewRuns(sessionId, { incognito, turnActive })
  const verificationRunsState = useVerificationRuns(sessionId, { incognito, turnActive })
  const domainQualityRunsState = useDomainQualityRuns(sessionId, { incognito, turnActive })
  const domainTaskWorkbenchState = useDomainTaskWorkbench(sessionId, { incognito, turnActive })
  const domainWorkbenchRef = useRef<HTMLDivElement | null>(null)
  const [domainWorkbenchFocusRequest, setDomainWorkbenchFocusRequest] = useState<{
    target: "operational" | "soak"
    nonce: number
  } | null>(null)
  const workflowSectionRef = useRef<HTMLDivElement | null>(null)
  const [workflowFocusTarget, setWorkflowFocusTarget] = useState<WorkflowFocusTarget | null>(null)
  const focusDomainWorkbench = useCallback((target: "operational" | "soak") => {
    setDomainWorkbenchFocusRequest((current) => ({
      target,
      nonce: (current?.nonce ?? 0) + 1,
    }))
    window.setTimeout(() => {
      domainWorkbenchRef.current?.scrollIntoView?.({ block: "start", behavior: "smooth" })
    }, 0)
  }, [])
  const focusWorkflowRun = useCallback((runId: string) => {
    setWorkflowFocusTarget((current) => ({
      runId,
      nonce: (current?.nonce ?? 0) + 1,
    }))
    window.setTimeout(() => {
      workflowSectionRef.current?.scrollIntoView?.({ block: "start", behavior: "smooth" })
    }, 0)
  }, [])

  const {
    visible: visibleFiles,
    hasMore: hasMoreFiles,
    setSentinel: setFilesSentinel,
  } = useScrollPagedRender(files, { step: RENDER_STEP, resetKey: sessionId })
  const {
    visible: visibleSources,
    hasMore: hasMoreSources,
    setSentinel: setSourcesSentinel,
  } = useScrollPagedRender(sources, { step: RENDER_STEP, resetKey: sessionId })

  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2">
        <LayoutDashboard className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate text-sm font-medium">{t("workspace.panelTitle", "工作台")}</span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      {/* 上下边缘柔化淡出 —— 内容滚到边界时渐隐不硬切（mask 渐变到透明，露出面板底色）。
          Tauri = WebKit,补 `-webkit-mask-image` 兜底。 */}
      <div className={cn("flex-1 space-y-2 overflow-auto p-2", PANEL_SCROLL_FADE)}>
        {/* 会话 — 复刻状态悬浮窗的能力(模型 / 上下文 / 动作),核心常驻 + 展开更多。 */}
        <SessionSection
          sessionId={sessionId}
          sessionMeta={sessionMeta}
          agentName={agentName}
          reasoningEffort={reasoningEffort}
          activeModel={activeModel}
          availableModels={availableModels}
          messages={messages}
          contextUsageOverride={contextUsageOverride}
          currentAgentId={currentAgentId}
          turnActive={turnActive}
          compacting={compacting}
          onCompactContext={onCompactContext}
          onCommandAction={onCommandAction}
          onViewSystemPrompt={onViewSystemPrompt}
          systemPromptLoading={systemPromptLoading}
        />

        <EnvironmentSection
          sessionId={sessionId}
          sessionMeta={sessionMeta}
          project={project}
          effectiveWorkingDir={effectiveWorkingDir}
          workingDirSource={workingDirSource}
          permissionMode={permissionMode}
          planState={planState}
          turnActive={turnActive}
          onOpenDiff={onOpenDiff}
        />

        <ContextRetrievalSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          workingDir={effectiveWorkingDir}
          onPreviewFile={onPreviewFile}
          onDomainEvidenceRecorded={domainTaskWorkbenchState.refreshAll}
        />

        <div ref={domainWorkbenchRef}>
          <DomainTaskWorkbenchSection
            sessionId={sessionId}
            incognito={incognito}
            workingDir={effectiveWorkingDir}
            reviewRunsState={reviewRunsState}
            verificationRunsState={verificationRunsState}
            domainQualityRunsState={domainQualityRunsState}
            domainWorkbenchState={domainTaskWorkbenchState}
            focusRequest={domainWorkbenchFocusRequest}
          />
        </div>

        <LspDiagnosticsSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
        />

        <ReviewSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          workingDir={effectiveWorkingDir}
          reviewRunsState={reviewRunsState}
        />

        <VerificationSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          workingDir={effectiveWorkingDir}
          verificationRunsState={verificationRunsState}
        />

        <DomainQualitySection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          domainQualityRunsState={domainQualityRunsState}
          domainWorkbenchState={domainTaskWorkbenchState}
        />

        <CodingTrendSection sessionId={sessionId} incognito={incognito} turnActive={turnActive} />

        {/* 进度 — 复用 TaskProgressPanel(自带「任务 · N/M」折叠头)。 */}
        {taskSnapshot && taskSnapshot.total > 0 ? (
          <TaskProgressPanel
            snapshot={taskSnapshot}
            variant="card"
            executionState={taskExecutionState}
          />
        ) : (
          <WorkspaceSection
            title={t("workspace.sectionProgress", "进度")}
            count={0}
            icon={LayoutDashboard}
          >
            <EmptyHint>{t("workspace.emptyProgress", "暂无任务")}</EmptyHint>
          </WorkspaceSection>
        )}

        {/* Workflow — 动态脚本 run 的可观察、可暂停、可批准控制面。 */}
        <div ref={workflowSectionRef}>
          <WorkflowRunsSection
            sessionId={sessionId}
            incognito={incognito}
            turnActive={turnActive}
            workingDir={effectiveWorkingDir}
            onEnsureSession={onEnsureSession}
            onViewSubagentSession={onViewSubagentSession}
            onOpenLoopCreate={() => setLoopCreateRequest((value) => value + 1)}
            onOpenLoopProblem={(loopId) =>
              setLoopInspectRequest((current) => ({
                loopId,
                nonce: (current?.nonce ?? 0) + 1,
              }))
            }
            onOpenDomainWorkbench={focusDomainWorkbench}
            workflowRunsState={sharedWorkflowRunsState}
            loopSchedulesState={loopSchedulesState}
            goalState={goalState}
            domainWorkbenchState={domainTaskWorkbenchState}
            focusedRunTarget={workflowFocusTarget}
          />
        </div>

        {/* Loop — Phase 2.9 真正的定时/重复/条件触发控制面。 */}
        <LoopSchedulesSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          workflowRuns={sharedWorkflowRunsState.runs}
          onSelectWorkflowRun={focusWorkflowRun}
          loopSchedulesState={loopSchedulesState}
          goalState={goalState}
          createRequest={loopCreateRequest}
          inspectRequest={loopInspectRequest}
        />

        {/* 后台任务 — R4 复用独立面板的任务行能力,工作台内保留紧凑展示。 */}
        <BackgroundJobsSection
          jobs={backgroundJobs}
          jobExpansionOverrides={backgroundJobExpansionOverrides}
          onJobExpandedChange={onBackgroundJobExpandedChange}
          onOpenPanel={onOpenBackgroundJobs}
          onViewSubagentSession={onViewSubagentSession}
        />

        {/* 输出 — 本会话碰到的文件(读 + 改),定高内部滚动 + 滚动增量渲染。 */}
        <WorkspaceSection
          title={t("workspace.sectionOutput", "输出")}
          count={files.length}
          icon={Files}
        >
          {files.length > 0 ? (
            <div className="max-h-[40vh] space-y-1 overflow-y-auto pr-0.5">
              {visibleFiles.map((entry) => (
                <FileRow
                  key={entry.path}
                  entry={entry}
                  sessionId={sessionId}
                  onOpenDiff={onOpenDiff}
                  onPreviewFile={onPreviewFile}
                />
              ))}
              {hasMoreFiles && <div ref={setFilesSentinel} className="h-px" />}
              {filesTruncated && <TruncatedNote />}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptyOutput", "还没有碰到文件")}</EmptyHint>
          )}
        </WorkspaceSection>

        {/* 来源 — web_search 命中 + 正文链接,定高内部滚动 + 滚动增量渲染。 */}
        <WorkspaceSection
          title={t("workspace.sectionSources", "来源")}
          count={sources.length}
          icon={Globe}
        >
          {sources.length > 0 ? (
            <div className="max-h-[40vh] space-y-0.5 overflow-y-auto pr-0.5">
              {visibleSources.map((source) => (
                <SourceRow key={source.url} source={source} />
              ))}
              {hasMoreSources && <div ref={setSourcesSentinel} className="h-px" />}
              {sourcesTruncated && <TruncatedNote />}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptySources", "还没有引用来源")}</EmptyHint>
          )}
        </WorkspaceSection>

        {/* 知识空间 — 挂载的库(读/写)+ 本会话笔记活动。 */}
        <KnowledgeSection
          sessionId={sessionId}
          projectId={project?.id ?? sessionMeta?.projectId ?? null}
          incognito={incognito}
          messages={messages}
        />
      </div>
    </div>
  )
}
