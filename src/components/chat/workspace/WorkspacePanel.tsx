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
  Loader2,
  Lock,
  MessageCircle,
  MessageSquare,
  Monitor,
  Radio,
  Search,
  Server,
  Shield,
  ShieldAlert,
  X,
  type LucideIcon,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
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
import type { SessionBrowserActivity } from "./useSessionBrowserActivity"
import { useWorkspaceArtifacts } from "./useWorkspaceArtifacts"
import { useWorkspaceEnvironment } from "./useWorkspaceEnvironment"
import { useScrollPagedRender } from "./useScrollPagedRender"
import { useSessionKnowledge } from "./useSessionKnowledge"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"
import {
  buildWorkspaceMemoryDiagnostics,
  formatWorkspaceMemoryDiagnosticsMarkdown,
  workspaceMemoryDiagnosticsCopyErrorToast,
  type WorkspaceMemoryLayerSummary,
} from "./workspaceMemoryDiagnostics"
import { workspaceSourceOpenErrorToast } from "./workspaceSourceFeedback"
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
    () => ({
      kind: "path",
      path: entry.path,
      name,
      language: entry.language ?? diff?.language ?? null,
    }),
    [diff?.language, entry.language, entry.path, name],
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
      </button>
    </IconTip>
  )
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

const STATUS_TONE_CLASS: Record<string, string> = {
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
  tone: "muted" | "good" | "warn" | "danger" | "info"
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
  if (status === "used") return "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
  if (status === "candidates") return "border-blue-500/35 bg-blue-500/10 text-blue-700 dark:text-blue-300"
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
            <MemoryMetric label={t("workspace.memoryDiagnostics.turns", "轮次")} value={diagnostics.turns} />
            <MemoryMetric label={t("workspace.memoryDiagnostics.contextRefs", "入上下文")} value={diagnostics.contextRefCount} />
            <MemoryMetric label={t("workspace.memoryDiagnostics.candidates", "候选")} value={diagnostics.candidateRefCount} />
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
                    title={`${retrievalTraceStatusLabel(turn.status, t)} · ${turn.contextRefCount}/${turn.candidateRefCount}`}
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
                    tone={status === "used" ? "good" : status === "candidate" ? "info" : status === "skipped" ? "warn" : "muted"}
                  />
                </div>
              )
            })}
          </div>

          {diagnostics.degradedLayers.length > 0 ? (
            <div className="space-y-1 rounded-md border border-amber-500/25 bg-amber-500/5 px-2 py-1.5">
              {diagnostics.degradedLayers.map((layer) => (
                <div key={`${layer.layer}:${layer.status}:${layer.reason ?? ""}`} className="flex min-w-0 items-center gap-2 text-[11px]">
                  <CircleAlert className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                  <span className="min-w-0 flex-1 truncate text-foreground/80">
                    {retrievalLayerLabel(layer.layer, t)}
                    {layer.reason ? ` · ${retrievalLayerReasonLabel(layer.reason, t)}` : ""}
                  </span>
                  <span className="shrink-0 tabular-nums text-muted-foreground">x{layer.count}</span>
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
        ) : workingDir ? (
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
  backgroundJobs = [],
  backgroundJobExpansionOverrides,
  onBackgroundJobExpandedChange,
  onOpenBackgroundJobs,
  onOpenBrowserPanel,
  onViewSubagentSession,
  onClose,
}: WorkspacePanelProps) {
  const { t } = useTranslation()
  const { files, sources, browser, filesTruncated, sourcesTruncated, browserTruncated } =
    useWorkspaceArtifacts(sessionId, messages, { incognito, turnActive })

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

        <MemoryDiagnosticsSection messages={messages} incognito={incognito} />

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
      </div>
    </div>
  )
}
