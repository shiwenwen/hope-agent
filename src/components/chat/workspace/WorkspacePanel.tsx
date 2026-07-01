import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
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
  Loader2,
  Lock,
  MessageCircle,
  MessageSquare,
  Monitor,
  Pause,
  Play,
  Plus,
  Radio,
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
import type { WorkspaceGitSnapshot } from "@/lib/transport"
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
  type GoalTimelineItem,
} from "./useGoal"
import {
  useLoopSchedules,
  type LoopSchedule,
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
}: {
  title: string
  count?: number
  icon: LucideIcon
  children: ReactNode
  meta?: ReactNode
  defaultExpanded?: boolean
}) {
  const [expanded, setExpanded] = useState(defaultExpanded)
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
const WORKFLOW_OP_PREVIEW = 6
const WORKFLOW_FOCUS_OP_PREVIEW = 4

type ExecutionMode = "off" | "guarded" | "deep" | "autonomous"
type WorkflowDetailTab = "trace" | "validation" | "agents"

const WORKFLOW_KIND_DEFAULT = "coding.workflow"
const WORKFLOW_SCRIPT_TEMPLATE = `export default async function main(workflow) {
  const observe = await workflow.task.create({
    title: "Observe workspace",
    label: "observe",
  });

  await workflow.trace({
    label: "observe",
    payload: { summary: "Manual workflow started" },
  });
  await workflow.fileSearch({
    query: "TODO",
    limit: 5,
    label: "observe-files",
  });
  await workflow.task.update({ task: observe, status: "completed" });

  const validate = await workflow.task.create({
    title: "Run targeted validation",
    label: "validate",
  });
  await workflow.validate({
    commands: ["pnpm typecheck"],
    reason: "targeted validation before finish; repair budget is bounded by execution mode",
    label: "targeted-validation",
  });
  await workflow.task.update({ task: validate, status: "completed" });

  await workflow.finish({
    summary: "Manual workflow completed",
    changedFiles: [],
    verification: ["pnpm typecheck"],
    residualRisk: [],
  });
}
`

function workflowJsLiteral(value: string): string {
  return JSON.stringify(value.trim()).replace(/</g, "\\u003C")
}

function buildGoalDrivenWorkflowScript(objective: string): string {
  const target = objective.trim() || "Complete the requested coding task."
  const targetLiteral = workflowJsLiteral(target)
  const implementationTask = workflowJsLiteral(`Implement this coding target end-to-end:

${target}

Work in the current workspace. First inspect the relevant files, then make the smallest coherent code change, and finish with a concise summary of changed files, validation performed, and residual risk. Follow the repository AGENTS.md instructions: run targeted validation only; do not run the full pre-push suite unless explicitly requested.`)

  return `export default async function main(workflow) {
  // Budget: owner create request sets maxScriptSecs/maxOps/maxOutputTokens by execution mode; waitAll and validation stay bounded.
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
    title: "Implement target",
    label: "implement",
  });
  const worker = await workflow.spawnAgent({
    task: ${implementationTask},
    label: "implement-target",
  });
  const implementation = await workflow.waitAll([worker], {
    waitTimeout: 90,
    label: "wait-implementation",
  });
  await workflow.task.update({ task: implement, status: "completed" });

  const validate = await workflow.task.create({
    title: "Run targeted validation",
    label: "validate",
  });
  const validation = await workflow.validate({
    commands: ["pnpm typecheck"],
    reason: "targeted validation after the implementation agent finishes; full pre-push suite remains user-gated",
    label: "targeted-validation",
  });
  const diff = await workflow.diff({ label: "final-diff" });
  await workflow.task.update({ task: validate, status: "completed" });

  await workflow.finish({
    summary: "Goal-driven coding workflow finished",
    target: ${targetLiteral},
    searchedFiles: files.matches ?? [],
    implementation,
    validation,
    diff,
    verification: ["pnpm typecheck"],
    residualRisk: validation.ok ? [] : ["Targeted validation did not pass; inspect the Validation tab."],
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

function executionModeHint(
  t: ReturnType<typeof useTranslation>["t"],
  mode: ExecutionMode,
): string {
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

function workflowRunActionSpecs(
  t: ReturnType<typeof useTranslation>["t"],
  state: WorkflowRunState,
): WorkflowRunActionSpec[] {
  const cancel: WorkflowRunActionSpec = {
    command: "cancel_workflow_run",
    label: t("workspace.workflow.cancel", "取消"),
    success: t("workspace.workflow.cancelled", "已取消 workflow"),
    icon: X,
    danger: true,
  }

  switch (state) {
    case "draft":
      return [
        {
          command: "run_workflow_run",
          label: t("workspace.workflow.run", "运行"),
          success: t("workspace.workflow.runStarted", "已启动 workflow"),
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
          success: t("workspace.workflow.approved", "已批准 workflow"),
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
          success: t("workspace.workflow.paused", "已暂停 workflow"),
          icon: Pause,
        },
        cancel,
      ]
    case "paused":
      return [
        {
          command: "resume_workflow_run",
          label: t("workspace.workflow.resume", "恢复"),
          success: t("workspace.workflow.resumed", "已恢复 workflow"),
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
    "请基于下面的 Workflow 失败上下文继续修复。先定位根因，必要时调整代码或 workflow 脚本，然后运行最小验证。",
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

function workflowEventTitle(
  t: ReturnType<typeof useTranslation>["t"],
  event: WorkflowEvent,
): string {
  const payload = asRecord(event.payload)
  switch (event.eventType) {
    case "run_created":
      return t("workspace.workflow.eventRunCreated", "Workflow 已创建")
    case "run_state_changed":
      return t("workspace.workflow.eventRunStateChanged", "状态已更新")
    case "run_recovery_claimed":
      return t("workspace.workflow.eventRecoveryClaimed", "恢复接管")
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
      return t("workspace.workflow.eventRunDerivedChildCreated", "已生成派生 run")
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
    case "script_permission_preview":
    case "script_permission_preview_blocked":
    case "script_permission_approval_required":
      return workflowPermissionSummaryText(t, asRecord(payload?.summary))
    case "budget_usage": {
      const spent = numberField(payload, "spentOutputTokens")
      const limit = numberField(payload, "maxOutputTokens")
      const exhausted = boolField(payload, "exhausted")
      const reason = stringField(payload, "reason")
      const usage =
        typeof spent === "number" && typeof limit === "number"
          ? `${compactCount(spent)}/${compactCount(limit)}`
          : null
      return [
        usage,
        exhausted ? t("workspace.workflow.budgetExhausted", "已达上限") : null,
        reason,
      ]
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

function LoopSchedulesSection({
  sessionId,
  incognito,
  turnActive,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
}) {
  const { t } = useTranslation()
  const { schedules, activeCount, loading, error, refresh } = useLoopSchedules(sessionId, {
    incognito,
    turnActive,
  })
  const goalState = useGoal(sessionId, { incognito })
  const [actionId, setActionId] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [createSaving, setCreateSaving] = useState(false)
  const [draftKind, setDraftKind] = useState<"interval" | "condition">("interval")
  const [draftInterval, setDraftInterval] = useState("10m")
  const [draftCondition, setDraftCondition] = useState("")
  const [draftPrompt, setDraftPrompt] = useState("")
  const [draftMaxRuns, setDraftMaxRuns] = useState("")
  const [draftMaxRuntime, setDraftMaxRuntime] = useState("")
  const [draftTokens, setDraftTokens] = useState("")
  const activeGoal = goalState.snapshot?.goal ?? null

  const runAction = useCallback(
    async (loop: LoopSchedule, action: "pause" | "resume" | "stop") => {
      setActionId(`${loop.id}:${action}`)
      try {
        await getTransport().call(`${action}_loop_schedule`, { loopId: loop.id })
        refresh()
      } catch (e) {
        logger.error("ui", "LoopSchedulesSection::action", "Loop action failed", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setActionId(null)
      }
    },
    [refresh],
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
      toast.error(t("workspace.loop.promptOrGoalRequired", "请输入 prompt，或先创建一个 active goal"))
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
        triggerSpec:
          draftKind === "condition"
            ? { condition, intervalSecs }
            : { intervalSecs },
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
    draftCondition,
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
                  ? t("workspace.loop.promptOptionalPlaceholder", "每次触发要做什么；留空则只检查条件并推进下一步")
                  : activeGoal
                    ? t("workspace.loop.promptGoalPlaceholder", "留空则继续当前 active goal")
                    : t("workspace.loop.promptPlaceholder", "check CI and continue fixing if failing")
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
                {createSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Radio className="h-3.5 w-3.5" />}
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
            return (
              <div key={loop.id} className="rounded-md border border-border/70 bg-background/70 px-2.5 py-2">
                <div className="flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <StatusPill label={loopStateLabel(t, loop.state)} tone={loopStateTone(loop.state)} />
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
                          {t("workspace.loop.maxRuntime", "最长")} {formatLoopDuration(loop.maxRuntimeSecs)}
                        </span>
                      ) : null}
                      <span>{formatMessageTime(loop.updatedAt)}</span>
                    </div>
                    {loop.blockedReason ? (
                      <p className="mt-1 text-[10px] text-destructive">{loop.blockedReason}</p>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
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
  workflowRunsState,
}: {
  sessionId?: string | null
  incognito?: boolean
  turnActive?: boolean
  workingDir?: string | null
  onEnsureSession?: () => Promise<string | null>
  onViewSubagentSession?: (sessionId: string) => void
  workflowRunsState?: WorkflowRunsState
}) {
  const { t } = useTranslation()
  const ownedWorkflowRuns = useWorkflowRuns(sessionId, {
    incognito,
    turnActive,
    disabled: Boolean(workflowRunsState),
  })
  const { runs, activeCount, loading, error, refresh } = workflowRunsState ?? ownedWorkflowRuns
  const goalState = useGoal(sessionId, { incognito })
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  const [snapshot, setSnapshot] = useState<WorkflowRunSnapshot | null>(null)
  const [snapshotLoading, setSnapshotLoading] = useState(false)
  const [actionKey, setActionKey] = useState<string | null>(null)
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
  const [draftObjective, setDraftObjective] = useState("")
  const [draftScript, setDraftScript] = useState(WORKFLOW_SCRIPT_TEMPLATE)
  const [draftOrigin, setDraftOrigin] = useState<WorkflowDraftOrigin | null>(null)
  const [showAllRuns, setShowAllRuns] = useState(false)
  const [pendingCancelRun, setPendingCancelRun] = useState<WorkflowRun | null>(null)
  const [goalActionKey, setGoalActionKey] = useState<string | null>(null)
  const [goalCreateOpen, setGoalCreateOpen] = useState(false)
  const [goalObjective, setGoalObjective] = useState("")
  const [goalCriteria, setGoalCriteria] = useState("")
  const [goalSaving, setGoalSaving] = useState(false)
  const snapshotReqRef = useRef(0)
  const executionModeReqRef = useRef(0)
  const previewReqRef = useRef(0)
  const autoDetailTabRunRef = useRef<string | null>(null)
  const ensureSessionRef = useRef<Promise<string | null> | null>(null)

  const selectedRun = runs.find((run) => run.id === selectedRunId) ?? null
  const visibleRuns = showAllRuns ? runs : runs.slice(0, WORKFLOW_RUN_PREVIEW)
  const canMaterializeSession = Boolean(sessionId || onEnsureSession)
  const activeGoal = goalState.snapshot?.goal ?? null

  const ensureWorkflowSession = useCallback(async () => {
    if (sessionId) return sessionId
    if (!onEnsureSession) {
      toast.error(t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建 workflow"))
      return null
    }
    if (!ensureSessionRef.current) {
      ensureSessionRef.current = onEnsureSession().finally(() => {
        ensureSessionRef.current = null
      })
    }
    const nextSessionId = await ensureSessionRef.current
    if (!nextSessionId) {
      toast.error(t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建 workflow"))
    }
    return nextSessionId
  }, [onEnsureSession, sessionId, t])

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
        setSnapshot(next)
        if (next && autoDetailTabRunRef.current !== next.run.id) {
          setDetailTab(workflowInitialDetailTab(next))
          autoDetailTabRunRef.current = next.run.id
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
    loadExecutionMode()
  }, [loadExecutionMode])

  const updateExecutionMode = useCallback(
    async (nextMode: ExecutionMode) => {
      if (!sessionId || incognito || nextMode === executionMode) return
      setExecutionModeSaving(nextMode)
      try {
        const next = await getTransport().call<unknown>("set_execution_mode", {
          sessionId,
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
    [incognito, executionMode, sessionId, t],
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
      })
      goalState.setSnapshot(snapshot)
      setGoalObjective("")
      setGoalCriteria("")
      setGoalCreateOpen(false)
      toast.success(t("workspace.goal.created", "已创建 Goal"))
    } catch (e) {
      logger.error("ui", "WorkflowRunsSection::createGoal", "Failed to create goal", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setGoalSaving(false)
    }
  }, [ensureWorkflowSession, goalCriteria, goalObjective, goalState, incognito, t])

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

  const clearDraftPreview = useCallback(() => {
    previewReqRef.current += 1
    setDraftPreview(null)
    setDraftPreviewError(null)
    setDraftPreviewLoading(false)
  }, [])

  const previewWorkflowScriptSource = useCallback(
    async (
      scriptSource: string,
      mode: ExecutionMode,
      toastMessages: { passed?: string; blocked?: string } = {},
    ) => {
      if (incognito) return null
      const script = scriptSource.trim()
      if (!script) {
        toast.error(t("workspace.workflow.scriptRequired", "请输入 workflow 脚本"))
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

  const generateGoalDrivenDraft = useCallback(() => {
    const objective = draftObjective.trim()
    if (!objective) {
      toast.error(t("workspace.workflow.objectiveRequired", "请输入要完成的 coding 目标"))
      return
    }
    setDraftKind(WORKFLOW_KIND_DEFAULT)
    setDraftScript(buildGoalDrivenWorkflowScript(objective))
    setDraftRunImmediately(Boolean(workingDir))
    setDraftOrigin(null)
    clearDraftPreview()
    if (workingDir) {
      toast.success(t("workspace.workflow.objectiveDraftReady", "已生成目标驱动 workflow 草稿"))
    } else {
      toast.warning(
        t(
          "workspace.workflow.objectiveDraftNeedsWorkspace",
          "已生成草稿；设置工作目录后再运行更稳妥",
        ),
      )
    }
  }, [clearDraftPreview, draftObjective, t, workingDir])

  const generateRepairDraft = useCallback(
    (repairPrompt: string, run: WorkflowRun) => {
      const sourceMode = normalizeExecutionMode(run.executionMode)
      const nextMode = sourceMode === "off" ? "guarded" : sourceMode
      const objective = `继续修复失败的 workflow run ${run.id}。

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
      const script = buildGoalDrivenWorkflowScript(objective)
      setDraftScript(script)
      setDraftRunImmediately(Boolean(workingDir))
      if (workingDir) {
        toast.success(t("workspace.workflow.repairDraftReady", "已生成修复 workflow 草稿"))
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
    [previewWorkflowScriptSource, t, workingDir],
  )

  const previewWorkflowDraft = useCallback(async () => {
    await previewWorkflowScriptSource(draftScript, draftMode)
  }, [draftMode, draftScript, previewWorkflowScriptSource])

  const createWorkflow = useCallback(async () => {
    if (incognito) return
    const script = draftScript.trim()
    if (!script) {
      toast.error(t("workspace.workflow.scriptRequired", "请输入 workflow 脚本"))
      return
    }
    if (!draftPreview?.canCreate) {
      toast.error(t("workspace.workflow.previewRequired", "请先完成预检并修复阻塞项"))
      return
    }
    const targetSessionId = await ensureWorkflowSession()
    if (!targetSessionId) return
    setCreateSaving(true)
    const runImmediatelyForCreate = Boolean(workingDir) && draftRunImmediately
    try {
      const run = await getTransport().call<WorkflowRun>("create_workflow_run", {
        sessionId: targetSessionId,
        kind: draftKind.trim() || WORKFLOW_KIND_DEFAULT,
        executionMode: draftMode,
        scriptSource: script,
        budget: workflowBudgetForMode(draftMode),
        parentRunId: draftOrigin?.type === "repair" ? draftOrigin.runId : undefined,
        origin: draftOrigin?.type === "repair" ? "repair" : undefined,
        goalId: activeGoal?.id ?? undefined,
        runImmediately: runImmediatelyForCreate,
      })
      setSelectedRunId(run.id)
      loadSnapshot(run.id)
      refresh()
      toast.success(
        draftOrigin?.type === "repair"
          ? runImmediatelyForCreate
            ? t("workspace.workflow.repairCreatedAndStarted", "已创建并启动修复 workflow")
            : t("workspace.workflow.repairCreated", "已创建修复 workflow")
          : runImmediatelyForCreate
            ? t("workspace.workflow.createdAndStarted", "已创建并启动 workflow")
            : t("workspace.workflow.created", "已创建 workflow"),
      )
      setCreateOpen(false)
      setDraftOrigin(null)
      clearDraftPreview()
    } catch (e) {
      logger.error("ui", "WorkflowRunsSection::createWorkflow", "Failed to create workflow", e)
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setCreateSaving(false)
    }
  }, [
    clearDraftPreview,
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
        title={t("workspace.workflow.title", "Workflow")}
        count={activeCount}
        icon={GitPullRequest}
        meta={
          loading ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" /> : null
        }
      >
        {incognito ? (
          <EmptyHint>{t("workspace.workflow.incognito", "无痕会话不持久化 workflow")}</EmptyHint>
        ) : (
          <div className="space-y-2">
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
              saving={goalSaving}
              actionKey={goalActionKey}
              disabled={!canMaterializeSession}
              onCreateOpenChange={setGoalCreateOpen}
              onObjectiveChange={setGoalObjective}
              onCriteriaChange={setGoalCriteria}
              onCreate={() => void createGoalFromDraft()}
              onPause={() => void runGoalAction("pause_goal")}
              onResume={() => void runGoalAction("resume_goal")}
              onClear={() => void runGoalAction("clear_goal")}
              onEvaluate={() => void runGoalAction("evaluate_goal")}
            />
            <WorkflowCreateComposer
              open={createOpen}
              disabled={!canMaterializeSession}
              disabledReason={
                !canMaterializeSession
                  ? t("workspace.workflow.sessionRequired", "先选择或创建一个会话后再新建 workflow")
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
              runImmediately={draftRunImmediately}
              onOpenChange={setCreateOpen}
              onKindChange={setDraftKind}
              onModeChange={(mode) => {
                setDraftMode(mode)
                clearDraftPreview()
              }}
              onScriptChange={(script) => {
                setDraftScript(script)
                clearDraftPreview()
              }}
              onObjectiveChange={(objective) => {
                setDraftObjective(objective)
                clearDraftPreview()
              }}
              onClearDraftOrigin={() => setDraftOrigin(null)}
              onRunImmediatelyChange={setDraftRunImmediately}
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
                          ? t("workspace.workflow.collapseRuns", "收起历史 run")
                          : t("workspace.workflow.moreRuns", "另有 {{count}} 个历史 run", {
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
              {t("workspace.workflow.cancelConfirmTitle", "取消这个 workflow run？")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                "workspace.workflow.cancelConfirmBody",
                "会停止这个 run，并尽量取消它拥有的后台任务、验证命令和子 Agent；已有 trace 会保留，方便之后复盘或生成修复草稿。",
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
          {t("workspace.workflow.emptyTitle", "准备开始 Coding run")}
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
        <span className="truncate">{t("workspace.workflow.emptyCreate", "开始 Coding run")}</span>
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
  runImmediately,
  onOpenChange,
  onKindChange,
  onModeChange,
  onObjectiveChange,
  onScriptChange,
  onClearDraftOrigin,
  onRunImmediatelyChange,
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
  runImmediately: boolean
  onOpenChange: (open: boolean) => void
  onKindChange: (kind: string) => void
  onModeChange: (mode: ExecutionMode) => void
  onObjectiveChange: (objective: string) => void
  onScriptChange: (script: string) => void
  onClearDraftOrigin: () => void
  onRunImmediatelyChange: (checked: boolean) => void
  onGenerateGoalDraft: () => void
  onPreview: () => void
  onSubmit: () => void
}) {
  const { t } = useTranslation()
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const effectiveRunImmediately = workspaceReady && runImmediately
  const canPreview = !disabled && !saving && !previewLoading && script.trim().length > 0
  const canSubmit =
    !disabled &&
    !saving &&
    !previewLoading &&
    script.trim().length > 0 &&
    preview?.canCreate === true
  const canGenerate = !disabled && !saving && !previewLoading && objective.trim().length > 0
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
          {t("workspace.workflow.createTitle", "新建 Workflow")}
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
                      "将创建同一 Goal 下的修复 run，不会覆盖原 run · {{kind}}",
                      {
                        kind: repairOrigin.runKind,
                      },
                    )
                  : t(
                      "workspace.workflow.repairDraftOriginDetail",
                      "将创建新的修复 run，不会覆盖原 run · {{kind}}",
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
                {t("workspace.workflow.objectiveTitle", "从 coding 目标开始")}
              </span>
            </div>
            <Textarea
              value={objective}
              disabled={saving || previewLoading}
              onChange={(event) => onObjectiveChange(event.target.value)}
              placeholder={t(
                "workspace.workflow.objectivePlaceholder",
                "例如：修复登录页空白问题，并跑一次 pnpm typecheck",
              )}
              className="min-h-20 resize-y text-xs"
              aria-label={t("workspace.workflow.objectiveTitle", "从 coding 目标开始")}
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
              disabled={saving || previewLoading || !workspaceReady}
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
              disabled={saving || previewLoading}
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
                    : t("workspace.workflow.createRepair", "创建修复 run")
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
  if (relation.includes("failed") || relation.includes("blocked") || relation.includes("cancelled")) {
    return "danger"
  }
  if (relation.includes("passed") || relation.includes("completed")) {
    return "good"
  }
  if (relation.includes("diff") || relation.includes("file") || relation.includes("artifact")) {
    return "info"
  }
  return "muted"
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

function GoalControlStrip({
  snapshot,
  loading,
  error,
  createOpen,
  objective,
  criteria,
  saving,
  actionKey,
  disabled,
  onCreateOpenChange,
  onObjectiveChange,
  onCriteriaChange,
  onCreate,
  onPause,
  onResume,
  onClear,
  onEvaluate,
}: {
  snapshot: GoalSnapshot | null
  loading?: boolean
  error?: string | null
  createOpen: boolean
  objective: string
  criteria: string
  saving?: boolean
  actionKey?: string | null
  disabled?: boolean
  onCreateOpenChange: (open: boolean) => void
  onObjectiveChange: (value: string) => void
  onCriteriaChange: (value: string) => void
  onCreate: () => void
  onPause: () => void
  onResume: () => void
  onClear: () => void
  onEvaluate: () => void
}) {
  const { t } = useTranslation()
  const [detailOpen, setDetailOpen] = useState(false)
  const goal = snapshot?.goal ?? null
  const audit = asRecord(goal?.finalEvidence)
  const auditEvidence = recordArrayField(audit, "evidence")
  const criteriaAudit = snapshot?.criteria ?? []
  const evidenceItems = snapshot?.evidence ?? []
  const timelineItems = snapshot?.timeline ?? []
  const achieved = arrayField(audit, "achieved").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const missing = arrayField(audit, "missing").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const blockers = arrayField(audit, "blockers").filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  )
  const workflowCount = snapshot?.workflowRuns.length ?? 0
  const taskCount = snapshot?.tasks.length ?? 0
  const taskDone = snapshot?.tasks.filter((task) => task.status === "completed").length ?? 0
  const evidenceCount = evidenceItems.length || auditEvidence.length
  const isBusy = saving || Boolean(actionKey)
  const canCreate = !disabled && !saving && objective.trim().length > 0

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
        {loading ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" /> : null}
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
              {t("workspace.goal.noAudit", "还没有 final audit；workflow 完成后会自动评估，也可以手动评估。")}
            </div>
          )}

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

          <div className="grid grid-cols-3 gap-1.5">
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
  const audit = asRecord(snapshot.goal.finalEvidence)
  const nextEvidence = recordArrayField(audit, "nextEvidenceNeeded").slice(0, 6)
  const budget = snapshot.budget

  return (
    <AnimatedCollapse open={open}>
      <div className="space-y-2 rounded-md border border-border/55 bg-background/45 p-2">
        {budget ? <GoalBudgetCard budget={budget} /> : null}

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

        <GoalDetailBlock
          title={t("workspace.goal.detailEvidence", "证据")}
          count={evidence.length}
        >
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

        <GoalDetailBlock title={t("workspace.goal.detailTimeline", "时间线")} count={timeline.length}>
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
  actions,
  onSelectDetailTab,
  onCreateRepairDraft,
}: {
  run: WorkflowRun
  snapshot: WorkflowRunSnapshot | null
  latestEvent?: WorkflowEvent
  actions?: ReactNode
  onSelectDetailTab?: (tab: WorkflowDetailTab) => void
  onCreateRepairDraft?: (repairPrompt: string, run: WorkflowRun) => void
}) {
  const { t } = useTranslation()
  const ops = snapshot?.ops ?? []
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
                    ? t("workspace.workflow.derivedChildRepair", "已生成修复 run {{id}}", {
                        id: childRunId,
                      })
                    : t("workspace.workflow.derivedChild", "已生成派生 run {{id}}", {
                        id: childRunId,
                      })}
                </span>
              </div>
            )
          })}
        </div>
      ) : null}

      <WorkflowRunFocusCard run={run} snapshot={snapshot} onSelectDetailTab={onSelectDetailTab} />
      <WorkflowApprovalPreview snapshot={snapshot} />
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
      t("workspace.workflow.focusUserBody", "当前 run 正在等待会话里的用户输入或外部确认。")
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
        : t("workspace.workflow.focusCompletedBodyNoOps", "run 已完成，trace 已保留。")
    tone = "good"
    Icon = CheckCircle2
    targetTab = "trace"
  } else {
    title = t("workspace.workflow.focusCancelledTitle", "当前焦点：已取消")
    body = t("workspace.workflow.focusCancelledBody", "run 已停止，已有 trace 可用于复盘。")
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
      "当前 run 已暂停，可恢复继续执行，也可取消并保留 trace。",
    )
    tone = "warn"
  } else if (run.state === "blocked") {
    title = t("workspace.workflow.nextBlockedTitle", "下一步：处理阻塞")
    body =
      blockedReason === "script_hash_mismatch"
        ? t(
            "workspace.workflow.nextBlockedScriptHash",
            "脚本内容已变化；请基于当前目标生成新的 workflow。",
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
  const command =
    stringField(result, "command") ?? t("workspace.workflow.validationCommand", "验证命令")
  const cwd = stringField(result, "cwd")
  const jobStatus = stringField(result, "jobStatus")
  const ok = boolField(result, "ok")
  const exitCode = numberField(result, "exitCode")
  const output = stringField(result, "output")
  const tone: "good" | "danger" | "info" =
    ok === true
      ? "good"
      : ok === false || (typeof exitCode === "number" && exitCode !== 0)
        ? "danger"
        : "info"

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
        <WorkflowRunsSection
          sessionId={sessionId}
          incognito={incognito}
          turnActive={turnActive}
          workingDir={effectiveWorkingDir}
          onEnsureSession={onEnsureSession}
          onViewSubagentSession={onViewSubagentSession}
          workflowRunsState={workflowRunsState}
        />

        {/* Loop — Phase 2.9 真正的定时/重复/条件触发控制面。 */}
        <LoopSchedulesSection sessionId={sessionId} incognito={incognito} turnActive={turnActive} />

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
