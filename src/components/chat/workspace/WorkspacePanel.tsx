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
  ExternalLink,
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
  Network,
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
import { SearchInput } from "@/components/ui/search-input"
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
import { getTransport, useTransport } from "@/lib/transport-provider"
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
  DomainConnectorE2EGateReport,
  DomainEvidenceItem,
  DomainEvidenceRequirement,
  DomainOperationalGateReport,
  DomainQualityCheck,
  DomainQualityCheckStatus,
  DomainQualityRunSnapshot,
  DomainQualityRunState,
  DomainQualitySeverity,
  DomainSoakIncident,
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
  GitPullRequestReviewComment,
  SessionGitDiffSnapshot,
  VerificationRisk,
  VerificationRunSnapshot,
  VerificationStep,
  VerificationStepState,
} from "@/lib/transport"
import {
  computeContextUsage,
  contextUsageBarClass,
  formatMessageTime,
  type ContextUsageInfo,
} from "../chatUtils"
import {
  memoryKindLabel,
  memoryOriginLabel,
  retrievalLayerLabel,
  retrievalLayerReasonLabel,
  retrievalLayerStatusLabel,
  retrievalTraceStatusLabel,
} from "../message/memoryTraceFormat"
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
  MediaItem,
  Message,
  SessionMeta,
  SessionMode,
  Task,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import { FileMimeIcon } from "@/components/chat/message/FileCard"
import { FileDeltaCounter } from "@/components/chat/message/FileDeltaCounter"
import { FileContextMenu, FileActionsMoreButton } from "@/components/chat/files/FileActionMenu"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import type { PlanModeState } from "@/components/chat/plan-mode/usePlanMode"
import type { SessionFileEntry } from "./useSessionFileChanges"
import { sessionSourceKey, type SessionUrlSource } from "./useSessionUrlSources"
import type { SessionBrowserActivity } from "./useSessionBrowserActivity"
import { useWorkspaceArtifacts } from "./useWorkspaceArtifacts"
import { useWorkspaceEnvironment } from "./useWorkspaceEnvironment"
import { GitControlCard } from "./GitControlCard"
import { useSessionGitControl } from "./useSessionGitControl"
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
  type SavedWorkflowTemplate,
  type WorkflowEvent,
  type WorkflowGateIssue,
  type WorkflowPermissionPreview,
  type WorkflowOp,
  type WorkflowRun,
  type WorkflowRunSnapshot,
  type WorkflowRunState,
  type WorkflowRunsState,
  type WorkflowScriptPreview,
  type WorkflowWatchdogFinding,
} from "./useWorkflowRuns"
import {
  useGoal,
  type AutonomyActivity,
  type Goal,
  type GoalBudgetSnapshot,
  type GoalClosureDecision,
  type GoalCriterionAudit,
  type GoalCriterionItem,
  type GoalCriterionKind,
  type GoalCriterionStatus,
  type GoalEvidenceItem,
  type GoalSnapshot,
  type GoalState,
  type GoalStateSnapshot,
  type GoalTimelineItem,
  type GoalWatchdogFinding,
} from "./useGoal"
import {
  useLoopSchedules,
  type LoopExecutionStrategy,
  type LoopRun,
  type LoopSnapshot,
  type LoopSchedule,
  type LoopSchedulesState,
  type LoopWatchdogFinding,
  type LoopProgressState,
  type LoopRunState,
  type LoopState,
  type LoopTriggerKind,
} from "./useLoopSchedules"
import { parseGoalCriteriaDraft, type DraftGoalCriterionKind } from "./goalCriteriaDraft"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"
import {
  buildWorkspaceMemoryDiagnostics,
  formatWorkspaceMemoryDiagnosticsMarkdown,
  workspaceMemoryDiagnosticsCopyErrorToast,
  type WorkspaceMemoryLayerSummary,
} from "./workspaceMemoryDiagnostics"
import { workspaceSourceOpenErrorToast } from "./workspaceSourceFeedback"
import { PANEL_SCROLL_FADE } from "../right-panel/panelFade"
import { shouldConsumeWorkspaceFocus } from "./workspaceFocus"
import {
  resolveWorkspaceEnvironmentStatus,
  workingDirSourceLabelKey,
} from "./workspaceEnvironment"

export interface WorkspaceFocusRequest {
  sessionId: string
  section: "goal" | "workflow" | "loop" | "progress"
  itemId?: string
  nonce: number
}

interface WorkspacePanelProps {
  taskSnapshot: TaskProgressSnapshot | null
  taskExecutionState?: WorkspaceTaskExecutionState
  /** 会话消息 —— 当前轮 live tail 在面板内部聚合,与后端历史全量合并。 */
  messages: Message[]
  contextUsageOverride?: ContextUsageInfo | null
  /** 改写类文件「查看 diff」→ 右侧 diff 面板。 */
  onOpenDiff: (payload: FileChangeMetadata | FileChangesMetadata) => void
  /** 仓库真实 staged / unstaged diff → 右侧 diff 面板。 */
  onOpenGitDiff?: (
    snapshot: SessionGitDiffSnapshot,
    sessionId: string,
    reviewComments?: GitPullRequestReviewComment[],
  ) => void
  /** 将 GitHub 检查/评论修复要求填入当前会话输入框，不自动发送。 */
  onFillInput?: (value: string) => void
  /** 打开当前分支关联 PR 的独立右侧面板。 */
  onOpenPullRequest?: () => void
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
  /** 切到实时 BrowserPanel。工作台里的浏览器活动行用它查看当前画面。 */
  onOpenBrowserPanel?: () => void
  /** 打开子 agent 实时会话弹层，不切换当前主会话。 */
  onViewSubagentSession?: (sessionId: string) => void
  /** 从输入框或其它全局入口请求打开「持续推进」创建器。 */
  openLoopCreateRequest?: number
  /** Dashboard attention deep-link: scroll to the relevant control section
   * and, where supported, open the exact Workflow run / Loop schedule. */
  focusRequest?: WorkspaceFocusRequest | null
  /** Confirms that an external focus signal was consumed so it cannot replay
   * after this panel is unmounted and later opened for another session. */
  onFocusRequestHandled?: (nonce: number) => void
  /** 草稿态新对话里创建 workflow 前,由 ChatScreen 物化一个真实会话并切过去。 */
  onEnsureSession?: () => Promise<string | null>
  /** 无 session 草稿态工作流模式:不提前创建会话。 */
  draftWorkflowMode?: WorkflowAutonomyMode
  onDraftWorkflowModeChange?: (mode: WorkflowAutonomyMode) => void
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
  autoExpandWhen,
}: {
  title: string
  count?: number
  icon: LucideIcon
  children: ReactNode
  meta?: ReactNode
  defaultExpanded?: boolean
  expandSignal?: number
  autoExpandWhen?: boolean
}) {
  const [expanded, setExpanded] = useState(defaultExpanded || Boolean(autoExpandWhen))
  const lastExpandSignalRef = useRef(expandSignal ?? 0)
  const lastAutoExpandWhenRef = useRef(Boolean(autoExpandWhen))

  /* eslint-disable react-hooks/set-state-in-effect -- external expand signals intentionally synchronize local disclosure state */
  useEffect(() => {
    const nextSignal = expandSignal ?? 0
    if (nextSignal === lastExpandSignalRef.current) return
    lastExpandSignalRef.current = nextSignal
    setExpanded(true)
  }, [expandSignal])

  useEffect(() => {
    const nextAutoExpand = Boolean(autoExpandWhen)
    if (nextAutoExpand && !lastAutoExpandWhenRef.current) {
      setExpanded(true)
    }
    lastAutoExpandWhenRef.current = nextAutoExpand
  }, [autoExpandWhen])
  /* eslint-enable react-hooks/set-state-in-effect */

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
    () => ({
      kind: "sessionPath",
      sessionId,
      path: entry.path,
      name,
      language: entry.language ?? diff?.language ?? null,
    }),
    [diff?.language, entry.language, entry.path, name, sessionId],
  )
  const overrides = useMemo(() => ({ sessionId, onPreviewFile }), [sessionId, onPreviewFile])
  const { primary, run } = useFileResource(target, overrides)
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

function UrlSourceRow({ source }: { source: Extract<SessionUrlSource, { kind: "url" }> }) {
  const { t } = useTranslation()
  const faviconUrl = useSafeFavicon(source.url)
  const openSource = useCallback(() => {
    openExternalUrl(source.url, {
      onError: (e) => {
        logger.error("ui", "WorkspaceSource::open", "Open source failed", e)
        const failure = workspaceSourceOpenErrorToast(t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      },
    })
  }, [source.url, t])
  return (
    <IconTip label={source.url}>
      <button
        type="button"
        onClick={openSource}
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
        {source.origin === "user_url" && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            <MessageCircle className="h-2.5 w-2.5" />
            {t("workspace.sourceFromUser", "用户")}
          </span>
        )}
      </button>
    </IconTip>
  )
}

function sourceMediaItem(source: Extract<SessionUrlSource, { kind: "attachment" }>): MediaItem {
  return {
    url: source.url ?? "",
    ...(source.localPath ? { localPath: source.localPath } : {}),
    name: source.name,
    mimeType: source.mimeType,
    sizeBytes: source.sizeBytes,
    kind: source.attachmentKind === "image" ? "image" : "file",
  }
}

function attachmentSourceTarget(
  source: Extract<SessionUrlSource, { kind: "attachment" }>,
  sessionId?: string | null,
): PreviewTarget | null {
  if (source.attachmentKind === "quote") {
    return null
  }
  if (source.localPath) {
    return {
      kind: "sessionPath",
      sessionId,
      path: source.localPath,
      name: source.name,
      mime: source.mimeType,
    }
  }
  if (source.url) {
    return { kind: "media", item: sourceMediaItem(source) }
  }
  return null
}

function AttachmentSourceRow({
  source,
  sessionId,
  onPreviewFile,
}: {
  source: Extract<SessionUrlSource, { kind: "attachment" }>
  sessionId?: string | null
  onPreviewFile?: (target: PreviewTarget) => void
}) {
  const { t } = useTranslation()
  const target = useMemo(() => attachmentSourceTarget(source, sessionId), [source, sessionId])
  const overrides = useMemo(() => ({ sessionId, onPreviewFile }), [sessionId, onPreviewFile])
  const { primary, run } = useFileResource(target, overrides)
  const label = source.quoteLines ? `${source.name} L${source.quoteLines}` : source.name

  return (
    <FileContextMenu target={target} overrides={overrides}>
      <div className="flex w-full items-center gap-1 rounded-md px-2 py-1.5 transition-colors hover:bg-secondary/45">
        <IconTip label={label}>
          <button
            type="button"
            disabled={!target}
            onClick={() => run(primary)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left disabled:cursor-default"
          >
            <FileMimeIcon
              mime={source.mimeType}
              name={source.name}
              className="h-3.5 w-3.5 shrink-0"
            />
            <span className="min-w-0 flex-1 truncate text-xs text-foreground/90">{label}</span>
            <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              <Files className="h-2.5 w-2.5" />
              {t("workspace.sourceFromAttachment", "附件")}
            </span>
          </button>
        </IconTip>
        <FileActionsMoreButton target={target} overrides={overrides} className="shrink-0" />
      </div>
    </FileContextMenu>
  )
}

function SourceRow({
  source,
  sessionId,
  onPreviewFile,
}: {
  source: SessionUrlSource
  sessionId?: string | null
  onPreviewFile?: (target: PreviewTarget) => void
}) {
  if (source.kind === "attachment") {
    return (
      <AttachmentSourceRow source={source} sessionId={sessionId} onPreviewFile={onPreviewFile} />
    )
  }
  return <UrlSourceRow source={source} />
}

function browserActivityLabel(
  t: ReturnType<typeof useTranslation>["t"],
  activity: SessionBrowserActivity,
): string {
  const op = activity.op
  switch (activity.action) {
    case "navigate":
      return t("workspace.browserAction.navigate", "导航")
    case "tabs":
      if (op === "new") return t("workspace.browserAction.newTab", "新标签")
      if (op === "claim") return t("workspace.browserAction.claim", "接管")
      if (op === "select") return t("workspace.browserAction.select", "切换")
      if (op === "close") return t("workspace.browserAction.close", "关闭")
      return t("workspace.browserAction.tabs", "标签页")
    case "act":
      return op
        ? t("workspace.browserAction.actWithOp", "{{op}}", { op })
        : t("workspace.browserAction.act", "操作")
    case "snapshot":
      if (op === "screenshot" || op === "image")
        return t("workspace.browserAction.screenshot", "截图")
      if (op === "pdf") return t("workspace.browserAction.pdf", "PDF")
      return t("workspace.browserAction.snapshot", "快照")
    case "observe":
      return t("workspace.browserAction.observe", "观察")
    case "control":
      if (op === "scroll") return t("workspace.browserAction.scroll", "滚动")
      if (op === "evaluate") return t("workspace.browserAction.evaluate", "脚本")
      if (op === "wait_for") return t("workspace.browserAction.wait", "等待")
      return t("workspace.browserAction.control", "控制")
    case "profile":
      return t("workspace.browserAction.profile", "浏览器")
    case "status":
      return t("workspace.browserAction.status", "状态")
  }
}

function BrowserActivityRow({
  activity,
  onOpenBrowserPanel,
}: {
  activity: SessionBrowserActivity
  onOpenBrowserPanel?: () => void
}) {
  const { t } = useTranslation()
  const faviconUrl = useSafeFavicon(activity.url ?? "")
  const label = browserActivityLabel(t, activity)
  const title = activity.title || (activity.url ? domainOf(activity.url) : label)
  const subtitle = activity.url || activity.targetId || activity.backend || ""
  const time = activity.at ? new Date(activity.at).toLocaleTimeString() : ""

  return (
    <IconTip label={subtitle || title}>
      <div className="flex items-center gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-secondary/45">
        {faviconUrl ? (
          <img
            src={faviconUrl}
            alt=""
            className="h-3.5 w-3.5 shrink-0 rounded-[3px] bg-background/70 object-contain"
          />
        ) : (
          <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <button
          type="button"
          className="min-w-0 flex-1 text-left"
          onClick={onOpenBrowserPanel}
          disabled={!onOpenBrowserPanel}
        >
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="truncate text-xs font-medium text-foreground/90">{title}</span>
            <span className="inline-flex shrink-0 rounded-full bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {label}
            </span>
          </div>
          {subtitle ? (
            <div className="truncate pt-0.5 text-[10px] text-muted-foreground/70">{subtitle}</div>
          ) : null}
        </button>
        <div className="flex shrink-0 items-center gap-1">
          {activity.backend ? (
            <span className="rounded bg-secondary/70 px-1.5 py-0.5 text-[10px] uppercase text-muted-foreground">
              {activity.backend}
            </span>
          ) : null}
          {time ? <span className="text-[10px] text-muted-foreground/60">{time}</span> : null}
          {activity.url ? (
            <IconTip label={t("chat.browserPanel.openExternal")}>
              <button
                type="button"
                className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                onClick={() => {
                  if (activity.url) openExternalUrl(activity.url)
                }}
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          ) : null}
        </div>
      </div>
    </IconTip>
  )
}

function EmptyHint({ children }: { children: ReactNode }) {
  return <div className="px-2 py-3 text-center text-xs text-muted-foreground/70">{children}</div>
}

type StatusTone = "muted" | "good" | "warn" | "danger" | "info"

type DomainArtifactReviewTarget = {
  title?: string | null
  kind?: string | null
  path?: string | null
  domain?: string | null
  guardStatus?: string | null
}

type DomainQualityReviewEvidenceTarget = {
  label: string
  title?: string | null
  kind?: string | null
  path?: string | null
  id?: string | null
  evidenceScope?: Record<string, unknown> | null
}

type DomainArtifactExportReviewMarker = "exportReview" | "exportReady" | "redactionChecked"
type DomainConnectorActionConfirmationMarker = "explicitUserApproval" | "rollbackPlan"
type DomainConnectorE2ESampleMarker = "action_execution" | "post_action_verification"
type DomainConnectorE2ENextSampleStep = "approval" | "execution" | "verification" | "complete"

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

function E2EBadge() {
  return (
    <span className="shrink-0 rounded-full border border-border/60 bg-secondary/35 px-1.5 py-0.5 text-[9px] font-medium leading-none text-muted-foreground">
      E2E
    </span>
  )
}

function compactCountEntries(counts: Record<string, number>, max = 4): Array<[string, number]> {
  return Object.entries(counts)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, max)
}

function traceTone(status: string | undefined): "muted" | "good" | "warn" | "danger" | "info" {
  switch (status) {
    case "used":
      return "good"
    case "candidates":
      return "info"
    case "partial":
      return "warn"
    case "degraded":
      return "danger"
    case "disabled":
    case "no_context":
      return "muted"
    default:
      return "muted"
  }
}

function dominantLayerStatus(layer: WorkspaceMemoryLayerSummary): string {
  if (layer.skipped > 0) return "skipped"
  if (layer.disabled > 0) return "disabled"
  if (layer.used > 0) return "used"
  if (layer.candidate > 0) return "candidate"
  if (layer.empty > 0) return "empty"
  return "empty"
}

function MemoryMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border border-border/60 bg-secondary/25 px-2 py-1.5">
      <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">{label}</div>
      <div className="mt-0.5 text-sm font-semibold tabular-nums text-foreground">{value}</div>
    </div>
  )
}

function turnToneClass(status: string, degraded: boolean): string {
  if (degraded) return "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300"
  if (status === "used")
    return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
  if (status === "candidates")
    return "border-blue-500/35 bg-blue-500/10 text-blue-700 dark:text-blue-300"
  return "border-border bg-muted/50 text-muted-foreground"
}

function MemoryDiagnosticsSection({
  messages,
  incognito,
}: {
  messages: Message[]
  incognito: boolean
}) {
  const { t } = useTranslation()
  const diagnostics = useMemo(() => buildWorkspaceMemoryDiagnostics(messages), [messages])
  const latestStatus = diagnostics.latest?.status

  const handleCopy = useCallback(async () => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error("clipboard unavailable")
      await navigator.clipboard.writeText(formatWorkspaceMemoryDiagnosticsMarkdown(diagnostics))
      toast.success(t("workspace.memoryDiagnostics.copyDone", "记忆诊断已复制"))
    } catch (e) {
      logger.error("ui", "WorkspaceMemoryDiagnostics::copy", "Copy diagnostics failed", e)
      const failure = workspaceMemoryDiagnosticsCopyErrorToast(t, e)
      toast.error(
        failure.title,
        failure.description ? { description: failure.description } : undefined,
      )
    }
  }, [diagnostics, t])

  return (
    <WorkspaceSection
      title={t("workspace.sectionMemoryDiagnostics", "记忆诊断")}
      count={diagnostics.turns}
      icon={Brain}
      meta={
        diagnostics.turns > 0 && latestStatus ? (
          <StatusPill
            label={retrievalTraceStatusLabel(latestStatus, t)}
            tone={traceTone(latestStatus)}
          />
        ) : null
      }
    >
      {diagnostics.turns === 0 ? (
        <EmptyHint>
          {incognito
            ? t("workspace.memoryDiagnostics.emptyIncognito", "无痕会话不会读取长期记忆")
            : t("workspace.memoryDiagnostics.empty", "本会话还没有记忆诊断")}
        </EmptyHint>
      ) : (
        <div className="space-y-2">
          <div className="grid grid-cols-3 gap-1.5">
            <MemoryMetric
              label={t("workspace.memoryDiagnostics.turns", "轮次")}
              value={diagnostics.turns}
            />
            <MemoryMetric
              label={t("workspace.memoryDiagnostics.contextRefs", "入上下文")}
              value={diagnostics.contextRefCount}
            />
            <MemoryMetric
              label={t("workspace.memoryDiagnostics.candidates", "候选")}
              value={diagnostics.candidateRefCount}
            />
          </div>

          <div className="flex flex-wrap items-center gap-1.5">
            {compactCountEntries(diagnostics.kindCounts).map(([kind, count]) => (
              <span
                key={kind}
                className="inline-flex max-w-full items-center rounded-full border border-border/60 bg-background/70 px-2 py-0.5 text-[10px] text-muted-foreground"
              >
                <span className="truncate">{memoryKindLabel({ kind }, t)}</span>
                <span className="ml-1 tabular-nums">{count}</span>
              </span>
            ))}
            {diagnostics.droppedCount > 0 ? (
              <StatusPill
                label={`${t("workspace.memoryDiagnostics.dropped", "裁剪")} ${diagnostics.droppedCount}`}
                tone="warn"
              />
            ) : null}
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="ml-auto h-7 px-2 text-[11px]"
              onClick={handleCopy}
            >
              <Copy className="mr-1 h-3 w-3" />
              {t("workspace.memoryDiagnostics.copy", "复制诊断")}
            </Button>
          </div>

          {Object.keys(diagnostics.originCounts).length > 0 ? (
            <div className="space-y-1 rounded-md border border-border/50 bg-secondary/20 px-2 py-1.5">
              <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">
                {t("workspace.memoryDiagnostics.origins", "来源对比")}
              </div>
              <div className="flex flex-wrap gap-1">
                {compactCountEntries(diagnostics.originCounts, 5).map(([origin, count]) => (
                  <span
                    key={origin}
                    className="inline-flex max-w-full items-center rounded-full border border-border/60 bg-background/70 px-2 py-0.5 text-[10px] text-muted-foreground"
                  >
                    <span className="truncate">{memoryOriginLabel(origin, t)}</span>
                    <span className="ml-1 tabular-nums">{count}</span>
                  </span>
                ))}
              </div>
            </div>
          ) : null}

          {diagnostics.recentTurns.length > 0 ? (
            <div className="space-y-1 rounded-md border border-border/50 bg-secondary/20 px-2 py-1.5">
              <div className="text-[10px] uppercase tracking-wide text-muted-foreground/70">
                {t("workspace.memoryDiagnostics.recentTurns", "最近轮次")}
              </div>
              <div className="flex flex-wrap gap-1">
                {diagnostics.recentTurns.map((turn) => (
                  <span
                    key={turn.index}
                    className={cn(
                      "inline-flex items-center rounded border px-1.5 py-0.5 text-[10px] tabular-nums",
                      turnToneClass(turn.status, turn.degraded),
                    )}
                    data-ha-title-tip={`${retrievalTraceStatusLabel(turn.status, t)} · ${turn.contextRefCount}/${turn.candidateRefCount}`}
                  >
                    #{turn.index + 1}
                    <span className="ml-1 text-muted-foreground">
                      {turn.contextRefCount}/{turn.candidateRefCount}
                    </span>
                  </span>
                ))}
              </div>
            </div>
          ) : null}

          <div className="space-y-1">
            {diagnostics.layers.slice(0, 5).map((layer) => {
              const status = dominantLayerStatus(layer)
              return (
                <div
                  key={layer.layer}
                  className="flex min-w-0 items-center gap-2 rounded-md border border-border/50 bg-secondary/25 px-2 py-1.5 text-xs"
                >
                  <Layers className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">
                    {retrievalLayerLabel(layer.layer, t)}
                  </span>
                  <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                    {layer.refCount} refs
                  </span>
                  <StatusPill
                    label={retrievalLayerStatusLabel(status, t)}
                    tone={
                      status === "used"
                        ? "good"
                        : status === "candidate"
                          ? "info"
                          : status === "skipped"
                            ? "warn"
                            : "muted"
                    }
                  />
                </div>
              )
            })}
          </div>

          {diagnostics.degradedLayers.length > 0 ? (
            <div className="space-y-1 rounded-md border border-amber-500/25 bg-amber-500/5 px-2 py-1.5">
              {diagnostics.degradedLayers.map((layer) => (
                <div
                  key={`${layer.layer}:${layer.status}:${layer.reason ?? ""}`}
                  className="flex min-w-0 items-center gap-2 text-[11px]"
                >
                  <CircleAlert className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                  <span className="min-w-0 flex-1 truncate text-foreground/80">
                    {retrievalLayerLabel(layer.layer, t)}
                    {layer.reason ? ` · ${retrievalLayerReasonLabel(layer.reason, t)}` : ""}
                  </span>
                  <span className="shrink-0 tabular-nums text-muted-foreground">
                    x{layer.count}
                  </span>
                </div>
              ))}
            </div>
          ) : (
            <div className="rounded-md border border-emerald-500/20 bg-emerald-500/5 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
              {t("workspace.memoryDiagnostics.noDegraded", "本会话未记录记忆层降级")}
            </div>
          )}
        </div>
      )}
    </WorkspaceSection>
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
  onOpenGitDiff = () => {},
  onFillInput,
  onOpenPullRequest,
}: {
  sessionId?: string | null
  sessionMeta?: SessionMeta | null
  project?: ProjectMeta | null
  effectiveWorkingDir?: string | null
  workingDirSource?: "session" | "project"
  permissionMode?: SessionMode
  planState?: PlanModeState
  turnActive?: boolean
  onOpenGitDiff: (
    snapshot: SessionGitDiffSnapshot,
    sessionId: string,
    reviewComments?: GitPullRequestReviewComment[],
  ) => void
  onFillInput?: (value: string) => void
  onOpenPullRequest?: () => void
}) {
  const { t } = useTranslation()
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
  const transport = useTransport()
  const isLocalRuntime = transport.fileRuntime().canReveal
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
  const managedWorktreesState = useManagedWorktrees(sessionId, {
    incognito: sessionMeta?.incognito,
    turnActive,
  })
  const managedWorktrees = managedWorktreesState.worktrees
  const gitControl = useSessionGitControl(sessionId, turnActive)
  const activeManagedWorktree =
    managedWorktrees.find((wt) => wt.state !== "archived" && wt.path === workingDir) ?? null
  const [worktreeActionKey, setWorktreeActionKey] = useState<string | null>(null)
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
    async (worktree: ManagedWorktree, action: "archive" | "restore") => {
      if (worktreeActionKey) return
      const command = action === "archive" ? "archive_managed_worktree" : "restore_managed_worktree"
      setWorktreeActionKey(`${action}:${worktree.id}`)
      try {
        await getTransport().call<ManagedWorktree>(command, { worktreeId: worktree.id })
        managedWorktreesState.refresh()
        toast.success(
          action === "archive"
            ? t("workspace.worktree.archived", "已归档工作树")
            : t("workspace.worktree.restored", "已恢复工作树"),
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
      {sessionId && git ? (
        <GitControlCard
          sessionId={sessionId}
          state={gitControl}
          managedWorktrees={managedWorktrees}
          onOpenGitDiff={onOpenGitDiff}
          onFillInput={onFillInput}
          onOpenPullRequest={onOpenPullRequest}
          managedWorktreeControls={
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
          }
        />
      ) : null}

      <details className="mt-2 rounded-lg border border-border/45 bg-secondary/10">
        <summary className="cursor-pointer select-none px-2.5 py-2 text-xs font-medium text-muted-foreground hover:text-foreground">
          {t("workspace.environment.details", "详细信息")}
        </summary>
        <div className="space-y-0.5 border-t border-border/45 p-1.5">
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
              tone={
                planState === "executing" ? "info" : planState === "completed" ? "good" : "muted"
              }
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

        {!git && env.snapshot && workingDir ? (
          <EnvRow
            icon={GitBranch}
            label={t("workspace.environment.git", "Git")}
            value={t("workspace.environment.nonGit", "非 Git 工作目录")}
          />
        ) : null}
        </div>
      </details>
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
  onAction: (worktree: ManagedWorktree, action: "archive" | "restore") => void
}) {
  const { t } = useTranslation()
  const createBusy = actionKey === "create"
  return (
    <div className="rounded-md border border-border/55 bg-secondary/15">
      <div className="flex min-w-0 items-center gap-2 px-2 py-1.5">
        <FolderGit2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/85">
          {t("workspace.worktree.managed", "托管工作树")}
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
      ) : worktrees.length === 0 ? (
        <div className="border-t border-border/60 px-2 py-1.5 text-[10px] text-muted-foreground">
          {t("workspace.worktree.empty", "暂无托管工作树")}
        </div>
      ) : (
        <div className="max-h-48 space-y-1 overflow-y-auto border-t border-border/60 p-1.5">
          {worktrees.map((worktree) => {
            const isActive = activeWorktree?.id === worktree.id
            const busyPrefix = actionKey?.endsWith(`:${worktree.id}`)
              ? actionKey.split(":")[0]
              : null
            return (
              <div
                key={worktree.id}
                className={cn(
                  "flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-[10px]",
                  isActive ? "bg-secondary/70" : "bg-background/35",
                )}
                data-ha-title-tip={worktree.path}
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
                    <IconTip label={t("workspace.worktree.archive", "归档")}>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                        disabled={Boolean(actionKey) || isActive}
                        onClick={() => onAction(worktree, "archive")}
                      >
                        {busyPrefix === "archive" ? (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        ) : (
                          <X className="h-3 w-3" />
                        )}
                      </Button>
                    </IconTip>
                  )}
                </div>
              </div>
            )
          })}
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
    case "bootstrap_failed":
      return t("workspace.worktree.stateBootstrapFailed", "Bootstrap failed")
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
  const { attachments, activity, loadErrorDetail } = useSessionKnowledge(sessionId, projectId, {
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
      {loadErrorDetail && (
        <KnowledgeLoadWarning
          title={t("workspace.kbAttachmentsLoadFailed", "无法读取已挂载知识空间")}
          detail={t("workspace.kbLoadDetail", "详细信息：{{error}}", {
            error: loadErrorDetail,
          })}
        />
      )}
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
      ) : loadErrorDetail ? null : (
        <EmptyHint>{t("workspace.emptyKnowledge", "未挂载知识空间")}</EmptyHint>
      )}
    </WorkspaceSection>
  )
}

const contextKindIcons: Record<ContextCandidateKind, LucideIcon> = {
  file: FileText,
  symbol: Hash,
  diagnostic: CircleAlert,
  review_finding: GitPullRequest,
  verification_step: CheckCircle2,
  goal_evidence: Brain,
  task: Check,
  workflow_op: Layers,
  ide_context: Monitor,
  url_source: Globe,
  document: FileText,
  email_thread: MessageCircle,
  calendar_event: CalendarClock,
  sheet_range: Database,
  knowledge_note: BookText,
  web_source: Globe,
  decision: Brain,
  artifact: Files,
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

function diagnosticStatusLabel(t: ReturnType<typeof useTranslation>["t"], status: string): string {
  const normalized = status.toLowerCase()
  switch (normalized) {
    case "open":
      return t("workspace.diagnosticStatus.open", "待处理")
    case "closed":
    case "resolved":
      return t("workspace.diagnosticStatus.resolved", "已处理")
    case "completed":
    case "passed":
    case "success":
    case "succeeded":
      return t("workspace.diagnosticStatus.completed", "已完成")
    case "failed":
    case "error":
      return t("workspace.diagnosticStatus.failed", "失败")
    case "blocked":
      return t("workspace.diagnosticStatus.blocked", "阻塞")
    case "pending":
    case "queued":
      return t("workspace.diagnosticStatus.pending", "等待中")
    case "running":
    case "in_progress":
      return t("workspace.diagnosticStatus.running", "运行中")
    case "skipped":
      return t("workspace.diagnosticStatus.skipped", "已跳过")
    case "warning":
    case "warn":
      return t("workspace.diagnosticStatus.warning", "警告")
    case "timed_out":
    case "timeout":
      return t("workspace.diagnosticStatus.timedOut", "超时")
    case "awaiting":
    case "awaiting_approval":
    case "awaiting_user":
      return t("workspace.diagnosticStatus.awaiting", "等待确认")
    case "cancelled":
    case "canceled":
      return t("workspace.diagnosticStatus.cancelled", "已取消")
    case "critical":
      return t("workspace.diagnosticStatus.critical", "严重")
    case "information":
    case "info":
      return t("workspace.diagnosticStatus.info", "信息")
    case "hint":
      return t("workspace.diagnosticStatus.hint", "提示")
    case "unknown":
      return t("workspace.diagnosticStatus.unknown", "未知")
    default:
      return status
  }
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
          className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border/50 bg-background/65 text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:opacity-45"
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
          className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border/50 bg-background/65 text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:opacity-45"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-amber-500/10 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
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
          className="inline-flex h-5 items-center gap-1 rounded border border-border/50 bg-background/55 px-1.5 text-[10px] text-muted-foreground transition-colors hover:bg-secondary/40 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-45"
        >
          {taskBusy ? <Loader2 className="h-3 w-3 animate-spin" /> : <Plus className="h-3 w-3" />}
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
  const Icon = contextKindIcons[candidate.kind]
  const path = candidate.path ?? ""
  const target = useMemo<PreviewTarget>(
    () => ({ kind: "sessionPath", sessionId, path, name: basename(path) || candidate.title }),
    [candidate.title, path, sessionId],
  )
  const overrides = useMemo(() => ({ sessionId, onPreviewFile }), [sessionId, onPreviewFile])
  const { primary, run } = useFileResource(target, overrides)
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
                <StatusPill
                  label={diagnosticStatusLabel(t, candidate.status)}
                  tone={contextCandidateTone(candidate)}
                />
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

function KnowledgeLoadWarning({ title, detail }: { title: string; detail?: string | null }) {
  return (
    <div className="mb-2 flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-2.5 py-2 text-[11px] leading-relaxed text-amber-800 dark:text-amber-200">
      <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <div className="min-w-0">
        <div className="font-medium">{title}</div>
        {detail && <div className="mt-0.5 break-words opacity-85">{detail}</div>}
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
  const Icon = contextKindIcons[candidate.kind]
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
            <StatusPill
              label={diagnosticStatusLabel(t, candidate.status)}
              tone={contextCandidateTone(candidate)}
            />
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
        logger.error("ui", "ContextRetrievalSection", "Record context candidate evidence failed", e)
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
        logger.error("ui", "ContextRetrievalSection", "Mark context candidate conflict failed", e)
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
        logger.error("ui", "ContextRetrievalSection", "Create context candidate task failed", e)
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
            <SearchInput
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
const DOMAIN_DRAFT_EVIDENCE_TYPES = new Set(["artifact_created", "connector_draft_created"])
const DOMAIN_REVIEW_EVIDENCE_TYPES = new Set(["artifact_reviewed", "connector_action_verified"])
const DOMAIN_DECISION_EVIDENCE_TYPES = new Set(["user_decision", "message_draft_approved"])

function domainEvidenceTypeLabel(t: ReturnType<typeof useTranslation>["t"], type: string): string {
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

function domainGuardCheckNameLabel(
  t: ReturnType<typeof useTranslation>["t"],
  name: string,
): string {
  switch (name) {
    case "evidence_scope":
      return t("workspace.domainGuardCheck.evidenceScope", "证据范围")
    case "artifact_created":
      return t("workspace.domainGuardCheck.artifactCreated", "产物已创建")
    case "artifact_reviewed":
      return t("workspace.domainGuardCheck.artifactReviewed", "产物已复核")
    case "redaction_status":
      return t("workspace.domainGuardCheck.redactionStatus", "脱敏状态")
    case "sensitive_evidence":
      return t("workspace.domainGuardCheck.sensitiveEvidence", "敏感证据")
    case "action_scope":
      return t("workspace.domainGuardCheck.actionScope", "动作范围")
    case "explicit_user_approval":
      return t("workspace.domainGuardCheck.explicitUserApproval", "用户明确批准")
    case "rollback_plan":
      return t("workspace.domainGuardCheck.rollbackPlan", "回滚方案")
    case "artifact_export_guard":
      return t("workspace.domainGuardCheck.artifactExportGuard", "交付守门")
    case "connector_input":
      return t("workspace.domainGuardCheck.connectorInput", "连接器输入")
    case "draft_or_preview":
      return t("workspace.domainGuardCheck.draftOrPreview", "草稿或预览")
    case "action_execution":
      return t("workspace.domainGuardCheck.actionExecution", "动作执行")
    case "execution_result":
      return t("workspace.domainGuardCheck.executionResult", "执行结果")
    case "post_action_verification":
      return t("workspace.domainGuardCheck.postActionVerification", "执行后复核")
    case "connector_action_guard":
      return t("workspace.domainGuardCheck.connectorActionGuard", "外部动作守门")
    case "sensitive_unreviewed":
      return t("workspace.domainGuardCheck.sensitiveUnreviewed", "敏感证据未复核")
    default:
      return name.replace(/_/g, " ")
  }
}

function domainAccessScopeLabel(
  t: ReturnType<typeof useTranslation>["t"],
  scope?: string | null,
): string {
  switch ((scope ?? "").toLowerCase()) {
    case "public":
      return t("workspace.domainAccessScope.public", "公开")
    case "session":
      return t("workspace.domainAccessScope.session", "当前会话")
    case "private":
      return t("workspace.domainAccessScope.private", "私有")
    case "connector":
      return t("workspace.domainAccessScope.connector", "连接器")
    case "restricted":
      return t("workspace.domainAccessScope.restricted", "受限")
    default:
      return scope || "-"
  }
}

function domainRedactionStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
): string {
  switch ((status ?? "").toLowerCase()) {
    case "none":
      return t("workspace.domainRedaction.none", "无需脱敏")
    case "clean":
      return t("workspace.domainRedaction.clean", "已清理")
    case "pending":
      return t("workspace.domainRedaction.pending", "待脱敏")
    case "sensitive":
      return t("workspace.domainRedaction.sensitive", "敏感")
    case "redacted":
      return t("workspace.domainRedaction.redacted", "已脱敏")
    case "not_required":
      return t("workspace.domainRedaction.notRequired", "不需要")
    default:
      return status || "-"
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
  controlRecords?: number
  blockingReviewFindings: number
  failedVerification: number
  domainFailed: number
  domainNeedsUser: number
  exportStatus?: string | null
  connectorStatus?: string | null
  connectorE2EStatus?: string | null
  operationalStatus?: string | null
  soakStatus?: string | null
}): StatusTone {
  if (args.incognito) return "muted"
  const hasObservedWork =
    args.evidenceCount > 0 ||
    (args.controlRecords ?? 0) > 0 ||
    args.blockingReviewFindings > 0 ||
    args.failedVerification > 0 ||
    args.domainFailed > 0 ||
    args.domainNeedsUser > 0
  const hasHardFailedGate =
    args.exportStatus === "failed" ||
    args.connectorStatus === "failed" ||
    args.connectorE2EStatus === "failed"
  const hasRuntimeFailedGate = args.operationalStatus === "failed" || args.soakStatus === "failed"
  const hasObservedGate =
    Boolean(args.exportStatus) ||
    Boolean(args.connectorStatus) ||
    Boolean(args.connectorE2EStatus) ||
    Boolean(args.operationalStatus) ||
    Boolean(args.soakStatus)
  if (
    args.blockingReviewFindings > 0 ||
    args.failedVerification > 0 ||
    args.domainFailed > 0 ||
    hasHardFailedGate ||
    (hasObservedWork && hasRuntimeFailedGate)
  ) {
    return "danger"
  }
  if (!hasObservedWork) return hasObservedGate ? "info" : "muted"
  if (
    args.domainNeedsUser > 0 ||
    args.evidenceCount === 0 ||
    args.sourceCount === 0 ||
    args.draftCount === 0 ||
    args.exportStatus === "insufficient_data" ||
    args.connectorStatus === "insufficient_data" ||
    args.connectorE2EStatus === "insufficient_data" ||
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
  if (tone === "info") return t("workspace.domainWorkbench.observing", "观察中")
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
    connectorE2eGate: DomainConnectorE2EGateReport | null
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
    steps.push(
      t("workspace.domainWorkbench.nextSources", "补齐来源或连接器上下文，避免无依据产物。"),
    )
  }
  if (args.draftCount === 0) {
    steps.push(t("workspace.domainWorkbench.nextDraft", "生成可审查的草稿、文件或外部动作草案。"))
  }
  if (args.reviewCount === 0 && args.draftCount > 0) {
    steps.push(t("workspace.domainWorkbench.nextReview", "对草稿做复核，再进入交付或外部执行。"))
  }
  if (args.domainFailed > 0 || args.domainNeedsUser > 0) {
    steps.push(
      t("workspace.domainWorkbench.nextDomainQuality", "处理领域复核里的阻塞项或用户确认项。"),
    )
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
  if (args.connectorE2eGate?.status && args.connectorE2eGate.status !== "passed") {
    steps.push(
      args.connectorE2eGate.recommendedNextSteps[0] ??
        t(
          "workspace.domainWorkbench.nextConnectorE2E",
          "补齐连接器输入、草稿、批准、执行、复核和回滚证据。",
        ),
    )
  }
  if (args.operationalGate?.status && args.operationalGate.status !== "passed") {
    steps.push(
      args.operationalGate.recommendedNextSteps[0] ??
        t(
          "workspace.domainWorkbench.nextOperationalGate",
          "等待工作流排空，或处理失败/阻塞的运行。",
        ),
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
    : [
        t(
          "workspace.domainWorkbench.nextReady",
          "证据链健康；交付或外部动作前仍会要求用户最终确认。",
        ),
      ]
}

type DomainAcceptanceCoverageSummary = {
  domains: string[]
  controlRecords: number
  drainedRuns: number
  connectorE2eEvidence: number
  connectorExecutionEvidence: number
  connectorVerificationEvidence: number
  criticalIncidents: number
  warningIncidents: number
  latestActivityAgeSecs?: number | null
  freshnessMaxAgeSecs: number
  sampleDays: number
  requiredSampleDays: number
  budgetExhaustedEvents: number
  outputTokenBudgetLabel: string | null
  readinessPercent: number
  requiredPassed: number
  requiredTotal: number
  requirements: DomainAcceptanceRequirement[]
  sampleLanes: DomainAcceptanceSampleLane[]
  controlMix: DomainAcceptanceControlMix
  provenance: DomainAcceptanceProvenanceSummary
  tone: StatusTone
  gaps: DomainAcceptanceGap[]
}

type DomainAcceptanceControlMix = {
  workflowRuns: number
  completedWorkflowRuns: number
  loopRuns: number
  succeededLoopRuns: number
  campaignItems: number
  passedCampaignItems: number
  connectorE2eEvidence: number
}

type DomainAcceptanceProvenanceSummary = {
  total: number
  workflow: number
  connector: number
  fixtureOrMock: number
  manual: number
  publicSource: number
  restrictedAccess: number
}

type DomainAcceptanceRequirement = {
  key: string
  label: string
  detail: string
  passed: boolean
  tone: StatusTone
}

type DomainAcceptanceSampleLane = {
  key: string
  label: string
  detail: string
  passed: boolean
  tone: StatusTone
  action: string
  evidence: string[]
  refreshTargets: string[]
}

type DomainAcceptanceGapSeverity = "danger" | "warn" | "info"

type DomainAcceptanceGap = {
  key: string
  message: string
  severity: DomainAcceptanceGapSeverity
}

type DomainAcceptanceVerdict = {
  label: string
  detail: string
  tone: StatusTone
}

type DomainAcceptanceEvidenceLevel = {
  label: string
  detail: string
  tone: StatusTone
}

type DomainAcceptanceReviewContext = {
  evidence: DomainEvidenceItem[]
  exportGuard: DomainArtifactExportGuardReport | null
  connectorGuard: DomainConnectorActionGuardReport | null
  connectorE2eGate: DomainConnectorE2EGateReport | null
  operationalGate: DomainOperationalGateReport | null
  soakReport: DomainSoakReport | null
}

type DomainAcceptanceReviewGate = {
  status?: string | null
  blockers?: string[]
  checks?: Array<{
    name: string
    status: string
    actual?: string
    detail?: string
  }>
  recommendedNextSteps?: string[]
}

const DOMAIN_ACCEPTANCE_FRESH_SAMPLE_MAX_AGE_SECS = 24 * 60 * 60

function domainAcceptanceProvenanceSummary(
  evidence: DomainEvidenceItem[],
): DomainAcceptanceProvenanceSummary {
  const summary: DomainAcceptanceProvenanceSummary = {
    total: evidence.length,
    workflow: 0,
    connector: 0,
    fixtureOrMock: 0,
    manual: 0,
    publicSource: 0,
    restrictedAccess: 0,
  }
  for (const item of evidence) {
    const metadata = asRecord(item.sourceMetadata) ?? {}
    const metadataText = JSON.stringify(metadata).toLowerCase()
    const evidenceType = item.evidenceType.toLowerCase()
    const accessScope = item.accessScope.toLowerCase()
    if (metadataText.includes("workflow") || metadataText.includes("runid")) {
      summary.workflow += 1
    }
    if (
      evidenceType.startsWith("connector") ||
      accessScope === "connector" ||
      metadataText.includes("connector") ||
      metadataText.includes("gmail") ||
      metadataText.includes("calendar") ||
      metadataText.includes("drive")
    ) {
      summary.connector += 1
    }
    if (
      metadataText.includes("fixture") ||
      metadataText.includes("mock") ||
      metadataText.includes("deterministic")
    ) {
      summary.fixtureOrMock += 1
    }
    if (
      evidenceType === "user_decision" ||
      evidenceType === "artifact_reviewed" ||
      evidenceType === "message_draft_approved" ||
      metadataText.includes("confirmation") ||
      metadataText.includes("explicituserapproval")
    ) {
      summary.manual += 1
    }
    if (evidenceType === "source_cited" && accessScope === "public") {
      summary.publicSource += 1
    }
    if (
      accessScope !== "public" ||
      (item.redactionStatus && item.redactionStatus !== "none" && item.redactionStatus !== "clean")
    ) {
      summary.restrictedAccess += 1
    }
  }
  return summary
}

function domainAcceptanceCoverageSummary(
  t: ReturnType<typeof useTranslation>["t"],
  args: {
    evidence: DomainEvidenceItem[]
    exportGuard: DomainArtifactExportGuardReport | null
    connectorGuard: DomainConnectorActionGuardReport | null
    connectorE2eGate: DomainConnectorE2EGateReport | null
    operationalGate: DomainOperationalGateReport | null
    soakReport: DomainSoakReport | null
  },
): DomainAcceptanceCoverageSummary {
  const domains = new Set<string>()
  for (const item of args.evidence) {
    if (item.domain) domains.add(item.domain)
  }
  const provenance = domainAcceptanceProvenanceSummary(args.evidence)
  if (args.exportGuard?.scope?.domain) domains.add(args.exportGuard.scope.domain)
  if (args.connectorGuard?.scope?.domain) domains.add(args.connectorGuard.scope.domain)
  if (args.connectorE2eGate?.scope?.domain) domains.add(args.connectorE2eGate.scope.domain)
  if (args.operationalGate?.domain) domains.add(args.operationalGate.domain)
  if (args.soakReport?.domain) domains.add(args.soakReport.domain)

  const operationalSummary = args.operationalGate?.summary
  const soakSummary = args.soakReport?.summary
  const connectorE2eSummary = args.connectorE2eGate?.summary
  const workflowRuns = soakSummary?.workflowRuns ?? operationalSummary?.workflowRuns ?? 0
  const completedWorkflowRuns =
    soakSummary?.completedWorkflowRuns ?? operationalSummary?.completedWorkflowRuns ?? 0
  const loopRuns = soakSummary?.loopRuns ?? operationalSummary?.loopRuns ?? 0
  const succeededLoopRuns =
    soakSummary?.succeededLoopRuns ?? operationalSummary?.succeededLoopRuns ?? 0
  const campaignItems = soakSummary?.campaignItems ?? operationalSummary?.campaignItems ?? 0
  const passedCampaignItems =
    soakSummary?.passedCampaignItems ?? operationalSummary?.passedCampaignItems ?? 0
  const controlRecords =
    soakSummary?.totalRecords ?? workflowRuns + loopRuns + campaignItems + args.evidence.length
  const drainedRuns = completedWorkflowRuns + succeededLoopRuns + passedCampaignItems
  const connectorE2eEvidence =
    connectorE2eSummary?.evidenceItems ??
    soakSummary?.connectorE2eEvidence ??
    (soakSummary?.connectorExecutionEvidence ?? 0) +
      (soakSummary?.connectorVerificationEvidence ?? 0)
  const connectorExecutionEvidence =
    connectorE2eSummary?.executionEvidence ?? soakSummary?.connectorExecutionEvidence ?? 0
  const connectorVerificationEvidence =
    connectorE2eSummary?.verificationEvidence ?? soakSummary?.connectorVerificationEvidence ?? 0
  const controlMix: DomainAcceptanceControlMix = {
    workflowRuns,
    completedWorkflowRuns,
    loopRuns,
    succeededLoopRuns,
    campaignItems,
    passedCampaignItems,
    connectorE2eEvidence,
  }
  const criticalIncidents = soakSummary?.criticalIncidents ?? 0
  const warningIncidents = soakSummary?.warningIncidents ?? 0
  const rawLatestActivityAgeSecs = soakSummary?.latestActivityAgeSecs
  const latestActivityAgeSecs =
    typeof rawLatestActivityAgeSecs === "number" && Number.isFinite(rawLatestActivityAgeSecs)
      ? Math.max(0, rawLatestActivityAgeSecs)
      : null
  const hasFreshSample =
    latestActivityAgeSecs != null &&
    latestActivityAgeSecs <= DOMAIN_ACCEPTANCE_FRESH_SAMPLE_MAX_AGE_SECS
  const sampleDays = Math.max(0, soakSummary?.sampleDays ?? (controlRecords > 0 ? 1 : 0))
  const requiredSampleDays = Math.max(1, soakSummary?.requiredSampleDays ?? 1)
  const hasRequiredSampleDays = sampleDays >= requiredSampleDays
  const budgetExhaustedEvents = Math.max(0, soakSummary?.workflowBudgetExhaustedEvents ?? 0)
  const hasAcceptanceSample =
    controlRecords > 0 ||
    args.evidence.length > 0 ||
    drainedRuns > 0 ||
    connectorE2eEvidence > 0 ||
    latestActivityAgeSecs != null
  const outputTokenBudgetLabel =
    soakSummary?.maxWorkflowOutputTokensSpent != null
      ? soakSummary.maxWorkflowOutputTokenBudget != null &&
        soakSummary.maxWorkflowOutputTokenBudget > 0
        ? `${compactCount(soakSummary.maxWorkflowOutputTokensSpent)}/${compactCount(
            soakSummary.maxWorkflowOutputTokenBudget,
          )}`
        : compactCount(soakSummary.maxWorkflowOutputTokensSpent)
      : null
  const budgetHealthy = controlRecords > 0 && budgetExhaustedEvents === 0
  const connectorE2eHasScope = Boolean(
    args.connectorE2eGate?.connector ||
    args.connectorE2eGate?.action ||
    args.connectorE2eGate?.toolName ||
    connectorE2eSummary?.evidenceItems ||
    args.connectorGuard?.summary?.actionEvidence ||
    connectorExecutionEvidence > 0 ||
    connectorVerificationEvidence > 0,
  )
  const hasFailedGate =
    hasAcceptanceSample &&
    (args.operationalGate?.status === "failed" ||
      args.soakReport?.status === "failed" ||
      args.exportGuard?.status === "failed" ||
      args.connectorGuard?.status === "failed" ||
      args.connectorE2eGate?.status === "failed")
  const observedGateStatuses = [
    args.exportGuard?.status,
    args.connectorGuard?.status,
    args.connectorE2eGate?.status,
    args.operationalGate?.status,
    args.soakReport?.status,
  ].filter(Boolean)
  const allObservedGatesPassed =
    observedGateStatuses.length > 0 && observedGateStatuses.every((status) => status === "passed")
  const failedGateLabels = [
    hasAcceptanceSample && args.exportGuard?.status === "failed"
      ? t("workspace.domainWorkbench.acceptanceGateExport", "交付守门")
      : null,
    hasAcceptanceSample && args.connectorGuard?.status === "failed"
      ? t("workspace.domainWorkbench.acceptanceGateConnector", "外部动作守门")
      : null,
    hasAcceptanceSample && args.connectorE2eGate?.status === "failed"
      ? t("workspace.domainWorkbench.acceptanceGateConnectorE2E", "连接器端到端（E2E）")
      : null,
    hasAcceptanceSample && args.operationalGate?.status === "failed"
      ? t("workspace.domainWorkbench.acceptanceGateOperational", "运行稳定性")
      : null,
    hasAcceptanceSample && args.soakReport?.status === "failed"
      ? t("workspace.domainWorkbench.acceptanceGateSoak", "长跑审计")
      : null,
  ].filter(Boolean)
  const gaps: DomainAcceptanceGap[] = []
  const pushGap = (key: string, message: string, severity: DomainAcceptanceGapSeverity) => {
    gaps.push({ key, message, severity })
  }

  if (hasAcceptanceSample && domains.size === 0) {
    pushGap(
      "domain",
      t("workspace.domainWorkbench.acceptanceGapDomain", "还没有真实领域样本。"),
      "warn",
    )
  }
  if (hasAcceptanceSample && args.evidence.length === 0) {
    pushGap(
      "evidence",
      t("workspace.domainWorkbench.acceptanceGapEvidence", "缺少来源、草稿、复核或用户决策证据。"),
      "warn",
    )
  }
  if (hasAcceptanceSample && drainedRuns === 0) {
    pushGap(
      "drain",
      t(
        "workspace.domainWorkbench.acceptanceGapDrain",
        "还没有已排空的 Workflow / Loop / Campaign 样本。",
      ),
      "warn",
    )
  }
  if (
    connectorE2eHasScope &&
    (connectorE2eEvidence === 0 || args.connectorE2eGate?.status !== "passed")
  ) {
    pushGap(
      "connector-e2e",
      t("workspace.domainWorkbench.acceptanceGapConnector", "外部动作还缺端到端执行与复核样本。"),
      args.connectorE2eGate?.status === "failed" ? "danger" : "warn",
    )
  }
  if (hasAcceptanceSample && (criticalIncidents > 0 || warningIncidents > 0)) {
    pushGap(
      "incidents",
      t("workspace.domainWorkbench.acceptanceGapIncidents", "长跑审计仍有事故需要收口。"),
      criticalIncidents > 0 ? "danger" : "warn",
    )
  }
  if (controlRecords > 0 && !hasFreshSample) {
    pushGap(
      "freshness",
      t("workspace.domainWorkbench.acceptanceGapFreshness", "最近长任务样本过旧或缺少新鲜度信号。"),
      "warn",
    )
  }
  if (controlRecords > 0 && !hasRequiredSampleDays) {
    pushGap(
      "sample-days",
      t("workspace.domainWorkbench.acceptanceGapSampleDays", "多天长跑窗口缺少跨天样本覆盖。"),
      "warn",
    )
  }
  if (connectorExecutionEvidence > 0 && connectorVerificationEvidence === 0) {
    pushGap(
      "connector-verification",
      t(
        "workspace.domainWorkbench.acceptanceGapConnectorVerification",
        "连接器动作已执行，但缺少执行后读回复核 evidence。",
      ),
      "warn",
    )
  }
  if (hasAcceptanceSample && budgetExhaustedEvents > 0) {
    pushGap(
      "budget",
      t(
        "workspace.domainWorkbench.acceptanceGapBudget",
        "工作流输出预算已耗尽，需收口性能或上下文策略。",
      ),
      "danger",
    )
  }
  if (failedGateLabels.length > 0 && criticalIncidents === 0) {
    pushGap(
      "failed-gates",
      t("workspace.domainWorkbench.acceptanceGapFailedGates", "仍有未通过守门：{{gates}}。", {
        gates: failedGateLabels.join("、"),
      }),
      "danger",
    )
  }
  if (domains.size === 1 && controlRecords > 0) {
    pushGap(
      "more-domains",
      t(
        "workspace.domainWorkbench.acceptanceGapMoreDomains",
        "继续补其它通用领域样本，避免只证明单一场景。",
      ),
      "info",
    )
  }
  const sortedGaps = gaps.sort(
    (a, b) => domainAcceptanceGapRank(a.severity) - domainAcceptanceGapRank(b.severity),
  )
  const requirements: DomainAcceptanceRequirement[] = [
    {
      key: "domain",
      label: t("workspace.domainWorkbench.acceptanceReqDomain", "领域样本"),
      detail:
        domains.size > 0
          ? t("workspace.domainWorkbench.acceptanceReqDomainOk", "{{count}} 个领域", {
              count: domains.size,
            })
          : t("workspace.domainWorkbench.acceptanceReqDomainMissing", "还没有真实领域"),
      passed: domains.size > 0,
      tone: domains.size > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
    },
    {
      key: "evidence",
      label: t("workspace.domainWorkbench.acceptanceReqEvidence", "证据链"),
      detail:
        args.evidence.length > 0
          ? t("workspace.domainWorkbench.acceptanceReqEvidenceOk", "{{count}} 条 evidence", {
              count: args.evidence.length,
            })
          : t("workspace.domainWorkbench.acceptanceReqEvidenceMissing", "缺来源/草稿/决策证据"),
      passed: args.evidence.length > 0,
      tone: args.evidence.length > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
    },
    {
      key: "drain",
      label: t("workspace.domainWorkbench.acceptanceReqDrain", "排空样本"),
      detail:
        drainedRuns > 0
          ? t("workspace.domainWorkbench.acceptanceReqDrainOk", "{{count}} 个已排空", {
              count: drainedRuns,
            })
          : t(
              "workspace.domainWorkbench.acceptanceReqDrainMissing",
              "缺 Workflow / Loop / Campaign",
            ),
      passed: drainedRuns > 0,
      tone: drainedRuns > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
    },
    {
      key: "freshness",
      label: t("workspace.domainWorkbench.acceptanceReqFreshness", "样本新鲜"),
      detail:
        latestActivityAgeSecs == null
          ? t("workspace.domainWorkbench.acceptanceReqFreshnessMissing", "缺最近活动时间")
          : hasFreshSample
            ? t("workspace.domainWorkbench.acceptanceReqFreshnessOk", "{{age}} 前", {
                age: formatDurationCompact(latestActivityAgeSecs),
              })
            : t(
                "workspace.domainWorkbench.acceptanceReqFreshnessStale",
                "{{age}} 前，超过 {{max}}",
                {
                  age: formatDurationCompact(latestActivityAgeSecs),
                  max: formatDurationCompact(DOMAIN_ACCEPTANCE_FRESH_SAMPLE_MAX_AGE_SECS),
                },
              ),
      passed: hasFreshSample,
      tone: hasFreshSample ? "good" : controlRecords > 0 ? "warn" : "muted",
    },
    {
      key: "sample-days",
      label: t("workspace.domainWorkbench.acceptanceReqSampleDays", "跨天覆盖"),
      detail:
        controlRecords === 0
          ? t("workspace.domainWorkbench.acceptanceReqSampleDaysNoSample", "先补控制面记录")
          : hasRequiredSampleDays
            ? t("workspace.domainWorkbench.acceptanceReqSampleDaysOk", "{{days}}/{{required}} 天", {
                days: sampleDays,
                required: requiredSampleDays,
              })
            : t(
                "workspace.domainWorkbench.acceptanceReqSampleDaysMissing",
                "{{days}}/{{required}} 天，缺跨天样本",
                { days: sampleDays, required: requiredSampleDays },
              ),
      passed: controlRecords > 0 && hasRequiredSampleDays,
      tone: controlRecords === 0 ? "muted" : hasRequiredSampleDays ? "good" : "warn",
    },
    {
      key: "budget",
      label: t("workspace.domainWorkbench.acceptanceReqBudget", "预算健康"),
      detail:
        budgetExhaustedEvents > 0
          ? t(
              "workspace.domainWorkbench.acceptanceReqBudgetExhausted",
              "耗尽 {{count}} 次{{budget}}",
              {
                count: budgetExhaustedEvents,
                budget: outputTokenBudgetLabel ? ` · ${outputTokenBudgetLabel}` : "",
              },
            )
          : controlRecords > 0
            ? outputTokenBudgetLabel
              ? t(
                  "workspace.domainWorkbench.acceptanceReqBudgetOkWithUsage",
                  "未耗尽 · {{budget}}",
                  {
                    budget: outputTokenBudgetLabel,
                  },
                )
              : t("workspace.domainWorkbench.acceptanceReqBudgetOk", "未观察到预算耗尽")
            : t("workspace.domainWorkbench.acceptanceReqBudgetNoSample", "先补控制面记录"),
      passed: budgetHealthy,
      tone: budgetExhaustedEvents > 0 ? "danger" : controlRecords > 0 ? "good" : "muted",
    },
    {
      key: "incidents",
      label: t("workspace.domainWorkbench.acceptanceReqIncidents", "事故清零"),
      detail:
        criticalIncidents > 0 || warningIncidents > 0
          ? t(
              "workspace.domainWorkbench.acceptanceReqIncidentsOpen",
              "critical {{critical}} / warning {{warning}}",
              { critical: criticalIncidents, warning: warningIncidents },
            )
          : controlRecords > 0
            ? t("workspace.domainWorkbench.acceptanceReqIncidentsOk", "无未收口事故")
            : t("workspace.domainWorkbench.acceptanceReqIncidentsNoSample", "先补控制面记录"),
      passed: controlRecords > 0 && criticalIncidents === 0 && warningIncidents === 0,
      tone:
        criticalIncidents > 0
          ? "danger"
          : warningIncidents > 0
            ? "warn"
            : controlRecords > 0
              ? "good"
              : "muted",
    },
    {
      key: "gates",
      label: t("workspace.domainWorkbench.acceptanceReqGates", "守门通过"),
      detail: hasFailedGate
        ? t("workspace.domainWorkbench.acceptanceReqGatesFailed", "{{gates}} 未通过", {
            gates: failedGateLabels.join("、"),
          })
        : allObservedGatesPassed
          ? t("workspace.domainWorkbench.acceptanceReqGatesOk", "已观察守门均通过")
          : t("workspace.domainWorkbench.acceptanceReqGatesPending", "缺少守门通过样本"),
      passed: allObservedGatesPassed,
      tone: hasFailedGate
        ? "danger"
        : allObservedGatesPassed
          ? "good"
          : observedGateStatuses.length > 0
            ? "info"
            : "muted",
    },
  ]
  if (connectorE2eHasScope) {
    const connectorPassed = connectorE2eEvidence > 0 && args.connectorE2eGate?.status === "passed"
    requirements.push({
      key: "connector-e2e",
      label: t("workspace.domainWorkbench.acceptanceReqConnectorE2E", "连接器端到端（E2E）"),
      detail: connectorPassed
        ? t("workspace.domainWorkbench.acceptanceReqConnectorE2EOk", "{{count}} 条 evidence", {
            count: connectorE2eEvidence,
          })
        : t("workspace.domainWorkbench.acceptanceReqConnectorE2EMissing", "缺执行/复核闭环"),
      passed: connectorPassed,
      tone:
        args.connectorE2eGate?.status === "failed" ? "danger" : connectorPassed ? "good" : "warn",
    })
  }
  const sampleLanes: DomainAcceptanceSampleLane[] = [
    {
      key: "workflow",
      label: t("workspace.domainWorkbench.acceptanceLaneWorkflow", "Workflow 样本"),
      detail:
        completedWorkflowRuns > 0
          ? t(
              "workspace.domainWorkbench.acceptanceLaneWorkflowOk",
              "{{completed}}/{{total}} 已完成",
              { completed: completedWorkflowRuns, total: workflowRuns },
            )
          : t("workspace.domainWorkbench.acceptanceLaneWorkflowMissing", "缺已完成 Workflow"),
      passed: completedWorkflowRuns > 0,
      tone: completedWorkflowRuns > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
      action: t(
        "workspace.domainWorkbench.acceptanceLaneWorkflowAction",
        "跑一个领域 Workflow 并等待排空，再刷新运行稳定性和长跑审计。",
      ),
      evidence: [
        t(
          "workspace.domainWorkbench.acceptanceLaneWorkflowEvidenceTerminal",
          "WorkflowRun 已进入 completed / failed / blocked / cancelled 终态，不能只看创建成功。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneWorkflowEvidenceTrace",
          "Workflow trace、validation/review 结果或 domain evidence 能从工作台复核。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneWorkflowEvidenceRecovery",
          "若失败，保留失败分类、恢复动作和下一轮 rerun 结果。",
        ),
      ],
      refreshTargets: [
        t("workspace.domainWorkbench.acceptanceLaneRefreshOperational", "刷新运行稳定性 Gate。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshSoak", "刷新长跑审计 Soak Report。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshReport", "重新复制真实样本验收报告。"),
      ],
    },
    {
      key: "loop",
      label: t("workspace.domainWorkbench.acceptanceLaneLoop", "Loop 样本"),
      detail:
        succeededLoopRuns > 0
          ? t("workspace.domainWorkbench.acceptanceLaneLoopOk", "{{succeeded}}/{{total}} 成功", {
              succeeded: succeededLoopRuns,
              total: loopRuns,
            })
          : t("workspace.domainWorkbench.acceptanceLaneLoopMissing", "缺成功 Loop tick"),
      passed: succeededLoopRuns > 0,
      tone: succeededLoopRuns > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
      action: t(
        "workspace.domainWorkbench.acceptanceLaneLoopAction",
        "创建一个短间隔领域 Loop，确认至少一次 tick 成功并关联 Workflow run。",
      ),
      evidence: [
        t(
          "workspace.domainWorkbench.acceptanceLaneLoopEvidenceGoal",
          "Loop 绑定 active Goal 或 domain template，tick 不是孤立定时器。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneLoopEvidenceTick",
          "至少一个 tick succeeded，并关联 workflowRunId 或明确说明无需 Workflow。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneLoopEvidenceTrace",
          "Loop trace 中能看到策略、退出条件和下一步决策。",
        ),
      ],
      refreshTargets: [
        t("workspace.domainWorkbench.acceptanceLaneRefreshOperational", "刷新运行稳定性 Gate。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshSoak", "刷新长跑审计 Soak Report。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshReport", "重新复制真实样本验收报告。"),
      ],
    },
    {
      key: "campaign",
      label: t("workspace.domainWorkbench.acceptanceLaneCampaign", "Campaign 样本"),
      detail:
        passedCampaignItems > 0
          ? t("workspace.domainWorkbench.acceptanceLaneCampaignOk", "{{passed}}/{{total}} 通过", {
              passed: passedCampaignItems,
              total: campaignItems,
            })
          : t("workspace.domainWorkbench.acceptanceLaneCampaignMissing", "缺通过的 Campaign item"),
      passed: passedCampaignItems > 0,
      tone: passedCampaignItems > 0 ? "good" : hasAcceptanceSample ? "warn" : "muted",
      action: t(
        "workspace.domainWorkbench.acceptanceLaneCampaignAction",
        "跑一个 deterministic 或真实 agent campaign item，确认可取消、可 retry、可复核。",
      ),
      evidence: [
        t(
          "workspace.domainWorkbench.acceptanceLaneCampaignEvidenceSample",
          "Campaign item 使用 deterministic trace pack 或真实 agent 样本。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneCampaignEvidenceResult",
          "至少一个 item passed，失败 item 有分类和 retry / cancel 证据。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneCampaignEvidenceTrace",
          "保留 campaign summary，能追溯输入、判断标准和输出。",
        ),
      ],
      refreshTargets: [
        t("workspace.domainWorkbench.acceptanceLaneRefreshOperational", "刷新运行稳定性 Gate。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshSoak", "刷新长跑审计 Soak Report。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshReport", "重新复制真实样本验收报告。"),
      ],
    },
    {
      key: "connector-e2e",
      label: t("workspace.domainWorkbench.acceptanceLaneConnector", "连接器端到端（E2E）"),
      detail: connectorE2eHasScope
        ? connectorE2eEvidence > 0 && args.connectorE2eGate?.status === "passed"
          ? t(
              "workspace.domainWorkbench.acceptanceLaneConnectorOk",
              "{{count}} 条执行/复核 evidence",
              { count: connectorE2eEvidence },
            )
          : t("workspace.domainWorkbench.acceptanceLaneConnectorMissing", "缺执行/复核闭环")
        : t("workspace.domainWorkbench.acceptanceLaneConnectorNoScope", "当前会话未观察外部动作"),
      passed: connectorE2eHasScope
        ? connectorE2eEvidence > 0 && args.connectorE2eGate?.status === "passed"
        : false,
      tone: connectorE2eHasScope
        ? connectorE2eEvidence > 0 && args.connectorE2eGate?.status === "passed"
          ? "good"
          : args.connectorE2eGate?.status === "failed"
            ? "danger"
            : "warn"
        : "info",
      action: t(
        "workspace.domainWorkbench.acceptanceLaneConnectorAction",
        "用测试账号完成读取 -> 草稿 -> 批准 -> 执行 -> 复核 -> 回滚说明，并记录端到端 evidence。",
      ),
      evidence: [
        t(
          "workspace.domainWorkbench.acceptanceLaneConnectorEvidenceAccount",
          "使用测试账号或沙箱数据，避免真实用户生产账号。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneConnectorEvidenceFlow",
          "记录读取上下文、草稿、用户批准、执行、执行后读回/复核。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneConnectorEvidenceAudit",
          "记录 connector_action_executed 与 connector_action_verified evidence；失败时保留回滚说明。",
        ),
      ],
      refreshTargets: [
        t("workspace.domainWorkbench.acceptanceLaneRefreshConnector", "刷新连接器端到端 Gate。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshOperational", "刷新运行稳定性 Gate。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshReport", "重新复制真实样本验收报告。"),
      ],
    },
    {
      key: "cross-domain",
      label: t("workspace.domainWorkbench.acceptanceLaneCrossDomain", "跨领域覆盖"),
      detail:
        domains.size >= 2
          ? t("workspace.domainWorkbench.acceptanceLaneCrossDomainOk", "{{count}} 个领域", {
              count: domains.size,
            })
          : t("workspace.domainWorkbench.acceptanceLaneCrossDomainMissing", "{{count}} 个领域", {
              count: domains.size,
            }),
      passed: domains.size >= 2,
      tone: domains.size >= 2 ? "good" : "info",
      action: t(
        "workspace.domainWorkbench.acceptanceLaneCrossDomainAction",
        "补一个不同领域样本，例如写作、数据分析、会议准备、知识整理、Inbox 或项目运营。",
      ),
      evidence: [
        t(
          "workspace.domainWorkbench.acceptanceLaneCrossDomainEvidenceDomain",
          "选择当前会话之外的另一个通用领域，例如写作、数据分析、会议准备、知识整理、Inbox 或项目运营。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneCrossDomainEvidenceTemplates",
          "确认 Goal、Workflow、Context Retrieval、Domain Quality 均读取到对应领域模板。",
        ),
        t(
          "workspace.domainWorkbench.acceptanceLaneCrossDomainEvidenceGeneral",
          "保留一条完整 evidence 链，证明不是只靠 coding 场景通过。",
        ),
      ],
      refreshTargets: [
        t("workspace.domainWorkbench.acceptanceLaneRefreshCoverage", "刷新真实样本验收卡片。"),
        t("workspace.domainWorkbench.acceptanceLaneRefreshReport", "重新复制真实样本验收报告。"),
        t(
          "workspace.domainWorkbench.acceptanceLaneRefreshReview",
          "更新给人工 / Claude Code / PR review 的验收包。",
        ),
      ],
    },
  ]
  const requiredChecks = requirements.map((requirement) => requirement.passed)
  const requiredPassed = requiredChecks.filter(Boolean).length
  const requiredTotal = requiredChecks.length
  const requiredPercent = requiredTotal > 0 ? (requiredPassed / requiredTotal) * 90 : 0
  const diversityBonus = domains.size >= 2 ? 10 : 0
  const readinessPercent = Math.max(0, Math.min(100, Math.round(requiredPercent + diversityBonus)))

  const tone: StatusTone =
    hasFailedGate || criticalIncidents > 0
      ? "danger"
      : !hasAcceptanceSample
        ? "muted"
        : gaps.length > 0
          ? "warn"
          : controlRecords > 0
            ? "good"
            : "muted"

  return {
    domains: [...domains].sort(),
    controlRecords,
    drainedRuns,
    connectorE2eEvidence,
    connectorExecutionEvidence,
    connectorVerificationEvidence,
    criticalIncidents,
    warningIncidents,
    latestActivityAgeSecs,
    freshnessMaxAgeSecs: DOMAIN_ACCEPTANCE_FRESH_SAMPLE_MAX_AGE_SECS,
    sampleDays,
    requiredSampleDays,
    budgetExhaustedEvents,
    outputTokenBudgetLabel,
    readinessPercent,
    requiredPassed,
    requiredTotal,
    requirements,
    sampleLanes,
    controlMix,
    provenance,
    tone,
    gaps: sortedGaps.slice(0, 3),
  }
}

function domainAcceptanceProgressBarClass(tone: StatusTone): string {
  if (tone === "danger") return "bg-destructive"
  if (tone === "warn") return "bg-amber-500"
  if (tone === "good") return "bg-emerald-500"
  if (tone === "info") return "bg-blue-500"
  return "bg-muted-foreground/50"
}

function domainAcceptanceRequirementStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  requirement: DomainAcceptanceRequirement,
): string {
  if (requirement.passed) return t("workspace.domainWorkbench.acceptanceReqPassed", "通过")
  if (requirement.tone === "danger")
    return t("workspace.domainWorkbench.acceptanceReqBlocked", "阻塞")
  if (requirement.tone === "muted")
    return t("workspace.domainWorkbench.acceptanceReqSampling", "待采样")
  if (requirement.tone === "info")
    return t("workspace.domainWorkbench.acceptanceLaneOptional", "待扩展")
  return t("workspace.domainWorkbench.acceptanceReqMissing", "待补")
}

function domainAcceptanceSampleLaneStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  lane: DomainAcceptanceSampleLane,
): string {
  if (lane.passed) return t("workspace.domainWorkbench.acceptanceReqPassed", "通过")
  if (lane.tone === "danger") return t("workspace.domainWorkbench.acceptanceReqBlocked", "阻塞")
  if (lane.tone === "muted") return t("workspace.domainWorkbench.acceptanceReqSampling", "待采样")
  if (lane.tone === "info") return t("workspace.domainWorkbench.acceptanceLaneOptional", "待扩展")
  return t("workspace.domainWorkbench.acceptanceReqMissing", "待补")
}

function domainAcceptanceSampleLaneTaskContent(
  t: ReturnType<typeof useTranslation>["t"],
  lane: DomainAcceptanceSampleLane,
): string {
  return [
    t("workspace.domainWorkbench.acceptanceLaneTaskTitle", "补齐真实样本验收跑道：{{lane}}", {
      lane: lane.label,
    }),
    "",
    t("workspace.domainWorkbench.acceptanceLaneTaskStatusHeading", "当前状态："),
    `- ${lane.detail}`,
    "",
    t("workspace.domainWorkbench.acceptanceLaneTaskActionHeading", "采样动作："),
    `- ${lane.action}`,
    "",
    t("workspace.domainWorkbench.acceptanceLaneTaskEvidenceHeading", "需要记录的证据："),
    ...lane.evidence.map((item) => `- ${item}`),
    "",
    t("workspace.domainWorkbench.acceptanceLaneTaskRefreshHeading", "完成后刷新："),
    ...lane.refreshTargets.map((item) => `- ${item}`),
  ].join("\n")
}

function domainAcceptanceSampleLaneChecklistLines(
  t: ReturnType<typeof useTranslation>["t"],
  lanes: DomainAcceptanceSampleLane[],
  options: { includeRefreshTargets?: boolean } = {},
): string[] {
  return lanes.flatMap((lane) => {
    const status = domainAcceptanceSampleLaneStatusLabel(t, lane)
    const evidenceLines = lane.evidence.map(
      (item) =>
        `  - ${t("workspace.domainWorkbench.acceptanceLaneChecklistEvidence", "证据：{{item}}", {
          item,
        })}`,
    )
    const refreshLines = options.includeRefreshTargets
      ? lane.refreshTargets.map(
          (item) =>
            `  - ${t("workspace.domainWorkbench.acceptanceLaneChecklistRefresh", "刷新：{{item}}", {
              item,
            })}`,
        )
      : []
    return [
      `- [${status}] ${lane.label}：${lane.detail}；${lane.action}`,
      ...evidenceLines,
      ...refreshLines,
    ]
  })
}

function domainAcceptanceGapRank(severity: DomainAcceptanceGapSeverity): number {
  if (severity === "danger") return 0
  if (severity === "warn") return 1
  return 2
}

function domainAcceptanceGapTone(severity: DomainAcceptanceGapSeverity): StatusTone {
  if (severity === "danger") return "danger"
  if (severity === "warn") return "warn"
  return "info"
}

function domainAcceptanceGapLabel(
  t: ReturnType<typeof useTranslation>["t"],
  severity: DomainAcceptanceGapSeverity,
): string {
  if (severity === "danger") return t("workspace.domainWorkbench.acceptanceGapDanger", "阻塞")
  if (severity === "warn") return t("workspace.domainWorkbench.acceptanceGapWarn", "待补")
  return t("workspace.domainWorkbench.acceptanceGapInfo", "扩展")
}

function domainAcceptanceStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
): string {
  if (summary.tone === "danger")
    return t("workspace.domainWorkbench.acceptanceFailed", "样本有事故")
  if (summary.tone === "warn")
    return t("workspace.domainWorkbench.acceptanceNeedsSamples", "待补样本")
  if (summary.tone === "good") return t("workspace.domainWorkbench.acceptanceReady", "样本可审")
  return t("workspace.domainWorkbench.acceptanceIdle", "待采样")
}

function domainAcceptanceHasActionableSamplingWork(
  summary: DomainAcceptanceCoverageSummary,
): boolean {
  return (
    summary.gaps.length > 0 ||
    summary.requirements.some((requirement) => !requirement.passed) ||
    summary.sampleLanes.some((lane) => !lane.passed && lane.tone !== "info")
  )
}

function domainAcceptanceVerdict(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
): DomainAcceptanceVerdict {
  const dangerGap = summary.gaps.find((gap) => gap.severity === "danger")
  const warningGap = summary.gaps.find((gap) => gap.severity === "warn")
  const infoGap = summary.gaps.find((gap) => gap.severity === "info")
  const missingRequirement = summary.requirements.find((requirement) => !requirement.passed)
  const dangerRequirement = summary.requirements.find(
    (requirement) => !requirement.passed && requirement.tone === "danger",
  )

  if (summary.controlRecords === 0) {
    return {
      label: t("workspace.domainWorkbench.acceptanceVerdictIdle", "待采样"),
      detail: t(
        "workspace.domainWorkbench.acceptanceVerdictIdleDetail",
        "还没有真实控制面记录，不能作为最终验收证据。",
      ),
      tone: "muted",
    }
  }
  if (dangerGap || dangerRequirement) {
    return {
      label: t("workspace.domainWorkbench.acceptanceVerdictBlocked", "不可验收"),
      detail:
        dangerGap?.message ??
        t(
          "workspace.domainWorkbench.acceptanceVerdictBlockedRequirement",
          "{{label}} 未通过：{{detail}}",
          {
            label: dangerRequirement?.label ?? "",
            detail: dangerRequirement?.detail ?? "",
          },
        ),
      tone: "danger",
    }
  }
  if (missingRequirement || warningGap) {
    return {
      label: t("workspace.domainWorkbench.acceptanceVerdictNeedsSamples", "待补样本"),
      detail: missingRequirement
        ? t(
            "workspace.domainWorkbench.acceptanceVerdictNeedsRequirement",
            "{{label}} 仍缺证据：{{detail}}",
            { label: missingRequirement.label, detail: missingRequirement.detail },
          )
        : (warningGap?.message ??
          t(
            "workspace.domainWorkbench.acceptanceVerdictNeedsSamplesDetail",
            "仍有样本缺口需要补齐。",
          )),
      tone: "warn",
    }
  }
  if (infoGap) {
    return {
      label: t("workspace.domainWorkbench.acceptanceVerdictNeedsExpansion", "可局部复核"),
      detail: infoGap.message,
      tone: "info",
    }
  }
  return {
    label: t("workspace.domainWorkbench.acceptanceVerdictReady", "可验收"),
    detail: t(
      "workspace.domainWorkbench.acceptanceVerdictReadyDetail",
      "必需项、守门、长跑健康与真实样本证据均已通过。",
    ),
    tone: "good",
  }
}

function domainAcceptanceEvidenceLevel(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
): DomainAcceptanceEvidenceLevel {
  const hasDangerGap = summary.gaps.some((gap) => gap.severity === "danger")
  const hasWarnGap = summary.gaps.some((gap) => gap.severity === "warn")
  const hasInfoGap = summary.gaps.some((gap) => gap.severity === "info")
  const hasDangerRequirement = summary.requirements.some(
    (requirement) => !requirement.passed && requirement.tone === "danger",
  )
  const requiredComplete = summary.requiredPassed === summary.requiredTotal

  if (summary.controlRecords === 0) {
    return {
      label: t("workspace.domainWorkbench.acceptanceEvidenceLevelNone", "未采样"),
      detail: t(
        "workspace.domainWorkbench.acceptanceEvidenceLevelNoneDetail",
        "没有控制面记录，仅能作为采样待办。",
      ),
      tone: "muted",
    }
  }
  if (hasDangerGap || hasDangerRequirement) {
    return {
      label: t("workspace.domainWorkbench.acceptanceEvidenceLevelBlocked", "阻塞样本"),
      detail: t(
        "workspace.domainWorkbench.acceptanceEvidenceLevelBlockedDetail",
        "仍有阻塞缺口或失败守门，不能作为验收证据。",
      ),
      tone: "danger",
    }
  }
  if (!requiredComplete || hasWarnGap) {
    return {
      label: t("workspace.domainWorkbench.acceptanceEvidenceLevelPartial", "局部样本"),
      detail: t(
        "workspace.domainWorkbench.acceptanceEvidenceLevelPartialDetail",
        "可用于定位问题或回归验证，但缺必需项或 warning 缺口。",
      ),
      tone: "warn",
    }
  }
  if (hasInfoGap) {
    return {
      label: t("workspace.domainWorkbench.acceptanceEvidenceLevelLocal", "局部验收"),
      detail: t(
        "workspace.domainWorkbench.acceptanceEvidenceLevelLocalDetail",
        "必需项已通过，但覆盖面仍窄；不能代表全量通用能力。",
      ),
      tone: "info",
    }
  }
  if (summary.connectorVerificationEvidence > 0) {
    return {
      label: t("workspace.domainWorkbench.acceptanceEvidenceLevelConnector", "真实 E2E 候选"),
      detail: t(
        "workspace.domainWorkbench.acceptanceEvidenceLevelConnectorDetail",
        "包含连接器执行和复核 evidence，可进入人工或 Claude Code 最终复核。",
      ),
      tone: "good",
    }
  }
  return {
    label: t("workspace.domainWorkbench.acceptanceEvidenceLevelNonExternal", "非外部动作候选"),
    detail: t(
      "workspace.domainWorkbench.acceptanceEvidenceLevelNonExternalDetail",
      "可支撑当前非外部动作场景；涉及连接器仍需沙箱端到端样本。",
    ),
    tone: "good",
  }
}

function domainAcceptanceReviewProtocolLines(t: ReturnType<typeof useTranslation>["t"]): string[] {
  return [
    t(
      "workspace.domainWorkbench.acceptanceReviewProtocolVerdict",
      "只有验收结论为“可验收”时，当前样本才可作为最终验收证据；“可局部复核”只能证明局部场景。",
    ),
    t(
      "workspace.domainWorkbench.acceptanceReviewProtocolEvidence",
      "逐项核对验收必需项和验收矩阵；转任务、按钮点击或人工声明不能替代真实 evidence。",
    ),
    t(
      "workspace.domainWorkbench.acceptanceReviewProtocolSoak",
      "长任务必须复核 Operational Gate 与 Soak Report：无 critical 事故、无预算耗尽、样本新鲜、跨天覆盖且守门通过。",
    ),
    t(
      "workspace.domainWorkbench.acceptanceReviewProtocolConnector",
      "连接器端到端（E2E）必须来自测试账号或沙箱数据，并包含执行结果、执行后读回复核和回滚说明。",
    ),
  ].map((line) => `- ${line}`)
}

function domainAcceptanceProvenanceText(
  t: ReturnType<typeof useTranslation>["t"],
  provenance: DomainAcceptanceProvenanceSummary,
): string {
  return t(
    "workspace.domainWorkbench.acceptanceProvenanceText",
    "evidence {{total}} · workflow {{workflow}} · connector {{connector}} · fixture/mock {{fixture}} · manual {{manual}} · public {{public}} · restricted {{restricted}}",
    {
      total: provenance.total,
      workflow: provenance.workflow,
      connector: provenance.connector,
      fixture: provenance.fixtureOrMock,
      manual: provenance.manual,
      public: provenance.publicSource,
      restricted: provenance.restrictedAccess,
    },
  )
}

function domainAcceptanceControlMixText(
  t: ReturnType<typeof useTranslation>["t"],
  mix: DomainAcceptanceControlMix,
): string {
  return t(
    "workspace.domainWorkbench.acceptanceControlMixText",
    "workflow {{completedWorkflow}}/{{workflow}} · loop {{succeededLoop}}/{{loop}} · campaign {{passedCampaign}}/{{campaign}} · connector {{connector}}",
    {
      completedWorkflow: mix.completedWorkflowRuns,
      workflow: mix.workflowRuns,
      succeededLoop: mix.succeededLoopRuns,
      loop: mix.loopRuns,
      passedCampaign: mix.passedCampaignItems,
      campaign: mix.campaignItems,
      connector: mix.connectorE2eEvidence,
    },
  )
}

function domainAcceptanceSampleDaysText(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
): string {
  return t("workspace.domainWorkbench.acceptanceSampleDaysText", "{{days}}/{{required}} 天", {
    days: summary.sampleDays,
    required: summary.requiredSampleDays,
  })
}

function domainAcceptanceSnapshotId(
  summary: DomainAcceptanceCoverageSummary,
  context?: DomainAcceptanceReviewContext,
): string {
  const gateStatuses = context
    ? [
        context.exportGuard?.status ?? "missing",
        context.connectorGuard?.status ?? "missing",
        context.connectorE2eGate?.status ?? "missing",
        context.operationalGate?.status ?? "missing",
        context.soakReport?.status ?? "missing",
      ].join("|")
    : "summary"
  const evidenceIds = context
    ? context.evidence
        .slice(0, 8)
        .map((item) => item.id)
        .filter(Boolean)
        .join("|") || "none"
    : "summary"
  const soakSnapshot = context?.soakReport
    ? [
        context.soakReport.status,
        context.soakReport.windowDays,
        context.soakReport.incidents.length,
        context.soakReport.timeline.length,
        context.soakReport.summary.totalRecords,
        context.soakReport.summary.connectorE2eEvidence,
        context.soakReport.summary.connectorExecutionEvidence,
        context.soakReport.summary.connectorVerificationEvidence,
        context.soakReport.summary.sampleDays,
        context.soakReport.summary.requiredSampleDays,
      ].join("|")
    : "missing"
  const parts = [
    `domains=${summary.domains.join(",")}`,
    `records=${summary.controlRecords}`,
    `drained=${summary.drainedRuns}`,
    `connector=${summary.connectorE2eEvidence}`,
    `connectorVerify=${summary.connectorExecutionEvidence}/${summary.connectorVerificationEvidence}`,
    `critical=${summary.criticalIncidents}`,
    `warning=${summary.warningIncidents}`,
    `freshness=${summary.latestActivityAgeSecs ?? "missing"}`,
    `freshnessMax=${summary.freshnessMaxAgeSecs}`,
    `sampleDays=${summary.sampleDays}/${summary.requiredSampleDays}`,
    `budget=${summary.budgetExhaustedEvents}:${summary.outputTokenBudgetLabel ?? "-"}`,
    `progress=${summary.requiredPassed}/${summary.requiredTotal}:${summary.readinessPercent}`,
    `requirements=${summary.requirements
      .map((requirement) => `${requirement.key}:${requirement.passed ? 1 : 0}:${requirement.tone}`)
      .join("|")}`,
    `control=${summary.controlMix.completedWorkflowRuns}/${summary.controlMix.workflowRuns}:${summary.controlMix.succeededLoopRuns}/${summary.controlMix.loopRuns}:${summary.controlMix.passedCampaignItems}/${summary.controlMix.campaignItems}:${summary.controlMix.connectorE2eEvidence}`,
    `provenance=${summary.provenance.total}:${summary.provenance.workflow}:${summary.provenance.connector}:${summary.provenance.fixtureOrMock}:${summary.provenance.manual}:${summary.provenance.publicSource}:${summary.provenance.restrictedAccess}`,
    `lanes=${summary.sampleLanes
      .map((lane) => `${lane.key}:${lane.passed ? 1 : 0}:${lane.tone}`)
      .join("|")}`,
    `gaps=${summary.gaps.map((gap) => `${gap.key}:${gap.severity}`).join("|")}`,
    `gates=${gateStatuses}`,
    `evidence=${evidenceIds}`,
    `soak=${soakSnapshot}`,
  ]
  let hash = 0x811c9dc5
  for (const char of parts.join("\n")) {
    hash ^= char.charCodeAt(0)
    hash = Math.imul(hash, 0x01000193)
  }
  return `acc-${(hash >>> 0).toString(16).padStart(8, "0")}`
}

function domainAcceptanceAuditIndexLines(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
  context?: DomainAcceptanceReviewContext,
): string[] {
  const domains = summary.domains.length > 0 ? summary.domains.join(", ") : "0"
  const sampleFreshness =
    summary.latestActivityAgeSecs != null
      ? t("workspace.domainWorkbench.acceptancePlanFreshnessAge", "{{age}} 前", {
          age: formatDurationCompact(summary.latestActivityAgeSecs),
        })
      : t("workspace.domainWorkbench.acceptancePlanFreshnessMissing", "缺最近活动时间")
  const budget =
    summary.outputTokenBudgetLabel != null
      ? `${summary.budgetExhaustedEvents} exhausted · ${summary.outputTokenBudgetLabel}`
      : `${summary.budgetExhaustedEvents} exhausted`
  const lines = [
    t("workspace.domainWorkbench.acceptanceAuditSnapshot", "快照 ID：{{id}}", {
      id: domainAcceptanceSnapshotId(summary, context),
    }),
    t("workspace.domainWorkbench.acceptanceAuditDomains", "领域：{{domains}}", { domains }),
    t(
      "workspace.domainWorkbench.acceptanceAuditControlRecords",
      "控制面：记录 {{records}} · 已排空 {{drained}} · 连接器端到端 {{connector}}",
      {
        records: summary.controlRecords,
        drained: summary.drainedRuns,
        connector: summary.connectorE2eEvidence,
      },
    ),
    t(
      "workspace.domainWorkbench.acceptanceAuditFreshness",
      "新鲜度：{{freshness}} · 上限 {{max}}",
      {
        freshness: sampleFreshness,
        max: formatDurationCompact(summary.freshnessMaxAgeSecs),
      },
    ),
    t("workspace.domainWorkbench.acceptanceAuditSampleDays", "跨天覆盖：{{days}}", {
      days: domainAcceptanceSampleDaysText(t, summary),
    }),
    t(
      "workspace.domainWorkbench.acceptanceAuditConnectorVerification",
      "连接器复核：执行 {{executed}} · 复核 {{verified}}",
      {
        executed: summary.connectorExecutionEvidence,
        verified: summary.connectorVerificationEvidence,
      },
    ),
    t("workspace.domainWorkbench.acceptanceAuditBudget", "预算：{{budget}}", { budget }),
  ]
  if (context) {
    lines.push(
      t(
        "workspace.domainWorkbench.acceptanceAuditGates",
        "守门快照：交付={{exportStatus}} · 连接器={{connectorStatus}} · 端到端={{e2eStatus}} · 运行={{operationalStatus}} · 长跑={{soakStatus}}",
        {
          exportStatus: context.exportGuard?.status ?? "missing",
          connectorStatus: context.connectorGuard?.status ?? "missing",
          e2eStatus: context.connectorE2eGate?.status ?? "missing",
          operationalStatus: context.operationalGate?.status ?? "missing",
          soakStatus: context.soakReport?.status ?? "missing",
        },
      ),
    )
    const evidenceIds = context.evidence
      .slice(0, 8)
      .map((item) => item.id)
      .filter(Boolean)
    lines.push(
      t("workspace.domainWorkbench.acceptanceAuditEvidenceIds", "Evidence IDs：{{ids}}", {
        ids:
          evidenceIds.length > 0
            ? evidenceIds.join(", ")
            : t("workspace.domainWorkbench.acceptanceAuditNoEvidenceIds", "无"),
      }),
    )
    if (context.soakReport) {
      lines.push(
        t(
          "workspace.domainWorkbench.acceptanceAuditSoakWindow",
          "Soak 窗口：{{window}}d · incidents {{incidents}} · timeline {{timeline}}",
          {
            window: context.soakReport.windowDays,
            incidents: context.soakReport.incidents.length,
            timeline: context.soakReport.timeline.length,
          },
        ),
      )
    }
  }
  return lines.map((line) => `- ${line}`)
}

function domainAcceptancePlanTaskContent(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
  gaps: DomainAcceptanceGap[],
  context: DomainAcceptanceReviewContext,
): string {
  const verdict = domainAcceptanceVerdict(t, summary)
  const evidenceLevel = domainAcceptanceEvidenceLevel(t, summary)
  const domains = summary.domains.length > 0 ? summary.domains.join(", ") : "0"
  const sampleFreshness =
    summary.latestActivityAgeSecs != null
      ? t("workspace.domainWorkbench.acceptancePlanFreshnessAge", "{{age}} 前", {
          age: formatDurationCompact(summary.latestActivityAgeSecs),
        })
      : t("workspace.domainWorkbench.acceptancePlanFreshnessMissing", "缺最近活动时间")
  const budgetHealth =
    summary.budgetExhaustedEvents > 0
      ? t(
          "workspace.domainWorkbench.acceptancePlanBudgetExhausted",
          "耗尽 {{count}} 次{{budget}}",
          {
            count: summary.budgetExhaustedEvents,
            budget: summary.outputTokenBudgetLabel ? ` · ${summary.outputTokenBudgetLabel}` : "",
          },
        )
      : summary.outputTokenBudgetLabel
        ? t("workspace.domainWorkbench.acceptancePlanBudgetOkWithUsage", "未耗尽 · {{budget}}", {
            budget: summary.outputTokenBudgetLabel,
          })
        : t("workspace.domainWorkbench.acceptancePlanBudgetOk", "未观察到预算耗尽")
  const metrics = [
    `${t("workspace.domainWorkbench.acceptancePlanStatus", "状态")}：${domainAcceptanceStatusLabel(t, summary)}`,
    `${t("workspace.domainWorkbench.acceptanceVerdict", "验收结论")}：${verdict.label} - ${verdict.detail}`,
    `${t("workspace.domainWorkbench.acceptanceEvidenceLevel", "证据等级")}：${evidenceLevel.label} - ${evidenceLevel.detail}`,
    `${t("workspace.domainWorkbench.acceptanceProvenance", "来源分布")}：${domainAcceptanceProvenanceText(t, summary.provenance)}`,
    `${t("workspace.domainWorkbench.acceptanceControlMix", "控制面组成")}：${domainAcceptanceControlMixText(t, summary.controlMix)}`,
    `${t("workspace.domainWorkbench.acceptancePlanProgress", "验收进度")}：${summary.readinessPercent}% (${summary.requiredPassed}/${summary.requiredTotal})`,
    `${t("workspace.domainWorkbench.acceptancePlanDomains", "领域")}：${domains}`,
    `${t("workspace.domainWorkbench.acceptancePlanRecords", "控制面记录")}：${summary.controlRecords}`,
    `${t("workspace.domainWorkbench.acceptancePlanDrained", "已排空样本")}：${summary.drainedRuns}`,
    `${t("workspace.domainWorkbench.acceptancePlanFreshness", "最近样本")}：${sampleFreshness}`,
    `${t("workspace.domainWorkbench.acceptancePlanSampleDays", "跨天覆盖")}：${domainAcceptanceSampleDaysText(t, summary)}`,
    `${t("workspace.domainWorkbench.acceptancePlanBudget", "输出预算")}：${budgetHealth}`,
    `${t("workspace.domainWorkbench.acceptancePlanConnector", "连接器端到端 evidence")}：${summary.connectorE2eEvidence}（执行 ${summary.connectorExecutionEvidence} / 复核 ${summary.connectorVerificationEvidence}）`,
    `${t("workspace.domainWorkbench.acceptancePlanIncidents", "事故")}：critical ${summary.criticalIncidents} / warning ${summary.warningIncidents}`,
  ]
  const requirements = summary.requirements.map((requirement) => {
    const status = domainAcceptanceRequirementStatusLabel(t, requirement)
    return `- [${status}] ${requirement.label}：${requirement.detail}`
  })
  const sampleLanes = domainAcceptanceSampleLaneChecklistLines(t, summary.sampleLanes, {
    includeRefreshTargets: true,
  })
  const gapLines =
    gaps.length > 0
      ? gaps.map((gap) => `- [${domainAcceptanceGapLabel(t, gap.severity)}] ${gap.message}`)
      : [t("workspace.domainWorkbench.acceptanceReviewNoGaps", "- 暂无验收缺口")]
  const actions = [
    t(
      "workspace.domainWorkbench.acceptancePlanActionEvidence",
      "补齐来源、草稿、复核或用户决策 evidence 后刷新工作台。",
    ),
    t(
      "workspace.domainWorkbench.acceptancePlanActionDrain",
      "至少排空一个 Workflow / Loop / Campaign，再刷新运行稳定性和长跑审计。",
    ),
    t(
      "workspace.domainWorkbench.acceptancePlanActionFreshness",
      "如果最近样本超过 24 小时或缺跨天覆盖，先跑一个新的 Workflow / Loop / Campaign 或连接器端到端样本。",
    ),
    t(
      "workspace.domainWorkbench.acceptancePlanActionBudget",
      "如果输出预算耗尽，先收窄上下文、拆分阶段或降低无效输出，再重新跑最小验证样本。",
    ),
    t(
      "workspace.domainWorkbench.acceptancePlanActionConnector",
      "涉及外部动作时按读取 -> 草稿 -> 批准 -> 执行 -> 复核 -> 回滚说明记录端到端 evidence。",
    ),
    t(
      "workspace.domainWorkbench.acceptancePlanActionSoak",
      "处理 Soak Report 事故或把事故转任务，直到 Operational Gate / Soak Report 不再 failed。",
    ),
  ]

  return [
    t("workspace.domainWorkbench.acceptancePlanTaskContent", "补齐真实样本验收清单："),
    "",
    t("workspace.domainWorkbench.acceptancePlanMetrics", "当前指标："),
    ...metrics.map((metric) => `- ${metric}`),
    "",
    t("workspace.domainWorkbench.acceptanceAuditIndex", "审计索引："),
    ...domainAcceptanceAuditIndexLines(t, summary, context),
    "",
    t("workspace.domainWorkbench.acceptanceReviewProtocol", "复核协议："),
    ...domainAcceptanceReviewProtocolLines(t),
    "",
    t("workspace.domainWorkbench.acceptancePlanRequirements", "验收必需项："),
    ...requirements,
    "",
    t("workspace.domainWorkbench.acceptancePlanSampleLanes", "验收矩阵："),
    ...sampleLanes,
    "",
    t("workspace.domainWorkbench.acceptancePlanGaps", "验收缺口："),
    ...gapLines,
    "",
    t("workspace.domainWorkbench.acceptancePlanActions", "采样动作："),
    ...actions.map((action) => `- ${action}`),
  ].join("\n")
}

function domainAcceptanceReviewGateLine(
  t: ReturnType<typeof useTranslation>["t"],
  label: string,
  report: DomainAcceptanceReviewGate | null,
  displayStatus: string,
): string {
  if (!report) {
    return `- ${label}：${t("workspace.domainWorkbench.acceptanceReviewGateMissing", "未评估")}`
  }
  const blockers = (report.blockers ?? []).filter(Boolean).slice(0, 2)
  const nonPassingChecks = (report.checks ?? [])
    .filter((check) => check.status !== "passed")
    .slice(0, 2)
    .map((check) => {
      const detail = check.detail || check.actual
      const name = domainGuardCheckNameLabel(t, check.name)
      const status = diagnosticStatusLabel(t, check.status)
      return detail ? `${name}=${status} (${detail})` : `${name}=${status}`
    })
  const extras = [
    blockers.length > 0
      ? t("workspace.domainWorkbench.acceptanceReviewGateBlockers", "blockers: {{items}}", {
          items: blockers.join("; "),
        })
      : null,
    nonPassingChecks.length > 0
      ? t("workspace.domainWorkbench.acceptanceReviewGateChecks", "checks: {{items}}", {
          items: nonPassingChecks.join("; "),
        })
      : null,
  ].filter(Boolean)
  return `- ${label}：${displayStatus} (${report.status ?? "unknown"})${
    extras.length > 0 ? ` · ${extras.join(" · ")}` : ""
  }`
}

function domainAcceptanceReviewGateLines(
  t: ReturnType<typeof useTranslation>["t"],
  context: DomainAcceptanceReviewContext,
): string[] {
  return [
    domainAcceptanceReviewGateLine(
      t,
      t("workspace.domainWorkbench.acceptanceGateExport", "交付守门"),
      context.exportGuard,
      domainArtifactExportGuardLabel(t, context.exportGuard?.status),
    ),
    domainAcceptanceReviewGateLine(
      t,
      t("workspace.domainWorkbench.acceptanceGateConnector", "外部动作守门"),
      context.connectorGuard,
      domainConnectorActionGuardLabel(t, context.connectorGuard?.status),
    ),
    domainAcceptanceReviewGateLine(
      t,
      t("workspace.domainWorkbench.acceptanceGateConnectorE2E", "连接器端到端（E2E）"),
      context.connectorE2eGate,
      domainConnectorE2EGateLabel(t, context.connectorE2eGate?.status),
    ),
    domainAcceptanceReviewGateLine(
      t,
      t("workspace.domainWorkbench.acceptanceGateOperational", "运行稳定性"),
      context.operationalGate,
      domainOperationalGateLabel(t, context.operationalGate?.status),
    ),
    domainAcceptanceReviewGateLine(
      t,
      t("workspace.domainWorkbench.acceptanceGateSoak", "长跑审计"),
      context.soakReport,
      domainSoakReportLabel(t, context.soakReport?.status),
    ),
  ]
}

function domainAcceptanceReviewEvidenceLines(
  t: ReturnType<typeof useTranslation>["t"],
  evidence: DomainEvidenceItem[],
): string[] {
  const rows = evidence.slice(0, 5).map((item) => {
    const privacy = `${item.accessScope}/${item.redactionStatus}`
    return `- ${item.evidenceType} · ${item.domain} · ${privacy} · ${formatMessageTime(
      item.createdAt,
    )} · ${truncateMiddle(item.title, 120)} (${item.id})`
  })
  return rows.length > 0
    ? rows
    : [t("workspace.domainWorkbench.acceptanceReviewNoEvidence", "- 暂无 evidence")]
}

function domainAcceptanceReviewSoakLines(
  t: ReturnType<typeof useTranslation>["t"],
  soakReport: DomainSoakReport | null,
): string[] {
  if (!soakReport) {
    return [t("workspace.domainWorkbench.acceptanceReviewNoSoak", "- 长跑审计尚未生成")]
  }
  const summary = soakReport.summary
  const lines = [
    `- ${t("workspace.domainWorkbench.acceptanceReviewSoakWindow", "窗口")}：${soakReport.windowDays}d · ${domainSoakReportLabel(
      t,
      soakReport.status,
    )} (${soakReport.status})`,
    `- ${t("workspace.domainWorkbench.acceptanceReviewSoakSamples", "样本")}：workflow ${summary.completedWorkflowRuns}/${summary.workflowRuns} · loop ${summary.succeededLoopRuns}/${summary.loopRuns} · campaign ${summary.passedCampaignItems}/${summary.campaignItems} · connector ${summary.connectorE2eEvidence} (执行 ${summary.connectorExecutionEvidence} / 复核 ${summary.connectorVerificationEvidence}) · days ${summary.sampleDays}/${summary.requiredSampleDays}`,
    `- ${t("workspace.domainWorkbench.acceptanceReviewSoakBudget", "预算")}：${summary.workflowBudgetExhaustedEvents} exhausted · ${
      summary.maxWorkflowOutputTokensSpent != null
        ? compactCount(summary.maxWorkflowOutputTokensSpent)
        : "-"
    }/${
      summary.maxWorkflowOutputTokenBudget != null
        ? compactCount(summary.maxWorkflowOutputTokenBudget)
        : "-"
    }`,
  ]
  const incidentLines = soakReport.incidents
    .slice(0, 3)
    .map(
      (incident) =>
        `- ${t("workspace.domainWorkbench.acceptanceReviewIncident", "事故")}：${incident.title} · ${incident.source}/${incident.status}/${incident.severity} · ${incident.recommendation}`,
    )
  const timelineLines = soakReport.timeline.slice(0, 5).map((item) => {
    const duration =
      item.durationSecs != null ? ` · ${formatDurationCompact(item.durationSecs)}` : ""
    return `- ${t("workspace.domainWorkbench.acceptanceReviewTimeline", "时间线")}：${item.label} · ${item.source}/${item.status}${duration}`
  })
  return [...lines, ...incidentLines, ...timelineLines]
}

function domainAcceptanceReviewNextStepLines(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
  context: DomainAcceptanceReviewContext,
): string[] {
  const candidates = [
    ...summary.gaps.map((gap) => gap.message),
    ...(context.exportGuard?.recommendedNextSteps ?? []),
    ...(context.connectorGuard?.recommendedNextSteps ?? []),
    ...(context.connectorE2eGate?.recommendedNextSteps ?? []),
    ...(context.operationalGate?.recommendedNextSteps ?? []),
    ...(context.soakReport?.recommendedNextSteps ?? []),
  ]
  const seen = new Set<string>()
  const unique = candidates.filter((item) => {
    const trimmed = item.trim()
    if (!trimmed || seen.has(trimmed)) return false
    seen.add(trimmed)
    return true
  })
  return unique.length > 0
    ? unique.slice(0, 8).map((item) => `- ${item}`)
    : [t("workspace.domainWorkbench.acceptanceReviewNoNextSteps", "- 暂无推荐下一步")]
}

function domainAcceptanceReviewMarkdown(
  t: ReturnType<typeof useTranslation>["t"],
  summary: DomainAcceptanceCoverageSummary,
  context: DomainAcceptanceReviewContext,
): string {
  const verdict = domainAcceptanceVerdict(t, summary)
  const evidenceLevel = domainAcceptanceEvidenceLevel(t, summary)
  const domains = summary.domains.length > 0 ? summary.domains.join(", ") : "0"
  const sampleFreshness =
    summary.latestActivityAgeSecs != null
      ? t("workspace.domainWorkbench.acceptancePlanFreshnessAge", "{{age}} 前", {
          age: formatDurationCompact(summary.latestActivityAgeSecs),
        })
      : t("workspace.domainWorkbench.acceptancePlanFreshnessMissing", "缺最近活动时间")
  const budgetHealth =
    summary.budgetExhaustedEvents > 0
      ? t(
          "workspace.domainWorkbench.acceptancePlanBudgetExhausted",
          "耗尽 {{count}} 次{{budget}}",
          {
            count: summary.budgetExhaustedEvents,
            budget: summary.outputTokenBudgetLabel ? ` · ${summary.outputTokenBudgetLabel}` : "",
          },
        )
      : summary.outputTokenBudgetLabel
        ? t("workspace.domainWorkbench.acceptancePlanBudgetOkWithUsage", "未耗尽 · {{budget}}", {
            budget: summary.outputTokenBudgetLabel,
          })
        : t("workspace.domainWorkbench.acceptancePlanBudgetOk", "未观察到预算耗尽")
  const requirementLines = summary.requirements.map((requirement) => {
    const status = domainAcceptanceRequirementStatusLabel(t, requirement)
    return `- [${status}] ${requirement.label}：${requirement.detail}`
  })
  const sampleLaneLines = domainAcceptanceSampleLaneChecklistLines(t, summary.sampleLanes)
  const gapLines =
    summary.gaps.length > 0
      ? summary.gaps.map((gap) => `- [${domainAcceptanceGapLabel(t, gap.severity)}] ${gap.message}`)
      : [t("workspace.domainWorkbench.acceptanceReviewNoGaps", "- 暂无验收缺口")]

  return [
    `# ${t("workspace.domainWorkbench.acceptanceTitle", "真实样本验收")}`,
    "",
    `${t("workspace.domainWorkbench.acceptancePlanStatus", "状态")}：${domainAcceptanceStatusLabel(t, summary)}`,
    `${t("workspace.domainWorkbench.acceptanceVerdict", "验收结论")}：${verdict.label} - ${verdict.detail}`,
    `${t("workspace.domainWorkbench.acceptanceEvidenceLevel", "证据等级")}：${evidenceLevel.label} - ${evidenceLevel.detail}`,
    `${t("workspace.domainWorkbench.acceptanceProvenance", "来源分布")}：${domainAcceptanceProvenanceText(t, summary.provenance)}`,
    `${t("workspace.domainWorkbench.acceptanceControlMix", "控制面组成")}：${domainAcceptanceControlMixText(t, summary.controlMix)}`,
    `${t("workspace.domainWorkbench.acceptancePlanProgress", "验收进度")}：${summary.readinessPercent}% (${summary.requiredPassed}/${summary.requiredTotal})`,
    `${t("workspace.domainWorkbench.acceptancePlanDomains", "领域")}：${domains}`,
    `${t("workspace.domainWorkbench.acceptancePlanRecords", "控制面记录")}：${summary.controlRecords}`,
    `${t("workspace.domainWorkbench.acceptancePlanDrained", "已排空样本")}：${summary.drainedRuns}`,
    `${t("workspace.domainWorkbench.acceptancePlanFreshness", "最近样本")}：${sampleFreshness}`,
    `${t("workspace.domainWorkbench.acceptancePlanSampleDays", "跨天覆盖")}：${domainAcceptanceSampleDaysText(t, summary)}`,
    `${t("workspace.domainWorkbench.acceptancePlanBudget", "输出预算")}：${budgetHealth}`,
    `${t("workspace.domainWorkbench.acceptancePlanConnector", "连接器端到端 evidence")}：${summary.connectorE2eEvidence}（执行 ${summary.connectorExecutionEvidence} / 复核 ${summary.connectorVerificationEvidence}）`,
    `${t("workspace.domainWorkbench.acceptancePlanIncidents", "事故")}：critical ${summary.criticalIncidents} / warning ${summary.warningIncidents}`,
    "",
    `## ${t("workspace.domainWorkbench.acceptanceAuditIndex", "审计索引")}`,
    ...domainAcceptanceAuditIndexLines(t, summary, context),
    "",
    `## ${t("workspace.domainWorkbench.acceptanceReviewProtocol", "复核协议")}`,
    ...domainAcceptanceReviewProtocolLines(t),
    "",
    `## ${t("workspace.domainWorkbench.acceptancePlanRequirements", "验收必需项")}`,
    ...requirementLines,
    "",
    `## ${t("workspace.domainWorkbench.acceptancePlanSampleLanes", "验收矩阵")}`,
    ...sampleLaneLines,
    "",
    `## ${t("workspace.domainWorkbench.acceptancePlanGaps", "验收缺口")}`,
    ...gapLines,
    "",
    `## ${t("workspace.domainWorkbench.acceptanceReviewGates", "守门状态")}`,
    ...domainAcceptanceReviewGateLines(t, context),
    "",
    `## ${t("workspace.domainWorkbench.acceptanceReviewEvidence", "最近证据")}`,
    ...domainAcceptanceReviewEvidenceLines(t, context.evidence),
    "",
    `## ${t("workspace.domainWorkbench.acceptanceReviewSoak", "长跑审计")}`,
    ...domainAcceptanceReviewSoakLines(t, context.soakReport),
    "",
    `## ${t("workspace.domainWorkbench.acceptanceReviewNextSteps", "推荐下一步")}`,
    ...domainAcceptanceReviewNextStepLines(t, summary, context),
  ].join("\n")
}

function DomainAcceptanceCoverageCard({
  summary,
  reviewContext,
  creatingRequirementTaskKey,
  creatingSampleLaneTaskKey,
  creatingGapTaskKey,
  creatingPlanTask,
  onCreateRequirementTask,
  onCreateSampleLaneTask,
  onCreateGapTask,
  onCreateGapPlan,
}: {
  summary: DomainAcceptanceCoverageSummary
  reviewContext: DomainAcceptanceReviewContext
  creatingRequirementTaskKey?: string | null
  creatingSampleLaneTaskKey?: string | null
  creatingGapTaskKey?: string | null
  creatingPlanTask?: boolean
  onCreateRequirementTask?: (requirement: DomainAcceptanceRequirement, index: number) => void
  onCreateSampleLaneTask?: (lane: DomainAcceptanceSampleLane, index: number) => void
  onCreateGapTask?: (gap: DomainAcceptanceGap, index: number) => void
  onCreateGapPlan?: (gaps: DomainAcceptanceGap[]) => void
}) {
  const { t } = useTranslation()
  const verdict = domainAcceptanceVerdict(t, summary)
  const evidenceLevel = domainAcceptanceEvidenceLevel(t, summary)
  const snapshotId = domainAcceptanceSnapshotId(summary, reviewContext)
  const acceptanceReviewLabel = t("workspace.domainWorkbench.copyAcceptanceReview", "复制验收报告")
  const copyAcceptanceReview = async () => {
    try {
      await navigator.clipboard.writeText(domainAcceptanceReviewMarkdown(t, summary, reviewContext))
      toast.success(t("workspace.domainWorkbench.acceptanceReviewCopied", "已复制验收报告"))
    } catch (e) {
      logger.error("ui", "DomainAcceptanceCoverageCard", "Copy acceptance review report failed", e)
      toast.error(t("workspace.domainWorkbench.acceptanceReviewCopyFailed", "复制验收报告失败"))
    }
  }

  return (
    <div className={cn("rounded-md border px-2.5 py-2", STATUS_TONE_CLASS[summary.tone])}>
      <div className="flex min-w-0 items-center gap-1.5">
        <ClipboardCheck className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium">
          {t("workspace.domainWorkbench.acceptanceTitle", "真实样本验收")}
        </span>
        <StatusPill label={domainAcceptanceStatusLabel(t, summary)} tone={summary.tone} />
        <IconTip label={acceptanceReviewLabel}>
          <button
            type="button"
            aria-label={acceptanceReviewLabel}
            onClick={() => void copyAcceptanceReview()}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/55 bg-background/45 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground"
          >
            <Copy className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        {domainAcceptanceHasActionableSamplingWork(summary) && onCreateGapPlan ? (
          <button
            type="button"
            onClick={() => onCreateGapPlan(summary.gaps)}
            disabled={
              Boolean(creatingRequirementTaskKey) ||
              Boolean(creatingSampleLaneTaskKey) ||
              Boolean(creatingGapTaskKey) ||
              Boolean(creatingPlanTask)
            }
            className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-2 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
          >
            {creatingPlanTask ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Plus className="h-3 w-3" />
            )}
            <span>{t("workspace.domainWorkbench.createAcceptancePlan", "采样清单")}</span>
          </button>
        ) : null}
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground/75">
        <span className="shrink-0">
          {t("workspace.domainWorkbench.acceptanceSnapshot", "验收快照")}
        </span>
        <code className="min-w-0 truncate rounded bg-background/45 px-1 py-0.5 font-mono text-[10px]">
          {snapshotId}
        </code>
      </div>
      <div className="mt-2 space-y-1">
        <div className="flex min-w-0 items-center justify-between gap-2 text-[10px]">
          <span className="truncate opacity-80">
            {t("workspace.domainWorkbench.acceptanceProgress", "验收进度")}
          </span>
          <span className="shrink-0 font-mono tabular-nums">
            {summary.readinessPercent}% · {summary.requiredPassed}/{summary.requiredTotal}
          </span>
        </div>
        <div className="h-1.5 overflow-hidden rounded-full bg-background/55">
          <div
            className={cn(
              "h-full rounded-full transition-all duration-300",
              domainAcceptanceProgressBarClass(summary.tone),
            )}
            style={{ width: `${summary.readinessPercent}%` }}
          />
        </div>
      </div>
      <div className="mt-2 flex min-w-0 items-start gap-1.5 text-[10px]">
        <Layers className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-muted-foreground">
          {t("workspace.domainWorkbench.acceptanceEvidenceLevel", "证据等级")}
        </span>
        <StatusPill label={evidenceLevel.label} tone={evidenceLevel.tone} />
        <span className="min-w-0 flex-1 text-muted-foreground/75">{evidenceLevel.detail}</span>
      </div>
      <div className="mt-2 flex min-w-0 items-start gap-1.5 text-[10px]">
        <GitBranch className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-muted-foreground">
          {t("workspace.domainWorkbench.acceptanceProvenance", "来源分布")}
        </span>
        <span className="min-w-0 flex-1 text-muted-foreground/75">
          {domainAcceptanceProvenanceText(t, summary.provenance)}
        </span>
      </div>
      <div className="mt-2 flex min-w-0 items-start gap-1.5 text-[10px]">
        <Radio className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-muted-foreground">
          {t("workspace.domainWorkbench.acceptanceControlMix", "控制面组成")}
        </span>
        <span className="min-w-0 flex-1 text-muted-foreground/75">
          {domainAcceptanceControlMixText(t, summary.controlMix)}
        </span>
      </div>
      <div className="mt-2 flex min-w-0 items-start gap-1.5 text-[10px]">
        <Shield className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-muted-foreground">
          {t("workspace.domainWorkbench.acceptanceVerdict", "验收结论")}
        </span>
        <StatusPill label={verdict.label} tone={verdict.tone} />
        <span className="min-w-0 flex-1 text-muted-foreground/75">{verdict.detail}</span>
      </div>
      <div className="mt-2 grid grid-cols-2 gap-1.5">
        {summary.requirements.map((requirement, index) => {
          const taskKey = `requirement:${index}:${requirement.key}`
          return (
            <div
              key={requirement.key}
              className="min-w-0 rounded-md bg-background/40 px-1.5 py-1 text-[10px]"
            >
              <div className="flex min-w-0 items-center gap-1">
                <span className="min-w-0 flex-1 truncate font-medium">{requirement.label}</span>
                <StatusPill
                  label={domainAcceptanceRequirementStatusLabel(t, requirement)}
                  tone={requirement.tone}
                />
              </div>
              <div className="mt-1 flex min-w-0 items-center gap-1">
                <span className="min-w-0 flex-1 truncate text-muted-foreground/75">
                  {requirement.detail}
                </span>
                {!requirement.passed && onCreateRequirementTask ? (
                  <button
                    type="button"
                    onClick={() => onCreateRequirementTask(requirement, index)}
                    disabled={
                      Boolean(creatingRequirementTaskKey) ||
                      Boolean(creatingSampleLaneTaskKey) ||
                      Boolean(creatingGapTaskKey) ||
                      Boolean(creatingPlanTask)
                    }
                    className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {creatingRequirementTaskKey === taskKey ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Plus className="h-3 w-3" />
                    )}
                    <span>
                      {t("workspace.domainWorkbench.createAcceptanceRequirementTask", "转任务")}
                    </span>
                  </button>
                ) : null}
              </div>
            </div>
          )
        })}
      </div>
      <div className="mt-2 grid grid-cols-4 gap-1.5">
        <DomainWorkbenchMetric
          icon={Globe}
          label={t("workspace.domainWorkbench.acceptanceDomains", "领域")}
          value={t("workspace.domainWorkbench.acceptanceDomainCount", "{{count}} 个", {
            count: summary.domains.length,
          })}
          tone={domainWorkbenchMetricTone(summary.domains.length)}
        />
        <DomainWorkbenchMetric
          icon={Database}
          label={t("workspace.domainWorkbench.acceptanceRecords", "记录")}
          value={t("workspace.domainWorkbench.acceptanceRecordCount", "{{count}} 条", {
            count: summary.controlRecords,
          })}
          tone={domainWorkbenchMetricTone(summary.controlRecords)}
        />
        <DomainWorkbenchMetric
          icon={CheckCircle2}
          label={t("workspace.domainWorkbench.acceptanceDrained", "排空")}
          value={t("workspace.domainWorkbench.acceptanceDrainedCount", "{{count}} 个", {
            count: summary.drainedRuns,
          })}
          tone={domainWorkbenchMetricTone(summary.drainedRuns)}
        />
        <DomainWorkbenchMetric
          icon={Shield}
          label={t("workspace.domainWorkbench.acceptanceConnector", "端到端")}
          value={summary.connectorE2eEvidence}
          tone={domainWorkbenchMetricTone(summary.connectorE2eEvidence)}
        />
      </div>
      <div className="mt-2 space-y-1 rounded-md bg-background/30 px-1.5 py-1.5">
        <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
          <Gauge className="h-3 w-3 shrink-0" />
          <span className="truncate">
            {t("workspace.domainWorkbench.acceptancePlanSampleLanes", "验收矩阵")}
          </span>
        </div>
        <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
          {summary.sampleLanes.map((lane, index) => {
            const taskKey = `lane:${index}:${lane.key}`
            return (
              <IconTip key={lane.key} label={lane.action}>
                <div className="min-w-0 rounded-md bg-secondary/25 px-1.5 py-1 text-[10px]">
                  <div className="flex min-w-0 items-center gap-1">
                    <span className="min-w-0 flex-1 truncate font-medium">{lane.label}</span>
                    <StatusPill
                      label={domainAcceptanceSampleLaneStatusLabel(t, lane)}
                      tone={lane.tone}
                    />
                  </div>
                  <div className="mt-0.5 flex min-w-0 items-center gap-1">
                    <span className="min-w-0 flex-1 truncate text-muted-foreground/75">
                      {lane.detail}
                    </span>
                    {!lane.passed && onCreateSampleLaneTask ? (
                      <button
                        type="button"
                        onClick={() => onCreateSampleLaneTask(lane, index)}
                        disabled={
                          Boolean(creatingRequirementTaskKey) ||
                          Boolean(creatingSampleLaneTaskKey) ||
                          Boolean(creatingGapTaskKey) ||
                          Boolean(creatingPlanTask)
                        }
                        className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {creatingSampleLaneTaskKey === taskKey ? (
                          <Loader2 className="h-3 w-3 animate-spin" />
                        ) : (
                          <Plus className="h-3 w-3" />
                        )}
                        <span>
                          {t("workspace.domainWorkbench.createAcceptanceLaneTask", "转任务")}
                        </span>
                      </button>
                    ) : null}
                  </div>
                </div>
              </IconTip>
            )
          })}
        </div>
      </div>
      {summary.domains.length > 0 ? (
        <div className="mt-2 flex min-w-0 flex-wrap gap-1">
          {summary.domains.slice(0, 4).map((domain) => (
            <StatusPill key={domain} label={domainLabel(t, domain)} tone="info" />
          ))}
          {summary.domains.length > 4 ? (
            <StatusPill
              label={t("workspace.domainWorkbench.acceptanceMoreDomains", "+{{count}}", {
                count: summary.domains.length - 4,
              })}
              tone="muted"
            />
          ) : null}
        </div>
      ) : null}
      {summary.gaps.length > 0 ? (
        <div className="mt-2 space-y-1">
          {summary.gaps.map((gap, index) => {
            const taskKey = `acceptance:${index}:${gap.key}`
            return (
              <div
                key={taskKey}
                className="flex min-w-0 items-start gap-1.5 rounded-md px-1.5 py-1 text-[11px] leading-snug text-muted-foreground"
              >
                <StatusPill
                  label={domainAcceptanceGapLabel(t, gap.severity)}
                  tone={domainAcceptanceGapTone(gap.severity)}
                />
                <span className="line-clamp-2 min-w-0 flex-1">{gap.message}</span>
                {onCreateGapTask ? (
                  <button
                    type="button"
                    onClick={() => onCreateGapTask(gap, index)}
                    disabled={
                      Boolean(creatingRequirementTaskKey) ||
                      Boolean(creatingSampleLaneTaskKey) ||
                      Boolean(creatingGapTaskKey) ||
                      Boolean(creatingPlanTask)
                    }
                    className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {creatingGapTaskKey === taskKey ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Plus className="h-3 w-3" />
                    )}
                    <span>{t("workspace.domainWorkbench.createAcceptanceTask", "转任务")}</span>
                  </button>
                ) : null}
              </div>
            )
          })}
        </div>
      ) : null}
    </div>
  )
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
          <span className="truncate">{domainLabel(t, item.domain)}</span>
          <span className="truncate">{domainAccessScopeLabel(t, item.accessScope)}</span>
          <span className="truncate">{domainRedactionStatusLabel(t, item.redactionStatus)}</span>
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
  focusRequest?: {
    target: "export" | "connector" | "connector-e2e" | "operational" | "soak"
    nonce: number
  } | null
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
    connectorE2eGate,
    connectorE2eGateLoading,
    connectorE2eGateError,
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
    (connectorGuard?.summary?.approvalEvidence ?? 0)
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
    connectorE2eGateLoading ||
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
  const actionableExportGuard = domainArtifactExportGuardHasScope(exportGuard) ? exportGuard : null
  const actionableConnectorGuard = domainConnectorActionGuardHasScope(connectorGuard)
    ? connectorGuard
    : null
  const actionableConnectorE2eGate = domainConnectorE2EGateHasScope(connectorE2eGate)
    ? connectorE2eGate
    : null
  const sampledOperationalGate = domainOperationalGateHasSamples(operationalGate)
    ? operationalGate
    : null
  const sampledSoakReport = domainSoakReportHasSamples(soakReport) ? soakReport : null
  const acceptanceSummary = domainAcceptanceCoverageSummary(t, {
    evidence,
    exportGuard: actionableExportGuard,
    connectorGuard: actionableConnectorGuard,
    connectorE2eGate: actionableConnectorE2eGate,
    operationalGate: sampledOperationalGate,
    soakReport: sampledSoakReport,
  })
  const tone = domainWorkbenchOverallTone({
    incognito,
    evidenceCount,
    sourceCount,
    draftCount,
    controlRecords: acceptanceSummary.controlRecords,
    blockingReviewFindings,
    failedVerification,
    domainFailed,
    domainNeedsUser,
    exportStatus: actionableExportGuard?.status,
    connectorStatus: actionableConnectorGuard?.status,
    connectorE2EStatus: actionableConnectorE2eGate?.status,
    operationalStatus: sampledOperationalGate?.status,
    soakStatus: sampledSoakReport?.status,
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
    exportGuard: actionableExportGuard,
    connectorGuard: actionableConnectorGuard,
    connectorE2eGate: actionableConnectorE2eGate,
    operationalGate: sampledOperationalGate,
    soakReport: sampledSoakReport,
  })
  const canCreateStepTasks = !disabled && tone !== "good"
  const acceptanceReviewContext: DomainAcceptanceReviewContext = {
    evidence,
    exportGuard,
    connectorGuard,
    connectorE2eGate,
    operationalGate,
    soakReport,
  }
  const recentEvidence = evidence.slice(0, 4)
  const error =
    evidenceError ??
    exportGuardError ??
    connectorGuardError ??
    connectorE2eGateError ??
    operationalGateError ??
    soakReportError
  const count =
    evidenceCount +
    openReviewFindings.length +
    verificationSteps.length +
    (domainQualityRunsState.snapshot?.checks.length ?? 0) +
    (actionableExportGuard?.checks?.length ?? 0) +
    (actionableConnectorGuard?.checks?.length ?? 0) +
    (actionableConnectorE2eGate?.checks?.length ?? 0) +
    (sampledOperationalGate?.checks?.length ?? 0) +
    (sampledSoakReport?.incidents?.length ?? 0)

  const focusSignal = focusRequest?.nonce ?? 0
  const shouldAutoExpand =
    !incognito && (tone === "danger" || Boolean(error) || Boolean(focusRequest))
  const [creatingStepTaskKey, setCreatingStepTaskKey] = useState<string | null>(null)
  const [creatingAcceptanceGapTaskKey, setCreatingAcceptanceGapTaskKey] = useState<string | null>(
    null,
  )
  const [creatingAcceptanceRequirementTaskKey, setCreatingAcceptanceRequirementTaskKey] = useState<
    string | null
  >(null)
  const [creatingAcceptanceSampleLaneTaskKey, setCreatingAcceptanceSampleLaneTaskKey] = useState<
    string | null
  >(null)
  const [creatingAcceptancePlanTask, setCreatingAcceptancePlanTask] = useState(false)

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

  const handleArtifactReview = async (target: DomainArtifactReviewTarget) => {
    const next = await domainQualityRunsState.runDomainQuality({
      domain: target.domain || undefined,
      artifactTitle: target.title || undefined,
      artifactKind: target.kind || undefined,
      sourceMetadata: {
        sourceType: "artifact_export_guard",
        artifactPath: target.path ?? null,
        artifactTitle: target.title ?? null,
        artifactKind: target.kind ?? null,
        artifactGuardStatus: target.guardStatus ?? null,
      },
    })
    if (next) void refreshAll()
    return next
  }

  const createNextStepTask = async (step: string, index: number) => {
    if (!sessionId || disabled || creatingStepTaskKey) return
    const taskKey = `${index}:${step}`
    setCreatingStepTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: step,
        activeForm: t(
          "workspace.domainWorkbench.stepTaskActiveForm",
          "正在处理通用任务缺口：{{step}}",
          {
            step,
          },
        ),
      })
      toast.success(t("workspace.domainWorkbench.stepTaskCreated", "已创建任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainTaskWorkbenchSection", "Create next-step task failed", e)
      toast.error(message)
    } finally {
      setCreatingStepTaskKey(null)
    }
  }

  const createAcceptanceGapTask = async (gap: DomainAcceptanceGap, index: number) => {
    if (!sessionId || disabled || creatingAcceptanceGapTaskKey) return
    const taskKey = `acceptance:${index}:${gap.key}`
    setCreatingAcceptanceGapTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainWorkbench.acceptanceTaskContent",
          "补齐真实样本验收缺口：{{gap}}",
          { gap: gap.message },
        ),
        activeForm: t(
          "workspace.domainWorkbench.acceptanceTaskActiveForm",
          "正在补齐真实样本验收缺口",
        ),
      })
      toast.success(t("workspace.domainWorkbench.acceptanceTaskCreated", "已创建真实样本任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainTaskWorkbenchSection", "Create acceptance gap task failed", e)
      toast.error(message)
    } finally {
      setCreatingAcceptanceGapTaskKey(null)
    }
  }

  const createAcceptanceRequirementTask = async (
    requirement: DomainAcceptanceRequirement,
    index: number,
  ) => {
    if (!sessionId || disabled || creatingAcceptanceRequirementTaskKey) return
    const taskKey = `requirement:${index}:${requirement.key}`
    setCreatingAcceptanceRequirementTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainWorkbench.acceptanceRequirementTaskContent",
          "补齐真实样本验收必需项：{{requirement}}（{{detail}}）",
          { requirement: requirement.label, detail: requirement.detail },
        ),
        activeForm: t(
          "workspace.domainWorkbench.acceptanceRequirementTaskActiveForm",
          "正在补齐真实样本验收必需项",
        ),
      })
      toast.success(
        t("workspace.domainWorkbench.acceptanceRequirementTaskCreated", "已创建验收必需项任务"),
      )
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "DomainTaskWorkbenchSection",
        "Create acceptance requirement task failed",
        e,
      )
      toast.error(message)
    } finally {
      setCreatingAcceptanceRequirementTaskKey(null)
    }
  }

  const createAcceptanceSampleLaneTask = async (
    lane: DomainAcceptanceSampleLane,
    index: number,
  ) => {
    if (!sessionId || disabled || creatingAcceptanceSampleLaneTaskKey) return
    const taskKey = `lane:${index}:${lane.key}`
    setCreatingAcceptanceSampleLaneTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: domainAcceptanceSampleLaneTaskContent(t, lane),
        activeForm: t(
          "workspace.domainWorkbench.acceptanceLaneTaskActiveForm",
          "正在补齐真实样本验收跑道",
        ),
      })
      toast.success(t("workspace.domainWorkbench.acceptanceLaneTaskCreated", "已创建验收跑道任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "DomainTaskWorkbenchSection",
        "Create acceptance sample lane task failed",
        e,
      )
      toast.error(message)
    } finally {
      setCreatingAcceptanceSampleLaneTaskKey(null)
    }
  }

  const createAcceptancePlanTask = async (gaps: DomainAcceptanceGap[]) => {
    if (
      !sessionId ||
      disabled ||
      creatingAcceptancePlanTask ||
      !domainAcceptanceHasActionableSamplingWork(acceptanceSummary)
    ) {
      return
    }
    setCreatingAcceptancePlanTask(true)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: domainAcceptancePlanTaskContent(
          t,
          acceptanceSummary,
          gaps,
          acceptanceReviewContext,
        ),
        activeForm: t(
          "workspace.domainWorkbench.acceptancePlanTaskActiveForm",
          "正在补齐真实样本验收清单",
        ),
      })
      toast.success(t("workspace.domainWorkbench.acceptancePlanTaskCreated", "已创建真实样本清单"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainTaskWorkbenchSection", "Create acceptance plan task failed", e)
      toast.error(message)
    } finally {
      setCreatingAcceptancePlanTask(false)
    }
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
      autoExpandWhen={shouldAutoExpand}
      meta={
        <StatusPill
          label={domainWorkbenchOverallLabel(t, tone, loading)}
          tone={tone}
          loading={loading}
        />
      }
      defaultExpanded={false}
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
            <span className="truncate">
              {t("workspace.domainWorkbench.runQuality", "运行领域复核")}
            </span>
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
            <span className="truncate">
              {t("workspace.domainWorkbench.planVerify", "推荐验证")}
            </span>
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
          <EmptyHint>
            {t("workspace.domainWorkbench.incognito", "无痕会话不持久化通用任务证据")}
          </EmptyHint>
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
                className="flex min-w-0 items-start gap-1.5 rounded-md px-1.5 py-1 text-[11px] leading-snug text-muted-foreground"
              >
                <span className="line-clamp-2 min-w-0 flex-1">{step}</span>
                {canCreateStepTasks ? (
                  <button
                    type="button"
                    onClick={() => void createNextStepTask(step, index)}
                    disabled={Boolean(creatingStepTaskKey)}
                    className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {creatingStepTaskKey === `${index}:${step}` ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Plus className="h-3 w-3" />
                    )}
                    <span>{t("workspace.domainWorkbench.createStepTask", "转任务")}</span>
                  </button>
                ) : null}
              </div>
            ))}
          </div>
        </div>

        <DomainAcceptanceCoverageCard
          summary={acceptanceSummary}
          reviewContext={acceptanceReviewContext}
          creatingRequirementTaskKey={creatingAcceptanceRequirementTaskKey}
          creatingSampleLaneTaskKey={creatingAcceptanceSampleLaneTaskKey}
          creatingGapTaskKey={creatingAcceptanceGapTaskKey}
          creatingPlanTask={creatingAcceptancePlanTask}
          onCreateRequirementTask={!disabled ? createAcceptanceRequirementTask : undefined}
          onCreateSampleLaneTask={!disabled ? createAcceptanceSampleLaneTask : undefined}
          onCreateGapTask={!disabled ? createAcceptanceGapTask : undefined}
          onCreateGapPlan={!disabled ? createAcceptancePlanTask : undefined}
        />

        <div className="grid grid-cols-1 gap-2">
          <DomainOperationalGatePanel
            sessionId={sessionId}
            report={operationalGate}
            loading={operationalGateLoading}
            error={operationalGateError}
            disabled={disabled || operationalGateLoading}
            onRefresh={domainWorkbenchState.refreshOperationalGate}
          />
          <DomainSoakReportPanel
            sessionId={sessionId}
            report={soakReport}
            loading={soakReportLoading}
            error={soakReportError}
            disabled={disabled || soakReportLoading}
            onRefresh={domainWorkbenchState.refreshSoakReport}
          />
          <DomainArtifactExportGuardPanel
            sessionId={sessionId}
            report={exportGuard}
            loading={exportGuardLoading}
            error={exportGuardError}
            disabled={disabled || exportGuardLoading}
            onRefresh={domainWorkbenchState.refreshExportGuard}
            onReviewArtifact={handleArtifactReview}
          />
          <DomainConnectorActionGuardPanel
            sessionId={sessionId}
            report={connectorGuard}
            loading={connectorGuardLoading}
            error={connectorGuardError}
            disabled={disabled || connectorGuardLoading}
            onRefresh={domainWorkbenchState.refreshConnectorGuard}
          />
          <DomainConnectorE2EGatePanel
            sessionId={sessionId}
            report={connectorE2eGate}
            loading={connectorE2eGateLoading}
            error={connectorE2eGateError}
            disabled={disabled || connectorE2eGateLoading}
            onRefresh={domainWorkbenchState.refreshConnectorE2eGate}
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

function lspSeverityLabel(
  t: ReturnType<typeof useTranslation>["t"],
  severity: LspDiagnostic["severity"],
): string {
  switch (severity) {
    case "error":
      return t("workspace.lsp.severityError", "错误")
    case "warning":
      return t("workspace.lsp.severityWarning", "警告")
    case "information":
      return t("workspace.lsp.severityInformation", "信息")
    case "hint":
      return t("workspace.lsp.severityHint", "提示")
    case "unknown":
      return t("workspace.lsp.severityUnknown", "未知")
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
                        label={lspSeverityLabel(t, diagnostic.severity)}
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

function reviewVerdictLabel(
  t: ReturnType<typeof useTranslation>["t"],
  verdict: ReviewVerdict,
): string {
  switch (verdict) {
    case "confirmed":
      return t("workspace.review.verdictConfirmed", "已确认")
    case "plausible":
      return t("workspace.review.verdictPlausible", "可能存在")
    case "refuted":
      return t("workspace.review.verdictRefuted", "已排除")
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
  { id: "correctness", labelKey: "workspace.review.profileCorrectness", defaultLabel: "正确性" },
  { id: "security", labelKey: "workspace.review.profileSecurity", defaultLabel: "安全" },
  {
    id: "maintainability",
    labelKey: "workspace.review.profileMaintainability",
    defaultLabel: "维护性",
  },
  { id: "tests", labelKey: "workspace.review.profileTests", defaultLabel: "测试" },
  { id: "concurrency", labelKey: "workspace.review.profileConcurrency", defaultLabel: "并发" },
  { id: "frontend", labelKey: "workspace.review.profileFrontend", defaultLabel: "前端" },
  {
    id: "accessibility",
    labelKey: "workspace.review.profileAccessibility",
    defaultLabel: "可访问性",
  },
  { id: "deep", labelKey: "workspace.review.profileDeep", defaultLabel: "深度" },
] as const

function reviewProfileLabel(t: ReturnType<typeof useTranslation>["t"], profile: string): string {
  const option = REVIEW_PROFILE_OPTIONS.find((item) => item.id === profile)
  if (option) return t(option.labelKey, option.defaultLabel)
  return profile
}

const DEFAULT_REVIEW_PROFILES = ["correctness", "security", "maintainability", "tests"]
const DOMAIN_WORKBENCH_EVIDENCE_LIMIT = 60

function eventBelongsToSession(payload: unknown, sessionId: string): boolean {
  if (typeof payload !== "object" || payload === null) return true
  const value = (payload as { sessionId?: unknown }).sessionId
  return typeof value !== "string" || value === sessionId
}

function isReportObject<T>(value: T | null | undefined): value is T {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    typeof (value as { status?: unknown }).status === "string"
  )
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
  connectorE2eGate: DomainConnectorE2EGateReport | null
  connectorE2eGateLoading: boolean
  connectorE2eGateError: string | null
  operationalGate: DomainOperationalGateReport | null
  operationalGateLoading: boolean
  operationalGateError: string | null
  soakReport: DomainSoakReport | null
  soakReportLoading: boolean
  soakReportError: string | null
  refreshEvidence: () => Promise<DomainEvidenceItem[]>
  refreshExportGuard: () => Promise<DomainArtifactExportGuardReport | null>
  refreshConnectorGuard: () => Promise<DomainConnectorActionGuardReport | null>
  refreshConnectorE2eGate: () => Promise<DomainConnectorE2EGateReport | null>
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
  const [connectorGuard, setConnectorGuard] = useState<DomainConnectorActionGuardReport | null>(
    null,
  )
  const [connectorGuardLoading, setConnectorGuardLoading] = useState(false)
  const [connectorGuardError, setConnectorGuardError] = useState<string | null>(null)
  const [connectorE2eGate, setConnectorE2eGate] = useState<DomainConnectorE2EGateReport | null>(
    null,
  )
  const [connectorE2eGateLoading, setConnectorE2eGateLoading] = useState(false)
  const [connectorE2eGateError, setConnectorE2eGateError] = useState<string | null>(null)
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
      logger.error("ui", "useDomainTaskWorkbench", "Failed to load domain task evidence", e)
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
      const report = await getTransport().call<DomainArtifactExportGuardReport | null>(
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
      const safeReport = isReportObject(report) ? report : null
      setExportGuard(safeReport)
      return safeReport
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useDomainTaskWorkbench", "Failed to evaluate artifact export guard", e)
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
      const report = await getTransport().call<DomainConnectorActionGuardReport | null>(
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
      const safeReport = isReportObject(report) ? report : null
      setConnectorGuard(safeReport)
      return safeReport
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useDomainTaskWorkbench", "Failed to evaluate connector action guard", e)
      setConnectorGuardError(message)
      return null
    } finally {
      setConnectorGuardLoading(false)
    }
  }, [disabled, incognito, sessionId])

  const refreshConnectorE2eGate = useCallback(async () => {
    if (disabled || !sessionId || incognito) {
      setConnectorE2eGate(null)
      setConnectorE2eGateError(null)
      setConnectorE2eGateLoading(false)
      return null
    }
    setConnectorE2eGateLoading(true)
    setConnectorE2eGateError(null)
    try {
      const report = await getTransport().call<DomainConnectorE2EGateReport | null>(
        "evaluate_domain_connector_e2e_gate",
        {
          input: {
            sessionId,
            requireConnectorInput: true,
            requireDraft: true,
            requireExplicitApproval: true,
            requireExecutionResult: true,
            requirePostActionVerification: true,
            requireRollbackPlan: true,
            requireExportGuardForDelivery: true,
          },
        },
      )
      const safeReport = isReportObject(report) ? report : null
      setConnectorE2eGate(safeReport)
      return safeReport
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useDomainTaskWorkbench", "Failed to evaluate connector E2E gate", e)
      setConnectorE2eGateError(message)
      return null
    } finally {
      setConnectorE2eGateLoading(false)
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
      const report = await getTransport().call<DomainOperationalGateReport | null>(
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
      const safeReport = isReportObject(report) ? report : null
      setOperationalGate(safeReport)
      return safeReport
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "useDomainTaskWorkbench", "Failed to evaluate domain operational gate", e)
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
      const report = await getTransport().call<DomainSoakReport | null>(
        "generate_domain_soak_report",
        {
          input: {
            sessionId,
            windowDays: 14,
            maxItems: 8,
          },
        },
      )
      const safeReport = isReportObject(report) ? report : null
      setSoakReport(safeReport)
      return safeReport
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
      refreshConnectorE2eGate(),
      refreshOperationalGate(),
      refreshSoakReport(),
    ])
  }, [
    refreshConnectorE2eGate,
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
    connectorE2eGate,
    connectorE2eGateLoading,
    connectorE2eGateError,
    operationalGate,
    operationalGateLoading,
    operationalGateError,
    soakReport,
    soakReportLoading,
    soakReportError,
    refreshEvidence,
    refreshExportGuard,
    refreshConnectorGuard,
    refreshConnectorE2eGate,
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
                    ? "border-border/50 bg-secondary/70 text-foreground"
                    : "border-border/50 bg-secondary/20 text-muted-foreground hover:bg-secondary/45 hover:text-foreground",
                )}
              >
                <span className="block truncate">{t(profile.labelKey, profile.defaultLabel)}</span>
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
                  <StatusPill key={profile} label={reviewProfileLabel(t, profile)} tone="muted" />
                ))}
                {ideContextPresent ? (
                  <StatusPill label={t("workspace.review.ideContext", "IDE 上下文")} tone="info" />
                ) : null}
                {llmReviewer && llmReviewer !== "not_requested" ? (
                  <StatusPill
                    label={
                      llmReviewer === "completed"
                        ? t("workspace.review.deepReviewer", "深度审查")
                        : t("workspace.review.deepSkipped", "已跳过深度审查")
                    }
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
                    <StatusPill
                      label={reviewVerdictLabel(t, finding.verdict)}
                      tone={reviewVerdictTone(finding.verdict)}
                    />
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

function domainLabel(t: ReturnType<typeof useTranslation>["t"], domain?: string | null): string {
  return domain
    ? domain.replace(/_/g, " ")
    : t("workspace.domainWorkbench.domainFallbackLabel", "领域")
}

function domainQualityTemplateLabel(run: DomainQualityRunSnapshot["run"]): string | null {
  if (!run.templateId) return null
  return run.templateVersion ? `${run.templateId}@${run.templateVersion}` : run.templateId
}

function domainQualityEvidenceScopeView(
  t: ReturnType<typeof useTranslation>["t"],
  run: DomainQualityRunSnapshot["run"],
): { label: string; tone: StatusTone; detail: string | null } | null {
  const scope = asRecord(run.stats?.evidenceScope)
  const mode = stringField(scope, "mode")
  if (!mode) return null
  const total = numberField(scope, "total")
  const matched = numberField(scope, "matched")
  const target = asRecord(scope?.target)
  const targetLabel =
    stringField(target, "title") ?? stringField(target, "path") ?? stringField(target, "kind")
  const countText =
    total !== null && matched !== null
      ? t("workspace.domainQuality.scopeCount", "{{matched}}/{{total}} 条", {
          matched,
          total,
        })
      : null
  if (mode === "artifact_matched") {
    return {
      label: t("workspace.domainQuality.scopeArtifact", "产物证据"),
      tone: matched === 0 ? "warn" : "info",
      detail: targetLabel
        ? t("workspace.domainQuality.scopeArtifactDetail", "{{target}} · {{scopeText}}匹配", {
            target: targetLabel,
            scopeText: countText ?? t("workspace.domainQuality.scopeUnknownCount", "已"),
          })
        : countText
          ? t("workspace.domainQuality.scopeArtifactCount", "{{scopeText}}匹配", {
              scopeText: countText,
            })
          : null,
    }
  }
  if (mode === "legacy_fallback_all") {
    return {
      label: t("workspace.domainQuality.scopeLegacyFallback", "旧证据回退"),
      tone: "warn",
      detail: countText
        ? t(
            "workspace.domainQuality.scopeLegacyFallbackDetail",
            "未发现 artifact 线索，使用 {{scopeText}}全量证据",
            { scopeText: countText },
          )
        : t(
            "workspace.domainQuality.scopeLegacyFallbackShort",
            "未发现 artifact 线索，使用全量证据",
          ),
    }
  }
  if (mode === "all") {
    return {
      label: t("workspace.domainQuality.scopeAll", "全量证据"),
      tone: "muted",
      detail:
        total !== null
          ? t("workspace.domainQuality.scopeAllDetail", "使用 {{count}} 条领域证据", {
              count: total,
            })
          : null,
    }
  }
  return {
    label: mode.replace(/_/g, " "),
    tone: "muted",
    detail: countText,
  }
}

function domainQualityReviewEvidenceTarget(
  run: DomainQualityRunSnapshot["run"],
): DomainQualityReviewEvidenceTarget | null {
  const stats = asRecord(run.stats)
  const artifact = asRecord(stats?.artifact)
  const source = asRecord(stats?.source)
  const evidenceScope = asRecord(stats?.evidenceScope)
  const target = asRecord(evidenceScope?.target)
  const title =
    stringField(artifact, "title") ??
    stringField(source, "artifactTitle") ??
    stringField(target, "title")
  const path = stringField(source, "artifactPath") ?? stringField(target, "path")
  const kind =
    stringField(artifact, "kind") ??
    stringField(source, "artifactKind") ??
    stringField(target, "kind")
  const id = stringField(source, "artifactId") ?? stringField(target, "id")
  const label = title ?? path ?? kind ?? id
  if (!label) return null
  return { label, title, kind, path, id, evidenceScope }
}

function domainQualityReviewEvidenceInput(
  run: DomainQualityRunSnapshot["run"],
  sessionId: string,
  target: DomainQualityReviewEvidenceTarget,
): RecordDomainEvidenceInput {
  return {
    sessionId,
    domain: run.domain,
    evidenceType: "artifact_reviewed",
    title: `复核通过：${target.label}`,
    summary: run.summary || null,
    sourceMetadata: {
      sourceType: "domain_quality",
      domainQualityRunId: run.id,
      qualityState: run.state,
      templateId: run.templateId ?? null,
      templateVersion: run.templateVersion ?? null,
      artifactId: target.id ?? null,
      artifactTitle: target.title ?? null,
      artifactKind: target.kind ?? null,
      artifactPath: target.path ?? null,
      evidenceScope: target.evidenceScope ?? null,
      reviewCompleted: true,
      reviewedAt: run.completedAt ?? run.updatedAt,
    },
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
}

function domainArtifactExportGuardArtifactLabel(
  t: ReturnType<typeof useTranslation>["t"],
  report: DomainArtifactExportGuardReport,
): string {
  return (
    report.artifactTitle ??
    report.artifactPath ??
    report.artifactKind ??
    report.scope.domain ??
    t("workspace.domainExportGuard.artifact", "产物")
  )
}

function domainArtifactExportReviewMarkerLabel(
  t: ReturnType<typeof useTranslation>["t"],
  marker: DomainArtifactExportReviewMarker,
): string {
  switch (marker) {
    case "exportReview":
      return t("workspace.domainExportGuard.recordExportReview", "导出复核")
    case "exportReady":
      return t("workspace.domainExportGuard.recordExportReady", "可交付确认")
    case "redactionChecked":
      return t("workspace.domainExportGuard.recordRedactionChecked", "脱敏复核")
  }
}

function domainArtifactExportReviewMarkerTitle(
  t: ReturnType<typeof useTranslation>["t"],
  marker: DomainArtifactExportReviewMarker,
  artifactLabel: string,
): string {
  switch (marker) {
    case "exportReview":
      return t("workspace.domainExportGuard.exportReviewEvidenceTitle", "导出复核：{{artifact}}", {
        artifact: artifactLabel,
      })
    case "exportReady":
      return t("workspace.domainExportGuard.exportReadyEvidenceTitle", "可交付确认：{{artifact}}", {
        artifact: artifactLabel,
      })
    case "redactionChecked":
      return t(
        "workspace.domainExportGuard.redactionCheckedEvidenceTitle",
        "脱敏复核：{{artifact}}",
        { artifact: artifactLabel },
      )
  }
}

function domainArtifactExportReviewMarkerSummary(
  t: ReturnType<typeof useTranslation>["t"],
  marker: DomainArtifactExportReviewMarker,
): string {
  switch (marker) {
    case "exportReview":
      return t(
        "workspace.domainExportGuard.exportReviewEvidenceSummary",
        "用户确认已完成最终交付复核。",
      )
    case "exportReady":
      return t(
        "workspace.domainExportGuard.exportReadyEvidenceSummary",
        "用户确认该产物可以进入发送、分享或导出前的最终确认。",
      )
    case "redactionChecked":
      return t(
        "workspace.domainExportGuard.redactionCheckedEvidenceSummary",
        "用户确认已检查脱敏状态；原始 pending/sensitive 证据仍需由守门重新计算。",
      )
  }
}

function domainArtifactExportReviewEvidenceInput(
  report: DomainArtifactExportGuardReport,
  sessionId: string,
  marker: DomainArtifactExportReviewMarker,
  t: ReturnType<typeof useTranslation>["t"],
): RecordDomainEvidenceInput {
  const artifactLabel = domainArtifactExportGuardArtifactLabel(t, report)
  const sourceMetadata: Record<string, unknown> = {
    sourceType: "artifact_export_guard_confirmation",
    marker,
    [marker]: true,
    guardStatus: report.status,
    guardGeneratedAt: report.generatedAt,
    artifactPath: report.artifactPath ?? null,
    artifactTitle: report.artifactTitle ?? null,
    artifactKind: report.artifactKind ?? null,
    reviewedEvidenceIds: report.evidenceRequiringReview.map((item) => item.id),
    reviewReasons: report.evidenceRequiringReview.map((item) => item.reason),
    blockers: report.blockers,
  }
  if (marker === "exportReview") {
    sourceMetadata.export = { reviewed: true }
  } else if (marker === "exportReady") {
    sourceMetadata.export = { ready: true }
  } else {
    sourceMetadata.review = { redactionChecked: true }
  }

  return {
    goalId: report.scope.goalId ?? null,
    sessionId,
    projectId: report.scope.projectId ?? null,
    domain: report.scope.domain ?? "general",
    evidenceType: "artifact_reviewed",
    title: domainArtifactExportReviewMarkerTitle(t, marker, artifactLabel),
    summary: domainArtifactExportReviewMarkerSummary(t, marker),
    sourceMetadata,
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
}

function domainConnectorActionLabel(
  t: ReturnType<typeof useTranslation>["t"],
  report: DomainConnectorActionGuardReport,
): string {
  return (
    [report.connector, report.action].filter(Boolean).join(":") ||
    report.toolName ||
    report.scope.domain ||
    t("workspace.domainConnectorGuard.actionFallbackLabel", "外部动作")
  )
}

function domainConnectorActionConfirmationLabel(
  t: ReturnType<typeof useTranslation>["t"],
  marker: DomainConnectorActionConfirmationMarker,
): string {
  switch (marker) {
    case "explicitUserApproval":
      return t("workspace.domainConnectorGuard.recordApproval", "批准动作")
    case "rollbackPlan":
      return t("workspace.domainConnectorGuard.recordRollback", "记录回滚")
  }
}

function domainConnectorActionBaseMetadata(
  report: DomainConnectorActionGuardReport,
  marker: DomainConnectorActionConfirmationMarker,
): Record<string, unknown> {
  return {
    sourceType: "connector_action_guard_confirmation",
    marker,
    guardStatus: report.status,
    guardGeneratedAt: report.generatedAt,
    toolName: report.toolName ?? null,
    connector: report.connector ?? null,
    action: report.action ?? null,
    risk: report.risk ?? null,
    relatedEvidenceIds: report.relatedEvidence.map((item) => item.id),
    blockers: report.blockers,
  }
}

function domainConnectorActionApprovalEvidenceInput(
  report: DomainConnectorActionGuardReport,
  sessionId: string,
  t: ReturnType<typeof useTranslation>["t"],
): RecordDomainEvidenceInput {
  const actionLabel = domainConnectorActionLabel(t, report)
  return {
    goalId: report.scope.goalId ?? null,
    sessionId,
    projectId: report.scope.projectId ?? null,
    domain: report.scope.domain ?? "general",
    evidenceType: "user_decision",
    title: t("workspace.domainConnectorGuard.approvalEvidenceTitle", "批准外部动作：{{action}}", {
      action: actionLabel,
    }),
    summary: t(
      "workspace.domainConnectorGuard.approvalEvidenceSummary",
      "用户确认该外部动作可以进入执行前审批流程；真正执行仍需工具审批。",
    ),
    sourceMetadata: {
      ...domainConnectorActionBaseMetadata(report, "explicitUserApproval"),
      explicitUserApproval: true,
      userApproved: true,
      approved: true,
      approval: { explicit: true, approved: true },
      decision: { approved: true, confirmed: true },
    },
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
}

function domainConnectorActionRollbackEvidenceInput(
  report: DomainConnectorActionGuardReport,
  sessionId: string,
  rollbackPlan: string,
  t: ReturnType<typeof useTranslation>["t"],
): RecordDomainEvidenceInput {
  const actionLabel = domainConnectorActionLabel(t, report)
  return {
    goalId: report.scope.goalId ?? null,
    sessionId,
    projectId: report.scope.projectId ?? null,
    domain: report.scope.domain ?? "general",
    evidenceType: "connector_context_collected",
    title: t("workspace.domainConnectorGuard.rollbackEvidenceTitle", "回滚方案：{{action}}", {
      action: actionLabel,
    }),
    summary: rollbackPlan,
    sourceMetadata: {
      ...domainConnectorActionBaseMetadata(report, "rollbackPlan"),
      rollbackPlan,
      canRollback: true,
      rollback: { available: true, plan: rollbackPlan },
    },
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
}

function domainConnectorE2EActionLabel(report: DomainConnectorE2EGateReport): string {
  return (
    [report.connector, report.action].filter(Boolean).join(":") ||
    report.toolName ||
    report.scope.domain ||
    "connector action"
  )
}

function domainConnectorE2ESampleLabel(
  t: ReturnType<typeof useTranslation>["t"],
  marker: DomainConnectorE2ESampleMarker,
): string {
  switch (marker) {
    case "action_execution":
      return t("workspace.domainConnectorE2E.recordExecution", "记录执行")
    case "post_action_verification":
      return t("workspace.domainConnectorE2E.recordVerification", "记录复核")
  }
}

function domainConnectorE2ENextSampleStep(
  summary: DomainConnectorE2EGateReport["summary"] | null | undefined,
  thresholds: DomainConnectorE2EGateReport["thresholds"] | null | undefined,
  recordedExecutionSample: boolean,
  recordedVerificationSample: boolean,
): DomainConnectorE2ENextSampleStep {
  if (!summary || !thresholds) return "execution"
  if (thresholds.requireExplicitApproval && summary.approvalEvidence === 0) return "approval"
  const hasExecution = summary.executionEvidence > 0 || recordedExecutionSample
  const hasVerification = summary.verificationEvidence > 0 || recordedVerificationSample
  if (thresholds.requireExecutionResult && !hasExecution) return "execution"
  if (thresholds.requirePostActionVerification && !hasVerification) return "verification"
  return "complete"
}

function domainConnectorE2ENextSampleTitle(
  t: ReturnType<typeof useTranslation>["t"],
  step: DomainConnectorE2ENextSampleStep,
): string {
  switch (step) {
    case "approval":
      return t("workspace.domainConnectorE2E.nextApproval", "先补批准证据")
    case "execution":
      return t("workspace.domainConnectorE2E.nextExecution", "下一步：记录执行结果")
    case "verification":
      return t("workspace.domainConnectorE2E.nextVerification", "下一步：记录执行后复核")
    case "complete":
      return t("workspace.domainConnectorE2E.nextComplete", "真实样本已闭环")
  }
}

function domainConnectorE2ENextSampleDetail(
  t: ReturnType<typeof useTranslation>["t"],
  step: DomainConnectorE2ENextSampleStep,
): string {
  switch (step) {
    case "approval":
      return t(
        "workspace.domainConnectorE2E.nextApprovalDetail",
        "外部动作需要先有用户批准；批准后再记录真实执行结果。",
      )
    case "execution":
      return t(
        "workspace.domainConnectorE2E.nextExecutionDetail",
        "只记录已经发生的真实外部动作结果，例如 provider result id、发送/修改结果或失败原因。",
      )
    case "verification":
      return t(
        "workspace.domainConnectorE2E.nextVerificationDetail",
        "执行后读回外部状态并记录复核结论，确认结果真的可见且符合预期。",
      )
    case "complete":
      return t(
        "workspace.domainConnectorE2E.nextCompleteDetail",
        "执行与复核 evidence 都已存在；如仍未通过，请处理剩余回滚、交付或守门缺口。",
      )
  }
}

function domainConnectorE2EBaseMetadata(
  report: DomainConnectorE2EGateReport,
  marker: DomainConnectorE2ESampleMarker,
): Record<string, unknown> {
  return {
    sourceType: "connector_e2e_gate_sample",
    marker,
    gateStatus: report.status,
    gateGeneratedAt: report.generatedAt,
    toolName: report.toolName ?? null,
    connector: report.connector ?? null,
    action: report.action ?? null,
    risk: report.risk ?? null,
    relatedEvidenceIds: report.relatedEvidence.map((item) => item.id),
    blockers: report.blockers,
  }
}

function domainConnectorE2EExecutionEvidenceInput(
  report: DomainConnectorE2EGateReport,
  sessionId: string,
  result: string,
  t: ReturnType<typeof useTranslation>["t"],
): RecordDomainEvidenceInput {
  const actionLabel = domainConnectorE2EActionLabel(report)
  return {
    goalId: report.scope.goalId ?? null,
    sessionId,
    projectId: report.scope.projectId ?? null,
    domain: report.scope.domain ?? "general",
    evidenceType: "connector_action_executed",
    title: t("workspace.domainConnectorE2E.executionEvidenceTitle", "执行结果：{{action}}", {
      action: actionLabel,
    }),
    summary: result,
    sourceMetadata: {
      ...domainConnectorE2EBaseMetadata(report, "action_execution"),
      actionExecuted: true,
      executed: true,
      execution: { status: "recorded", summary: result },
      result: { status: "recorded", summary: result },
    },
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
}

function domainConnectorE2EVerificationEvidenceInput(
  report: DomainConnectorE2EGateReport,
  sessionId: string,
  verification: string,
  t: ReturnType<typeof useTranslation>["t"],
): RecordDomainEvidenceInput {
  const actionLabel = domainConnectorE2EActionLabel(report)
  return {
    goalId: report.scope.goalId ?? null,
    sessionId,
    projectId: report.scope.projectId ?? null,
    domain: report.scope.domain ?? "general",
    evidenceType: "connector_action_verified",
    title: t("workspace.domainConnectorE2E.verificationEvidenceTitle", "执行后复核：{{action}}", {
      action: actionLabel,
    }),
    summary: verification,
    sourceMetadata: {
      ...domainConnectorE2EBaseMetadata(report, "post_action_verification"),
      postActionVerification: true,
      externalStateVerified: true,
      deliveryVerified: true,
      verification: { passed: true, verified: true, summary: verification },
    },
    confidence: 1,
    accessScope: "session",
    redactionStatus: "none",
  }
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
  const [recordingReviewEvidence, setRecordingReviewEvidence] = useState(false)
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
  const evidenceScopeView = latest ? domainQualityEvidenceScopeView(t, latest) : null
  const reviewEvidenceTarget = latest ? domainQualityReviewEvidenceTarget(latest) : null
  const learning = learningRunId !== null && latest !== undefined && learningRunId === latest.id
  const canGenerateLearning =
    !!sessionId &&
    !incognito &&
    !!latest &&
    !active &&
    latest.state !== "cancelled" &&
    !loading &&
    !learning
  const canRecordReviewEvidence =
    !!sessionId &&
    !incognito &&
    !!latest &&
    latest.state === "completed" &&
    !!reviewEvidenceTarget &&
    !active &&
    !loading
  const recordReviewEvidenceDisabled = !canRecordReviewEvidence || recordingReviewEvidence
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

  const handleArtifactReview = async (target: DomainArtifactReviewTarget) => {
    const next = await runDomainQuality({
      domain: target.domain || undefined,
      artifactTitle: target.title || undefined,
      artifactKind: target.kind || undefined,
      sourceMetadata: {
        sourceType: "artifact_export_guard",
        artifactPath: target.path ?? null,
        artifactTitle: target.title ?? null,
        artifactKind: target.kind ?? null,
        artifactGuardStatus: target.guardStatus ?? null,
      },
    })
    if (next) {
      void refreshExportGuard()
      void refreshConnectorGuard()
    }
    return next
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

  const handleRecordReviewEvidence = async () => {
    if (
      !sessionId ||
      !latest ||
      !reviewEvidenceTarget ||
      !canRecordReviewEvidence ||
      recordingReviewEvidence
    ) {
      return
    }
    setRecordingReviewEvidence(true)
    try {
      const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
        input: domainQualityReviewEvidenceInput(latest, sessionId, reviewEvidenceTarget),
      })
      toast.success(
        t("workspace.domainQuality.reviewEvidenceRecorded", "已记录复核证据：{{title}}", {
          title: item.title,
        }),
      )
      void refreshExportGuard()
      void refreshConnectorGuard()
    } catch (e) {
      logger.error(
        "ui",
        "DomainQualitySection::recordReviewEvidence",
        "Failed to record domain quality review evidence",
        e,
      )
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setRecordingReviewEvidence(false)
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
              sessionId={sessionId}
              report={connectorGuard}
              loading={connectorGuardLoading}
              error={connectorGuardError}
              disabled={!sessionId || connectorGuardLoading}
              onRefresh={refreshConnectorGuard}
            />
            <DomainArtifactExportGuardPanel
              sessionId={sessionId}
              report={exportGuard}
              loading={exportGuardLoading}
              error={exportGuardError}
              disabled={!sessionId || exportGuardLoading}
              onRefresh={refreshExportGuard}
              onReviewArtifact={handleArtifactReview}
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
              {canRecordReviewEvidence ? (
                <button
                  type="button"
                  onClick={() => void handleRecordReviewEvidence()}
                  disabled={recordReviewEvidenceDisabled}
                  className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {recordingReviewEvidence ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <Check className="h-3 w-3" />
                  )}
                  <span>{t("workspace.domainQuality.recordReviewEvidence", "记录复核证据")}</span>
                </button>
              ) : null}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap gap-1 pl-5">
              <StatusPill label={domainLabel(t, latest.domain)} tone="info" />
              {domainQualityTemplateLabel(latest) ? (
                <StatusPill label={domainQualityTemplateLabel(latest)!} tone="muted" />
              ) : null}
              {evidenceScopeView ? (
                <StatusPill label={evidenceScopeView.label} tone={evidenceScopeView.tone} />
              ) : null}
            </div>
            {evidenceScopeView?.detail ? (
              <div className="mt-1 flex min-w-0 items-center gap-1.5 pl-5 text-[10px] text-muted-foreground/70">
                <Database className="h-3 w-3 shrink-0" />
                <span className="truncate">{evidenceScopeView.detail}</span>
              </div>
            ) : null}
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
  sessionId,
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  sessionId?: string | null
  report: DomainOperationalGateReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainOperationalGateReport | null>
}) {
  const { t } = useTranslation()
  const [creatingCheckTaskKey, setCreatingCheckTaskKey] = useState<string | null>(null)
  const [creatingRecommendationTaskKey, setCreatingRecommendationTaskKey] = useState<string | null>(
    null,
  )
  const recommendedSteps = (report?.recommendedNextSteps ?? []).filter(Boolean).slice(0, 2)
  const summary = report?.summary
  const hasSamples = domainOperationalGateHasSamples(report)
  const issueChecks = hasSamples
    ? (report?.checks ?? []).filter(
        (check) => check.status !== "passed" && check.severity !== "advisory",
      )
    : []
  const clean = hasSamples && report?.status === "passed"
  const maxActiveWorkAge =
    summary?.maxActiveWorkAgeSecs != null
      ? formatLoopDuration(Math.max(1, Math.round(summary.maxActiveWorkAgeSecs)))
      : "-"
  const canCreateCheckTasks = Boolean(sessionId) && !disabled
  const canCreateRecommendationTasks = Boolean(sessionId) && !disabled

  const createCheckTask = async (
    check: DomainOperationalGateReport["checks"][number],
    index: number,
  ) => {
    if (!sessionId || disabled || creatingCheckTaskKey) return
    const taskKey = `${index}:${check.name}`
    const checkLabel = domainGuardCheckNameLabel(t, check.name)
    setCreatingCheckTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainOperationalGate.checkTaskContent",
          "处理运行稳定性缺口：{{name}}（{{actual}}）- {{detail}}",
          {
            name: checkLabel,
            actual: check.actual,
            detail: check.detail || check.expected,
          },
        ),
        activeForm: t(
          "workspace.domainOperationalGate.checkTaskActiveForm",
          "正在处理运行稳定性缺口：{{name}}",
          { name: checkLabel },
        ),
      })
      toast.success(t("workspace.domainOperationalGate.checkTaskCreated", "已创建运行稳定性任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainOperationalGatePanel", "Create operational check task failed", e)
      toast.error(message)
    } finally {
      setCreatingCheckTaskKey(null)
    }
  }

  const createRecommendationTask = async (step: string, index: number) => {
    if (!sessionId || disabled || creatingRecommendationTaskKey) return
    const taskKey = `${index}:${step}`
    setCreatingRecommendationTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainOperationalGate.recommendationTaskContent",
          "处理运行稳定性建议：{{step}}",
          { step },
        ),
        activeForm: t(
          "workspace.domainOperationalGate.recommendationTaskActiveForm",
          "正在处理运行稳定性建议",
        ),
      })
      toast.success(
        t("workspace.domainOperationalGate.recommendationTaskCreated", "已创建运行稳定性建议任务"),
      )
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "DomainOperationalGatePanel",
        "Create operational recommendation task failed",
        e,
      )
      toast.error(message)
    } finally {
      setCreatingRecommendationTaskKey(null)
    }
  }

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
              : t(
                  "workspace.domainOperationalGate.emptyHint",
                  "检查工作流、Loop 和评测活动是否已排空",
                )}
          </div>
        </div>
        <StatusPill
          label={domainOperationalGateLabel(t, report?.status, loading, hasSamples)}
          tone={domainOperationalGateTone(report?.status, loading, hasSamples)}
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
        <div className="mt-2 grid grid-cols-2 gap-1.5 sm:grid-cols-5">
          {[
            [
              t("workspace.domainOperationalGate.workflows", "工作流"),
              `${summary.completedWorkflowRuns}/${summary.workflowRuns}`,
              summary.workflowRuns > 0 ? "good" : hasSamples ? "warn" : "muted",
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
            [
              t("workspace.domainOperationalGate.maxActiveAge", "最长"),
              maxActiveWorkAge,
              summary.maxActiveWorkAgeSecs != null ? "warn" : "muted",
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

      {hasSamples && report && !clean && recommendedSteps.length > 0 ? (
        <div className="mt-2 space-y-1">
          <div className="flex min-w-0 items-center gap-1.5 px-1 text-[10px] font-medium text-muted-foreground">
            <Lightbulb className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {t("workspace.domainOperationalGate.recommendations", "稳定性建议")}
            </span>
          </div>
          {recommendedSteps.map((step, index) => {
            const taskKey = `${index}:${step}`
            return (
              <div
                key={taskKey}
                className="rounded-md border border-sky-500/20 bg-sky-500/10 px-2 py-1.5 text-[11px] text-sky-700 dark:text-sky-300"
              >
                <div className="flex min-w-0 items-start gap-1.5">
                  <span className="min-w-0 flex-1 line-clamp-2">{step}</span>
                  {canCreateRecommendationTasks ? (
                    <button
                      type="button"
                      onClick={() => void createRecommendationTask(step, index)}
                      disabled={Boolean(creatingRecommendationTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/75 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingRecommendationTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>
                        {t("workspace.domainOperationalGate.createRecommendationTask", "转任务")}
                      </span>
                    </button>
                  ) : null}
                </div>
              </div>
            )
          })}
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
          {issueChecks.slice(0, 3).map((check, index) => {
            const taskKey = `${index}:${check.name}`
            return (
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
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {domainGuardCheckNameLabel(t, check.name)}
                  </span>
                  <span className="shrink-0 tabular-nums">{check.actual}</span>
                  {canCreateCheckTasks ? (
                    <button
                      type="button"
                      onClick={() => void createCheckTask(check, index)}
                      disabled={Boolean(creatingCheckTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/75 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingCheckTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>{t("workspace.domainOperationalGate.createCheckTask", "转任务")}</span>
                    </button>
                  ) : null}
                </div>
                <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
              </div>
            )
          })}
        </div>
      ) : !loading ? (
        <EmptyHint>
          {report && !hasSamples
            ? t("workspace.domainOperationalGate.noSamples", "还没有运行样本")
            : t("workspace.domainOperationalGate.empty", "还没有运行稳定性结果")}
        </EmptyHint>
      ) : null}
    </div>
  )
}

function DomainSoakReportPanel({
  sessionId,
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  sessionId?: string | null
  report: DomainSoakReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainSoakReport | null>
}) {
  const { t } = useTranslation()
  const summary = report?.summary
  const hasSamples = domainSoakReportHasSamples(report)
  const clean = hasSamples && report?.status === "passed"
  const [creatingIncidentTaskKey, setCreatingIncidentTaskKey] = useState<string | null>(null)
  const [creatingRecommendationTaskKey, setCreatingRecommendationTaskKey] = useState<string | null>(
    null,
  )
  const maxDrain =
    summary?.maxWorkflowDrainSecs != null
      ? formatLoopDuration(Math.max(1, Math.round(summary.maxWorkflowDrainSecs)))
      : "-"
  const latestActivityAge =
    summary?.latestActivityAgeSecs != null
      ? formatLoopDuration(Math.max(1, Math.round(summary.latestActivityAgeSecs)))
      : "-"
  const latestActivityTone: StatusTone =
    summary?.latestActivityAgeSecs == null
      ? "muted"
      : summary.latestActivityAgeSecs > 24 * 60 * 60
        ? "warn"
        : "info"
  const sampleDayCoverage =
    summary != null ? `${summary.sampleDays}/${summary.requiredSampleDays}` : "-"
  const sampleDayTone: StatusTone =
    summary == null
      ? "muted"
      : !hasSamples
        ? "muted"
        : summary.sampleDays >= summary.requiredSampleDays
          ? "info"
          : "warn"
  const maxApprovalWait =
    summary?.maxOpenApprovalWaitSecs != null
      ? formatLoopDuration(Math.max(1, Math.round(summary.maxOpenApprovalWaitSecs)))
      : summary?.maxApprovalWaitSecs != null
        ? formatLoopDuration(Math.max(1, Math.round(summary.maxApprovalWaitSecs)))
        : "-"
  const approvalWaitTone: StatusTone =
    (summary?.openApprovalWaits ?? 0) > 0
      ? "warn"
      : (summary?.approvalRequestEvents ?? 0) > (summary?.approvalDecisionEvents ?? 0)
        ? "warn"
        : "muted"
  const maxOutputTokensSpent = summary?.maxWorkflowOutputTokensSpent
  const maxOutputTokenBudget = summary?.maxWorkflowOutputTokenBudget
  const outputTokenBudget =
    maxOutputTokensSpent != null
      ? maxOutputTokenBudget != null && maxOutputTokenBudget > 0
        ? `${compactCount(maxOutputTokensSpent)}/${compactCount(maxOutputTokenBudget)}`
        : compactCount(maxOutputTokensSpent)
      : "-"
  const outputTokenTone: StatusTone =
    (summary?.workflowBudgetExhaustedEvents ?? 0) > 0
      ? "danger"
      : (summary?.workflowBudgetUsageEvents ?? 0) > 0
        ? "info"
        : "muted"
  const interventionTone: StatusTone =
    (summary?.workflowControlInterventionEvents ?? 0) > 1
      ? "warn"
      : (summary?.workflowControlInterventionEvents ?? 0) > 0
        ? "info"
        : "muted"
  const connectorVerificationTone: StatusTone =
    (summary?.connectorExecutionEvidence ?? 0) > 0 &&
    (summary?.connectorVerificationEvidence ?? 0) === 0
      ? "warn"
      : (summary?.connectorVerificationEvidence ?? 0) > 0
        ? "info"
        : "muted"
  const canCreateIncidentTasks = Boolean(sessionId) && !disabled
  const recommendedSteps = hasSamples
    ? (report?.recommendedNextSteps ?? []).filter(Boolean).slice(0, 2)
    : []
  const canCreateRecommendationTasks = Boolean(sessionId) && !disabled
  const timelineItems = (report?.timeline ?? []).slice(0, 3)
  const canCopyReport = Boolean(report?.markdown)

  const createIncidentTask = async (incident: DomainSoakIncident, index: number) => {
    if (!sessionId || disabled || creatingIncidentTaskKey) return
    const taskKey = `${index}:${incident.source}:${incident.id}`
    setCreatingIncidentTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainSoakReport.incidentTaskContent",
          "处理长跑事故：{{title}}（{{source}}/{{status}}）- {{recommendation}}",
          {
            title: incident.title,
            source: incident.source,
            status: incident.status,
            recommendation: incident.recommendation,
          },
        ),
        activeForm: t(
          "workspace.domainSoakReport.incidentTaskActiveForm",
          "正在处理长跑事故：{{title}}",
          {
            title: incident.title,
          },
        ),
      })
      toast.success(t("workspace.domainSoakReport.incidentTaskCreated", "已创建事故处理任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainSoakReportPanel", "Create soak incident task failed", e)
      toast.error(message)
    } finally {
      setCreatingIncidentTaskKey(null)
    }
  }

  const createRecommendationTask = async (step: string, index: number) => {
    if (!sessionId || disabled || creatingRecommendationTaskKey) return
    const taskKey = `${index}:${step}`
    setCreatingRecommendationTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainSoakReport.recommendationTaskContent",
          "处理长跑审计建议：{{step}}",
          { step },
        ),
        activeForm: t(
          "workspace.domainSoakReport.recommendationTaskActiveForm",
          "正在处理长跑审计建议",
        ),
      })
      toast.success(t("workspace.domainSoakReport.recommendationTaskCreated", "已创建长跑建议任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainSoakReportPanel", "Create soak recommendation task failed", e)
      toast.error(message)
    } finally {
      setCreatingRecommendationTaskKey(null)
    }
  }

  const copyReportMarkdown = async () => {
    if (!report?.markdown) return
    try {
      await navigator.clipboard.writeText(report.markdown)
      toast.success(t("workspace.domainSoakReport.reportCopied", "已复制长跑报告"))
    } catch (e) {
      logger.error("ui", "DomainSoakReportPanel", "Copy soak report failed", e)
      toast.error(t("workspace.domainSoakReport.reportCopyFailed", "复制长跑报告失败"))
    }
  }

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
          label={domainSoakReportLabel(t, report?.status, loading, hasSamples)}
          tone={domainSoakReportTone(report?.status, loading, hasSamples)}
          loading={loading}
        />
        {canCopyReport ? (
          <IconTip label={t("workspace.domainSoakReport.copyReport", "复制报告")}>
            <button
              type="button"
              onClick={() => void copyReportMarkdown()}
              disabled={disabled}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-secondary/25 text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:opacity-55"
            >
              <Copy className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        ) : null}
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
        <div className="mt-2 grid grid-cols-3 gap-1.5">
          {[
            [
              t("workspace.domainSoakReport.records", "样本"),
              summary.totalRecords,
              summary.totalRecords > 0 ? "info" : "muted",
            ],
            [
              t("workspace.domainSoakReport.freshness", "新鲜"),
              latestActivityAge,
              latestActivityTone,
            ],
            [t("workspace.domainSoakReport.sampleDays", "跨天"), sampleDayCoverage, sampleDayTone],
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
            [
              t("workspace.domainSoakReport.approvalWait", "审批"),
              maxApprovalWait,
              approvalWaitTone,
            ],
            [
              t("workspace.domainSoakReport.recovery", "恢复"),
              summary.recoveryEvents,
              summary.recoveryEvents > 0 ? "info" : "muted",
            ],
            [
              t("workspace.domainSoakReport.interventions", "干预"),
              summary.workflowControlInterventionEvents,
              interventionTone,
            ],
            [
              t("workspace.domainSoakReport.connectorVerified", "复核"),
              summary.connectorVerificationEvidence,
              connectorVerificationTone,
            ],
            [
              t("workspace.domainSoakReport.outputTokens", "Token"),
              outputTokenBudget,
              outputTokenTone,
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

      {timelineItems.length > 0 ? (
        <div className="mt-2 rounded-md border border-border/45 bg-secondary/15 px-2 py-1.5">
          <div className="mb-1 flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
            <Clock className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {t("workspace.domainSoakReport.timeline", "最近时间线")}
            </span>
          </div>
          <div className="space-y-1">
            {timelineItems.map((item) => {
              const duration =
                item.durationSecs != null
                  ? formatLoopDuration(Math.max(1, Math.round(item.durationSecs)))
                  : null
              return (
                <div
                  key={`${item.source}:${item.id}:${item.at}`}
                  className="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground"
                >
                  <StatusPill
                    label={domainSoakSourceLabel(t, item.source)}
                    tone={domainSoakTimelineTone(item.status)}
                  />
                  <span className="min-w-0 flex-1 truncate text-foreground/80">{item.label}</span>
                  {duration ? (
                    <span className="shrink-0 tabular-nums text-muted-foreground/80">
                      {duration}
                    </span>
                  ) : null}
                  <span className="shrink-0 text-muted-foreground/65">
                    {formatMessageTime(item.at)}
                  </span>
                </div>
              )
            })}
          </div>
        </div>
      ) : null}

      {report && !clean && recommendedSteps.length > 0 ? (
        <div className="mt-2 space-y-1">
          {recommendedSteps.map((step, index) => {
            const taskKey = `${index}:${step}`
            return (
              <div
                key={taskKey}
                className="rounded-md border border-sky-500/20 bg-sky-500/10 px-2 py-1.5 text-[11px] text-sky-700 dark:text-sky-300"
              >
                <div className="flex min-w-0 items-start gap-1.5">
                  <Lightbulb className="mt-0.5 h-3 w-3 shrink-0" />
                  <span className="min-w-0 flex-1 line-clamp-2">{step}</span>
                  {canCreateRecommendationTasks ? (
                    <button
                      type="button"
                      onClick={() => void createRecommendationTask(step, index)}
                      disabled={Boolean(creatingRecommendationTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/75 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingRecommendationTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>
                        {t("workspace.domainSoakReport.createRecommendationTask", "转任务")}
                      </span>
                    </button>
                  ) : null}
                </div>
              </div>
            )
          })}
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
      ) : hasSamples && report?.incidents?.length ? (
        <div className="mt-2 space-y-1">
          {report.incidents.slice(0, 2).map((incident, index) => (
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
                  label={domainSoakSourceLabel(t, incident.source)}
                  tone={incident.severity === "critical" ? "danger" : "warn"}
                />
                {canCreateIncidentTasks ? (
                  <button
                    type="button"
                    onClick={() => void createIncidentTask(incident, index)}
                    disabled={Boolean(creatingIncidentTaskKey)}
                    className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/75 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {creatingIncidentTaskKey === `${index}:${incident.source}:${incident.id}` ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Plus className="h-3 w-3" />
                    )}
                    <span>{t("workspace.domainSoakReport.createIncidentTask", "转任务")}</span>
                  </button>
                ) : null}
              </div>
              <div className="mt-0.5 line-clamp-2 opacity-85">{incident.recommendation}</div>
            </div>
          ))}
        </div>
      ) : !loading ? (
        <EmptyHint>
          {report && !hasSamples
            ? t("workspace.domainSoakReport.noSamples", "还没有长跑样本")
            : t("workspace.domainSoakReport.empty", "还没有长跑审计结果")}
        </EmptyHint>
      ) : null}
    </div>
  )
}

function domainOperationalGateTone(
  status?: string | null,
  loading?: boolean,
  hasSamples = true,
): StatusTone {
  if (loading) return "info"
  if (status && !hasSamples) return "muted"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainOperationalGateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
  hasSamples = true,
): string {
  if (loading) return t("workspace.domainOperationalGate.loading", "评估中")
  if (status && !hasSamples) return t("workspace.domainOperationalGate.noSamples", "未采样")
  if (status === "passed") return t("workspace.domainOperationalGate.passed", "稳定")
  if (status === "failed") return t("workspace.domainOperationalGate.failed", "阻塞")
  if (status === "insufficient_data")
    return t("workspace.domainOperationalGate.insufficient", "待排空")
  return t("workspace.domainOperationalGate.idle", "未评估")
}

function domainSoakReportTone(
  status?: string | null,
  loading?: boolean,
  hasSamples = true,
): StatusTone {
  if (loading) return "info"
  if (status && !hasSamples) return "muted"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainSoakTimelineTone(status?: string | null): StatusTone {
  const normalized = (status ?? "").toLowerCase()
  if (
    normalized === "failed" ||
    normalized === "blocked" ||
    normalized === "cancelled" ||
    normalized === "interrupted"
  ) {
    return "danger"
  }
  if (
    normalized === "running" ||
    normalized === "queued" ||
    normalized === "awaiting_approval" ||
    normalized === "awaiting_user"
  ) {
    return "warn"
  }
  if (
    normalized === "completed" ||
    normalized === "passed" ||
    normalized === "succeeded" ||
    normalized === "success"
  ) {
    return "good"
  }
  return "muted"
}

function domainSoakSourceLabel(t: ReturnType<typeof useTranslation>["t"], source: string): string {
  switch (source.toLowerCase()) {
    case "workflow":
      return t("workspace.domainSoakReport.sourceWorkflow", "工作流")
    case "loop":
      return t("workspace.domainSoakReport.sourceLoop", "持续推进")
    case "campaign":
      return t("workspace.domainSoakReport.sourceCampaign", "评测活动")
    case "connector":
      return t("workspace.domainSoakReport.sourceConnector", "连接器")
    case "approval":
      return t("workspace.domainSoakReport.sourceApproval", "审批")
    default:
      return source
  }
}

function domainSoakReportLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
  hasSamples = true,
): string {
  if (loading) return t("workspace.domainSoakReport.loading", "评估中")
  if (status && !hasSamples) return t("workspace.domainSoakReport.noSamplesLabel", "未采样")
  if (status === "passed") return t("workspace.domainSoakReport.passed", "干净")
  if (status === "failed") return t("workspace.domainSoakReport.failed", "有事故")
  if (status === "insufficient_data")
    return t("workspace.domainSoakReport.insufficient", "样本不足")
  return t("workspace.domainSoakReport.idle", "未评估")
}

function DomainArtifactExportGuardPanel({
  sessionId,
  report,
  loading,
  error,
  disabled,
  onRefresh,
  onReviewArtifact,
}: {
  sessionId?: string | null
  report: DomainArtifactExportGuardReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainArtifactExportGuardReport | null>
  onReviewArtifact?: (
    target: DomainArtifactReviewTarget,
  ) => Promise<DomainQualityRunSnapshot | null>
}) {
  const { t } = useTranslation()
  const [creatingTaskKey, setCreatingTaskKey] = useState<string | null>(null)
  const [reviewingArtifact, setReviewingArtifact] = useState(false)
  const [recordingReviewMarker, setRecordingReviewMarker] =
    useState<DomainArtifactExportReviewMarker | null>(null)
  const summary = report?.summary
  const evidenceRequiringReview = report?.evidenceRequiringReview ?? []
  const hasScope = domainArtifactExportGuardHasScope(report)
  const issueChecks = hasScope
    ? (report?.checks ?? []).filter((check) => check.status !== "passed")
    : []
  const clean = hasScope && report?.status === "passed"
  const canCreateTasks = hasScope && Boolean(sessionId) && !disabled
  const artifactLabel =
    report?.artifactTitle || report?.artifactPath || report?.artifactKind || null
  const canReviewArtifact =
    Boolean(onReviewArtifact) && Boolean(artifactLabel) && Boolean(sessionId) && !disabled
  const canRecordReviewMarker = hasScope && Boolean(report) && Boolean(sessionId) && !disabled

  const reviewArtifact = async () => {
    if (!report || !onReviewArtifact || !canReviewArtifact || reviewingArtifact) return
    setReviewingArtifact(true)
    try {
      const next = await onReviewArtifact({
        title: report.artifactTitle,
        kind: report.artifactKind,
        path: report.artifactPath,
        domain: report.scope.domain,
        guardStatus: report.status,
      })
      if (!next) return
      if (next.run.state === "completed") {
        toast.success(t("workspace.domainExportGuard.artifactReviewClean", "产物复核通过"))
      } else if (next.run.state === "needs_user") {
        toast.warning(t("workspace.domainExportGuard.artifactReviewNeedsUser", "产物复核需要确认"))
      } else {
        toast.error(t("workspace.domainExportGuard.artifactReviewBlocked", "产物复核发现阻塞项"))
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainArtifactExportGuardPanel", "Run artifact review failed", e)
      toast.error(message)
    } finally {
      setReviewingArtifact(false)
    }
  }

  const recordReviewMarker = async (marker: DomainArtifactExportReviewMarker) => {
    if (!sessionId || !report || !canRecordReviewMarker || recordingReviewMarker) return
    setRecordingReviewMarker(marker)
    try {
      const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
        input: domainArtifactExportReviewEvidenceInput(report, sessionId, marker, t),
      })
      toast.success(
        t("workspace.domainExportGuard.reviewMarkerRecorded", "已记录交付确认：{{title}}", {
          title: item.title,
        }),
      )
      void onRefresh()
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainArtifactExportGuardPanel", "Record export review marker failed", e)
      toast.error(message)
    } finally {
      setRecordingReviewMarker(null)
    }
  }

  const createCheckTask = async (
    check: DomainArtifactExportGuardReport["checks"][number],
    index: number,
  ) => {
    if (!sessionId || disabled || creatingTaskKey) return
    const taskKey = `check:${index}:${check.name}`
    const checkLabel = domainGuardCheckNameLabel(t, check.name)
    setCreatingTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainExportGuard.checkTaskContent",
          "处理交付守门缺口：{{name}}（{{actual}}）- {{detail}}",
          {
            name: checkLabel,
            actual: check.actual,
            detail: check.detail || check.expected,
          },
        ),
        activeForm: t(
          "workspace.domainExportGuard.checkTaskActiveForm",
          "正在处理交付守门缺口：{{name}}",
          { name: checkLabel },
        ),
      })
      toast.success(t("workspace.domainExportGuard.checkTaskCreated", "已创建交付复核任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainArtifactExportGuardPanel", "Create export guard task failed", e)
      toast.error(message)
    } finally {
      setCreatingTaskKey(null)
    }
  }

  const createEvidenceReviewTask = async (
    item: DomainArtifactExportGuardReport["evidenceRequiringReview"][number],
    index: number,
  ) => {
    if (!sessionId || disabled || creatingTaskKey) return
    const taskKey = `evidence:${index}:${item.id}`
    const reasonLabel = domainGuardCheckNameLabel(t, item.reason)
    const scopeLabel = domainAccessScopeLabel(t, item.accessScope)
    const redactionLabel = domainRedactionStatusLabel(t, item.redactionStatus)
    setCreatingTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainExportGuard.evidenceTaskContent",
          "复核交付证据：{{title}}（{{reason}}）- {{scope}} / {{redaction}}",
          {
            title: item.title,
            reason: reasonLabel,
            scope: scopeLabel,
            redaction: redactionLabel,
          },
        ),
        activeForm: t(
          "workspace.domainExportGuard.evidenceTaskActiveForm",
          "正在复核交付证据：{{title}}",
          { title: item.title },
        ),
      })
      toast.success(t("workspace.domainExportGuard.evidenceTaskCreated", "已创建证据复核任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "DomainArtifactExportGuardPanel",
        "Create export evidence review task failed",
        e,
      )
      toast.error(message)
    } finally {
      setCreatingTaskKey(null)
    }
  }

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
          label={domainArtifactExportGuardLabel(t, report?.status, loading, hasScope)}
          tone={domainArtifactExportGuardTone(report?.status, loading, hasScope)}
          loading={loading}
        />
        {canReviewArtifact ? (
          <button
            type="button"
            onClick={() => void reviewArtifact()}
            disabled={reviewingArtifact}
            className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-border/60 bg-secondary/35 px-2 text-[11px] font-medium text-foreground transition-colors hover:bg-secondary/55 disabled:cursor-not-allowed disabled:opacity-55"
          >
            {reviewingArtifact ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ClipboardCheck className="h-3.5 w-3.5" />
            )}
            <span>{t("workspace.domainExportGuard.reviewArtifact", "复核产物")}</span>
          </button>
        ) : null}
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
            [
              t("workspace.domainExportGuard.artifact", "产物"),
              summary.artifactCreated,
              summary.artifactCreated > 0 ? "info" : "muted",
            ],
            [
              t("workspace.domainExportGuard.reviewed", "复核"),
              summary.artifactReviewed,
              summary.artifactReviewed > 0 ? "good" : hasScope ? "warn" : "muted",
            ],
            [
              t("workspace.domainExportGuard.sensitive", "敏感"),
              summary.sensitiveEvidence,
              summary.sensitiveEvidence > 0 ? "warn" : "muted",
            ],
            [
              t("workspace.domainExportGuard.redaction", "待脱敏"),
              summary.redactionPending,
              summary.redactionPending > 0 ? "danger" : "muted",
            ],
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

      {hasScope && report && !clean ? (
        <div className="mt-2 rounded-md border border-border/50 bg-secondary/20 px-2 py-1.5">
          <div className="mb-1 flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
            <CheckCircle2 className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {t("workspace.domainExportGuard.explicitConfirm", "显式确认")}
            </span>
          </div>
          <div className="grid grid-cols-3 gap-1">
            {(["exportReview", "exportReady", "redactionChecked"] as const).map((marker) => (
              <button
                key={marker}
                type="button"
                onClick={() => void recordReviewMarker(marker)}
                disabled={!canRecordReviewMarker || Boolean(recordingReviewMarker)}
                className="inline-flex min-w-0 items-center justify-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
              >
                {recordingReviewMarker === marker ? (
                  <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
                ) : (
                  <Check className="h-3 w-3 shrink-0" />
                )}
                <span className="truncate">{domainArtifactExportReviewMarkerLabel(t, marker)}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t(
            "workspace.domainExportGuard.clean",
            "最终交付证据通过，可以进入发送、分享或导出前的用户确认。",
          )}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 3).map((check, index) => {
            const taskKey = `check:${index}:${check.name}`
            return (
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
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {domainGuardCheckNameLabel(t, check.name)}
                  </span>
                  <span className="shrink-0 tabular-nums">{check.actual}</span>
                  {canCreateTasks ? (
                    <button
                      type="button"
                      onClick={() => void createCheckTask(check, index)}
                      disabled={Boolean(creatingTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/70 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>{t("workspace.domainExportGuard.createCheckTask", "转任务")}</span>
                    </button>
                  ) : null}
                </div>
                <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
              </div>
            )
          })}
        </div>
      ) : !loading ? (
        <EmptyHint>
          {report && !hasScope
            ? t("workspace.domainExportGuard.noScope", "还没有交付产物需要守门")
            : t("workspace.domainExportGuard.empty", "还没有交付守门结果")}
        </EmptyHint>
      ) : null}

      {evidenceRequiringReview.length ? (
        <div className="mt-2 space-y-1">
          {evidenceRequiringReview.slice(0, 2).map((item, index) => {
            const taskKey = `evidence:${index}:${item.id}`
            return (
              <div
                key={item.id}
                className="min-w-0 rounded-md border border-amber-500/20 bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300"
              >
                <div className="flex min-w-0 items-center gap-1.5">
                  <ShieldAlert className="h-3 w-3 shrink-0" />
                  <span className="min-w-0 flex-1 truncate font-medium">{item.title}</span>
                  <StatusPill label={domainGuardCheckNameLabel(t, item.reason)} tone="warn" />
                  {canCreateTasks ? (
                    <button
                      type="button"
                      onClick={() => void createEvidenceReviewTask(item, index)}
                      disabled={Boolean(creatingTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/70 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>{t("workspace.domainExportGuard.createEvidenceTask", "转任务")}</span>
                    </button>
                  ) : null}
                </div>
                <div className="mt-0.5 truncate font-mono text-[10px] opacity-75">
                  {domainAccessScopeLabel(t, item.accessScope)} ·{" "}
                  {domainRedactionStatusLabel(t, item.redactionStatus)}
                </div>
              </div>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}

function DomainConnectorActionGuardPanel({
  sessionId,
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  sessionId?: string | null
  report: DomainConnectorActionGuardReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainConnectorActionGuardReport | null>
}) {
  const { t } = useTranslation()
  const [creatingTaskKey, setCreatingTaskKey] = useState<string | null>(null)
  const [recordingConfirmation, setRecordingConfirmation] =
    useState<DomainConnectorActionConfirmationMarker | null>(null)
  const [rollbackPlanDraft, setRollbackPlanDraft] = useState("")
  const summary = report?.summary
  const relatedEvidence = report?.relatedEvidence ?? []
  const hasScope = domainConnectorActionGuardHasScope(report)
  const issueChecks = hasScope
    ? (report?.checks ?? []).filter((check) => check.status !== "passed")
    : []
  const clean = hasScope && report?.status === "passed"
  const canCreateTasks = hasScope && Boolean(sessionId) && !disabled
  const canRecordConfirmation = hasScope && Boolean(report) && Boolean(sessionId) && !disabled
  const rollbackPlan = rollbackPlanDraft.trim()

  const createCheckTask = async (
    check: DomainConnectorActionGuardReport["checks"][number],
    index: number,
  ) => {
    if (!sessionId || disabled || creatingTaskKey) return
    const taskKey = `check:${index}:${check.name}`
    const checkLabel = domainGuardCheckNameLabel(t, check.name)
    setCreatingTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainConnectorGuard.checkTaskContent",
          "处理外部动作守门缺口：{{name}}（{{actual}}）- {{detail}}",
          {
            name: checkLabel,
            actual: check.actual,
            detail: check.detail || check.expected,
          },
        ),
        activeForm: t(
          "workspace.domainConnectorGuard.checkTaskActiveForm",
          "正在处理外部动作守门缺口：{{name}}",
          { name: checkLabel },
        ),
      })
      toast.success(t("workspace.domainConnectorGuard.checkTaskCreated", "已创建外部动作复核任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainConnectorActionGuardPanel", "Create connector guard task failed", e)
      toast.error(message)
    } finally {
      setCreatingTaskKey(null)
    }
  }

  const recordConfirmation = async (marker: DomainConnectorActionConfirmationMarker) => {
    if (!sessionId || !report || !canRecordConfirmation || recordingConfirmation) return
    if (marker === "rollbackPlan" && !rollbackPlan) return
    setRecordingConfirmation(marker)
    try {
      const input =
        marker === "explicitUserApproval"
          ? domainConnectorActionApprovalEvidenceInput(report, sessionId, t)
          : domainConnectorActionRollbackEvidenceInput(report, sessionId, rollbackPlan, t)
      const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
        input,
      })
      if (marker === "rollbackPlan") {
        setRollbackPlanDraft("")
      }
      toast.success(
        t("workspace.domainConnectorGuard.confirmationRecorded", "已记录外部动作证据：{{title}}", {
          title: item.title,
        }),
      )
      void onRefresh()
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "ui",
        "DomainConnectorActionGuardPanel",
        "Record connector confirmation failed",
        e,
      )
      toast.error(message)
    } finally {
      setRecordingConfirmation(null)
    }
  }

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
          label={domainConnectorActionGuardLabel(t, report?.status, loading, hasScope)}
          tone={domainConnectorActionGuardTone(report?.status, loading, hasScope)}
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
            [
              t("workspace.domainConnectorGuard.action", "动作"),
              summary.actionEvidence,
              summary.actionEvidence > 0 ? "info" : "muted",
            ],
            [
              t("workspace.domainConnectorGuard.approval", "批准"),
              summary.approvalEvidence,
              summary.approvalEvidence > 0 ? "good" : hasScope ? "danger" : "muted",
            ],
            [
              t("workspace.domainConnectorGuard.rollback", "回滚"),
              summary.rollbackEvidence,
              summary.rollbackEvidence > 0 ? "good" : hasScope ? "warn" : "muted",
            ],
            [
              t("workspace.domainConnectorGuard.sensitive", "敏感"),
              summary.sensitiveEvidence,
              summary.sensitiveEvidence > 0 ? "warn" : "muted",
            ],
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

      {hasMeaningfulScopeValue(report?.connector) ||
      hasMeaningfulScopeValue(report?.action) ||
      hasMeaningfulScopeValue(report?.toolName) ? (
        <div className="mt-2 flex min-w-0 flex-wrap gap-1">
          {hasMeaningfulScopeValue(report?.connector) ? (
            <StatusPill label={report?.connector ?? ""} tone="info" />
          ) : null}
          {hasMeaningfulScopeValue(report?.action) ? (
            <StatusPill label={report?.action ?? ""} tone="muted" />
          ) : null}
          {hasMeaningfulScopeValue(report?.toolName) ? (
            <StatusPill label={report?.toolName ?? ""} tone="muted" />
          ) : null}
        </div>
      ) : null}

      {hasScope && report && !clean ? (
        <div className="mt-2 rounded-md border border-border/50 bg-secondary/20 px-2 py-1.5">
          <div className="mb-1 flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
            <CheckCircle2 className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {t("workspace.domainConnectorGuard.explicitConfirm", "显式确认")}
            </span>
          </div>
          <div className="grid grid-cols-2 gap-1">
            <button
              type="button"
              onClick={() => void recordConfirmation("explicitUserApproval")}
              disabled={!canRecordConfirmation || Boolean(recordingConfirmation)}
              className="inline-flex min-w-0 items-center justify-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            >
              {recordingConfirmation === "explicitUserApproval" ? (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
              ) : (
                <Check className="h-3 w-3 shrink-0" />
              )}
              <span className="truncate">
                {domainConnectorActionConfirmationLabel(t, "explicitUserApproval")}
              </span>
            </button>
            <button
              type="button"
              onClick={() => void recordConfirmation("rollbackPlan")}
              disabled={!canRecordConfirmation || !rollbackPlan || Boolean(recordingConfirmation)}
              className="inline-flex min-w-0 items-center justify-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            >
              {recordingConfirmation === "rollbackPlan" ? (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
              ) : (
                <Check className="h-3 w-3 shrink-0" />
              )}
              <span className="truncate">
                {domainConnectorActionConfirmationLabel(t, "rollbackPlan")}
              </span>
            </button>
          </div>
          <Textarea
            value={rollbackPlanDraft}
            onChange={(event) => setRollbackPlanDraft(event.target.value)}
            placeholder={t("workspace.domainConnectorGuard.rollbackPlaceholder", "回滚方案")}
            rows={2}
            className="mt-1.5 min-h-12 resize-none px-2 py-1 text-[11px]"
          />
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t(
            "workspace.domainConnectorGuard.clean",
            "外部动作证据通过，真正执行前仍会逐次弹出确认。",
          )}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 3).map((check, index) => {
            const taskKey = `check:${index}:${check.name}`
            return (
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
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {domainGuardCheckNameLabel(t, check.name)}
                  </span>
                  <span className="shrink-0 tabular-nums">{check.actual}</span>
                  {canCreateTasks ? (
                    <button
                      type="button"
                      onClick={() => void createCheckTask(check, index)}
                      disabled={Boolean(creatingTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/70 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>{t("workspace.domainConnectorGuard.createCheckTask", "转任务")}</span>
                    </button>
                  ) : null}
                </div>
                <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
              </div>
            )
          })}
        </div>
      ) : !loading ? (
        <EmptyHint>
          {report && !hasScope
            ? t("workspace.domainConnectorGuard.noScope", "还没有外部动作需要守门")
            : t("workspace.domainConnectorGuard.empty", "还没有外部动作守门结果")}
        </EmptyHint>
      ) : null}

      {relatedEvidence.length ? (
        <div className="mt-2 space-y-1">
          {relatedEvidence.slice(0, 2).map((item) => (
            <div
              key={item.id}
              className="min-w-0 rounded-md border border-border/40 bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <Shield className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium text-foreground/80">
                  {item.title}
                </span>
                <StatusPill label={domainGuardCheckNameLabel(t, item.reason)} tone="muted" />
              </div>
              <div className="mt-0.5 truncate font-mono text-[10px] opacity-75">
                {domainAccessScopeLabel(t, item.accessScope)} ·{" "}
                {domainRedactionStatusLabel(t, item.redactionStatus)}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function DomainConnectorE2EGatePanel({
  sessionId,
  report,
  loading,
  error,
  disabled,
  onRefresh,
}: {
  sessionId?: string | null
  report: DomainConnectorE2EGateReport | null
  loading: boolean
  error: string | null
  disabled: boolean
  onRefresh: () => Promise<DomainConnectorE2EGateReport | null>
}) {
  const { t } = useTranslation()
  const [recordingSample, setRecordingSample] = useState<DomainConnectorE2ESampleMarker | null>(
    null,
  )
  const [creatingTaskKey, setCreatingTaskKey] = useState<string | null>(null)
  const [executionResultDraft, setExecutionResultDraft] = useState("")
  const [verificationDraft, setVerificationDraft] = useState("")
  const [recordedExecutionSample, setRecordedExecutionSample] = useState(false)
  const [recordedVerificationSample, setRecordedVerificationSample] = useState(false)
  const summary = report?.summary
  const relatedEvidence = report?.relatedEvidence ?? []
  const hasScope = domainConnectorE2EGateHasScope(report)
  const issueChecks = hasScope
    ? (report?.checks ?? []).filter((check) => check.status !== "passed")
    : []
  const clean = hasScope && report?.status === "passed"
  const canRecordSample = hasScope && Boolean(report) && Boolean(sessionId) && !disabled
  const canCreateTasks = hasScope && Boolean(sessionId) && !disabled
  const executionResult = executionResultDraft.trim()
  const verification = verificationDraft.trim()
  const reportKey = report
    ? [report.generatedAt, report.connector ?? "", report.action ?? "", report.toolName ?? ""].join(
        ":",
      )
    : "none"
  useEffect(() => {
    setRecordedExecutionSample(false)
    setRecordedVerificationSample(false)
  }, [reportKey])
  const nextSampleStep = domainConnectorE2ENextSampleStep(
    summary,
    report?.thresholds,
    recordedExecutionSample,
    recordedVerificationSample,
  )
  const executionReady = nextSampleStep === "execution"
  const verificationReady =
    nextSampleStep === "verification" ||
    (summary?.executionEvidence ?? 0) > 0 ||
    recordedExecutionSample
  const metrics = summary
    ? [
        {
          label: t("workspace.domainConnectorE2E.input", "输入"),
          value: summary.connectorInputEvidence,
          tone: summary.connectorInputEvidence > 0 ? "good" : hasScope ? "warn" : "muted",
        },
        {
          label: t("workspace.domainConnectorE2E.draft", "草稿"),
          value: summary.draftEvidence,
          tone: summary.draftEvidence > 0 ? "good" : "muted",
        },
        {
          label: t("workspace.domainConnectorE2E.approval", "批准"),
          value: summary.approvalEvidence,
          tone: summary.approvalEvidence > 0 ? "good" : hasScope ? "danger" : "muted",
        },
        {
          label: t("workspace.domainConnectorE2E.execution", "执行"),
          value: summary.executionEvidence,
          tone: summary.executionEvidence > 0 ? "good" : hasScope ? "warn" : "muted",
        },
        {
          label: t("workspace.domainConnectorE2E.verification", "复核"),
          value: summary.verificationEvidence,
          tone: summary.verificationEvidence > 0 ? "good" : hasScope ? "warn" : "muted",
        },
        {
          label: t("workspace.domainConnectorE2E.rollback", "回滚"),
          value: summary.rollbackEvidence,
          tone: summary.rollbackEvidence > 0 ? "good" : hasScope ? "warn" : "muted",
        },
      ]
    : []

  const recordSample = async (marker: DomainConnectorE2ESampleMarker) => {
    if (!sessionId || !report || !canRecordSample || recordingSample) return
    if (marker === "action_execution" && !executionResult) return
    if (marker === "post_action_verification" && !verification) return
    setRecordingSample(marker)
    try {
      const input =
        marker === "action_execution"
          ? domainConnectorE2EExecutionEvidenceInput(report, sessionId, executionResult, t)
          : domainConnectorE2EVerificationEvidenceInput(report, sessionId, verification, t)
      const item = await getTransport().call<DomainEvidenceItem>("record_domain_evidence", {
        input,
      })
      if (marker === "action_execution") {
        setExecutionResultDraft("")
        setRecordedExecutionSample(true)
      } else {
        setVerificationDraft("")
        setRecordedVerificationSample(true)
      }
      toast.success(
        t("workspace.domainConnectorE2E.sampleRecorded", "已记录端到端证据：{{title}}", {
          title: item.title,
        }),
      )
      void onRefresh()
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainConnectorE2EGatePanel", "Record connector E2E sample failed", e)
      toast.error(message)
    } finally {
      setRecordingSample(null)
    }
  }

  const createCheckTask = async (
    check: DomainConnectorE2EGateReport["checks"][number],
    index: number,
  ) => {
    if (!sessionId || disabled || creatingTaskKey) return
    const taskKey = `check:${index}:${check.name}`
    const checkLabel = domainGuardCheckNameLabel(t, check.name)
    setCreatingTaskKey(taskKey)
    try {
      await getTransport().call<Task[]>("create_session_task", {
        sessionId,
        content: t(
          "workspace.domainConnectorE2E.checkTaskContent",
          "处理连接器端到端（E2E）缺口：{{name}}（{{actual}}）- {{detail}}",
          {
            name: checkLabel,
            actual: check.actual,
            detail: check.detail || check.expected,
          },
        ),
        activeForm: t(
          "workspace.domainConnectorE2E.checkTaskActiveForm",
          "正在处理连接器端到端（E2E）缺口：{{name}}",
          { name: checkLabel },
        ),
      })
      toast.success(t("workspace.domainConnectorE2E.checkTaskCreated", "已创建连接器端到端任务"))
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error("ui", "DomainConnectorE2EGatePanel", "Create connector E2E task failed", e)
      toast.error(message)
    } finally {
      setCreatingTaskKey(null)
    }
  }

  return (
    <div className="rounded-md border border-border/55 bg-background/45 px-2.5 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <GitCompare className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-foreground/90">
            <span className="truncate">
              {t("workspace.domainConnectorE2E.title", "连接器端到端")}
            </span>
            <E2EBadge />
          </div>
          <div className="truncate text-[10px] text-muted-foreground">
            {report
              ? t("workspace.domainConnectorE2E.generated", "最近评估 {{time}}", {
                  time: formatMessageTime(report.generatedAt),
                })
              : t(
                  "workspace.domainConnectorE2E.emptyHint",
                  "检查输入、草稿、批准、执行、复核和回滚证据",
                )}
          </div>
        </div>
        <StatusPill
          label={domainConnectorE2EGateLabel(t, report?.status, loading, hasScope)}
          tone={domainConnectorE2EGateTone(report?.status, loading, hasScope)}
          loading={loading}
        />
        <IconTip label={t("workspace.domainConnectorE2E.refresh", "刷新连接器端到端（E2E）")}>
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
        <>
          <div className="mt-2 grid grid-cols-3 gap-1.5">
            {metrics.map((metric) => (
              <div
                key={metric.label}
                className={cn(
                  "rounded-md border px-2 py-1.5",
                  STATUS_TONE_CLASS[metric.tone as StatusTone],
                )}
              >
                <div className="truncate text-[10px]">{metric.label}</div>
                <div className="text-xs font-semibold tabular-nums">{metric.value}</div>
              </div>
            ))}
          </div>
          <div className="mt-2 flex min-w-0 flex-wrap gap-1">
            {hasMeaningfulScopeValue(report?.connector) ? (
              <StatusPill label={report?.connector ?? ""} tone="info" />
            ) : null}
            {hasMeaningfulScopeValue(report?.action) ? (
              <StatusPill label={report?.action ?? ""} tone="muted" />
            ) : null}
            {hasMeaningfulScopeValue(report?.toolName) ? (
              <StatusPill label={report?.toolName ?? ""} tone="muted" />
            ) : null}
            {summary.connectorActionGuardStatus ? (
              <StatusPill
                label={t("workspace.domainConnectorE2E.actionGuard", "动作 {{status}}", {
                  status: summary.connectorActionGuardStatus,
                })}
                tone={domainConnectorE2EGateTone(summary.connectorActionGuardStatus)}
              />
            ) : null}
            {summary.exportGuardStatus ? (
              <StatusPill
                label={t("workspace.domainConnectorE2E.exportGuard", "交付 {{status}}", {
                  status: summary.exportGuardStatus,
                })}
                tone={domainConnectorE2EGateTone(summary.exportGuardStatus)}
              />
            ) : null}
          </div>
        </>
      ) : null}

      {hasScope && report && !clean ? (
        <div className="mt-2 rounded-md border border-border/50 bg-secondary/20 px-2 py-1.5">
          <div className="mb-1 flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
            <CheckCircle2 className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {t("workspace.domainConnectorE2E.realSample", "真实样本")}
            </span>
          </div>
          <div
            className={cn(
              "mb-1.5 rounded-md border px-2 py-1.5 text-[11px] leading-snug",
              STATUS_TONE_CLASS[
                nextSampleStep === "approval"
                  ? "danger"
                  : nextSampleStep === "complete"
                    ? "good"
                    : "info"
              ],
            )}
          >
            <div className="font-medium">
              {domainConnectorE2ENextSampleTitle(t, nextSampleStep)}
            </div>
            <div className="mt-0.5 opacity-85">
              {domainConnectorE2ENextSampleDetail(t, nextSampleStep)}
            </div>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            <div className="space-y-1">
              <Textarea
                value={executionResultDraft}
                onChange={(event) => setExecutionResultDraft(event.target.value)}
                placeholder={t("workspace.domainConnectorE2E.executionPlaceholder", "执行结果")}
                rows={2}
                className="min-h-12 resize-none px-2 py-1 text-[11px]"
              />
              <button
                type="button"
                onClick={() => void recordSample("action_execution")}
                disabled={
                  !canRecordSample ||
                  !executionReady ||
                  !executionResult ||
                  Boolean(recordingSample)
                }
                className="inline-flex w-full min-w-0 items-center justify-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
              >
                {recordingSample === "action_execution" ? (
                  <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
                ) : (
                  <Check className="h-3 w-3 shrink-0" />
                )}
                <span className="truncate">
                  {domainConnectorE2ESampleLabel(t, "action_execution")}
                </span>
              </button>
            </div>
            <div className="space-y-1">
              <Textarea
                value={verificationDraft}
                onChange={(event) => setVerificationDraft(event.target.value)}
                placeholder={t(
                  "workspace.domainConnectorE2E.verificationPlaceholder",
                  "执行后复核",
                )}
                rows={2}
                className="min-h-12 resize-none px-2 py-1 text-[11px]"
              />
              <button
                type="button"
                onClick={() => void recordSample("post_action_verification")}
                disabled={
                  !canRecordSample ||
                  !verificationReady ||
                  !verification ||
                  Boolean(recordingSample)
                }
                className="inline-flex w-full min-w-0 items-center justify-center gap-1 rounded-md border border-border/55 bg-background/45 px-1.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
              >
                {recordingSample === "post_action_verification" ? (
                  <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
                ) : (
                  <Check className="h-3 w-3 shrink-0" />
                )}
                <span className="truncate">
                  {domainConnectorE2ESampleLabel(t, "post_action_verification")}
                </span>
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      ) : clean ? (
        <div className="mt-2 rounded-md bg-emerald-500/10 px-2 py-1.5 text-[11px] text-emerald-700 dark:text-emerald-300">
          {t(
            "workspace.domainConnectorE2E.clean",
            "连接器端到端证据通过；真正写入外部系统前仍会逐次确认。",
          )}
        </div>
      ) : issueChecks.length > 0 ? (
        <div className="mt-2 space-y-1">
          {issueChecks.slice(0, 4).map((check, index) => {
            const taskKey = `check:${index}:${check.name}`
            return (
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
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {domainGuardCheckNameLabel(t, check.name)}
                  </span>
                  <span className="shrink-0 tabular-nums">{check.actual}</span>
                  {canCreateTasks ? (
                    <button
                      type="button"
                      onClick={() => void createCheckTask(check, index)}
                      disabled={Boolean(creatingTaskKey)}
                      className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-current/20 bg-background/45 px-1.5 text-[10px] font-medium transition-colors hover:bg-background/70 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {creatingTaskKey === taskKey ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Plus className="h-3 w-3" />
                      )}
                      <span>{t("workspace.domainConnectorE2E.createCheckTask", "转任务")}</span>
                    </button>
                  ) : null}
                </div>
                <div className="mt-0.5 truncate opacity-85">{check.detail}</div>
              </div>
            )
          })}
        </div>
      ) : !loading ? (
        <EmptyHint>
          {report && !hasScope
            ? t("workspace.domainConnectorE2E.noScope", "还没有连接器端到端样本需要验收")
            : t("workspace.domainConnectorE2E.empty", "还没有连接器端到端评估结果")}
        </EmptyHint>
      ) : null}

      {relatedEvidence.length ? (
        <div className="mt-2 space-y-1">
          {relatedEvidence.slice(0, 2).map((item) => (
            <div
              key={item.id}
              className="min-w-0 rounded-md border border-border/40 bg-secondary/25 px-2 py-1.5 text-[11px] text-muted-foreground"
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <GitCompare className="h-3 w-3 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium text-foreground/80">
                  {item.title}
                </span>
                <StatusPill label={domainGuardCheckNameLabel(t, item.reason)} tone="muted" />
              </div>
              <div className="mt-0.5 truncate font-mono text-[10px] opacity-75">
                {domainAccessScopeLabel(t, item.accessScope)} ·{" "}
                {domainRedactionStatusLabel(t, item.redactionStatus)}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function domainConnectorE2EGateTone(
  status?: string | null,
  loading?: boolean,
  hasScope = true,
): StatusTone {
  if (loading) return "info"
  if (status && !hasScope) return "muted"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainConnectorE2EGateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
  hasScope = true,
): string {
  if (loading) return t("workspace.domainConnectorE2E.loading", "评估中")
  if (status && !hasScope) return t("workspace.domainConnectorE2E.noScopeLabel", "未采样")
  if (status === "passed") return t("workspace.domainConnectorE2E.passed", "已闭环")
  if (status === "failed") return t("workspace.domainConnectorE2E.failed", "阻塞")
  if (status === "insufficient_data") {
    return t("workspace.domainConnectorE2E.insufficient", "缺证据")
  }
  return t("workspace.domainConnectorE2E.idle", "未评估")
}

function domainConnectorActionGuardTone(
  status?: string | null,
  loading?: boolean,
  hasScope = true,
): StatusTone {
  if (loading) return "info"
  if (status && !hasScope) return "muted"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainConnectorActionGuardLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
  hasScope = true,
): string {
  if (loading) return t("workspace.domainConnectorGuard.loading", "评估中")
  if (status && !hasScope) return t("workspace.domainConnectorGuard.noScopeLabel", "未配置")
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
  hasScope = true,
): StatusTone {
  if (loading) return "info"
  if (status && !hasScope) return "muted"
  if (status === "passed") return "good"
  if (status === "failed") return "danger"
  if (status === "insufficient_data") return "warn"
  return "muted"
}

function domainArtifactExportGuardLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status?: string | null,
  loading?: boolean,
  hasScope = true,
): string {
  if (loading) return t("workspace.domainExportGuard.loading", "评估中")
  if (status && !hasScope) return t("workspace.domainExportGuard.noScopeLabel", "未配置")
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
    proposal.kind === "domain_eval_case" &&
    proposal.status === "promoted" &&
    !importingDomainEvalCase
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
      toast.success(t("workspace.codingTrend.domainEvalAlreadyImported", "领域评测已在任务库中"))
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
const GOAL_CRITERION_NONE_VALUE = "__whole_goal__"

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

function isWorkflowRunState(value: string | null | undefined): value is WorkflowRunState {
  return (
    value === "draft" ||
    value === "awaiting_approval" ||
    value === "running" ||
    value === "awaiting_user" ||
    value === "paused" ||
    value === "recovering" ||
    value === "completed" ||
    value === "failed" ||
    value === "cancelled" ||
    value === "blocked"
  )
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

function workflowRunDisplayState(
  t: ReturnType<typeof useTranslation>["t"],
  run: WorkflowRun,
  snapshot: WorkflowRunSnapshot | null,
): { label: string; tone: StatusTone; loading: boolean; kind: "run" | "children" | "results" } {
  const usage = snapshot?.agentUsage
  if (usage && usage.runningAgents > 0) {
    return {
      label: t("workspace.workflow.stateWaitingAgents", "等待子 Agent {{done}}/{{total}}", {
        done: usage.terminalAgents,
        total: usage.spawnedAgents,
      }),
      tone: "info",
      loading: true,
      kind: "children",
    }
  }
  if (usage && usage.pendingResults > 0) {
    return {
      label: t("workspace.workflow.statePartialResults", "阶段结果 {{done}}/{{total}}", {
        done: usage.terminalAgents,
        total: usage.spawnedAgents,
      }),
      tone: "warn",
      loading: false,
      kind: "results",
    }
  }
  return {
    label: workflowRunStateLabel(t, run.state),
    tone: workflowRunTone(run.state),
    loading: run.state === "running" || run.state === "recovering",
    kind: "run",
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

function goalCriterionMetadataId(metadata: unknown): string | null {
  const record = asRecord(metadata)
  const nested = asRecord(record?.goalCriterion)
  return stringField(nested, "id") ?? stringField(record, "goalCriterionId")
}

function goalCriterionOptionLabel(criterion: GoalCriterionItem): string {
  return `${criterion.id} · ${criterion.text}`
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

function workflowDateMs(value?: string | null): number | null {
  if (!value) return null
  const ms = Date.parse(value)
  return Number.isFinite(ms) ? ms : null
}

function workflowRunDurationSeconds(run: WorkflowRun): number | null {
  const started = workflowDateMs(run.createdAt)
  const ended =
    workflowDateMs(run.completedAt) ??
    workflowDateMs(run.updatedAt) ??
    (workflowRunIsLive(run.state) ? Date.now() : null)
  if (started === null || ended === null || ended < started) return null
  return Math.round((ended - started) / 1000)
}

type WorkflowSizeGuideline = "unrestricted" | "small" | "medium" | "large"

function workflowSizeGuideline(run: WorkflowRun): WorkflowSizeGuideline | null {
  const budget = asRecord(run.budget)
  const raw = stringField(budget, "sizeGuideline") ?? stringField(budget, "size_guideline")
  switch (raw) {
    case "unrestricted":
    case "small":
    case "medium":
    case "large":
      return raw
    default:
      return null
  }
}

function workflowSizeGuidelineLabel(
  t: ReturnType<typeof useTranslation>["t"],
  guideline: WorkflowSizeGuideline | null,
): string {
  switch (guideline) {
    case "unrestricted":
      return t("workspace.workflow.summarySizeUnrestricted", "不限制")
    case "small":
      return t("workspace.workflow.summarySizeSmall", "小")
    case "medium":
      return t("workspace.workflow.summarySizeMedium", "中")
    case "large":
      return t("workspace.workflow.summarySizeLarge", "大")
    default:
      return t("workspace.workflow.summaryUnknown", "未记录")
  }
}

function workflowRunRuntimeCaps(
  t: ReturnType<typeof useTranslation>["t"],
  run: WorkflowRun,
): string | null {
  const budget = asRecord(run.budget)
  const scriptSecs =
    numberField(budget, "maxScriptSecs") ??
    numberField(budget, "max_script_secs") ??
    numberField(budget, "maxRuntimeSecs") ??
    numberField(budget, "max_runtime_secs")
  const ops = numberField(budget, "maxOps") ?? numberField(budget, "max_ops")
  const output = numberField(budget, "maxOutputTokens") ?? numberField(budget, "max_output_tokens")
  const parts = [
    typeof scriptSecs === "number" && scriptSecs > 0
      ? `${formatDurationCompact(scriptSecs)}`
      : null,
    typeof ops === "number" && ops > 0
      ? t("workspace.workflow.summaryCapOps", "{{value}} 步", { value: compactCount(ops) })
      : null,
    typeof output === "number" && output > 0
      ? t("workspace.workflow.summaryCapTokens", "{{value}} Token", { value: compactCount(output) })
      : null,
  ].filter(Boolean)
  return parts.length > 0 ? parts.join(" · ") : null
}

function workflowLatestRuntimeSummary(
  t: ReturnType<typeof useTranslation>["t"],
  events: WorkflowEvent[],
): { label: string; detail: string | null; tone: StatusTone } | null {
  const event = [...events].reverse().find((item) => item.eventType === "run_runtime_result")
  if (!event) return null
  const payload = asRecord(event.payload)
  const status = stringField(payload, "status")
  const finalState = stringField(payload, "finalState") ?? stringField(payload, "final_state")
  const reason = stringField(payload, "reason")
  const error = stringField(payload, "error")
  const tone = workflowEventTone(event)
  const label = isWorkflowRunState(finalState)
    ? workflowRunStateLabel(t, finalState)
    : (status ?? t("workspace.workflow.summaryUnknown", "未记录"))
  const detail = [status, reason, error ? truncateMiddle(error, 96) : null]
    .filter(Boolean)
    .join(" · ")
  return { label, detail: detail || null, tone }
}

function workflowRunSummaryCounts(events: WorkflowEvent[]) {
  return {
    phasesStarted: events.filter((event) => event.eventType === "workflow_phase_started").length,
    phasesCompleted: events.filter((event) => event.eventType === "workflow_phase_completed")
      .length,
    phasesFailed: events.filter((event) => event.eventType === "workflow_phase_failed").length,
    checkpoints: events.filter((event) => event.eventType === "workflow_checkpoint").length,
    reports: events.filter((event) => event.eventType === "workflow_report").length,
    milestoneRequested: events.filter(
      (event) => event.eventType === "workflow_milestone_injection_requested",
    ).length,
    milestoneDelivered: events.filter(
      (event) => event.eventType === "workflow_milestone_injection_delivered",
    ).length,
  }
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

function hasMeaningfulScopeValue(value?: string | null): boolean {
  const normalized = value?.trim().toLowerCase()
  return Boolean(
    normalized &&
    normalized !== "unknown" &&
    normalized !== "none" &&
    normalized !== "null" &&
    normalized !== "-",
  )
}

function domainArtifactExportGuardHasScope(
  report?: DomainArtifactExportGuardReport | null,
): boolean {
  const summary = report?.summary
  return Boolean(
    report &&
    (hasMeaningfulScopeValue(report.artifactPath) ||
      hasMeaningfulScopeValue(report.artifactTitle) ||
      hasMeaningfulScopeValue(report.artifactKind) ||
      (summary?.evidenceItems ?? 0) > 0 ||
      (summary?.artifactCreated ?? 0) > 0 ||
      (summary?.artifactReviewed ?? 0) > 0 ||
      (summary?.exportReviewed ?? 0) > 0 ||
      (summary?.sensitiveEvidence ?? 0) > 0 ||
      (summary?.privateOrConnectorEvidence ?? 0) > 0 ||
      (summary?.redactionPending ?? 0) > 0 ||
      (report.evidenceRequiringReview?.length ?? 0) > 0),
  )
}

function domainConnectorActionGuardHasScope(
  report?: DomainConnectorActionGuardReport | null,
): boolean {
  const summary = report?.summary
  const hasConcreteTarget =
    hasMeaningfulScopeValue(report?.toolName) ||
    hasMeaningfulScopeValue(report?.connector) ||
    hasMeaningfulScopeValue(report?.action)
  return Boolean(
    report &&
    (hasConcreteTarget ||
      (summary?.evidenceItems ?? 0) > 0 ||
      (summary?.actionEvidence ?? 0) > 0 ||
      (summary?.approvalEvidence ?? 0) > 0 ||
      (summary?.rollbackEvidence ?? 0) > 0 ||
      (summary?.sensitiveEvidence ?? 0) > 0 ||
      (report.relatedEvidence?.length ?? 0) > 0),
  )
}

function domainConnectorE2EGateHasScope(report?: DomainConnectorE2EGateReport | null): boolean {
  const summary = report?.summary
  const hasConcreteTarget =
    hasMeaningfulScopeValue(report?.toolName) ||
    hasMeaningfulScopeValue(report?.connector) ||
    hasMeaningfulScopeValue(report?.action)
  return Boolean(
    report &&
    (hasConcreteTarget ||
      (summary?.evidenceItems ?? 0) > 0 ||
      (summary?.connectorInputEvidence ?? 0) > 0 ||
      (summary?.draftEvidence ?? 0) > 0 ||
      (summary?.approvalEvidence ?? 0) > 0 ||
      (summary?.executionEvidence ?? 0) > 0 ||
      (summary?.verificationEvidence ?? 0) > 0 ||
      (summary?.rollbackEvidence ?? 0) > 0 ||
      (summary?.sensitiveEvidence ?? 0) > 0 ||
      (report.relatedEvidence?.length ?? 0) > 0),
  )
}

function domainOperationalGateHasSamples(report?: DomainOperationalGateReport | null): boolean {
  const summary = report?.summary
  return Boolean(
    summary &&
    (summary.workflowRuns > 0 ||
      summary.loopSchedules > 0 ||
      summary.loopRuns > 0 ||
      summary.campaigns > 0 ||
      summary.campaignItems > 0 ||
      summary.activeWorkflowRuns > 0 ||
      summary.activeLoopSchedules > 0 ||
      summary.activeLoopRuns > 0 ||
      summary.activeCampaigns > 0 ||
      hasMeaningfulScopeValue(summary.latestActivityAt) ||
      summary.maxActiveWorkAgeSecs != null),
  )
}

function domainSoakReportHasSamples(report?: DomainSoakReport | null): boolean {
  const summary = report?.summary
  return Boolean(
    summary &&
    (summary.totalRecords > 0 ||
      summary.workflowRuns > 0 ||
      summary.loopRuns > 0 ||
      summary.campaignItems > 0 ||
      summary.connectorE2eEvidence > 0 ||
      summary.connectorExecutionEvidence > 0 ||
      summary.connectorVerificationEvidence > 0 ||
      summary.incidents > 0 ||
      (report?.incidents.length ?? 0) > 0 ||
      (report?.timeline.length ?? 0) > 0 ||
      hasMeaningfulScopeValue(summary.latestActivityAt)),
  )
}

function GoalWorkspaceSection({
  sessionId,
  incognito,
  onEnsureSession,
  goalState,
}: {
  sessionId?: string | null
  incognito?: boolean
  onEnsureSession?: () => Promise<string | null>
  goalState: GoalStateSnapshot
}) {
  const { t } = useTranslation()
  const [goalActionKey, setGoalActionKey] = useState<string | null>(null)
  const [goalCreateOpen, setGoalCreateOpen] = useState(false)
  const [goalObjective, setGoalObjective] = useState("")
  const [goalCriteria, setGoalCriteria] = useState("")
  const [goalTemplateId, setGoalTemplateId] = useState(GOAL_DOMAIN_FREE_VALUE)
  const [goalTaskType, setGoalTaskType] = useState("")
  const [goalSaving, setGoalSaving] = useState(false)
  const [domainTemplates, setDomainTemplates] = useState<DomainWorkflowTemplate[]>([])
  const [domainTemplatesLoading, setDomainTemplatesLoading] = useState(false)
  const [domainTemplatesError, setDomainTemplatesError] = useState<string | null>(null)
  const ensureSessionRef = useRef<Promise<string | null> | null>(null)
  const domainTemplatesReqRef = useRef(0)
  const domainTemplatesRequestedRef = useRef(false)
  const activeGoal = goalState.snapshot?.goal ?? null
  const goalWatchdogFindings = goalState.watchdogFindings ?? []
  const topGoalWatchdogFindings = activeGoal
    ? goalWatchdogFindings.filter((finding) => finding.goalId === activeGoal.id).slice(0, 3)
    : []
  const selectedGoalTemplate =
    goalTemplateId === GOAL_DOMAIN_FREE_VALUE
      ? null
      : findDomainTemplateByValue(domainTemplates, goalTemplateId)
  const canMaterializeSession = Boolean(sessionId || onEnsureSession)

  const ensureGoalSession = useCallback(async () => {
    if (sessionId) return sessionId
    if (!onEnsureSession) {
      toast.error(t("workspace.goal.sessionRequired", "先选择或创建一个会话后再创建目标"))
      return null
    }
    if (!ensureSessionRef.current) {
      ensureSessionRef.current = onEnsureSession().finally(() => {
        ensureSessionRef.current = null
      })
    }
    const nextSessionId = await ensureSessionRef.current
    if (!nextSessionId) {
      toast.error(t("workspace.goal.sessionRequired", "先选择或创建一个会话后再创建目标"))
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
          "GoalWorkspaceSection::loadDomainWorkflowTemplates",
          "Failed to load domain workflow templates",
          e,
        )
        setDomainTemplates([])
        setDomainTemplatesError(e instanceof Error ? e.message : String(e))
        setDomainTemplatesLoading(false)
      })
  }, [incognito])

  useEffect(() => {
    if (incognito || domainTemplatesRequestedRef.current) return
    if (!goalCreateOpen && !activeGoal?.workflowTemplateId) return
    loadDomainWorkflowTemplates()
  }, [activeGoal?.workflowTemplateId, goalCreateOpen, incognito, loadDomainWorkflowTemplates])

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

  const createGoalFromDraft = useCallback(async () => {
    if (incognito) return
    const objective = goalObjective.trim()
    if (!objective) {
      toast.error(t("workspace.goal.objectiveRequired", "请输入目标"))
      return
    }
    const targetSessionId = await ensureGoalSession()
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
      setGoalObjective("")
      setGoalCriteria("")
      setGoalTemplateId(GOAL_DOMAIN_FREE_VALUE)
      setGoalTaskType("")
      setGoalCreateOpen(false)
      toast.success(t("workspace.goal.created", "已创建目标"))
    } catch (e) {
      logger.error("ui", "GoalWorkspaceSection::createGoal", "Failed to create goal", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setGoalSaving(false)
    }
  }, [
    ensureGoalSession,
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
          toast.success(t("workspace.goal.cleared", "目标已清除"))
        } else if (command === "evaluate_goal") {
          toast.success(t("workspace.goal.evaluated", "目标评估已更新"))
        } else {
          toast.success(t("workspace.goal.updated", "目标状态已更新"))
        }
        goalState.refresh()
      } catch (e) {
        logger.error("ui", "GoalWorkspaceSection::goalAction", `Goal action failed: ${command}`, e)
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
        toast.success(t("workspace.goal.updated", "目标状态已更新"))
        goalState.refresh()
        return true
      } catch (e) {
        logger.error("ui", "GoalWorkspaceSection::updateGoal", "Failed to update goal", e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      } finally {
        setGoalActionKey(null)
      }
    },
    [activeGoal, goalState, t],
  )

  const closeActiveGoal = useCallback(
    async (decision: GoalClosureDecision, reason?: string, followUpItems: string[] = []) => {
      if (!activeGoal) return
      const key = `close_goal:${decision}:${activeGoal.id}`
      setGoalActionKey(key)
      try {
        const snapshot = await getTransport().call<GoalSnapshot>("close_goal", {
          goalId: activeGoal.id,
          decision,
          reason,
          followUpItems,
        })
        goalState.setSnapshot(
          decision === "accepted_v1" || decision === "cancelled" ? null : snapshot,
        )
        toast.success(
          decision === "accepted_v1"
            ? t("workspace.goal.closedAccepted", "目标已按当前证据关闭")
            : t("workspace.goal.strictEvidenceRequested", "已要求补充严格证据"),
        )
        goalState.refresh()
      } catch (e) {
        logger.error("ui", "GoalWorkspaceSection::closeGoal", "Failed to close goal", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setGoalActionKey(null)
      }
    },
    [activeGoal, goalState, t],
  )

  const appendActiveGoalFollowUp = useCallback(
    async (items: string[]) => {
      if (!activeGoal) return false
      const normalizedItems = items.map((item) => item.trim()).filter(Boolean)
      if (normalizedItems.length === 0) return false
      const key = `append_goal_follow_up:${activeGoal.id}`
      setGoalActionKey(key)
      try {
        const snapshot = await getTransport().call<GoalSnapshot>("append_goal_follow_up", {
          goalId: activeGoal.id,
          items: normalizedItems,
          source: "workspace",
        })
        goalState.setSnapshot(snapshot)
        toast.success(t("workspace.goal.followUpAdded", "后续项已加入目标"))
        goalState.refresh()
        return true
      } catch (e) {
        logger.error(
          "ui",
          "GoalWorkspaceSection::appendGoalFollowUp",
          "Failed to append goal follow-up",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      } finally {
        setGoalActionKey(null)
      }
    },
    [activeGoal, goalState, t],
  )

  return (
    <WorkspaceSection
      title={t("workspace.goal.sectionTitle", "目标")}
      count={activeGoal ? 1 : 0}
      icon={Sparkles}
      meta={
        activeGoal ? (
          <div className="flex items-center gap-1">
            <StatusPill
              label={goalStateLabel(t, activeGoal.state)}
              tone={goalStateTone(activeGoal.state)}
              loading={activeGoal.state === "evaluating"}
            />
            {topGoalWatchdogFindings.length > 0 ? (
              <StatusPill
                label={t("workspace.goal.watchdogNeedsAttention", "需确认")}
                tone="warn"
              />
            ) : null}
          </div>
        ) : (
          <StatusPill label={t("workspace.goal.noActive", "未设置")} tone="muted" />
        )
      }
    >
      {incognito ? (
        <EmptyHint>{t("workspace.goal.incognito", "无痕会话不持久化目标")}</EmptyHint>
      ) : (
        <>
          {topGoalWatchdogFindings.length > 0 ? (
            <div className="mb-2 space-y-1 rounded-md border border-amber-500/25 bg-amber-500/10 p-2 text-[11px] text-amber-800 dark:text-amber-200">
              <div className="flex items-center gap-1.5 font-medium">
                <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
                {t("workspace.goal.watchdogTitle", "有目标需要确认")}
              </div>
              <div className="space-y-1">
                {topGoalWatchdogFindings.map((finding) => (
                  <div
                    key={`${finding.goalId}:${finding.code}`}
                    className="flex min-w-0 items-center gap-2 rounded-md bg-background/55 px-2 py-1.5"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="truncate font-medium text-foreground/85">
                        {activeGoal?.objective ?? t("workspace.goal.title", "目标")}
                      </div>
                      <div className="truncate text-muted-foreground">
                        {goalWatchdogFindingLabel(t, finding)}
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 shrink-0 gap-1 px-2 text-[11px]"
                      disabled={goalActionKey === `evaluate_goal:${activeGoal?.id}`}
                      onClick={() => void runGoalAction("evaluate_goal")}
                    >
                      <RefreshCw className="h-3.5 w-3.5" />
                      {t("workspace.goal.evaluate", "评估")}
                    </Button>
                  </div>
                ))}
              </div>
            </div>
          ) : null}
          {goalState.activity && goalState.activity.state !== "idle" ? (
            <div className="mb-2 flex min-w-0 items-center gap-2 border-y border-border/55 bg-muted/25 px-2 py-1.5 text-[11px]">
              <Radio className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="shrink-0 font-medium text-foreground/85">
                {autonomyActivityLabel(t, goalState.activity)}
              </span>
              {goalState.activity.currentStep ? (
                <span className="min-w-0 flex-1 truncate text-muted-foreground">
                  {goalState.activity.currentStep}
                </span>
              ) : null}
              {goalState.activity.needsUser ? (
                <StatusPill label={t("chat.activity.needsUser", "需要你处理")} tone="warn" />
              ) : null}
            </div>
          ) : null}
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
            editRequest={0}
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
            onCloseGoal={(decision, reason, followUpItems) =>
              void closeActiveGoal(decision, reason, followUpItems)
            }
            onAppendFollowUp={appendActiveGoalFollowUp}
          />
        </>
      )}
    </WorkspaceSection>
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
  if (boolField(call, "strict")) return t("workspace.workflow.permissionDecisionStrict", "严格")
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
      return t("workspace.workflow.tabTrace", "轨迹")
    case "validation":
      return t("workspace.workflow.tabValidation", "验证")
    case "agents":
      return t("workspace.workflow.tabAgents", "子 Agent")
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
    event.eventType === "workflow_checkpoint" ||
    event.eventType === "workflow_report" ||
    event.eventType === "workflow_milestone_injection_requested" ||
    event.eventType === "workflow_milestone_injection_delivered" ||
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
  "workflow_phase_started",
  "workflow_phase_completed",
  "workflow_phase_failed",
  "workflow_checkpoint",
  "workflow_report",
  "workflow_milestone_injection_requested",
  "workflow_milestone_injection_delivered",
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
  if (event.eventType === "workflow_phase_failed") return "danger"
  if (event.eventType === "workflow_checkpoint") {
    const importance = stringField(payload, "importance")
    if (importance === "critical") return "danger"
    if (importance === "high") return "good"
    return "info"
  }
  if (event.eventType === "workflow_report") {
    return boolField(payload, "needsUser") ? "warn" : "info"
  }
  if (event.eventType === "workflow_milestone_injection_requested") return "info"
  if (event.eventType === "workflow_milestone_injection_delivered") return "good"
  if (event.eventType === "workflow_phase_completed") return "good"
  if (event.eventType === "workflow_phase_started" || event.eventType === "workflow_progress") {
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
    case "workflow_phase_started":
      return t("workspace.workflow.eventPhaseStarted", "阶段开始")
    case "workflow_phase_completed":
      return t("workspace.workflow.eventPhaseCompleted", "阶段完成")
    case "workflow_phase_failed":
      return t("workspace.workflow.eventPhaseFailed", "阶段失败")
    case "workflow_progress":
      return t("workspace.workflow.eventProgress", "阶段进度")
    case "workflow_checkpoint":
      return stringField(payload, "title") ?? t("workspace.workflow.eventCheckpoint", "阶段检查点")
    case "workflow_report":
      return stringField(payload, "title") ?? t("workspace.workflow.eventReport", "阶段报告")
    case "workflow_milestone_injection_requested":
      return t("workspace.workflow.eventMilestoneInjectionRequested", "已请求通知模型")
    case "workflow_milestone_injection_delivered":
      return t("workspace.workflow.eventMilestoneInjectionDelivered", "模型已收到")
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
      return stringField(payload, "label") ?? t("workspace.workflow.eventTrace", "轨迹")
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
    case "workflow_phase_started": {
      const label = stringField(payload, "label") ?? stringField(payload, "name")
      const expected = stringField(payload, "expected")
      return [label, expected].filter(Boolean).join(" · ")
    }
    case "workflow_phase_completed": {
      const phaseKey = stringField(payload, "phaseKey")
      const summary = stringField(payload, "summary")
      return [phaseKey, summary].filter(Boolean).join(" · ")
    }
    case "workflow_phase_failed": {
      const phaseKey = stringField(payload, "phaseKey")
      const error = stringField(payload, "error")
      return [phaseKey, error ? truncateMiddle(error, 96) : null].filter(Boolean).join(" · ")
    }
    case "workflow_progress": {
      const message = stringField(payload, "message")
      const percent = numberField(payload, "percent")
      const phaseKey = stringField(payload, "phaseKey")
      const percentLabel = typeof percent === "number" ? `${Math.round(percent)}%` : null
      return [message, percentLabel, phaseKey].filter(Boolean).join(" · ")
    }
    case "workflow_checkpoint": {
      const summary = stringField(payload, "summary")
      const importance = stringField(payload, "importance")
      const injectPolicy = stringField(payload, "injectPolicy")
      return [summary, importance, injectPolicy ? `inject=${injectPolicy}` : null]
        .filter(Boolean)
        .join(" · ")
    }
    case "workflow_report": {
      const summary = stringField(payload, "summary")
      const nextAction = stringField(payload, "nextAction")
      const needsUser = boolField(payload, "needsUser")
      return [summary, nextAction, needsUser ? t("workspace.workflow.needsUser", "需要用户") : null]
        .filter(Boolean)
        .join(" · ")
    }
    case "workflow_milestone_injection_requested":
    case "workflow_milestone_injection_delivered": {
      const sourceEventType = stringField(payload, "sourceEventType")
      const sourceEventSeq = numberField(payload, "sourceEventSeq")
      const title = stringField(payload, "title")
      const summary = stringField(payload, "summary")
      return [
        sourceEventSeq ? `#${sourceEventSeq}` : null,
        sourceEventType,
        title,
        summary ? truncateMiddle(summary, 96) : null,
      ]
        .filter(Boolean)
        .join(" · ")
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

function workflowWatchdogFindingLabel(
  t: ReturnType<typeof useTranslation>["t"],
  finding: WorkflowWatchdogFinding,
): string {
  const age =
    typeof finding.staleSecs === "number" && finding.staleSecs > 0
      ? formatDurationCompact(finding.staleSecs)
      : null
  if (finding.code === "workflow_recoverable_owner") {
    return age
      ? t(
          "workspace.workflow.watchdogRecoverableOwnerWithAge",
          "运行 owner 不可用，已无进展 {{age}}",
          {
            age,
          },
        )
      : t("workspace.workflow.watchdogRecoverableOwner", "运行 owner 不可用")
  }
  if (finding.code === "workflow_no_recent_progress") {
    return age
      ? t(
          "workspace.workflow.watchdogNoRecentProgressWithAge",
          "运行中但没有新进展，已等待 {{age}}",
          {
            age,
          },
        )
      : t("workspace.workflow.watchdogNoRecentProgress", "运行中但没有新进展")
  }
  return finding.message || t("workspace.workflow.watchdogUnknown", "工作流需要确认")
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

function loopGroupLabel(t: ReturnType<typeof useTranslation>["t"], state: LoopState): string {
  switch (state) {
    case "blocked":
      return t("workspace.loop.groupBlocked", "需要处理")
    case "active":
      return t("workspace.loop.groupActive", "运行中")
    case "paused":
      return t("workspace.loop.groupPaused", "已暂停")
    case "completed":
      return t("workspace.loop.groupCompleted", "已完成")
    case "cancelled":
      return t("workspace.loop.groupCancelled", "已停止")
  }
}

function loopRunStateLabel(t: ReturnType<typeof useTranslation>["t"], state: LoopRunState): string {
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

function loopProgressLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state?: LoopProgressState | null,
): string | null {
  switch (state) {
    case "progressed":
      return t("workspace.loop.progressed", "有进展")
    case "weak_progress":
      return t("workspace.loop.weakProgress", "弱进展")
    case "no_progress":
      return t("workspace.loop.noProgress", "无进展")
    case "blocked":
      return t("workspace.loop.progressBlocked", "阻塞")
    case "failed":
      return t("workspace.loop.progressFailed", "失败")
    case "awaiting_approval":
      return t("workspace.loop.awaitingApproval", "等待后续")
    default:
      return null
  }
}

function loopProgressTone(state?: LoopProgressState | null): StatusTone {
  switch (state) {
    case "progressed":
      return "good"
    case "weak_progress":
    case "awaiting_approval":
      return "info"
    case "no_progress":
      return "warn"
    case "blocked":
    case "failed":
      return "danger"
    default:
      return "muted"
  }
}

function isLoopTerminal(state: LoopState): boolean {
  return state === "completed" || state === "cancelled"
}

function loopTriggerSummary(
  t: ReturnType<typeof useTranslation>["t"],
  kind: LoopTriggerKind,
  spec: Record<string, unknown>,
): string {
  if (kind === "interval") {
    const secs = typeof spec.intervalSecs === "number" ? spec.intervalSecs : null
    return secs
      ? t("workspace.loop.triggerEvery", "每 {{duration}}", {
          duration: formatLoopDuration(secs),
        })
      : t("workspace.loop.triggerInterval", "定期推进")
  }
  if (kind === "condition") {
    const condition = typeof spec.condition === "string" ? spec.condition : ""
    const label = condition.length > 48 ? `${condition.slice(0, 48)}...` : condition
    return label
      ? t("workspace.loop.triggerUntil", "直到 {{condition}}", { condition: label })
      : t("workspace.loop.triggerUntilPlain", "直到条件满足")
  }
  if (kind === "event") {
    const eventName = typeof spec.eventName === "string" ? spec.eventName : "event"
    const filters =
      typeof spec.filters === "object" && spec.filters !== null
        ? (spec.filters as Record<string, unknown>)
        : {}
    const state =
      typeof filters.workflowState === "string"
        ? filters.workflowState
        : typeof filters.goalState === "string"
          ? filters.goalState
          : typeof filters.taskStatus === "string"
            ? filters.taskStatus
            : null
    const eventLabel =
      eventName === "workflow:updated"
        ? t("workspace.loop.eventWorkflow", "工作流状态")
        : eventName === "goal:updated"
          ? t("workspace.loop.eventGoal", "目标状态")
          : eventName === "task_updated"
            ? t("workspace.loop.eventTask", "任务状态")
            : eventName
    return state ? `${eventLabel} · ${state}` : eventLabel
  }
  if (kind === "dynamic") {
    const fallbackSecs = typeof spec.fallbackSecs === "number" ? spec.fallbackSecs : null
    return fallbackSecs
      ? t("workspace.loop.triggerDynamicWithFallback", "模型自定 · 回退 {{duration}}", {
          duration: formatLoopDuration(fallbackSecs),
        })
      : t("workspace.loop.triggerDynamic", "模型自定")
  }
  if (kind === "cron") return t("workspace.loop.triggerCron", "Cron")
  return t("workspace.loop.triggerEvent", "事件触发")
}

function loopExecutionStrategyLabel(
  t: ReturnType<typeof useTranslation>["t"],
  strategy: LoopExecutionStrategy,
): string {
  return strategy === "workflow"
    ? t("workspace.loop.strategyWorkflowShort", "工作流")
    : t("workspace.loop.strategyContinueShort", "会话")
}

function loopScheduleStory(
  t: ReturnType<typeof useTranslation>["t"],
  loop: LoopSchedule,
  nextRunLabel: string | null,
  latestWorkflowRun?: WorkflowRun | null,
): string {
  if (loop.state === "blocked") {
    return loop.blockedReason
      ? t("workspace.loop.storyBlockedReason", "已阻塞：{{reason}}", {
          reason: loop.blockedReason,
        })
      : t("workspace.loop.storyBlocked", "已阻塞，需要处理后再恢复。")
  }
  if (loop.state === "paused") {
    return t("workspace.loop.storyPaused", "已暂停，恢复后会继续按策略触发。")
  }
  if (loop.state === "completed") {
    return t("workspace.loop.storyCompleted", "已完成，历史记录仍可复盘。")
  }
  if (loop.state === "cancelled") {
    return t("workspace.loop.storyCancelled", "已停止，不会再触发。")
  }
  const progressLabel = loopProgressLabel(t, loop.progressState)
  if (loop.executionStrategy === "workflow" && latestWorkflowRun) {
    return t(
      "workspace.loop.storyWorkflow",
      "已运行 {{count}} 次，最近一次创建了工作流 {{state}}。",
      {
        count: loop.runCount,
        state: workflowRunStateLabel(t, latestWorkflowRun.state),
      },
    )
  }
  if (nextRunLabel) {
    return progressLabel
      ? t(
          "workspace.loop.storyNextWithProgress",
          "{{next}}，已运行 {{count}} 次，最近{{progress}}。",
          {
            next: nextRunLabel,
            count: loop.runCount,
            progress: progressLabel,
          },
        )
      : t("workspace.loop.storyNext", "{{next}}，已运行 {{count}} 次。", {
          next: nextRunLabel,
          count: loop.runCount,
        })
  }
  return t("workspace.loop.storyActive", "正在等待下一次触发，已运行 {{count}} 次。", {
    count: loop.runCount,
  })
}

function loopGuardStory(t: ReturnType<typeof useTranslation>["t"], loop: LoopSchedule): string {
  const noProgress = `${loop.noProgressStreak}/${loop.maxNoProgressRuns ?? "∞"}`
  const failures = `${loop.failureStreak}/${loop.maxFailures ?? "∞"}`
  const backoff = loop.backoffSecs ? formatLoopDuration(loop.backoffSecs) : "off"
  return t(
    "workspace.loop.guardStory",
    "连续无进展 {{noProgress}}，失败 {{failures}}，降频 {{backoff}}",
    { noProgress, failures, backoff },
  )
}

function loopWatchdogFindingLabel(
  t: ReturnType<typeof useTranslation>["t"],
  finding: LoopWatchdogFinding,
): string {
  const overdue =
    typeof finding.overdueSecs === "number" && finding.overdueSecs > 0
      ? formatDurationCompact(finding.overdueSecs)
      : null
  if (finding.code === "loop_cron_missing") {
    return overdue
      ? t("workspace.loop.watchdogCronMissingWithAge", "调度记录缺失，已延迟 {{age}}", {
          age: overdue,
        })
      : t("workspace.loop.watchdogCronMissing", "调度记录缺失")
  }
  if (finding.code === "loop_due_not_claimed") {
    return overdue
      ? t("workspace.loop.watchdogDueNotClaimedWithAge", "到点后还未开始，已延迟 {{age}}", {
          age: overdue,
        })
      : t("workspace.loop.watchdogDueNotClaimed", "到点后还未开始")
  }
  if (finding.code === "loop_run_maybe_interrupted") {
    return overdue
      ? t("workspace.loop.watchdogRunMaybeInterruptedWithAge", "上次运行可能中断，已持续 {{age}}", {
          age: overdue,
        })
      : t("workspace.loop.watchdogRunMaybeInterrupted", "上次运行可能中断")
  }
  return finding.message || t("workspace.loop.watchdogUnknown", "持续推进需要确认")
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

type LoopEventName = "workflow:updated" | "goal:updated" | "task_updated"

function loopEventFilterKey(
  eventName: LoopEventName,
): "workflowState" | "goalState" | "taskStatus" {
  if (eventName === "workflow:updated") return "workflowState"
  if (eventName === "goal:updated") return "goalState"
  return "taskStatus"
}

function loopEventStateOptions(eventName: LoopEventName): string[] {
  if (eventName === "workflow:updated") {
    return ["completed", "failed", "blocked", "cancelled", "awaiting_user"]
  }
  if (eventName === "goal:updated") {
    return ["completed", "blocked", "failed", "cancelled", "evaluating", "active"]
  }
  return ["completed", "in_progress", "pending"]
}

function loopNextRunLabel(
  t: ReturnType<typeof useTranslation>["t"],
  loop: LoopSchedule,
): string | null {
  if (loop.state !== "active") return null
  if (loop.triggerKind === "event") {
    return t("workspace.loop.waitingEvent", "等待事件")
  }
  if (loop.triggerKind === "dynamic") {
    if (!loop.nextRunAt) return t("workspace.loop.nextDynamicUnknown", "等待模型决策")
    return t("workspace.loop.nextDynamicRun", "模型将在 {{time}} 继续", {
      time: formatMessageTime(loop.nextRunAt),
    })
  }
  if (!loop.nextRunAt) return t("workspace.loop.nextUnknown", "下次待定")
  return t("workspace.loop.nextRun", "下次 {{time}}", {
    time: formatMessageTime(loop.nextRunAt),
  })
}

function loopSchedulingDecisionLabel(
  t: ReturnType<typeof useTranslation>["t"],
  decision?: string | null,
): string | null {
  if (!decision) return null
  if (decision.startsWith("backoff_")) {
    const secs = Number(decision.slice("backoff_".length).replace(/s$/, ""))
    return Number.isFinite(secs)
      ? t("workspace.loop.backingOff", "已降频 {{duration}}", {
          duration: formatLoopDuration(secs),
        })
      : t("workspace.loop.backingOffPlain", "已降频")
  }
  switch (decision) {
    case "continue":
      return t("workspace.loop.decisionContinue", "继续")
    case "awaiting_follow_up_turn":
      return t("workspace.loop.decisionAwaiting", "等待后续回合")
    case "blocked_no_progress_limit":
      return t("workspace.loop.decisionBlockedNoProgress", "无进展达到上限")
    case "blocked_failure_limit":
      return t("workspace.loop.decisionBlockedFailure", "失败达到上限")
    case "completed_condition_satisfied":
      return t("workspace.loop.decisionConditionMet", "条件已满足")
    case "completed_max_runs":
      return t("workspace.loop.decisionMaxRuns", "达到最大次数")
    case "completed_max_runtime":
      return t("workspace.loop.decisionMaxRuntime", "达到最长运行时间")
    case "completed_dynamic_stop":
      return t("workspace.loop.decisionDynamicStop", "模型已停止")
    case "blocked_dynamic":
      return t("workspace.loop.decisionDynamicBlocked", "模型标记阻塞")
    case "blocked_dynamic_missing_decision":
      return t("workspace.loop.decisionDynamicMissing", "缺少下次调度")
    default:
      if (decision.startsWith("dynamic_reschedule_")) {
        const secs = Number.parseInt(decision.replace("dynamic_reschedule_", ""), 10)
        return Number.isFinite(secs)
          ? t("workspace.loop.decisionDynamicReschedule", "{{duration}} 后继续", {
              duration: formatLoopDuration(secs),
            })
          : t("workspace.loop.decisionDynamicReschedulePlain", "模型已安排下次")
      }
      if (decision.startsWith("dynamic_fallback_")) {
        const secs = Number.parseInt(decision.replace("dynamic_fallback_", ""), 10)
        return Number.isFinite(secs)
          ? t("workspace.loop.decisionDynamicFallback", "{{duration}} 后回退检查", {
              duration: formatLoopDuration(secs),
            })
          : t("workspace.loop.decisionDynamicFallbackPlain", "已安排回退检查")
      }
      return decision.replaceAll("_", " ")
  }
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

function loopRunDynamicDecisionReason(run: LoopRun): string | null {
  const decision = loopRunTrace(run).dynamicDecision
  if (typeof decision !== "object" || decision === null || Array.isArray(decision)) return null
  const reason = (decision as Record<string, unknown>).reason
  return typeof reason === "string" && reason.trim() ? reason.trim() : null
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
        {t("workspace.loop.historyLoading", "加载持续推进记录")}
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
  const watches = snapshot.watches ?? []
  return (
    <>
      {watches.length > 0 ? (
        <div className="mt-2 space-y-1 border-y border-border/55 bg-muted/20 px-2 py-1.5">
          <div className="flex items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
            <Radio className="h-3 w-3" />
            {t("workspace.loop.watchesTitle", "监听器")}
          </div>
          {watches.slice(0, 4).map((watch) => (
            <div key={watch.id} className="flex min-w-0 items-center gap-2 text-[10px]">
              <span className="min-w-0 flex-1 truncate font-mono text-foreground/75">
                {watch.kind}
              </span>
              <span className="shrink-0 text-muted-foreground">
                {t("workspace.loop.watchGeneration", "第 {{value}} 代", {
                  value: watch.generation,
                })}
              </span>
              <StatusPill
                label={
                  watch.active
                    ? t("workspace.loop.watchArmed", "监听中")
                    : t("workspace.loop.watchSettled", "已结束")
                }
                tone={watch.active ? "info" : watch.lastError ? "danger" : "muted"}
              />
              {watch.failureCount > 0 ? (
                <span className="shrink-0 text-destructive">
                  {t("workspace.loop.watchFailures", "失败 {{count}} 次", {
                    count: watch.failureCount,
                  })}
                </span>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}
      {snapshot.runs.length === 0 ? (
        <div className="mt-2 rounded-md bg-secondary/20 px-2 py-1.5 text-[11px] text-muted-foreground">
          {t("workspace.loop.historyEmpty", "还没有触发记录")}
        </div>
      ) : (
        <div className="mt-2 space-y-1 rounded-md border border-border/60 bg-secondary/15 p-1.5">
          <div className="flex items-center gap-1.5 px-0.5 text-[10px] font-medium text-muted-foreground">
            <Clock className="h-3 w-3" />
            {t("workspace.loop.historyTitle", "最近运行")}
          </div>
          {snapshot.runs.slice(0, 5).map((run) => {
            const workflowRunId = loopRunTraceString(run, "workflowRunId")
            const template = loopRunTemplateLabel(run)
            const summary = run.error || run.resultSummary
            const progressLabel = loopProgressLabel(t, run.progressState)
            const decisionLabel = loopSchedulingDecisionLabel(t, run.schedulingDecision)
            const decisionReason = loopRunDynamicDecisionReason(run)
            const usage = run.usage
            const usageVisible = Boolean(usage && usage.totalTokens > 0)
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
                  {progressLabel ? (
                    <StatusPill label={progressLabel} tone={loopProgressTone(run.progressState)} />
                  ) : null}
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
                {workflowRunId || template || usageVisible ? (
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
                    {usageVisible && usage ? (
                      <span
                        className="min-w-0 truncate"
                        data-ha-title-tip={t(
                          "workspace.loop.runUsageBoundary",
                          "优先按 Loop 触发消息到下一条用户消息之间统计；历史数据无触发元数据时回退到运行窗口。不代表完整成本。",
                        )}
                      >
                        {t("workspace.loop.runUsage", "本轮 Token")}{" "}
                        {t(
                          "workspace.loop.runUsageValue",
                          "{{total}} · 输入 {{input}} / 输出 {{output}}",
                          {
                            total: compactCount(usage.totalTokens),
                            input: compactCount(usage.inputTokens),
                            output: compactCount(usage.outputTokens),
                          },
                        )}
                      </span>
                    ) : null}
                  </div>
                ) : null}
                {decisionLabel || decisionReason || run.noProgressReason ? (
                  <div className="mt-1 flex min-w-0 flex-wrap gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
                    {decisionLabel ? (
                      <span>
                        {t("workspace.loop.decision", "调度")} {decisionLabel}
                      </span>
                    ) : null}
                    {decisionReason ? (
                      <span className="min-w-0 truncate">
                        {t("workspace.loop.decisionReason", "原因")} {decisionReason}
                      </span>
                    ) : null}
                    {run.noProgressReason ? (
                      <span className="min-w-0 truncate text-muted-foreground">
                        {run.noProgressReason}
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
      )}
    </>
  )
}

type LoopDraftKind = "interval" | "condition" | "event" | "dynamic"
type LoopTemplateKey = "ci" | "report" | "task" | "summary" | "external"

interface LoopTemplateOption {
  key: LoopTemplateKey
  label: string
  icon: LucideIcon
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
  const {
    schedules,
    watchdogFindings = [],
    activeCount,
    loading,
    error,
    refresh,
  } = loopSchedulesState ?? ownedLoopSchedulesState
  const [actionId, setActionId] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [createSaving, setCreateSaving] = useState(false)
  const [draftKind, setDraftKind] = useState<LoopDraftKind>("interval")
  const [draftExecutionStrategy, setDraftExecutionStrategy] =
    useState<LoopExecutionStrategy>("continue")
  const [draftInterval, setDraftInterval] = useState("10m")
  const [draftCondition, setDraftCondition] = useState("")
  const [draftEventName, setDraftEventName] = useState<LoopEventName>("workflow:updated")
  const [draftEventState, setDraftEventState] = useState("completed")
  const [draftEventDebounce, setDraftEventDebounce] = useState("30s")
  const [draftDynamicFallback, setDraftDynamicFallback] = useState("20m")
  const [draftPrompt, setDraftPrompt] = useState("")
  const [draftGoalCriterionId, setDraftGoalCriterionId] = useState(GOAL_CRITERION_NONE_VALUE)
  const [draftMaxRuns, setDraftMaxRuns] = useState("")
  const [draftMaxRuntime, setDraftMaxRuntime] = useState("")
  const [draftTokens, setDraftTokens] = useState("")
  const [draftMaxNoProgress, setDraftMaxNoProgress] = useState("3")
  const [draftMaxFailures, setDraftMaxFailures] = useState("3")
  const [draftBackoff, setDraftBackoff] = useState("5m")
  const [draftAdvancedOpen, setDraftAdvancedOpen] = useState(false)
  const [showAllLoops, setShowAllLoops] = useState(false)
  const [policyLoopId, setPolicyLoopId] = useState<string | null>(null)
  const [policyMaxRuns, setPolicyMaxRuns] = useState("")
  const [policyMaxRuntime, setPolicyMaxRuntime] = useState("")
  const [policyTokens, setPolicyTokens] = useState("")
  const [policyMaxNoProgress, setPolicyMaxNoProgress] = useState("")
  const [policyMaxFailures, setPolicyMaxFailures] = useState("")
  const [policyBackoff, setPolicyBackoff] = useState("")
  const [policySaving, setPolicySaving] = useState(false)
  const [detailLoopId, setDetailLoopId] = useState<string | null>(null)
  const [detailSnapshot, setDetailSnapshot] = useState<LoopSnapshot | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [detailError, setDetailError] = useState<string | null>(null)
  const detailReqRef = useRef(0)
  const lastCreateRequestRef = useRef(createRequest ?? 0)
  const lastInspectRequestRef = useRef(inspectRequest?.nonce ?? 0)
  const activeGoal = goalState.snapshot?.goal ?? null
  const activeGoalCriteria = useMemo(
    () => goalState.snapshot?.criteriaItems ?? [],
    [goalState.snapshot?.criteriaItems],
  )
  const canUseWorkflowLoop = draftKind === "interval" && Boolean(activeGoal?.workflowTemplateId)
  const loopTemplates = useMemo<LoopTemplateOption[]>(
    () => [
      {
        key: "ci",
        label: t("workspace.loop.templateCi", "检查 CI"),
        icon: GitBranch,
      },
      {
        key: "report",
        label: t("workspace.loop.templateReport", "刷新报告"),
        icon: BookText,
      },
      {
        key: "task",
        label: t("workspace.loop.templateTask", "任务后续"),
        icon: ClipboardCheck,
      },
      {
        key: "summary",
        label: t("workspace.loop.templateSummary", "进展总结"),
        icon: CalendarClock,
      },
      {
        key: "external",
        label: t("workspace.loop.templateExternal", "外部状态"),
        icon: Globe,
      },
    ],
    [t],
  )
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
  const sortedSchedules = useMemo(() => {
    const rank: Record<LoopState, number> = {
      blocked: 0,
      active: 1,
      paused: 2,
      completed: 3,
      cancelled: 4,
    }
    return [...schedules].sort((a, b) => {
      const rankDiff = rank[a.state] - rank[b.state]
      if (rankDiff !== 0) return rankDiff
      return (Date.parse(b.updatedAt) || 0) - (Date.parse(a.updatedAt) || 0)
    })
  }, [schedules])
  const visibleSchedules = showAllLoops ? sortedSchedules : sortedSchedules.slice(0, 5)
  const hiddenLoopCount = Math.max(0, sortedSchedules.length - visibleSchedules.length)
  const watchdogFindingsByLoop = useMemo(() => {
    const byLoop = new Map<string, LoopWatchdogFinding[]>()
    for (const finding of watchdogFindings) {
      const list = byLoop.get(finding.loopId)
      if (list) {
        list.push(finding)
      } else {
        byLoop.set(finding.loopId, [finding])
      }
    }
    return byLoop
  }, [watchdogFindings])
  const watchdogSchedules = useMemo(() => {
    if (watchdogFindings.length === 0) return []
    const byId = new Map(schedules.map((loop) => [loop.id, loop]))
    return watchdogFindings
      .map((finding) => ({ finding, loop: byId.get(finding.loopId) ?? null }))
      .filter((item) => item.loop !== null)
      .slice(0, 3)
  }, [schedules, watchdogFindings])

  useEffect(() => {
    if (!canUseWorkflowLoop && draftExecutionStrategy === "workflow") {
      setDraftExecutionStrategy("continue")
    }
  }, [canUseWorkflowLoop, draftExecutionStrategy])

  useEffect(() => {
    const options = loopEventStateOptions(draftEventName)
    if (!options.includes(draftEventState)) {
      setDraftEventState(options[0] ?? "completed")
    }
  }, [draftEventName, draftEventState])

  useEffect(() => {
    if (draftGoalCriterionId === GOAL_CRITERION_NONE_VALUE) return
    if (!activeGoalCriteria.some((criterion) => criterion.id === draftGoalCriterionId)) {
      setDraftGoalCriterionId(GOAL_CRITERION_NONE_VALUE)
    }
  }, [activeGoalCriteria, draftGoalCriterionId])

  useEffect(() => {
    const nextRequest = createRequest ?? 0
    if (nextRequest === lastCreateRequestRef.current) return
    lastCreateRequestRef.current = nextRequest
    if (!sessionId || incognito) return
    setCreateOpen(true)
    setDraftAdvancedOpen(false)
    setDraftKind("interval")
    if (activeGoal?.workflowTemplateId) {
      setDraftExecutionStrategy("workflow")
    }
  }, [activeGoal?.workflowTemplateId, createRequest, incognito, sessionId])

  const applyLoopTemplate = useCallback(
    (template: LoopTemplateKey) => {
      setCreateOpen(true)
      setDraftAdvancedOpen(false)
      setDraftGoalCriterionId(GOAL_CRITERION_NONE_VALUE)
      setDraftMaxRuns("")
      setDraftMaxRuntime("")
      setDraftTokens("")
      setDraftMaxNoProgress("3")
      setDraftMaxFailures("3")
      setDraftBackoff("5m")
      switch (template) {
        case "ci":
          setDraftKind("interval")
          setDraftInterval("10m")
          setDraftCondition("")
          setDraftExecutionStrategy("continue")
          setDraftPrompt(
            t(
              "workspace.loop.templateCiPrompt",
              "检查 CI 状态；如果仍失败，定位下一个失败项并继续修复。通过后总结结果。",
            ),
          )
          break
        case "report":
          setDraftKind("interval")
          setDraftInterval("30m")
          setDraftCondition("")
          setDraftExecutionStrategy(activeGoal?.workflowTemplateId ? "workflow" : "continue")
          setDraftPrompt(
            t(
              "workspace.loop.templateReportPrompt",
              "刷新研究或报告：补充来源、更新草稿、记录复核证据，并说明下一步。",
            ),
          )
          break
        case "task":
          setDraftKind("event")
          setDraftEventName("task_updated")
          setDraftEventState("completed")
          setDraftEventDebounce("30s")
          setDraftExecutionStrategy("continue")
          setDraftPrompt(
            t(
              "workspace.loop.templateTaskPrompt",
              "相关任务完成后，读取最新状态并推进下一步；如果仍缺信息，创建清晰待办。",
            ),
          )
          break
        case "summary":
          setDraftKind("interval")
          setDraftInterval("1d")
          setDraftCondition("")
          setDraftExecutionStrategy("continue")
          setDraftPrompt(
            t(
              "workspace.loop.templateSummaryPrompt",
              "总结当前目标进展、阻塞、已完成证据和下一步。",
            ),
          )
          break
        case "external":
          setDraftKind("interval")
          setDraftInterval("15m")
          setDraftCondition("")
          setDraftExecutionStrategy("continue")
          setDraftPrompt(
            t(
              "workspace.loop.templateExternalPrompt",
              "检查外部状态变化；只记录可验证结果，如需外部写操作必须等待明确审批。",
            ),
          )
          break
      }
    },
    [activeGoal?.workflowTemplateId, t],
  )

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

  const runLoopNow = useCallback(
    async (loop: LoopSchedule) => {
      setActionId(`${loop.id}:run-now`)
      try {
        await getTransport().call("run_loop_schedule_now", { loopId: loop.id })
        toast.success(t("workspace.loop.runNowStarted", "持续推进已开始立即运行"))
        refresh()
        if (detailLoopId === loop.id) {
          loadLoopDetail(loop.id)
        }
      } catch (e) {
        logger.error("ui", "LoopSchedulesSection::runNow", "Loop run-now failed", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setActionId(null)
      }
    },
    [detailLoopId, loadLoopDetail, refresh, t],
  )

  const openPolicyEditor = useCallback((loop: LoopSchedule) => {
    setPolicyLoopId((current) => (current === loop.id ? null : loop.id))
    setPolicyMaxRuns(loop.maxRuns ? String(loop.maxRuns) : "")
    setPolicyMaxRuntime(loop.maxRuntimeSecs ? formatLoopDuration(loop.maxRuntimeSecs) : "")
    setPolicyTokens(loop.tokenBudget ? String(loop.tokenBudget) : "")
    setPolicyMaxNoProgress(loop.maxNoProgressRuns ? String(loop.maxNoProgressRuns) : "3")
    setPolicyMaxFailures(loop.maxFailures ? String(loop.maxFailures) : "3")
    setPolicyBackoff(loop.backoffSecs ? formatLoopDuration(loop.backoffSecs) : "5m")
  }, [])

  const savePolicy = useCallback(
    async (loop: LoopSchedule) => {
      const maxRuns = parseOptionalPositiveInt(policyMaxRuns)
      if (policyMaxRuns.trim() && !maxRuns) {
        toast.error(t("workspace.loop.maxRunsInvalid", "最大次数必须是正整数"))
        return
      }
      const maxRuntimeSecs = policyMaxRuntime.trim()
        ? parseLoopDurationSecs(policyMaxRuntime)
        : null
      if (policyMaxRuntime.trim() && !maxRuntimeSecs) {
        toast.error(t("workspace.loop.maxRuntimeInvalid", "请输入有效最长运行时间，例如 2h"))
        return
      }
      const tokenBudget = parseOptionalPositiveInt(policyTokens)
      if (policyTokens.trim() && !tokenBudget) {
        toast.error(t("workspace.loop.tokensInvalid", "Token 预算必须是正整数"))
        return
      }
      const maxNoProgressRuns = parseOptionalPositiveInt(policyMaxNoProgress)
      if (policyMaxNoProgress.trim() && !maxNoProgressRuns) {
        toast.error(t("workspace.loop.maxNoProgressInvalid", "无进展上限必须是正整数"))
        return
      }
      const maxFailures = parseOptionalPositiveInt(policyMaxFailures)
      if (policyMaxFailures.trim() && !maxFailures) {
        toast.error(t("workspace.loop.maxFailuresInvalid", "失败上限必须是正整数"))
        return
      }
      const backoffSecs = policyBackoff.trim() ? parseLoopDurationSecs(policyBackoff) : null
      if (policyBackoff.trim() && !backoffSecs) {
        toast.error(t("workspace.loop.backoffInvalid", "请输入有效降频间隔，例如 5m"))
        return
      }
      setPolicySaving(true)
      setActionId(`${loop.id}:policy`)
      try {
        await getTransport().call("update_loop_schedule_policy", {
          loopId: loop.id,
          maxRuns,
          maxRuntimeSecs,
          tokenBudget,
          maxNoProgressRuns,
          maxFailures,
          backoffSecs,
        })
        toast.success(t("workspace.loop.policySaved", "持续推进策略已更新"))
        setPolicyLoopId(null)
        refresh()
        if (detailLoopId === loop.id) {
          loadLoopDetail(loop.id)
        }
      } catch (e) {
        logger.error("ui", "LoopSchedulesSection::policy", "Loop policy update failed", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setPolicySaving(false)
        setActionId(null)
      }
    },
    [
      detailLoopId,
      loadLoopDetail,
      policyBackoff,
      policyMaxFailures,
      policyMaxNoProgress,
      policyMaxRuns,
      policyMaxRuntime,
      policyTokens,
      refresh,
      t,
    ],
  )

  const createLoop = useCallback(async () => {
    if (!sessionId) {
      toast.error(t("workspace.loop.sessionRequired", "先选择一个会话"))
      return
    }
    const intervalSecs =
      draftKind === "interval" || draftKind === "condition"
        ? parseLoopDurationSecs(draftInterval)
        : null
    if ((draftKind === "interval" || draftKind === "condition") && !intervalSecs) {
      toast.error(t("workspace.loop.intervalInvalid", "请输入有效间隔，例如 10m"))
      return
    }
    const dynamicFallbackSecs =
      draftKind === "dynamic" ? parseLoopDurationSecs(draftDynamicFallback) : null
    if (draftKind === "dynamic" && !dynamicFallbackSecs) {
      toast.error(t("workspace.loop.dynamicFallbackInvalid", "请输入有效回退间隔，例如 20m"))
      return
    }
    const eventDebounceSecs =
      draftKind === "event" ? parseLoopDurationSecs(draftEventDebounce) : null
    if (draftKind === "event" && !eventDebounceSecs) {
      toast.error(t("workspace.loop.eventDebounceInvalid", "请输入有效事件去重窗口，例如 30s"))
      return
    }
    const condition = draftCondition.trim()
    if (draftKind === "condition" && !condition) {
      toast.error(t("workspace.loop.conditionRequired", "请输入停止条件"))
      return
    }
    const prompt = draftPrompt.trim()
    if (
      (draftKind === "interval" || draftKind === "event" || draftKind === "dynamic") &&
      !prompt &&
      !activeGoal
    ) {
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
    const maxNoProgressRuns = parseOptionalPositiveInt(draftMaxNoProgress)
    if (draftMaxNoProgress.trim() && !maxNoProgressRuns) {
      toast.error(t("workspace.loop.maxNoProgressInvalid", "无进展上限必须是正整数"))
      return
    }
    const maxFailures = parseOptionalPositiveInt(draftMaxFailures)
    if (draftMaxFailures.trim() && !maxFailures) {
      toast.error(t("workspace.loop.maxFailuresInvalid", "失败上限必须是正整数"))
      return
    }
    const backoffSecs = draftBackoff.trim() ? parseLoopDurationSecs(draftBackoff) : null
    if (draftBackoff.trim() && !backoffSecs) {
      toast.error(t("workspace.loop.backoffInvalid", "请输入有效降频间隔，例如 5m"))
      return
    }
    const defaultConditionPrompt = t(
      "workspace.loop.defaultConditionPrompt",
      "Continue until this condition is true: {{condition}}. Check the condition first, stop when it is satisfied, otherwise take the next useful step.",
      { condition },
    )
    const triggerSpec =
      draftKind === "condition"
        ? { condition, intervalSecs }
        : draftKind === "event"
          ? {
              eventName: draftEventName,
              filters: { [loopEventFilterKey(draftEventName)]: draftEventState },
              debounceSecs: eventDebounceSecs,
            }
          : draftKind === "dynamic"
            ? { fallbackSecs: dynamicFallbackSecs }
            : { intervalSecs }
    setCreateSaving(true)
    try {
      await getTransport().call("create_loop_schedule", {
        sessionId,
        prompt: draftKind === "condition" && !prompt ? defaultConditionPrompt : prompt,
        triggerKind: draftKind,
        triggerSpec,
        executionStrategy: draftExecutionStrategy,
        goalId: activeGoal?.id ?? undefined,
        goalCriterionId:
          draftGoalCriterionId === GOAL_CRITERION_NONE_VALUE ? undefined : draftGoalCriterionId,
        maxRuns,
        maxRuntimeSecs,
        tokenBudget,
        maxNoProgressRuns,
        maxFailures,
        backoffSecs,
      })
      toast.success(t("workspace.loop.created", "持续推进已创建"))
      setDraftPrompt("")
      setDraftCondition("")
      setDraftGoalCriterionId(GOAL_CRITERION_NONE_VALUE)
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
    draftDynamicFallback,
    draftEventDebounce,
    draftEventName,
    draftEventState,
    draftExecutionStrategy,
    draftGoalCriterionId,
    draftInterval,
    draftKind,
    draftBackoff,
    draftMaxFailures,
    draftMaxNoProgress,
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
      title={t("workspace.loop.title", "持续推进")}
      count={schedules.length}
      icon={Radio}
      expandSignal={(createRequest ?? 0) + (inspectRequest?.nonce ?? 0)}
      autoExpandWhen={activeCount > 0}
      meta={
        activeCount > 0 ? (
          <StatusPill
            label={t("workspace.loop.activeCount", "{{count}} 进行中", { count: activeCount })}
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
              ? t("workspace.loop.boundGoal", "绑定当前目标")
              : t("workspace.loop.promptMode", "按提示词持续推进")}
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
            {createOpen
              ? t("workspace.loop.closeCreate", "收起")
              : t("workspace.loop.new", "新建持续推进")}
          </Button>
        </div>
        {createOpen ? (
          <div className="space-y-2 rounded-md border border-border/70 bg-background/70 p-2">
            <div className="grid grid-cols-2 gap-1 sm:grid-cols-5">
              {loopTemplates.map((template) => {
                const TemplateIcon = template.icon
                return (
                  <Button
                    key={template.key}
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-8 min-w-0 justify-start gap-1.5 px-2 text-[11px]"
                    onClick={() => applyLoopTemplate(template.key)}
                  >
                    <TemplateIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate">{template.label}</span>
                  </Button>
                )
              })}
            </div>
            <div className="grid grid-cols-2 gap-1 rounded-md bg-secondary/40 p-1 sm:grid-cols-4">
              {(["interval", "dynamic", "condition", "event"] as const).map((kind) => (
                <Button
                  key={kind}
                  type="button"
                  variant={draftKind === kind ? "secondary" : "ghost"}
                  size="sm"
                  className="h-7 text-[11px]"
                  onClick={() => setDraftKind(kind)}
                >
                  {kind === "interval"
                    ? t("workspace.loop.kindInterval", "定期推进")
                    : kind === "dynamic"
                      ? t("workspace.loop.kindDynamic", "模型自定")
                      : kind === "condition"
                        ? t("workspace.loop.kindCondition", "直到条件满足")
                        : t("workspace.loop.kindEvent", "事件后继续")}
                </Button>
              ))}
            </div>
            {draftKind === "event" ? (
              <div className="grid grid-cols-3 gap-2">
                <Select
                  value={draftEventName}
                  onValueChange={(value) => setDraftEventName(value as LoopEventName)}
                >
                  <SelectTrigger className="h-8 text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="workflow:updated">
                      {t("workspace.loop.eventWorkflow", "工作流状态")}
                    </SelectItem>
                    <SelectItem value="goal:updated">
                      {t("workspace.loop.eventGoal", "目标状态")}
                    </SelectItem>
                    <SelectItem value="task_updated">
                      {t("workspace.loop.eventTask", "任务状态")}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <Select value={draftEventState} onValueChange={setDraftEventState}>
                  <SelectTrigger className="h-8 text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {loopEventStateOptions(draftEventName).map((state) => (
                      <SelectItem key={state} value={state}>
                        {state}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Input
                  value={draftEventDebounce}
                  onChange={(e) => setDraftEventDebounce(e.target.value)}
                  placeholder={t("workspace.loop.eventDebouncePlaceholder", "例如 30s")}
                  className="h-8 text-xs"
                  aria-label={t("workspace.loop.eventDebounce", "事件去重窗口")}
                />
              </div>
            ) : draftKind === "dynamic" ? (
              <Input
                value={draftDynamicFallback}
                onChange={(e) => setDraftDynamicFallback(e.target.value)}
                placeholder="20m"
                className="h-8 text-xs"
                aria-label={t("workspace.loop.dynamicFallback", "回退间隔")}
              />
            ) : (
              <Input
                value={draftInterval}
                onChange={(e) => setDraftInterval(e.target.value)}
                placeholder="10m"
                className="h-8 text-xs"
                aria-label={t("workspace.loop.interval", "触发间隔")}
              />
            )}
            {draftKind === "condition" ? (
              <Input
                value={draftCondition}
                onChange={(e) => setDraftCondition(e.target.value)}
                placeholder={t("workspace.loop.conditionPlaceholder", "例如 CI 已通过")}
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
                    ? t("workspace.loop.promptGoalPlaceholder", "留空则继续当前目标")
                    : t("workspace.loop.promptPlaceholder", "检查状态并推进下一步")
              }
              className="min-h-[64px] resize-none text-xs"
              aria-label={t("workspace.loop.prompt", "每次推进内容")}
            />
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
                      ? t("workspace.loop.strategyWorkflow", "按工作流执行")
                      : t("workspace.loop.strategyContinue", "继续当前对话")}
                  </Button>
                )
              })}
            </div>
            {activeGoal && activeGoalCriteria.length > 0 ? (
              <Select value={draftGoalCriterionId} onValueChange={setDraftGoalCriterionId}>
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={GOAL_CRITERION_NONE_VALUE}>
                    {t("workspace.goal.wholeGoal", "整个目标")}
                  </SelectItem>
                  {activeGoalCriteria.map((criterion) => (
                    <SelectItem key={criterion.id} value={criterion.id}>
                      {goalCriterionOptionLabel(criterion)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 w-full justify-between px-2 text-[11px] text-muted-foreground"
              onClick={() => setDraftAdvancedOpen((value) => !value)}
            >
              <span>{t("workspace.loop.advancedProtection", "高级保护")}</span>
              {draftAdvancedOpen ? (
                <ChevronUp className="h-3.5 w-3.5" />
              ) : (
                <ChevronDown className="h-3.5 w-3.5" />
              )}
            </Button>
            <AnimatedCollapse open={draftAdvancedOpen}>
              <div className="space-y-2 rounded-md bg-secondary/15 p-2">
                <div className="grid grid-cols-3 gap-2">
                  <Input
                    value={draftMaxRuns}
                    onChange={(e) => setDraftMaxRuns(e.target.value)}
                    placeholder={t("workspace.loop.maxRunsPlaceholder", "最大次数")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.maxRunsLabel", "最大次数")}
                  />
                  <Input
                    value={draftMaxRuntime}
                    onChange={(e) => setDraftMaxRuntime(e.target.value)}
                    placeholder={t("workspace.loop.maxRuntimePlaceholder", "最长时间")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.maxRuntimeLabel", "最长运行时间")}
                  />
                  <Input
                    value={draftTokens}
                    onChange={(e) => setDraftTokens(e.target.value)}
                    placeholder={t("workspace.loop.tokensPlaceholder", "Token 预算")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.tokensLabel", "Token 预算")}
                  />
                </div>
                <div className="grid grid-cols-3 gap-2">
                  <Input
                    value={draftMaxNoProgress}
                    onChange={(e) => setDraftMaxNoProgress(e.target.value)}
                    placeholder={t("workspace.loop.maxNoProgressPlaceholder", "无进展次数")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.maxNoProgressLabel", "无进展上限")}
                  />
                  <Input
                    value={draftMaxFailures}
                    onChange={(e) => setDraftMaxFailures(e.target.value)}
                    placeholder={t("workspace.loop.maxFailuresPlaceholder", "失败次数")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.maxFailuresLabel", "失败上限")}
                  />
                  <Input
                    value={draftBackoff}
                    onChange={(e) => setDraftBackoff(e.target.value)}
                    placeholder={t("workspace.loop.backoffPlaceholder", "降频间隔")}
                    className="h-8 text-xs"
                    aria-label={t("workspace.loop.backoffLabel", "降频间隔")}
                  />
                </div>
              </div>
            </AnimatedCollapse>
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
                {t("workspace.loop.create", "创建持续推进")}
              </Button>
            </div>
          </div>
        ) : null}
      </div>
      {watchdogSchedules.length > 0 ? (
        <div className="mb-2 space-y-1 rounded-md border border-amber-500/25 bg-amber-500/10 p-2 text-[11px] text-amber-800 dark:text-amber-200">
          <div className="flex items-center gap-1.5 font-medium">
            <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
            {t("workspace.loop.watchdogTitle", "有持续推进需要确认")}
          </div>
          <div className="space-y-1">
            {watchdogSchedules.map(({ finding, loop }) => {
              if (!loop) return null
              const isBusy = actionId?.startsWith(`${loop.id}:`)
              return (
                <div
                  key={`${finding.loopId}:${finding.code}`}
                  className="flex min-w-0 items-center gap-2 rounded-md bg-background/55 px-2 py-1.5"
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium text-foreground/85">
                      {loop.prompt || loopTriggerSummary(t, loop.triggerKind, loop.triggerSpec)}
                    </div>
                    <div className="truncate text-muted-foreground">
                      {loopWatchdogFindingLabel(t, finding)}
                    </div>
                  </div>
                  {loop.state === "active" ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 shrink-0 gap-1 px-2 text-[11px]"
                      disabled={isBusy}
                      onClick={() => void runLoopNow(loop)}
                    >
                      <RefreshCw className="h-3.5 w-3.5" />
                      {t("workspace.loop.runNow", "立即运行")}
                    </Button>
                  ) : null}
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 shrink-0 gap-1 px-2 text-[11px]"
                    onClick={() => toggleLoopDetail(loop.id)}
                  >
                    <Clock className="h-3.5 w-3.5" />
                    {t("workspace.loop.history", "运行记录")}
                  </Button>
                </div>
              )
            })}
          </div>
          {watchdogFindings.length > watchdogSchedules.length ? (
            <div className="px-1 text-[10px] text-muted-foreground">
              {t("workspace.loop.watchdogMore", "还有 {{count}} 条诊断在运行记录中可查看", {
                count: watchdogFindings.length - watchdogSchedules.length,
              })}
            </div>
          ) : null}
        </div>
      ) : null}
      {loading && schedules.length === 0 ? (
        <div className="flex items-center justify-center py-4 text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
        </div>
      ) : error ? (
        <EmptyHint>{error}</EmptyHint>
      ) : schedules.length === 0 ? (
        <EmptyHint>
          {incognito
            ? t("workspace.loop.incognitoEmpty", "无痕会话不保存持续推进")
            : t("workspace.loop.empty", "暂无持续推进")}
        </EmptyHint>
      ) : (
        <div className="space-y-2">
          {visibleSchedules.map((loop, index) => {
            const isBusy = actionId?.startsWith(`${loop.id}:`)
            const derivedWorkflowRuns = workflowRunsByLoop.get(loop.id) ?? []
            const latestWorkflowRun = derivedWorkflowRuns[0]
            const detailOpen = detailLoopId === loop.id
            const policyOpen = policyLoopId === loop.id
            const loopWatchdogFindings = watchdogFindingsByLoop.get(loop.id) ?? []
            const progressLabel = loopProgressLabel(t, loop.progressState)
            const nextRunLabel = loopNextRunLabel(t, loop)
            const story = loopScheduleStory(t, loop, nextRunLabel, latestWorkflowRun)
            const showGroup = index === 0 || visibleSchedules[index - 1]?.state !== loop.state
            return (
              <div key={loop.id} className="space-y-1">
                {showGroup ? (
                  <div className="px-1 pt-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    {loopGroupLabel(t, loop.state)}
                  </div>
                ) : null}
                <div className="rounded-md border border-border/70 bg-background/70 px-2.5 py-2">
                  <div className="flex items-start gap-2">
                    <div className="min-w-0 flex-1">
                      <div className="flex min-w-0 items-center gap-2">
                        <StatusPill
                          label={loopStateLabel(t, loop.state)}
                          tone={loopStateTone(loop.state)}
                        />
                        {progressLabel ? (
                          <StatusPill
                            label={progressLabel}
                            tone={loopProgressTone(loop.progressState)}
                          />
                        ) : null}
                        {loop.executionStrategy === "workflow" ? (
                          <StatusPill
                            label={loopExecutionStrategyLabel(t, loop.executionStrategy)}
                            tone="info"
                          />
                        ) : null}
                        {loopWatchdogFindings.length > 0 ? (
                          <StatusPill
                            label={t("workspace.loop.watchdogPill", "需确认")}
                            tone="warn"
                          />
                        ) : null}
                        {loop.goalCriterionId ? (
                          <StatusPill
                            label={loop.goalCriterionText ?? loop.goalCriterionId}
                            tone="info"
                          />
                        ) : null}
                        <span className="truncate text-xs font-medium text-foreground">
                          {loopTriggerSummary(t, loop.triggerKind, loop.triggerSpec)}
                        </span>
                      </div>
                      <p className="mt-1 line-clamp-2 text-xs text-foreground/80">{story}</p>
                      <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                        {loop.prompt}
                      </p>
                      <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[10px] text-muted-foreground">
                        <span>{loopGuardStory(t, loop)}</span>
                        {loop.maxRuntimeSecs ? (
                          <span>
                            {t("workspace.loop.maxRuntime", "最长")}{" "}
                            {formatLoopDuration(loop.maxRuntimeSecs)}
                          </span>
                        ) : null}
                        {loop.tokenBudget ? (
                          <span>
                            {t("workspace.loop.tokenBudget", "Token")} {loop.tokenBudget}
                          </span>
                        ) : null}
                        <span>{formatMessageTime(loop.updatedAt)}</span>
                      </div>
                      {loop.progressSummary ? (
                        <p className="mt-1 line-clamp-2 text-[10px] text-muted-foreground">
                          {loop.progressSummary}
                        </p>
                      ) : null}
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
                              {t(
                                "workspace.loop.workflowPending",
                                "等待下一次触发创建 Workflow run",
                              )}
                            </span>
                          </div>
                        )
                      ) : null}
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {loop.state === "active" ? (
                        <IconTip label={t("workspace.loop.runNow", "立即运行")}>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7"
                            disabled={isBusy}
                            aria-label={t("workspace.loop.runNow", "立即运行")}
                            onClick={() => void runLoopNow(loop)}
                          >
                            <RefreshCw className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                      ) : null}
                      {!isLoopTerminal(loop.state) ? (
                        <IconTip label={t("workspace.loop.editPolicy", "编辑策略")}>
                          <Button
                            variant={policyOpen ? "secondary" : "ghost"}
                            size="icon"
                            className="h-7 w-7"
                            disabled={isBusy}
                            aria-label={t("workspace.loop.editPolicy", "编辑策略")}
                            onClick={() => openPolicyEditor(loop)}
                          >
                            <Pencil className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                      ) : null}
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
                  {policyOpen ? (
                    <div className="mt-2 rounded-md border border-border/60 bg-secondary/15 p-2">
                      <div className="mb-1 flex items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
                        <Gauge className="h-3 w-3" />
                        {t("workspace.loop.policyTitle", "运行策略")}
                      </div>
                      <div className="grid grid-cols-3 gap-2">
                        <Input
                          value={policyMaxRuns}
                          onChange={(e) => setPolicyMaxRuns(e.target.value)}
                          placeholder={t("workspace.loop.maxRunsPlaceholder", "最大次数")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.maxRunsLabel", "最大次数")}
                        />
                        <Input
                          value={policyMaxRuntime}
                          onChange={(e) => setPolicyMaxRuntime(e.target.value)}
                          placeholder={t("workspace.loop.maxRuntimePlaceholder", "最长时间")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.maxRuntimeLabel", "最长运行时间")}
                        />
                        <Input
                          value={policyTokens}
                          onChange={(e) => setPolicyTokens(e.target.value)}
                          placeholder={t("workspace.loop.tokensPlaceholder", "Token 预算")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.tokensLabel", "Token 预算")}
                        />
                      </div>
                      <div className="mt-2 grid grid-cols-3 gap-2">
                        <Input
                          value={policyMaxNoProgress}
                          onChange={(e) => setPolicyMaxNoProgress(e.target.value)}
                          placeholder={t("workspace.loop.maxNoProgressPlaceholder", "无进展次数")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.maxNoProgressLabel", "无进展上限")}
                        />
                        <Input
                          value={policyMaxFailures}
                          onChange={(e) => setPolicyMaxFailures(e.target.value)}
                          placeholder={t("workspace.loop.maxFailuresPlaceholder", "失败次数")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.maxFailuresLabel", "失败上限")}
                        />
                        <Input
                          value={policyBackoff}
                          onChange={(e) => setPolicyBackoff(e.target.value)}
                          placeholder={t("workspace.loop.backoffPlaceholder", "降频间隔")}
                          className="h-8 text-xs"
                          aria-label={t("workspace.loop.backoffLabel", "降频间隔")}
                        />
                      </div>
                      <div className="mt-2 flex justify-end gap-1">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-7 px-2 text-[11px]"
                          onClick={() => setPolicyLoopId(null)}
                        >
                          {t("common.cancel", "取消")}
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          className="h-7 gap-1 px-2 text-[11px]"
                          disabled={policySaving}
                          onClick={() => void savePolicy(loop)}
                        >
                          {policySaving ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Check className="h-3.5 w-3.5" />
                          )}
                          {t("common.save", "保存")}
                        </Button>
                      </div>
                    </div>
                  ) : null}
                  {detailOpen ? (
                    <LoopRunHistory
                      snapshot={detailSnapshot}
                      loading={detailLoading}
                      error={detailError}
                      onSelectWorkflowRun={onSelectWorkflowRun}
                    />
                  ) : null}
                </div>
              </div>
            )
          })}
          {hiddenLoopCount > 0 ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 w-full gap-1 text-[11px] text-muted-foreground"
              onClick={() => setShowAllLoops(true)}
            >
              <ChevronDown className="h-3.5 w-3.5" />
              {t("workspace.loop.viewMore", "查看更多持续推进（{{count}}）", {
                count: hiddenLoopCount,
              })}
            </Button>
          ) : showAllLoops && sortedSchedules.length > 5 ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 w-full gap-1 text-[11px] text-muted-foreground"
              onClick={() => setShowAllLoops(false)}
            >
              <ChevronUp className="h-3.5 w-3.5" />
              {t("workspace.loop.viewLess", "收起持续推进")}
            </Button>
          ) : null}
        </div>
      )}
    </WorkspaceSection>
  )
}

function WorkflowRunsSection({
  sessionId,
  projectId,
  incognito,
  turnActive,
  workingDir,
  onEnsureSession,
  draftWorkflowMode = "off",
  onDraftWorkflowModeChange,
  onViewSubagentSession,
  workflowRunsState,
  goalState,
  focusedRunTarget,
}: {
  sessionId?: string | null
  projectId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  onEnsureSession?: () => Promise<string | null>
  draftWorkflowMode?: WorkflowAutonomyMode
  onDraftWorkflowModeChange?: (mode: WorkflowAutonomyMode) => void
  onViewSubagentSession?: (sessionId: string) => void
  workflowRunsState?: WorkflowRunsState
  goalState: GoalStateSnapshot
  focusedRunTarget?: WorkflowFocusTarget | null
}) {
  const { t } = useTranslation()
  const ownedWorkflowRuns = useWorkflowRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(workflowRunsState),
  })
  const {
    runs,
    watchdogFindings = [],
    activeCount,
    loading,
    error,
    refresh,
  } = workflowRunsState ?? ownedWorkflowRuns
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
  const [draftGoalCriterionId, setDraftGoalCriterionId] = useState(GOAL_CRITERION_NONE_VALUE)
  const [domainTemplates, setDomainTemplates] = useState<DomainWorkflowTemplate[]>([])
  const [domainTemplatesLoading, setDomainTemplatesLoading] = useState(false)
  const [domainTemplatesError, setDomainTemplatesError] = useState<string | null>(null)
  const [savedTemplates, setSavedTemplates] = useState<SavedWorkflowTemplate[]>([])
  const [savedTemplatesLoading, setSavedTemplatesLoading] = useState(false)
  const [savedTemplatesError, setSavedTemplatesError] = useState<string | null>(null)
  const [savingTemplateRunId, setSavingTemplateRunId] = useState<string | null>(null)
  const [applyingTemplateId, setApplyingTemplateId] = useState<string | null>(null)
  const [selectedDomainTemplateId, setSelectedDomainTemplateId] = useState("")
  const [selectedDomainTaskType, setSelectedDomainTaskType] = useState("")
  const [domainDraft, setDomainDraft] = useState<DomainWorkflowDraft | null>(null)
  const [domainDraftLoading, setDomainDraftLoading] = useState(false)
  const [showAllRuns, setShowAllRuns] = useState(false)
  const [pendingCancelRun, setPendingCancelRun] = useState<WorkflowRun | null>(null)
  const [creatingRepairTaskRunId, setCreatingRepairTaskRunId] = useState<string | null>(null)
  const snapshotReqRef = useRef(0)
  const workflowModeReqRef = useRef(0)
  const executionModeReqRef = useRef(0)
  const previewReqRef = useRef(0)
  const domainTemplatesReqRef = useRef(0)
  const domainTemplatesRequestedRef = useRef(false)
  const savedTemplatesReqRef = useRef(0)
  const savedTemplatesRequestedRef = useRef(false)
  const domainDraftReqRef = useRef(0)
  const autoDetailTabRunRef = useRef<string | null>(null)
  const ensureSessionRef = useRef<Promise<string | null> | null>(null)

  const selectedRun = runs.find((run) => run.id === selectedRunId) ?? null
  const visibleRuns = showAllRuns ? runs : runs.slice(0, WORKFLOW_RUN_PREVIEW)
  const watchdogFindingsByRun = useMemo(() => {
    const byRun = new Map<string, WorkflowWatchdogFinding[]>()
    for (const finding of watchdogFindings) {
      const list = byRun.get(finding.runId)
      if (list) {
        list.push(finding)
      } else {
        byRun.set(finding.runId, [finding])
      }
    }
    return byRun
  }, [watchdogFindings])
  const watchdogRuns = useMemo(() => {
    if (watchdogFindings.length === 0) return []
    const byId = new Map(runs.map((run) => [run.id, run]))
    return watchdogFindings
      .map((finding) => ({ finding, run: byId.get(finding.runId) ?? null }))
      .filter((item) => item.run !== null)
      .slice(0, 3)
  }, [runs, watchdogFindings])
  const canMaterializeSession = Boolean(sessionId || onEnsureSession)
  const activeGoal = goalState.snapshot?.goal ?? null
  const activeGoalId = activeGoal?.id ?? null
  const activeGoalTemplateValue = activeGoal
    ? goalDomainTemplateValue(activeGoal)
    : GOAL_DOMAIN_FREE_VALUE
  const activeGoalWorkflowTaskType = activeGoal?.workflowTaskType ?? ""
  const activeGoalCriteria = useMemo(
    () => goalState.snapshot?.criteriaItems ?? [],
    [goalState.snapshot?.criteriaItems],
  )
  const selectedDomainTemplate = findDomainTemplateByValue(
    domainTemplates,
    selectedDomainTemplateId,
  )
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

  useEffect(() => {
    if (draftGoalCriterionId === GOAL_CRITERION_NONE_VALUE) return
    if (!activeGoalCriteria.some((criterion) => criterion.id === draftGoalCriterionId)) {
      setDraftGoalCriterionId(GOAL_CRITERION_NONE_VALUE)
    }
  }, [activeGoalCriteria, draftGoalCriterionId])

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

  const loadSavedWorkflowTemplates = useCallback(() => {
    if (incognito) {
      savedTemplatesReqRef.current += 1
      savedTemplatesRequestedRef.current = false
      setSavedTemplates([])
      setSavedTemplatesLoading(false)
      setSavedTemplatesError(null)
      return
    }
    savedTemplatesRequestedRef.current = true
    const req = ++savedTemplatesReqRef.current
    setSavedTemplatesLoading(true)
    setSavedTemplatesError(null)
    getTransport()
      .call<SavedWorkflowTemplate[]>("list_saved_workflow_templates", {
        projectId: projectId ?? undefined,
        limit: 24,
      })
      .then((next) => {
        if (savedTemplatesReqRef.current !== req) return
        setSavedTemplates(Array.isArray(next) ? next.filter((template) => template.enabled) : [])
        setSavedTemplatesLoading(false)
      })
      .catch((e) => {
        if (savedTemplatesReqRef.current !== req) return
        logger.error(
          "ui",
          "WorkflowRunsSection::loadSavedWorkflowTemplates",
          "Failed to load saved workflow templates",
          e,
        )
        setSavedTemplates([])
        setSavedTemplatesError(e instanceof Error ? e.message : String(e))
        setSavedTemplatesLoading(false)
      })
  }, [incognito, projectId])

  useEffect(() => {
    if (!createOpen || incognito || domainTemplatesRequestedRef.current) return
    loadDomainWorkflowTemplates()
  }, [createOpen, incognito, loadDomainWorkflowTemplates])

  useEffect(() => {
    if (!createOpen || incognito || savedTemplatesRequestedRef.current) return
    loadSavedWorkflowTemplates()
  }, [createOpen, incognito, loadSavedWorkflowTemplates])

  useEffect(() => {
    savedTemplatesRequestedRef.current = false
  }, [projectId])

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
    if (
      activeGoalTemplateValue === GOAL_DOMAIN_FREE_VALUE ||
      domainTemplates.length === 0 ||
      domainDraft
    ) {
      return
    }
    const template = findDomainTemplateByValue(domainTemplates, activeGoalTemplateValue)
    if (!template) return
    const templateValue = domainTemplateOptionValue(template)
    if (selectedDomainTemplateId === templateValue) return
    setSelectedDomainTemplateId(templateValue)
    setSelectedDomainTaskType(activeGoalWorkflowTaskType || template.taskTypes[0] || "")
    setDraftKind(`domain:${template.domain}`)
    setDraftMode(normalizeExecutionMode(template.defaultMode))
  }, [
    activeGoalTemplateValue,
    activeGoalWorkflowTaskType,
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

  const loadWorkflowMode = useCallback(() => {
    if (!sessionId || incognito) {
      workflowModeReqRef.current += 1
      setWorkflowMode(incognito ? "off" : normalizeWorkflowAutonomyMode(draftWorkflowMode))
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
  }, [draftWorkflowMode, incognito, sessionId])

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
      if (!sessionId) {
        setWorkflowMode(nextMode)
        onDraftWorkflowModeChange?.(nextMode)
        toast.success(
          nextMode === "off"
            ? t("workspace.workflow.modeDraftOff", "工作流模式已关闭")
            : t("workspace.workflow.modeDraftSaved", "工作流模式已开启：{{mode}}", {
                mode: workflowAutonomyModeLabel(t, nextMode),
              }),
        )
        return
      }
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
          saved === "off"
            ? t("workspace.workflow.modeDraftOff", "工作流模式已关闭")
            : t("workspace.workflow.modeSaved", "工作流模式已开启：{{mode}}", {
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
    [
      ensureWorkflowSession,
      incognito,
      onDraftWorkflowModeChange,
      sessionId,
      t,
      workflowMode,
      workflowModeSaving,
    ],
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
          t("workspace.workflow.executionModeSaved", "执行模式已切换为 {{mode}}", {
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
      setDraftGoalCriterionId(run.goalCriterionId ?? GOAL_CRITERION_NONE_VALUE)
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

  const createRepairTask = useCallback(
    async (repairPrompt: string, run: WorkflowRun) => {
      if (incognito || creatingRepairTaskRunId) return
      setCreatingRepairTaskRunId(run.id)
      try {
        const targetSessionId = await ensureWorkflowSession()
        if (!targetSessionId) return
        await getTransport().call<Task[]>("create_session_task", {
          sessionId: targetSessionId,
          content: t(
            "workspace.workflow.repairTaskContent",
            "修复失败工作流 {{id}}：\n\n{{prompt}}",
            { id: run.id, prompt: repairPrompt },
          ),
          activeForm: t("workspace.workflow.repairTaskActiveForm", "正在修复失败工作流 {{id}}", {
            id: run.id,
          }),
        })
        toast.success(t("workspace.workflow.repairTaskCreated", "已创建工作流修复任务"))
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::createRepairTask",
          "Failed to create repair task",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setCreatingRepairTaskRunId(null)
      }
    },
    [creatingRepairTaskRunId, ensureWorkflowSession, incognito, t],
  )

  const applySavedTemplateToDraft = useCallback(
    (template: SavedWorkflowTemplate) => {
      setCreateOpen(true)
      setDraftKind(template.kind)
      setDraftMode(normalizeExecutionMode(template.executionMode))
      setDraftScript(template.scriptSource)
      setDraftObjective("")
      setDraftOrigin(null)
      setDraftRunImmediately(Boolean(workingDir))
      clearDomainDraft()
      clearDraftPreview()
      toast.success(
        t("workspace.workflow.savedTemplateApplied", "已载入模板：{{name}}", {
          name: template.name,
        }),
      )
    },
    [clearDomainDraft, clearDraftPreview, t, workingDir],
  )

  const createWorkflowFromSavedTemplate = useCallback(
    async (template: SavedWorkflowTemplate, runImmediately: boolean) => {
      if (incognito || applyingTemplateId) return
      const targetSessionId = await ensureWorkflowSession()
      if (!targetSessionId) return
      const mode = normalizeExecutionMode(template.executionMode)
      const canRunNow = Boolean(workingDir) && runImmediately
      setApplyingTemplateId(template.id)
      try {
        const run = await getTransport().call<WorkflowRun>("create_workflow_run_from_template", {
          input: {
            sessionId: targetSessionId,
            templateId: template.id,
            goalId: activeGoal?.id ?? undefined,
            goalCriterionId:
              activeGoal && draftGoalCriterionId !== GOAL_CRITERION_NONE_VALUE
                ? draftGoalCriterionId
                : undefined,
            budget: workflowBudgetForMode(mode),
          },
          runImmediately: canRunNow,
        })
        setSelectedRunId(run.id)
        loadSnapshot(run.id)
        refresh()
        toast.success(
          canRunNow
            ? t("workspace.workflow.savedTemplateCreatedAndStarted", "已从模板创建并请求启动")
            : t("workspace.workflow.savedTemplateCreated", "已从模板创建工作流"),
        )
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::createWorkflowFromSavedTemplate",
          "Failed to create workflow from saved template",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setApplyingTemplateId(null)
      }
    },
    [
      activeGoal,
      applyingTemplateId,
      draftGoalCriterionId,
      ensureWorkflowSession,
      incognito,
      loadSnapshot,
      refresh,
      t,
      workingDir,
    ],
  )

  const saveWorkflowTemplate = useCallback(
    async (run: WorkflowRun) => {
      if (incognito || savingTemplateRunId) return
      setSavingTemplateRunId(run.id)
      try {
        const scope = projectId ? "project" : "user"
        const template = await getTransport().call<SavedWorkflowTemplate>(
          "save_workflow_template_from_run",
          {
            input: {
              sourceRunId: run.id,
              name: run.kind || t("workspace.workflow.savedTemplateDefaultName", "工作流模板"),
              description: run.goalCriterionText ?? undefined,
              scope,
              projectId: scope === "project" ? projectId : undefined,
              explicitSaveConsent: true,
            },
          },
        )
        savedTemplatesRequestedRef.current = false
        loadSavedWorkflowTemplates()
        toast.success(
          t("workspace.workflow.savedTemplateSaved", "已保存模板：{{name}}", {
            name: template.name,
          }),
        )
      } catch (e) {
        logger.error(
          "ui",
          "WorkflowRunsSection::saveWorkflowTemplate",
          "Failed to save workflow template",
          e,
        )
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setSavingTemplateRunId(null)
      }
    },
    [incognito, loadSavedWorkflowTemplates, projectId, savingTemplateRunId, t],
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
        goalId: activeGoalId ?? undefined,
        goalCriterionId:
          activeGoalId && draftGoalCriterionId !== GOAL_CRITERION_NONE_VALUE
            ? draftGoalCriterionId
            : undefined,
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
      setDraftGoalCriterionId(GOAL_CRITERION_NONE_VALUE)
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
    draftGoalCriterionId,
    draftPreview?.canCreate,
    draftRunImmediately,
    draftScript,
    ensureWorkflowSession,
    activeGoalId,
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
    const canSaveTemplate = run.state === "completed" && !incognito
    if (actions.length === 0 && !canSaveTemplate) return null
    return (
      <div className="flex shrink-0 items-center gap-1">
        {canSaveTemplate ? (
          <IconTip label={t("workspace.workflow.saveTemplate", "保存模板")}>
            <button
              type="button"
              className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border/50 text-muted-foreground transition-colors hover:bg-secondary/65 hover:text-foreground disabled:opacity-50"
              disabled={!!actionKey || !!savingTemplateRunId}
              onClick={(e) => {
                e.stopPropagation()
                void saveWorkflowTemplate(run)
              }}
              aria-label={t("workspace.workflow.saveTemplate", "保存模板")}
            >
              {savingTemplateRunId === run.id ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <BookText className="h-3 w-3" />
              )}
            </button>
          </IconTip>
        ) : null}
        {actions.map((action) => {
          const Icon = action.icon
          const key = `${action.command}:${run.id}`
          return (
            <IconTip key={action.command} label={action.label}>
              <button
                type="button"
                className={cn(
                  "inline-flex h-6 w-6 items-center justify-center rounded-md border border-border/50 text-muted-foreground transition-colors hover:bg-secondary/65 hover:text-foreground disabled:opacity-50",
                  action.danger && "hover:bg-destructive/10 hover:text-destructive",
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
    const canSaveTemplate = run.state === "completed" && !incognito
    if (actions.length === 0 && !canSaveTemplate) return null
    return (
      <div className="grid grid-cols-2 gap-1.5">
        {canSaveTemplate ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            className={cn("h-8 min-w-0 gap-1.5 text-xs", actions.length === 0 && "col-span-2")}
            disabled={!!actionKey || !!savingTemplateRunId}
            onClick={() => void saveWorkflowTemplate(run)}
          >
            {savingTemplateRunId === run.id ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <BookText className="h-3.5 w-3.5" />
            )}
            <span className="truncate">{t("workspace.workflow.saveTemplate", "保存模板")}</span>
          </Button>
        ) : null}
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
                actions.length === 1 && !canSaveTemplate && "col-span-2",
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
        count={runs.length}
        icon={GitPullRequest}
        defaultExpanded={
          activeCount > 0 ||
          runs.length > 0 ||
          workflowMode !== "off" ||
          watchdogFindings.length > 0
        }
        autoExpandWhen={
          activeCount > 0 ||
          runs.length > 0 ||
          workflowMode !== "off" ||
          watchdogFindings.length > 0
        }
        meta={
          loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
          ) : activeCount > 0 ? (
            <StatusPill
              label={t("workspace.workflow.activeCount", "{{count}} 运行中", {
                count: activeCount,
              })}
              tone="info"
            />
          ) : null
        }
      >
        {incognito ? (
          <EmptyHint>{t("workspace.workflow.incognito", "无痕会话不持久化工作流")}</EmptyHint>
        ) : (
          <div className="space-y-2">
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
              goalCriteria={activeGoalCriteria}
              selectedGoalCriterionId={draftGoalCriterionId}
              domainTemplates={domainTemplates}
              domainTemplatesLoading={domainTemplatesLoading}
              domainTemplatesError={domainTemplatesError}
              selectedDomainTemplate={selectedDomainTemplate}
              selectedDomainTaskType={selectedDomainTaskType}
              domainDraft={domainDraft}
              domainDraftLoading={domainDraftLoading}
              savedTemplates={savedTemplates}
              savedTemplatesLoading={savedTemplatesLoading}
              savedTemplatesError={savedTemplatesError}
              applyingTemplateId={applyingTemplateId}
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
              onGoalCriterionChange={setDraftGoalCriterionId}
              onReloadDomainTemplates={loadDomainWorkflowTemplates}
              onDomainTemplateChange={selectDomainTemplate}
              onDomainTaskTypeChange={selectDomainTaskType}
              onGenerateDomainDraft={() => void generateDomainWorkflowDraft()}
              onReloadSavedTemplates={loadSavedWorkflowTemplates}
              onApplySavedTemplate={applySavedTemplateToDraft}
              onCreateFromSavedTemplate={(template) =>
                void createWorkflowFromSavedTemplate(template, draftRunImmediately)
              }
              onRunImmediatelyChange={setDraftRunImmediately}
              onWorktreeModeChange={setDraftWorktreeMode}
              onGenerateGoalDraft={generateGoalDrivenDraft}
              onPreview={() => void previewWorkflowDraft()}
              onSubmit={() => void createWorkflow()}
            />

            {watchdogRuns.length > 0 ? (
              <div className="space-y-1 rounded-md border border-amber-500/25 bg-amber-500/10 p-2 text-[11px] text-amber-800 dark:text-amber-200">
                <div className="flex items-center gap-1.5 font-medium">
                  <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
                  {t("workspace.workflow.watchdogTitle", "有工作流需要确认")}
                </div>
                <div className="space-y-1">
                  {watchdogRuns.map(({ finding, run }) => {
                    if (!run) return null
                    return (
                      <div
                        key={`${finding.runId}:${finding.code}`}
                        className="flex min-w-0 items-center gap-2 rounded-md bg-background/55 px-2 py-1.5"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-medium text-foreground/85">{run.kind}</div>
                          <div className="truncate text-muted-foreground">
                            {workflowWatchdogFindingLabel(t, finding)}
                          </div>
                        </div>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-7 shrink-0 gap-1 px-2 text-[11px]"
                          onClick={() => {
                            setSelectedRunId(run.id)
                            if (
                              !runs
                                .slice(0, WORKFLOW_RUN_PREVIEW)
                                .some((item) => item.id === run.id)
                            ) {
                              setShowAllRuns(true)
                            }
                          }}
                        >
                          <Eye className="h-3.5 w-3.5" />
                          {t("workspace.workflow.viewDetails", "查看详情")}
                        </Button>
                      </div>
                    )
                  })}
                </div>
                {watchdogFindings.length > watchdogRuns.length ? (
                  <div className="px-1 text-[10px] text-muted-foreground">
                    {t(
                      "workspace.workflow.watchdogMore",
                      "还有 {{count}} 条诊断在运行详情中可查看",
                      {
                        count: watchdogFindings.length - watchdogRuns.length,
                      },
                    )}
                  </div>
                ) : null}
              </div>
            ) : null}

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
                    const runWatchdogFindings = watchdogFindingsByRun.get(run.id) ?? []
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
                                  {t("workspace.workflow.worktreeBadge", "工作树")}
                                </span>
                              </>
                            ) : null}
                            {run.goalCriterionId ? (
                              <>
                                <span className="px-1 text-muted-foreground/50">·</span>
                                <span className="text-muted-foreground">
                                  {run.goalCriterionText ?? run.goalCriterionId}
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
                          {runWatchdogFindings.length > 0 ? (
                            <StatusPill
                              label={t("workspace.workflow.watchdogNeedsAttention", "需确认")}
                              tone="warn"
                            />
                          ) : null}
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
                      onCreateRepairTask={createRepairTask}
                      creatingRepairTask={creatingRepairTaskRunId === detailRun.id}
                    />

                    {snapshotLoading ? (
                      <div className="flex items-center justify-center gap-2 py-2 text-xs text-muted-foreground">
                        <Loader2 className="h-3 w-3 animate-spin" />
                        {t("workspace.workflow.loadingTrace", "加载轨迹")}
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
                              {t("workspace.workflow.tabTrace", "轨迹")}
                            </TabsTrigger>
                            <TabsTrigger value="validation" className="text-[11px]">
                              {t("workspace.workflow.tabValidation", "验证")}
                              {validationCount > 0 ? (
                                <span className="ml-1 text-[10px] text-muted-foreground">
                                  {validationCount}
                                </span>
                              ) : null}
                            </TabsTrigger>
                            <TabsTrigger value="agents" className="text-[11px]">
                              {t("workspace.workflow.tabAgents", "子 Agent")}
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
                      <EmptyHint>{t("workspace.workflow.emptyTrace", "暂无轨迹")}</EmptyHint>
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
                "会停止这个运行，并尽量取消它拥有的后台任务、验证命令和子 Agent；已有轨迹会保留，方便之后复盘或生成修复草稿。",
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
  goalCriteria,
  selectedGoalCriterionId,
  domainTemplates,
  domainTemplatesLoading,
  domainTemplatesError,
  selectedDomainTemplate,
  selectedDomainTaskType,
  domainDraft,
  domainDraftLoading,
  savedTemplates,
  savedTemplatesLoading,
  savedTemplatesError,
  applyingTemplateId,
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
  onGoalCriterionChange,
  onReloadDomainTemplates,
  onDomainTemplateChange,
  onDomainTaskTypeChange,
  onGenerateDomainDraft,
  onReloadSavedTemplates,
  onApplySavedTemplate,
  onCreateFromSavedTemplate,
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
  goalCriteria: GoalCriterionItem[]
  selectedGoalCriterionId: string
  domainTemplates: DomainWorkflowTemplate[]
  domainTemplatesLoading?: boolean
  domainTemplatesError?: string | null
  selectedDomainTemplate?: DomainWorkflowTemplate | null
  selectedDomainTaskType: string
  domainDraft?: DomainWorkflowDraft | null
  domainDraftLoading?: boolean
  savedTemplates: SavedWorkflowTemplate[]
  savedTemplatesLoading?: boolean
  savedTemplatesError?: string | null
  applyingTemplateId?: string | null
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
  onGoalCriterionChange: (criterionId: string) => void
  onReloadDomainTemplates: () => void
  onDomainTemplateChange: (templateId: string) => void
  onDomainTaskTypeChange: (taskType: string) => void
  onGenerateDomainDraft: () => void
  onReloadSavedTemplates: () => void
  onApplySavedTemplate: (template: SavedWorkflowTemplate) => void
  onCreateFromSavedTemplate: (template: SavedWorkflowTemplate) => void
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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- closing the parent disclosure resets its advanced child
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

          {linkedGoal && goalCriteria.length > 0 ? (
            <div className="space-y-1.5 rounded-md border border-border/55 bg-secondary/15 p-2">
              <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
                <ClipboardCheck className="h-3.5 w-3.5 shrink-0 text-primary" />
                <span className="min-w-0 flex-1 truncate">
                  {t("workspace.goal.boundCriterion", "推进标准")}
                </span>
                <span className="shrink-0 font-mono text-[10px]">r{linkedGoal.revision ?? 1}</span>
              </div>
              <Select value={selectedGoalCriterionId} onValueChange={onGoalCriterionChange}>
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={GOAL_CRITERION_NONE_VALUE}>
                    {t("workspace.goal.wholeGoal", "整个目标")}
                  </SelectItem>
                  {goalCriteria.map((criterion) => (
                    <SelectItem key={criterion.id} value={criterion.id}>
                      {goalCriterionOptionLabel(criterion)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
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
                    <SelectTrigger className="h-8 text-xs">
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
                    <SelectTrigger className="h-8 text-xs">
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

          <div className="space-y-1.5 rounded-md border border-border/55 bg-secondary/15 p-2">
            <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-medium text-muted-foreground">
              <BookText className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1 truncate">
                {t("workspace.workflow.savedTemplateTitle", "已保存模板")}
              </span>
              {savedTemplatesLoading ? <Loader2 className="h-3 w-3 animate-spin" /> : null}
              <button
                type="button"
                className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground disabled:opacity-50"
                disabled={saving || savedTemplatesLoading}
                onClick={onReloadSavedTemplates}
                aria-label={t("workspace.workflow.savedTemplateReload", "刷新模板")}
              >
                <RefreshCw className="h-3 w-3" />
              </button>
            </div>
            {savedTemplatesError ? (
              <div className="rounded-md border border-destructive/20 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
                {truncateMiddle(savedTemplatesError, 120)}
              </div>
            ) : savedTemplates.length === 0 && !savedTemplatesLoading ? (
              <div className="rounded-md bg-background/55 px-2 py-1.5 text-[11px] text-muted-foreground">
                {t("workspace.workflow.savedTemplateEmpty", "完成的工作流可保存到这里复用")}
              </div>
            ) : (
              <div className="space-y-1">
                {savedTemplates.slice(0, 4).map((template) => {
                  const busy = applyingTemplateId === template.id
                  return (
                    <div
                      key={template.id}
                      className="rounded-md border border-border/45 bg-background/45 px-2 py-1.5"
                    >
                      <div className="flex min-w-0 items-center gap-1.5">
                        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
                          {template.name}
                        </span>
                        <StatusPill
                          label={executionModeLabel(
                            t,
                            normalizeExecutionMode(template.executionMode),
                          )}
                          tone="muted"
                        />
                        {template.scope === "project" ? (
                          <StatusPill
                            label={t("workspace.workflow.savedTemplateProject", "项目")}
                            tone="info"
                          />
                        ) : null}
                      </div>
                      <div className="mt-1 flex items-center gap-1.5">
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="h-7 flex-1 gap-1.5 text-[11px]"
                          disabled={saving || previewLoading || busy}
                          onClick={() => onApplySavedTemplate(template)}
                        >
                          <Copy className="h-3.5 w-3.5" />
                          <span className="truncate">
                            {t("workspace.workflow.savedTemplateApply", "套用")}
                          </span>
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="h-7 flex-1 gap-1.5 text-[11px]"
                          disabled={saving || previewLoading || busy}
                          onClick={() => onCreateFromSavedTemplate(template)}
                        >
                          {busy ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Play className="h-3.5 w-3.5" />
                          )}
                          <span className="truncate">
                            {effectiveRunImmediately
                              ? t("workspace.workflow.savedTemplateRun", "创建并运行")
                              : t("workspace.workflow.savedTemplateCreate", "创建")}
                          </span>
                        </Button>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          <div className="space-y-1">
            <label className="block text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.createKind", "类型")}
            </label>
            <Input
              value={kind}
              disabled={saving || previewLoading}
              onChange={(event) => onKindChange(event.target.value)}
              className="h-8 text-xs"
              aria-label={t("workspace.workflow.createKind", "类型")}
            />
          </div>

          <div className="space-y-1">
            <div className="text-[10px] font-medium text-muted-foreground">
              {t("workspace.workflow.createMode", "执行模式")}
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
                        ? "border-border/45 bg-secondary/70 text-foreground"
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
                    ? "border-border/45 bg-secondary/70 text-foreground"
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
                    ? "border-border/45 bg-secondary/70 text-foreground"
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
                        ? "border-border/45 bg-secondary/70 text-foreground"
                        : "border-border/45 bg-background/35 text-muted-foreground hover:bg-secondary/55 hover:text-foreground",
                    )}
                    disabled={saving || previewLoading}
                    aria-pressed={worktreeMode === worktree.id}
                    data-ha-title-tip={worktree.path}
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
                  {t("workspace.workflow.createScript", "脚本")}
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
                  aria-label={t("workspace.workflow.createScript", "脚本")}
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
          values={evidence.map((item) => domainEvidenceRequirementLabel(t, item))}
        />
      ) : null}
      {gates.length > 0 ? (
        <WorkflowDomainDraftRow
          icon={ShieldAlert}
          label={t("workspace.workflow.domainDraftGates", "审批")}
          values={gates.map((gate) => domainApprovalGateLabel(t, gate))}
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

function domainEvidenceRequirementLabel(
  t: ReturnType<typeof useTranslation>["t"],
  requirement: DomainEvidenceRequirement,
): string {
  const count = requirement.minCount ? ` x${requirement.minCount}` : ""
  const optional = requirement.required
    ? ""
    : t("workspace.workflow.domainDraftOptionalSuffix", "（可选）")
  return `${requirement.title}${count}${optional}`
}

function domainApprovalGateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  gate: DomainApprovalGate,
): string {
  return gate.required
    ? gate.action
    : `${gate.action}${t("workspace.workflow.domainDraftOptionalSuffix", "（可选）")}`
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
          {t("workspace.workflow.noGateIssues", "脚本门禁未发现阻塞项")}
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
                  ? "border-border/45 bg-secondary/70 text-foreground"
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
          {t("workspace.workflow.executionMode", "执行模式")}
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
                  ? "border-border/45 bg-secondary/70 text-foreground"
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

function autonomyActivityLabel(
  t: ReturnType<typeof useTranslation>["t"],
  activity: AutonomyActivity,
): string {
  switch (activity.headlineCode) {
    case "waiting_job_approval":
      return t("chat.activity.waitingJobApproval", "等待工具审批")
    case "waiting_workflow_user":
      return t("chat.activity.waitingWorkflowUser", "等待你处理")
    case "waiting_goal_acceptance":
      return t("chat.activity.waitingGoalAcceptance", "等待确认目标结果")
    case "evaluating_goal":
      return t("chat.activity.evaluatingGoal", "正在验收目标")
    case "running_workflow":
      return t("chat.activity.runningWorkflow", "工作流执行中")
    case "running_task":
      return t("chat.activity.runningTask", "任务执行中")
    case "waiting_background_work":
      return t("chat.activity.waitingBackgroundWork", "等待后台结果")
    case "waiting_loop_trigger":
      return t("chat.activity.waitingLoopTrigger", "等待持续推进触发")
    case "goal_paused":
      return t("chat.activity.goalPaused", "目标已暂停")
    case "workflow_paused":
      return t("chat.activity.workflowPaused", "工作流已暂停")
    case "workflow_blocked":
      return t("chat.activity.workflowBlocked", "工作流待处理")
    case "goal_blocked":
      return t("chat.activity.goalBlocked", "目标待处理")
    case "loop_paused":
      return t("chat.activity.loopPaused", "持续推进已暂停")
    case "loop_blocked":
      return t("chat.activity.loopBlocked", "持续推进待处理")
    case "active_goal":
      return t("chat.activity.activeGoal", "持续推进目标")
    case "goal_terminal":
      return t("chat.activity.goalTerminal", "目标已结束")
    default:
      return t("chat.activity.active", "正在推进")
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

function goalWatchdogFindingLabel(
  t: ReturnType<typeof useTranslation>["t"],
  finding: GoalWatchdogFinding,
): string {
  const age =
    typeof finding.staleSecs === "number" && finding.staleSecs > 0
      ? formatDurationCompact(finding.staleSecs)
      : null
  if (finding.code === "goal_stale_evaluating") {
    return age
      ? t("workspace.goal.watchdogStaleEvaluatingWithAge", "评估中但没有新进展，已等待 {{age}}", {
          age,
        })
      : t("workspace.goal.watchdogStaleEvaluating", "评估中但没有新进展")
  }
  if (finding.code === "goal_no_recent_progress") {
    return age
      ? t(
          "workspace.goal.watchdogNoRecentProgressWithAge",
          "目标一段时间没有新进展，已等待 {{age}}",
          {
            age,
          },
        )
      : t("workspace.goal.watchdogNoRecentProgress", "目标一段时间没有新进展")
  }
  return finding.message || t("workspace.goal.watchdogUnknown", "目标需要确认")
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

function goalCriterionKindLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: GoalCriterionKind,
): string {
  switch (kind) {
    case "required":
      return t("workspace.goal.kindRequired", "必须")
    case "optional":
      return t("workspace.goal.kindOptional", "可选")
    case "follow_up":
      return t("workspace.goal.kindFollowUp", "后续")
  }
}

function goalCriterionKindTone(kind: GoalCriterionKind): StatusTone {
  switch (kind) {
    case "required":
      return "info"
    case "optional":
      return "muted"
    case "follow_up":
      return "warn"
  }
}

function goalClosureDecisionLabel(
  t: ReturnType<typeof useTranslation>["t"],
  decision: GoalClosureDecision | null,
): string {
  switch (decision) {
    case "accepted_v1":
      return t("workspace.goal.closureAcceptedV1", "用户已接受当前证据关闭")
    case "needs_strict_evidence":
      return t("workspace.goal.closureNeedsStrict", "用户要求补充严格证据")
    case "cancelled":
      return t("workspace.goal.closureCancelled", "用户已取消目标")
    case "superseded":
      return t("workspace.goal.closureSuperseded", "目标已被新目标替代")
    default:
      return t("workspace.goal.closurePending", "等待用户确认关闭取舍")
  }
}

function goalClosureReviewPacket(
  snapshot: GoalSnapshot,
  criteria: GoalCriterionAudit[],
  evidence: GoalEvidenceItem[],
): string {
  const goal = snapshot.goal
  const audit = asRecord(goal.finalEvidence)
  const stringList = (key: string) =>
    arrayField(audit, key).filter((item): item is string => typeof item === "string")
  const followUps = [
    ...(goal.followUpItems ?? []).map((item) => item.text),
    ...recordArrayField(audit, "followUpItems")
      .map((item) => stringField(item, "text"))
      .filter((item): item is string => Boolean(item)),
  ]
  const lines = [
    "# Goal Closure Packet",
    "",
    `- Goal: ${goal.objective}`,
    `- State: ${goal.state}`,
    `- Revision: r${goal.revision ?? 1}`,
    `- Audit status: ${stringField(audit, "status") ?? "unknown"}`,
    `- Audit stale: ${snapshot.auditStale ? "yes" : "no"}`,
    `- Closure decision: ${goal.closureDecision ?? "pending"}`,
  ]
  const summary = goal.finalSummary ?? stringField(audit, "summary")
  if (summary) lines.push(`- Summary: ${summary}`)
  const blockedReason = goal.blockedReason ?? stringField(audit, "blockedReason")
  if (blockedReason) lines.push(`- Blocked reason: ${blockedReason}`)
  lines.push("", "## Criteria")
  if (criteria.length === 0) {
    lines.push("- none")
  } else {
    for (const item of criteria) {
      lines.push(
        `- [${item.status}] ${item.kind ?? "required"} ${item.id}: ${item.text}${
          item.reason ? ` (${item.reason})` : ""
        }`,
      )
    }
  }
  const achieved = stringList("achieved")
  const missing = stringList("missing")
  const blockers = stringList("blockers")
  const nextEvidence = recordArrayField(audit, "nextEvidenceNeeded")
    .map(
      (item) =>
        stringField(item, "summary") ?? stringField(item, "text") ?? stringField(item, "kind"),
    )
    .filter((item): item is string => Boolean(item))
  const section = (title: string, items: string[]) => {
    lines.push("", `## ${title}`)
    if (items.length === 0) {
      lines.push("- none")
      return
    }
    for (const item of items) lines.push(`- ${item}`)
  }
  section("Achieved", achieved)
  section("Missing", missing)
  section("Blockers", blockers)
  section("Next Evidence Needed", nextEvidence)
  section("Follow-up Pool", followUps)
  lines.push("", "## Recent Evidence")
  const recentEvidence = evidence.slice(-8).reverse()
  if (recentEvidence.length === 0) {
    lines.push("- none")
  } else {
    for (const item of recentEvidence) {
      lines.push(`- ${item.relation}: ${item.title}${item.summary ? ` — ${item.summary}` : ""}`)
    }
  }
  return lines.join("\n")
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
  return value === "archived" || value === "handoff" || value === "bootstrap_failed"
    ? value
    : "active"
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
          <SelectTrigger className="h-8 text-xs">
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
          <SelectTrigger className="h-8 text-xs">
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
  onCloseGoal,
  onAppendFollowUp,
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
  onCloseGoal: (decision: GoalClosureDecision, reason?: string, followUpItems?: string[]) => void
  onAppendFollowUp: (items: string[]) => Promise<boolean>
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
  const goalEditTemplateValue = goal
    ? goalDomainTemplateValue(goal)
    : GOAL_DOMAIN_FREE_VALUE

  /* eslint-disable react-hooks/set-state-in-effect -- durable Goal changes intentionally reset the local editor draft */
  useEffect(() => {
    setEditObjective(goal?.objective ?? "")
    setEditCriteria(goal?.completionCriteria ?? "")
    setEditTemplateId(goalEditTemplateValue)
    setEditTaskType(goal?.workflowTaskType ?? "")
    setEditOpen(false)
  }, [
    goal?.completionCriteria,
    goal?.id,
    goal?.objective,
    goal?.workflowTaskType,
    goalEditTemplateValue,
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
  /* eslint-enable react-hooks/set-state-in-effect */

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
            : t("workspace.goal.title", "目标")}
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
                  "例如：完整实现目标模式，并通过针对性检查",
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
                  "每行一个标准；可用 [required] / [optional] / [follow-up] 标记",
                )}
                className="min-h-16 resize-y text-xs"
              />
              <GoalCriteriaDraftPreview criteriaText={criteria} />
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
              <span className="truncate">{t("workspace.goal.create", "创建目标")}</span>
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
              label={t("workspace.goal.metricWorkflows", "工作流")}
              value={workflowCount.toString()}
            />
            <WorkflowMetric
              label={t("workspace.goal.metricTasks", "任务")}
              value={`${taskDone}/${taskCount}`}
            />
            <WorkflowMetric
              label={t("workspace.goal.metricEvidence", "证据")}
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
                "还没有最终评估；工作流完成后会自动评估，也可以手动评估。",
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
                <GoalCriteriaDraftPreview criteriaText={editCriteria} />
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
            actionKey={actionKey}
            onCloseGoal={onCloseGoal}
            onAppendFollowUp={onAppendFollowUp}
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

function GoalCriteriaDraftPreview({ criteriaText }: { criteriaText: string }) {
  const { t } = useTranslation()
  const items = useMemo(() => parseGoalCriteriaDraft(criteriaText), [criteriaText])
  if (items.length === 0) return null
  const required = items.filter((item) => item.kind === "required").length
  const optional = items.filter((item) => item.kind === "optional").length
  const followUp = items.filter((item) => item.kind === "follow_up").length
  return (
    <div className="space-y-1 rounded-md border border-border/45 bg-background/45 p-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
        <span className="shrink-0 font-medium text-foreground/75">
          {t("workspace.goal.criteriaPreview", "标准预览")}
        </span>
        <StatusPill
          label={t("workspace.goal.criteriaPreviewRequired", "必须 {{count}}", {
            count: required,
          })}
          tone={required > 0 ? "warn" : "muted"}
        />
        <StatusPill
          label={t("workspace.goal.criteriaPreviewOptional", "可选 {{count}}", {
            count: optional,
          })}
          tone={optional > 0 ? "info" : "muted"}
        />
        <StatusPill
          label={t("workspace.goal.criteriaPreviewFollowUp", "后续 {{count}}", {
            count: followUp,
          })}
          tone={followUp > 0 ? "good" : "muted"}
        />
      </div>
      <div className="space-y-0.5">
        {items.slice(0, 4).map((item) => (
          <div key={item.id} className="flex min-w-0 items-center gap-1 text-[10px]">
            <StatusPill
              label={goalDraftCriterionKindLabel(t, item.kind)}
              tone={goalCriterionKindTone(item.kind)}
            />
            <span className="min-w-0 flex-1 truncate text-muted-foreground">{item.text}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function goalDraftCriterionKindLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: DraftGoalCriterionKind,
): string {
  switch (kind) {
    case "required":
      return t("workspace.goal.criterionKindRequired", "必须")
    case "optional":
      return t("workspace.goal.criterionKindOptional", "可选")
    case "follow_up":
      return t("workspace.goal.criterionKindFollowUp", "后续")
  }
}

function GoalDetailSection({
  open,
  snapshot,
  criteria,
  evidence,
  timeline,
  actionKey,
  onCloseGoal,
  onAppendFollowUp,
}: {
  open: boolean
  snapshot: GoalSnapshot
  criteria: GoalCriterionAudit[]
  evidence: GoalEvidenceItem[]
  timeline: GoalTimelineItem[]
  actionKey?: string | null
  onCloseGoal: (decision: GoalClosureDecision, reason?: string, followUpItems?: string[]) => void
  onAppendFollowUp: (items: string[]) => Promise<boolean>
}) {
  const { t } = useTranslation()
  const [followUpDraft, setFollowUpDraft] = useState("")
  const runs = snapshot.workflowRuns
  const tasks = snapshot.tasks
  const latestTimeline = timeline.slice(-8).reverse()
  const latestEvidence = evidence.slice(-8).reverse()
  const worktreeEvidence = goalWorktreeEvidenceItems(evidence).slice(-4).reverse()
  const domainEvidence = goalDomainEvidenceItems(evidence).slice(-6).reverse()
  const audit = asRecord(snapshot.goal.finalEvidence)
  const nextEvidence = recordArrayField(audit, "nextEvidenceNeeded").slice(0, 6)
  const auditFollowUps = recordArrayField(audit, "followUpItems")
  const followUpTexts = auditFollowUps
    .map((item) => stringField(item, "text"))
    .filter((text): text is string => Boolean(text))
  const followUpItems = snapshot.goal.followUpItems ?? []
  const closureDecision = snapshot.goal.closureDecision ?? null
  const closeBusy = actionKey?.startsWith("close_goal")
  const followUpBusy = actionKey?.startsWith("append_goal_follow_up")
  const finalAuditCompleted = stringField(audit, "status") === "completed"
  const canCloseGoal = snapshot.goal.state !== "evaluating" && snapshot.goal.state !== "cancelled"
  const canAcceptGoal = canCloseGoal && finalAuditCompleted && !snapshot.auditStale
  const requiredDone = criteria.filter(
    (criterion) =>
      (criterion.kind ?? "required") === "required" && criterion.status === "satisfied",
  ).length
  const requiredTotal = criteria.filter(
    (criterion) => (criterion.kind ?? "required") === "required",
  ).length
  const budget = snapshot.budget
  const copyClosurePacket = async () => {
    try {
      await navigator.clipboard.writeText(goalClosureReviewPacket(snapshot, criteria, evidence))
      toast.success(t("workspace.goal.closurePacketCopied", "关闭摘要已复制"))
    } catch (e) {
      logger.error("ui", "GoalDetailSection::copyClosurePacket", "Copy goal packet failed", e)
      toast.error(t("workspace.goal.closurePacketCopyFailed", "复制失败"))
    }
  }
  const addFollowUp = async () => {
    const text = followUpDraft.trim()
    if (!text || followUpBusy) return
    const ok = await onAppendFollowUp([text])
    if (ok) setFollowUpDraft("")
  }

  return (
    <AnimatedCollapse open={open}>
      <div className="space-y-2 rounded-md border border-border/55 bg-background/45 p-2">
        {budget ? <GoalBudgetCard budget={budget} /> : null}

        <GoalDetailBlock title={t("workspace.goal.detailClosure", "关闭取舍")} count={1}>
          <div className="space-y-1.5">
            <div className="grid grid-cols-3 gap-1 text-[10px]">
              <GoalWorktreeMetric
                label={t("workspace.goal.revision", "修订")}
                value={`r${snapshot.goal.revision ?? 1}`}
              />
              <GoalWorktreeMetric
                label={t("workspace.goal.requiredProgress", "必须")}
                value={`${requiredDone}/${requiredTotal}`}
              />
              <GoalWorktreeMetric
                label={t("workspace.goal.followUps", "后续")}
                value={`${followUpItems.length + auditFollowUps.length}`}
              />
            </div>
            <div
              className={cn(
                "rounded-md border px-2 py-1.5 text-[11px]",
                snapshot.auditStale
                  ? "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300"
                  : closureDecision === "accepted_v1"
                    ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                    : "border-border/55 bg-secondary/25 text-muted-foreground",
              )}
            >
              <div className="flex min-w-0 items-center gap-1.5">
                <ClipboardCheck className="h-3.5 w-3.5 shrink-0" />
                <span className="min-w-0 flex-1 truncate font-medium">
                  {snapshot.auditStale
                    ? t("workspace.goal.auditStale", "目标或证据已变化，需要重新评估")
                    : goalClosureDecisionLabel(t, closureDecision)}
                </span>
                {snapshot.goal.closedAt ? (
                  <span className="shrink-0 text-[10px] opacity-75">
                    {formatMessageTime(snapshot.goal.closedAt)}
                  </span>
                ) : null}
              </div>
              {snapshot.goal.closureReason ? (
                <div className="mt-0.5 truncate opacity-80">{snapshot.goal.closureReason}</div>
              ) : null}
            </div>
            {followUpItems.length > 0 || followUpTexts.length > 0 ? (
              <div className="space-y-1">
                {[...followUpItems.map((item) => item.text), ...followUpTexts]
                  .slice(0, 4)
                  .map((item, index) => (
                    <div
                      key={`${item}:${index}`}
                      className="truncate rounded-md bg-secondary/25 px-2 py-1 text-[10px] text-muted-foreground"
                    >
                      {item}
                    </div>
                  ))}
              </div>
            ) : null}
            <div className="grid grid-cols-2 gap-1.5">
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                onClick={() => void copyClosurePacket()}
              >
                <Copy className="h-3.5 w-3.5" />
                <span className="truncate">
                  {t("workspace.goal.copyClosurePacket", "复制摘要")}
                </span>
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                disabled={followUpBusy || !followUpDraft.trim()}
                onClick={() => void addFollowUp()}
              >
                {followUpBusy ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Plus className="h-3.5 w-3.5" />
                )}
                <span className="truncate">{t("workspace.goal.addFollowUp", "加入后续")}</span>
              </Button>
            </div>
            <Input
              value={followUpDraft}
              disabled={followUpBusy}
              onChange={(event) => setFollowUpDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return
                event.preventDefault()
                void addFollowUp()
              }}
              className="h-8 text-xs"
              placeholder={t("workspace.goal.followUpPlaceholder", "新增后续项")}
            />
            <div className="grid grid-cols-2 gap-1.5">
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                disabled={!canAcceptGoal || closeBusy}
                onClick={() =>
                  onCloseGoal(
                    "accepted_v1",
                    t("workspace.goal.acceptedV1Reason", "用户接受当前证据与剩余风险"),
                    followUpTexts,
                  )
                }
              >
                {actionKey?.startsWith("close_goal:accepted_v1") ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <CheckCircle2 className="h-3.5 w-3.5" />
                )}
                <span className="truncate">{t("workspace.goal.acceptV1", "接受 v1 关闭")}</span>
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-8 min-w-0 gap-1.5 text-xs"
                disabled={!canCloseGoal || closeBusy}
                onClick={() =>
                  onCloseGoal(
                    "needs_strict_evidence",
                    t("workspace.goal.needsStrictReason", "用户要求补充真实或更严格证据"),
                  )
                }
              >
                {actionKey?.startsWith("close_goal:needs_strict_evidence") ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <ShieldAlert className="h-3.5 w-3.5" />
                )}
                <span className="truncate">{t("workspace.goal.needStrict", "要求严格证据")}</span>
              </Button>
            </div>
          </div>
        </GoalDetailBlock>

        {worktreeEvidence.length > 0 ? (
          <GoalDetailBlock
            title={t("workspace.goal.detailWorktrees", "工作树")}
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
                        label={t("workspace.goal.worktreeBase", "基线")}
                        value={goalWorktreeBaseLabel(worktree)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.worktreeDirty", "改动")}
                        value={goalWorktreeDirtyLabel(t, worktree)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.worktreeHandoff", "交接")}
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
                        label={t("workspace.goal.domainConfidence", "置信度")}
                        value={domainEvidenceConfidenceLabel(evidenceItem.confidence)}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.domainAccess", "访问")}
                        value={evidenceItem.accessScope ?? "-"}
                      />
                      <GoalWorktreeMetric
                        label={t("workspace.goal.domainWorkflow", "工作流")}
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
                        label={goalCriterionKindLabel(t, criterion.kind ?? "required")}
                        tone={goalCriterionKindTone(criterion.kind ?? "required")}
                      />
                      <StatusPill
                        label={goalCriterionStatusLabel(t, criterion.status)}
                        tone={goalCriterionStatusTone(criterion.status)}
                      />
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-muted-foreground">
                      <StatusPill
                        label={t("workspace.goal.criterionWorkflowCount", "工作流 {{count}}", {
                          count: runs.filter((run) => run.goalCriterionId === criterion.id).length,
                        })}
                        tone="muted"
                      />
                      <StatusPill
                        label={t("workspace.goal.criterionLoopCount", "循环 {{count}}", {
                          count: snapshot.links.filter(
                            (link) =>
                              (link.targetType === "loop_schedule" ||
                                link.targetType === "loop_run") &&
                              goalCriterionMetadataId(link.metadata) === criterion.id,
                          ).length,
                        })}
                        tone="muted"
                      />
                      <StatusPill
                        label={t("workspace.goal.criterionEvidenceCount", "证据 {{count}}", {
                          count: evidence.filter(
                            (item) => goalCriterionMetadataId(item.metadata) === criterion.id,
                          ).length,
                        })}
                        tone="muted"
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
                      {goalCriterionMetadataId(item.metadata) ? (
                        <StatusPill
                          label={goalCriterionMetadataId(item.metadata) ?? ""}
                          tone="info"
                        />
                      ) : null}
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
              {t("workspace.goal.detailNoEvidence", "还没有工作流 / 验证 / diff 证据")}
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
            title={t("workspace.goal.detailWorkflows", "工作流")}
            value={`${runs.filter((run) => run.state === "completed").length}/${runs.length}`}
            detail={
              runs[0]
                ? `${runs[0].kind} · ${workflowRunStateLabel(t, runs[0].state)}`
                : t("workspace.goal.detailNoWorkflow", "暂无工作流")
            }
          />
          <GoalLinkedSummary
            title={t("workspace.goal.detailTasks", "任务")}
            value={`${tasks.filter((task) => task.status === "completed").length}/${tasks.length}`}
            detail={tasks[0]?.content ?? t("workspace.goal.detailNoTask", "暂无任务")}
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
          label={t("workspace.goal.budgetTokens", "Token")}
          value={
            budget.tokenLimit
              ? `${compactCount(budget.tokensUsed)}/${compactCount(budget.tokenLimit)}`
              : compactCount(budget.tokensUsed)
          }
          ratio={goalBudgetRatioText(budget.tokenRatio)}
        />
        <GoalBudgetMetric
          label={t("workspace.goal.budgetTime", "时间")}
          value={
            budget.timeLimitSecs
              ? `${formatDurationCompact(budget.elapsedSecs)}/${formatDurationCompact(budget.timeLimitSecs)}`
              : formatDurationCompact(budget.elapsedSecs)
          }
          ratio={goalBudgetRatioText(budget.timeRatio)}
        />
        <GoalBudgetMetric
          label={t("workspace.goal.budgetTurns", "回合")}
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
  onCreateRepairTask,
  creatingRepairTask,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  latestEvent?: WorkflowEvent
  worktree?: ManagedWorktree | null
  actions?: ReactNode
  onSelectDetailTab?: (tab: WorkflowDetailTab) => void
  onCreateRepairDraft?: (repairPrompt: string, run: WorkflowRun) => void
  onCreateRepairTask?: (repairPrompt: string, run: WorkflowRun) => void
  creatingRepairTask?: boolean
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
  const displayState = workflowRunDisplayState(t, run, snapshot)
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
              label={displayState.label}
              tone={displayState.tone}
              loading={displayState.loading}
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
            label={t("workspace.workflow.metricOps", "步骤")}
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
            label={t("workspace.workflow.metricAgents", "子 Agent")}
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
        {run.goalCriterionId ? (
          <div className="rounded-md border border-primary/20 bg-primary/5 px-2 py-1.5 text-[11px] text-muted-foreground">
            <div className="flex min-w-0 items-center gap-1.5">
              <ClipboardCheck className="h-3.5 w-3.5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1 truncate font-medium text-foreground/85">
                {run.goalCriterionText ?? run.goalCriterionId}
              </span>
              <span className="shrink-0 font-mono text-[10px]">
                {run.goalRevision ? `r${run.goalRevision}` : run.goalCriterionId}
              </span>
            </div>
          </div>
        ) : null}
        <WorkflowRunSummaryCard run={run} snapshot={snapshot} budget={budget} />
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
        onSelectDetailTab={onSelectDetailTab}
        onCreateRepairDraft={onCreateRepairDraft}
        onCreateRepairTask={onCreateRepairTask}
        creatingRepairTask={creatingRepairTask}
      />
      {actions ? <div>{actions}</div> : null}
    </div>
  )
}

function WorkflowRunSummaryCard({
  run,
  snapshot,
  budget,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  budget: { spent: number; limit: number; exhausted: boolean } | null
}) {
  const { t } = useTranslation()
  const events = snapshot?.events ?? []
  const counts = workflowRunSummaryCounts(events)
  const durationSeconds = workflowRunDurationSeconds(run)
  const runtimeSummary = workflowLatestRuntimeSummary(t, events)
  const runtimeCaps = workflowRunRuntimeCaps(t, run)
  const sizeLabel = workflowSizeGuidelineLabel(t, workflowSizeGuideline(run))
  const agentUsage = snapshot?.agentUsage ?? null
  const workflowUsage = snapshot?.usage ?? null
  const hasWorkflowUsage = !!workflowUsage && workflowUsage.totalTokens > 0
  const hasAgentUsage =
    !!agentUsage && (agentUsage.totalTokens > 0 || agentUsage.attributedAgents > 0)
  const phaseTone: StatusTone =
    counts.phasesFailed > 0 ? "danger" : counts.phasesCompleted > 0 ? "good" : "muted"
  const milestoneTone: StatusTone =
    counts.milestoneRequested > counts.milestoneDelivered
      ? "warn"
      : counts.milestoneDelivered > 0
        ? "good"
        : "muted"
  const outputBudgetLabel = budget
    ? `${compactCount(budget.spent)}/${compactCount(budget.limit)}`
    : t("workspace.workflow.summaryNoOutputBudget", "未设置")

  return (
    <div className="rounded-md border border-border/55 bg-secondary/20 p-2">
      <div className="mb-1.5 flex min-w-0 items-center gap-2">
        <BarChart3 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground/90">
          {t("workspace.workflow.summaryTitle", "运行摘要")}
        </span>
        <span className="shrink-0 text-[10px] text-muted-foreground/70">
          {t("workspace.workflow.summaryReliable", "已记录指标")}
        </span>
      </div>

      <div className="grid grid-cols-4 gap-1 text-[10px]">
        <WorkflowMetric
          label={t("workspace.workflow.summaryDuration", "耗时")}
          value={
            durationSeconds === null ? "-" : formatDurationCompact(Math.max(0, durationSeconds))
          }
        />
        <WorkflowMetric
          label={t("workspace.workflow.summaryPhases", "阶段")}
          value={
            counts.phasesStarted > 0
              ? `${counts.phasesCompleted}/${counts.phasesStarted}`
              : counts.phasesCompleted.toString()
          }
          tone={phaseTone}
        />
        <WorkflowMetric
          label={t("workspace.workflow.summaryReviews", "检查")}
          value={`${counts.checkpoints}/${counts.reports}`}
        />
        <WorkflowMetric
          label={t("workspace.workflow.summaryMilestones", "注入")}
          value={`${counts.milestoneDelivered}/${counts.milestoneRequested}`}
          tone={milestoneTone}
        />
      </div>

      <div className="mt-1 grid grid-cols-2 gap-1 text-[10px]">
        <WorkflowMetric label={t("workspace.workflow.summarySize", "规模")} value={sizeLabel} />
        <WorkflowMetric
          label={t("workspace.workflow.summaryCaps", "上限")}
          value={runtimeCaps ?? t("workspace.workflow.summaryUnknown", "未记录")}
        />
      </div>

      {hasAgentUsage && agentUsage ? (
        <div className="mt-1 flex min-w-0 items-center justify-between gap-2 rounded-md bg-background/45 px-2 py-1 text-[10px] text-muted-foreground">
          <span className="truncate">
            {t("workspace.workflow.summaryAgentTokens", "子代理 Token")}
          </span>
          <span className="shrink-0 truncate text-right font-mono text-foreground/80">
            {t("workspace.workflow.summaryAgentUsageValue", "{{total}} · {{agents}} 个 Agent", {
              total: compactCount(agentUsage.totalTokens),
              agents: `${agentUsage.attributedAgents}/${agentUsage.spawnedAgents}`,
            })}
          </span>
        </div>
      ) : null}

      {hasWorkflowUsage && workflowUsage ? (
        <div className="mt-1 flex min-w-0 items-center justify-between gap-2 rounded-md bg-background/45 px-2 py-1 text-[10px] text-muted-foreground">
          <span className="truncate">
            {t("workspace.workflow.summaryWindowTokens", "窗口 Token")}
          </span>
          <span className="shrink-0 truncate text-right font-mono text-foreground/80">
            {t(
              "workspace.workflow.summaryWindowUsageValue",
              "{{total}} · 父会话 {{parent}} / 子代理 {{agents}}",
              {
                total: compactCount(workflowUsage.totalTokens),
                parent: compactCount(workflowUsage.parentTotalTokens),
                agents: compactCount(workflowUsage.agentTotalTokens),
              },
            )}
          </span>
        </div>
      ) : null}

      <div className="mt-1 flex min-w-0 items-center justify-between gap-2 rounded-md bg-background/45 px-2 py-1 text-[10px] text-muted-foreground">
        <span className="truncate">{t("workspace.workflow.outputBudget", "输出预算")}</span>
        <span
          className={cn(
            "shrink-0 font-mono",
            budget?.exhausted ? "text-amber-700 dark:text-amber-300" : "text-foreground/80",
          )}
        >
          {outputBudgetLabel}
        </span>
      </div>

      {runtimeSummary ? (
        <div className="mt-1 flex min-w-0 items-center gap-2 rounded-md bg-background/45 px-2 py-1 text-[10px]">
          <StatusPill label={runtimeSummary.label} tone={runtimeSummary.tone} />
          <span className="min-w-0 flex-1 truncate text-muted-foreground">
            {runtimeSummary.detail ??
              t("workspace.workflow.summaryRuntimeDone", "runtime 已回报结果")}
          </span>
        </div>
      ) : null}

      <div className="mt-1 truncate text-[10px] text-muted-foreground/65">
        {hasWorkflowUsage
          ? t(
              "workspace.workflow.summaryWindowUsageBoundary",
              "窗口 Token = 父会话运行窗口 + 本工作流关联子代理；工作流注入回合另有强关联口径；不是 provider 级完整成本。",
            )
          : hasAgentUsage
            ? t(
                "workspace.workflow.summaryAgentUsageBoundary",
                "仅统计本工作流关联子代理用量；完整成本仍等待运行归因。",
              )
            : t(
                "workspace.workflow.summaryUsageBoundary",
                "Token/成本等待工作流运行归因接入；当前不估算。",
              )}
      </div>
    </div>
  )
}

const WORKFLOW_METRIC_TEXT_CLASS: Record<StatusTone, string> = {
  muted: "text-foreground/85",
  info: "text-blue-700 dark:text-blue-300",
  good: "text-emerald-700 dark:text-emerald-300",
  warn: "text-amber-700 dark:text-amber-300",
  danger: "text-destructive",
}

function WorkflowMetric({
  label,
  value,
  tone = "muted",
}: {
  label: string
  value: string
  tone?: StatusTone
}) {
  return (
    <div className="min-w-0 rounded-md bg-secondary/35 px-1.5 py-1 text-center">
      <div className={cn("truncate font-medium", WORKFLOW_METRIC_TEXT_CLASS[tone])}>{value}</div>
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
            label={t("workspace.workflow.worktreeBase", "基线")}
            value={info.baseLabel ?? "-"}
          />
          <GoalWorktreeMetric
            label={t("workspace.workflow.worktreeChanges", "改动")}
            value={info.dirtyLabel ?? "-"}
          />
          <GoalWorktreeMetric
            label={t("workspace.workflow.worktreeSource", "来源")}
            value={
              info.source === "managed"
                ? t("workspace.workflow.worktreeSourceManaged", "托管")
                : t("workspace.workflow.worktreeSourceTrace", "轨迹")
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
  const displayState = workflowRunDisplayState(t, run, snapshot)
  const agentUsage = snapshot?.agentUsage
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
  let Icon: LucideIcon
  let targetTab: WorkflowDetailTab | null = null

  if (displayState.kind === "children" && agentUsage) {
    title = t("workspace.workflow.focusWaitingAgentsTitle", "当前焦点：等待子 Agent")
    body = t(
      "workspace.workflow.focusWaitingAgentsBody",
      "已有 {{done}}/{{total}} 个子 Agent 结束，后台运行不会阻塞当前对话。",
      {
        done: agentUsage.terminalAgents,
        total: agentUsage.spawnedAgents,
      },
    )
    tone = "info"
    Icon = Bot
    targetTab = "agents"
  } else if (displayState.kind === "results" && agentUsage) {
    title = t("workspace.workflow.focusPartialResultsTitle", "当前焦点：处理阶段结果")
    body = t(
      "workspace.workflow.focusPartialResultsBody",
      "{{pending}} 个子 Agent 结果等待模型读取或汇总。",
      { pending: agentUsage.pendingResults },
    )
    tone = "warn"
    Icon = Network
    targetTab = "agents"
  } else if (run.state === "draft") {
    title = t("workspace.workflow.focusDraftTitle", "当前焦点：草稿待启动")
    body = t("workspace.workflow.focusDraftBody", "脚本已保存，运行前仍会保留轨迹与审批记录。")
    Icon = Play
  } else if (run.state === "awaiting_approval") {
    title = t("workspace.workflow.focusApprovalTitle", "当前焦点：等待授权")
    body =
      workflowPermissionSummaryText(t, permissionPreview?.summary) ||
      t("workspace.workflow.focusApprovalBody", "有调用需要确认，批准后运行会继续。")
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
      ? t("workspace.workflow.focusPausedBodyWithOp", "暂停在 {{op}}，恢复后会继续该运行。", {
          op: truncateMiddle(workflowOpTitle(focusOp), 64),
        })
      : t("workspace.workflow.focusPausedBody", "恢复后会从当前轨迹继续，取消则保留已有记录。")
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
        t("workspace.workflow.nextFailedBody", "查看轨迹与验证，基于失败步骤继续修复。"),
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
        : t("workspace.workflow.focusCompletedBodyNoOps", "运行已完成，轨迹已保留。")
    tone = "good"
    Icon = CheckCircle2
    targetTab = "trace"
  } else {
    title = t("workspace.workflow.focusCancelledTitle", "当前焦点：已取消")
    body = t("workspace.workflow.focusCancelledBody", "运行已停止，已有轨迹可用于复盘。")
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
            label={t("workspace.workflow.permissionMetricStrict", "严格")}
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
  onSelectDetailTab,
  onCreateRepairDraft,
  onCreateRepairTask,
  creatingRepairTask,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  onSelectDetailTab?: (tab: WorkflowDetailTab) => void
  onCreateRepairDraft?: (repairPrompt: string, run: WorkflowRun) => void
  onCreateRepairTask?: (repairPrompt: string, run: WorkflowRun) => void
  creatingRepairTask?: boolean
}) {
  const { t } = useTranslation()
  const ops = snapshot?.ops ?? []
  const failedOp = [...ops].reverse().find((op) => op.state === "failed")
  const failedError = asRecord(failedOp?.error)
  const failedMessage = stringField(failedError, "message")
  const blockedReason = run.blockedReason
  const hasValidationFailure = ops.some(workflowOpHasValidationFailure)
  const activeOp = [...ops].reverse().find((op) => op.state === "started")
  const pendingOp = ops.find((op) => op.state === "pending")
  const focusOp = activeOp ?? pendingOp
  const validationCount = ops.filter((op) => op.opType === "validate").length
  const repairPrompt = buildWorkflowRepairPrompt(run, snapshot)

  let title: string | null = null
  let body: string | null = null
  let tone: "muted" | "good" | "warn" | "danger" | "info" = "info"
  let Icon: LucideIcon = Lightbulb
  let targetTab: WorkflowDetailTab | null = null

  if (run.state === "draft") {
    title = t("workspace.workflow.nextDraftTitle", "下一步：启动工作流")
    body = t(
      "workspace.workflow.nextDraftBody",
      "确认脚本、预算和运行位置后启动；需要隔离改动时先绑定 worktree。",
    )
    Icon = Play
    targetTab = "trace"
  } else if (run.state === "awaiting_approval") {
    title = t("workspace.workflow.nextApproveTitle", "下一步：确认授权")
    body = t(
      "workspace.workflow.nextApproveBody",
      "检查上面的授权清单，确认后批准；不符合预期就取消。",
    )
    tone = "warn"
    Icon = ShieldAlert
    targetTab = "trace"
  } else if (run.state === "awaiting_user") {
    title = t("workspace.workflow.nextUserTitle", "下一步：补充用户确认")
    body = t(
      "workspace.workflow.nextUserBody",
      "回到对话里补充问题答案或外部确认；trace 会保留等待点。",
    )
    tone = "warn"
    Icon = MessageCircle
    targetTab = "trace"
  } else if (run.state === "running" || run.state === "recovering") {
    title =
      run.state === "recovering"
        ? t("workspace.workflow.nextRecoveringTitle", "下一步：观察恢复进度")
        : t("workspace.workflow.nextRunningTitle", "下一步：观察运行进度")
    body = focusOp
      ? t(
          "workspace.workflow.nextRunningOpBody",
          "当前步骤是 {{op}}；如长时间无进展，可先看时间线，再决定暂停或取消。",
          { op: truncateMiddle(workflowOpTitle(focusOp), 72) },
        )
      : t(
          "workspace.workflow.nextRunningBody",
          "保持运行并观察时间线；如卡住，可暂停保留现场或取消后从 trace 修复。",
        )
    tone = "info"
    Icon = run.state === "recovering" ? Clock : Radio
    targetTab = focusOp ? workflowOpDetailTab(focusOp) : "trace"
  } else if (run.state === "paused") {
    title = t("workspace.workflow.nextPausedTitle", "下一步：恢复或取消")
    body = t(
      "workspace.workflow.nextPausedBody",
      "当前运行已暂停，可恢复继续执行，也可取消并保留 trace。",
    )
    tone = "warn"
    Icon = Pause
    targetTab = focusOp ? workflowOpDetailTab(focusOp) : "trace"
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
    Icon = CircleAlert
    targetTab = hasValidationFailure ? "validation" : "trace"
  } else if (run.state === "failed" || failedOp || hasValidationFailure) {
    title = hasValidationFailure
      ? t("workspace.workflow.nextValidationTitle", "下一步：修复验证失败")
      : t("workspace.workflow.nextFailedTitle", "下一步：定位失败步骤")
    body =
      failedMessage ??
      failedOp?.opKey ??
      t("workspace.workflow.nextFailedBody", "查看轨迹与验证，基于失败步骤继续修复。")
    tone = "danger"
    Icon = CircleAlert
    targetTab = hasValidationFailure
      ? "validation"
      : failedOp
        ? workflowOpDetailTab(failedOp)
        : "trace"
  } else if (run.state === "completed") {
    title = t("workspace.workflow.nextCompletedTitle", "下一步：复核完成证据")
    body =
      validationCount > 0
        ? t(
            "workspace.workflow.nextCompletedValidationBody",
            "运行已完成；复核验证结果、产物和残余风险后再关闭 Goal 或继续派生后续工作。",
          )
        : t(
            "workspace.workflow.nextCompletedBody",
            "运行已完成；复核 trace、产物和残余风险后再关闭 Goal 或继续派生后续工作。",
          )
    tone = "good"
    Icon = CheckCircle2
    targetTab = validationCount > 0 ? "validation" : "trace"
  } else if (run.state === "cancelled") {
    title = t("workspace.workflow.nextCancelledTitle", "下一步：复盘或重建")
    body = t(
      "workspace.workflow.nextCancelledBody",
      "运行已停止；可以从 trace 复盘原因，必要时基于当前目标创建新的工作流。",
    )
    tone = "muted"
    Icon = X
    targetTab = "trace"
  }

  if (!title || !body) return null
  const tabLabel = targetTab ? workflowDetailTabLabel(t, targetTab) : null
  const showRepairActions = Boolean(
    repairPrompt &&
    (run.state === "failed" || run.state === "blocked" || failedOp || hasValidationFailure),
  )

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
              {t("workspace.workflow.nextOpenTab", "查看 {{tab}}", { tab: tabLabel })}
            </span>
          </Button>
        ) : null}
      </div>
      <div className="mt-0.5 truncate opacity-85">{body}</div>
      {showRepairActions ? (
        <div
          className={cn("mt-1.5 grid gap-1.5", onCreateRepairTask ? "grid-cols-3" : "grid-cols-2")}
        >
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-7 min-w-0 gap-1.5 border-current/25 bg-background/45 text-[11px] hover:bg-background/70"
            onClick={() => {
              if (repairPrompt) onCreateRepairDraft?.(repairPrompt, run)
            }}
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span className="truncate">
              {t("workspace.workflow.createRepairDraft", "生成修复草稿")}
            </span>
          </Button>
          {onCreateRepairTask ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-7 min-w-0 gap-1.5 border-current/25 bg-background/45 text-[11px] hover:bg-background/70"
              disabled={creatingRepairTask}
              onClick={() => {
                if (repairPrompt) onCreateRepairTask(repairPrompt, run)
              }}
            >
              {creatingRepairTask ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Plus className="h-3.5 w-3.5" />
              )}
              <span className="truncate">{t("workspace.workflow.createRepairTask", "转任务")}</span>
            </Button>
          ) : null}
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
    return <EmptyHint>{t("workspace.workflow.emptyTrace", "暂无轨迹")}</EmptyHint>
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
      : status === "failed" ||
          status === "error" ||
          status === "timeout" ||
          status === "killed" ||
          status === "cancelled" ||
          status === "not_found" ||
          op.state === "failed"
        ? "danger"
        : status === "queued" ||
            status === "running" ||
            status === "spawned" ||
            op.state === "started"
          ? "info"
          : "muted"
  return { status, tone }
}

function workflowAgentStatusLabel(
  t: ReturnType<typeof useTranslation>["t"],
  status: string,
): string {
  switch (status) {
    case "queued":
      return String(t("workspace.workflow.agentStateQueued", "排队中"))
    case "spawning":
    case "spawned":
      return String(t("workspace.workflow.agentStateSpawning", "启动中"))
    case "running":
      return String(t("workspace.workflow.agentStateRunning", "运行中"))
    case "completed":
    case "success":
      return String(t("workspace.workflow.agentStateCompleted", "已完成"))
    case "timeout":
      return String(t("workspace.workflow.agentStateTimeout", "已超时"))
    case "killed":
    case "cancelled":
      return String(t("workspace.workflow.agentStateCancelled", "已取消"))
    case "error":
    case "failed":
      return String(t("workspace.workflow.agentStateFailed", "失败"))
    default:
      return status
  }
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
          label={t("workspace.workflow.agentMetricTotal", "子 Agent")}
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
                    label={workflowAgentStatusLabel(t, status)}
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
  onOpenGitDiff = () => {},
  onFillInput,
  onOpenPullRequest,
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
  onOpenBrowserPanel,
  onViewSubagentSession,
  openLoopCreateRequest = 0,
  focusRequest,
  onFocusRequestHandled,
  onEnsureSession,
  draftWorkflowMode = "off",
  onDraftWorkflowModeChange,
  onClose,
}: WorkspacePanelProps) {
  const { t } = useTranslation()
  const { files, sources, browser, filesTruncated, sourcesTruncated, browserTruncated } =
    useWorkspaceArtifacts(sessionId, messages, { incognito, turnActive })
  const ownedWorkflowRunsState = useWorkflowRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(workflowRunsState),
  })
  const sharedWorkflowRunsState = workflowRunsState ?? ownedWorkflowRunsState
  const loopSchedulesState = useLoopSchedules(sessionId, { incognito, turnActive })
  const goalState = useGoal(sessionId, { incognito })
  const [loopCreateRequest, setLoopCreateRequest] = useState(0)
  const loopSectionRef = useRef<HTMLDivElement | null>(null)
  const goalSectionRef = useRef<HTMLDivElement | null>(null)
  const progressSectionRef = useRef<HTMLDivElement | null>(null)
  const lastExternalLoopCreateRequestRef = useRef(0)
  const lastFocusRequestRef = useRef(0)
  const [loopInspectRequest, setLoopInspectRequest] = useState<{
    loopId: string
    nonce: number
  } | null>(null)
  const reviewRunsState = useReviewRuns(sessionId, { incognito, turnActive })
  const verificationRunsState = useVerificationRuns(sessionId, { incognito, turnActive })
  const domainQualityRunsState = useDomainQualityRuns(sessionId, { incognito, turnActive })
  const domainTaskWorkbenchState = useDomainTaskWorkbench(sessionId, { incognito, turnActive })
  const domainWorkbenchRef = useRef<HTMLDivElement | null>(null)
  const workflowSectionRef = useRef<HTMLDivElement | null>(null)
  const [workflowFocusTarget, setWorkflowFocusTarget] = useState<WorkflowFocusTarget | null>(null)
  const openLoopCreate = useCallback(() => {
    setLoopCreateRequest((value) => value + 1)
    window.setTimeout(() => {
      loopSectionRef.current?.scrollIntoView?.({ block: "start", behavior: "smooth" })
    }, 0)
  }, [])
  useEffect(() => {
    if (openLoopCreateRequest === lastExternalLoopCreateRequestRef.current) return
    lastExternalLoopCreateRequestRef.current = openLoopCreateRequest
    if (openLoopCreateRequest <= 0) return
    // eslint-disable-next-line react-hooks/set-state-in-effect -- an external composer request opens and focuses the local creator
    openLoopCreate()
  }, [openLoopCreate, openLoopCreateRequest])
  const focusWorkflowRun = useCallback((runId: string) => {
    setWorkflowFocusTarget((current) => ({
      runId,
      nonce: (current?.nonce ?? 0) + 1,
    }))
    window.setTimeout(() => {
      workflowSectionRef.current?.scrollIntoView?.({ block: "start", behavior: "smooth" })
    }, 0)
  }, [])
  /* eslint-disable react-hooks/set-state-in-effect -- an external Dashboard deep-link intentionally focuses local panel state */
  useEffect(() => {
    if (!shouldConsumeWorkspaceFocus(focusRequest, sessionId, lastFocusRequestRef.current)) {
      return
    }
    lastFocusRequestRef.current = focusRequest.nonce
    const scroll = (target: HTMLDivElement | null) =>
      window.setTimeout(() => target?.scrollIntoView?.({ block: "start", behavior: "smooth" }), 0)
    if (focusRequest.section === "goal") {
      scroll(goalSectionRef.current)
    } else if (focusRequest.section === "workflow") {
      if (focusRequest.itemId) focusWorkflowRun(focusRequest.itemId)
      else scroll(workflowSectionRef.current)
    } else if (focusRequest.section === "loop") {
      if (focusRequest.itemId) {
        setLoopInspectRequest({ loopId: focusRequest.itemId, nonce: focusRequest.nonce })
      }
      scroll(loopSectionRef.current)
    } else {
      scroll(progressSectionRef.current)
    }
    onFocusRequestHandled?.(focusRequest.nonce)
  }, [focusRequest, focusWorkflowRun, onFocusRequestHandled, sessionId])
  /* eslint-enable react-hooks/set-state-in-effect */

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
  const {
    visible: visibleBrowser,
    hasMore: hasMoreBrowser,
    setSentinel: setBrowserSentinel,
  } = useScrollPagedRender(browser, { step: RENDER_STEP, resetKey: sessionId })

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
        <EnvironmentSection
          sessionId={sessionId}
          sessionMeta={sessionMeta}
          project={project}
          effectiveWorkingDir={effectiveWorkingDir}
          workingDirSource={workingDirSource}
          permissionMode={permissionMode}
          planState={planState}
          turnActive={turnActive}
          onOpenGitDiff={onOpenGitDiff}
          onFillInput={onFillInput}
          onOpenPullRequest={onOpenPullRequest}
        />

        <div ref={goalSectionRef}>
          <GoalWorkspaceSection
            sessionId={sessionId}
            incognito={incognito}
            onEnsureSession={onEnsureSession}
            goalState={goalState}
          />
        </div>

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

        <MemoryDiagnosticsSection messages={messages} incognito={incognito} />
        <div ref={progressSectionRef}>
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
        </div>

        <div ref={workflowSectionRef}>
          <WorkflowRunsSection
            sessionId={sessionId}
            projectId={project?.id ?? sessionMeta?.projectId ?? null}
            incognito={incognito}
            turnActive={turnActive}
            workingDir={effectiveWorkingDir}
            onEnsureSession={onEnsureSession}
            draftWorkflowMode={draftWorkflowMode}
            onDraftWorkflowModeChange={onDraftWorkflowModeChange}
            onViewSubagentSession={onViewSubagentSession}
            workflowRunsState={sharedWorkflowRunsState}
            goalState={goalState}
            focusedRunTarget={workflowFocusTarget}
          />
        </div>

        <div ref={loopSectionRef}>
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
        </div>

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
                <SourceRow
                  key={sessionSourceKey(source)}
                  source={source}
                  sessionId={sessionId}
                  onPreviewFile={onPreviewFile}
                />
              ))}
              {hasMoreSources && <div ref={setSourcesSentinel} className="h-px" />}
              {sourcesTruncated && <TruncatedNote />}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptySources", "还没有引用来源")}</EmptyHint>
          )}
        </WorkspaceSection>

        {/* 浏览器 — 本会话浏览器工具活动。实时画面仍在 BrowserPanel。 */}
        <WorkspaceSection
          title={t("workspace.sectionBrowser", "浏览器")}
          count={browser.length}
          icon={Monitor}
        >
          {browser.length > 0 ? (
            <div className="max-h-[40vh] space-y-0.5 overflow-y-auto pr-0.5">
              {visibleBrowser.map((activity, index) => (
                <BrowserActivityRow
                  key={
                    activity.callId ??
                    `${activity.at ?? index}:${activity.action}:${activity.op ?? ""}:${activity.url ?? ""}`
                  }
                  activity={activity}
                  onOpenBrowserPanel={onOpenBrowserPanel}
                />
              ))}
              {hasMoreBrowser && <div ref={setBrowserSentinel} className="h-px" />}
              {browserTruncated && <TruncatedNote />}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptyBrowser", "还没有浏览器活动")}</EmptyHint>
          )}
        </WorkspaceSection>

        {/* 知识空间 — 挂载的库(读/写)+ 本会话笔记活动。 */}
        <KnowledgeSection
          sessionId={sessionId}
          projectId={project?.id ?? sessionMeta?.projectId ?? null}
          incognito={incognito}
          messages={messages}
        />

        <div className="px-1 pt-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground/75">
          {t("workspace.advancedDiagnostics.title", "高级诊断")}
        </div>

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
      </div>
    </div>
  )
}
