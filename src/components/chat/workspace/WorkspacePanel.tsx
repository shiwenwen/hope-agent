import { useCallback, useMemo, useRef, useState, type ReactNode } from "react"
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
import { getTransport } from "@/lib/transport-provider"
import { useDangerousModeStatus } from "@/hooks/useDangerousModeStatus"
import type { WorkspaceGitSnapshot } from "@/lib/transport"
import { computeContextUsage, contextUsageLevel, formatMessageTime } from "../chatUtils"
import { formatCacheUsageDisplay, formatCompactTokenCount } from "../cacheUsageDisplay"
import type { CommandResult } from "../slash-commands/types"
import type {
  ActiveModel,
  AvailableModel,
  FileChangeMetadata,
  Message,
  SessionMeta,
  SessionMode,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import { FileMimeIcon } from "@/components/chat/message/FileCard"
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
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"
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
  /** 改写类文件「查看 diff」→ 右侧 diff 面板。 */
  onOpenDiff: (payload: FileChangeMetadata) => void
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
  /** 「查看系统提示词」—— 复用 ChatScreen 的 loadSystemPrompt。 */
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
  /** 无痕会话:跳过后端聚合,只用 live tail（守「关闭即焚」）。 */
  incognito?: boolean
  /** 当前会话是否正在跑一轮:true→false 跳变时面板重新拉后端聚合。 */
  turnActive?: boolean
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
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/95 shadow-sm">
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
  onOpenDiff: (payload: FileChangeMetadata) => void
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
              <span className="shrink-0 text-[10px] tabular-nums">
                <span className="text-emerald-600 dark:text-emerald-400">+{entry.linesAdded}</span>{" "}
                <span className="text-rose-600 dark:text-rose-400">-{entry.linesRemoved}</span>
              </span>
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
  return (
    <IconTip label={source.url}>
      <button
        type="button"
        onClick={() => openExternalUrl(source.url)}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-secondary/45"
      >
        <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-xs text-foreground/90">{domainOf(source.url)}</span>
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

function EnvRow({
  icon: Icon,
  label,
  value,
  detail,
  tone = "muted",
  title,
}: {
  icon: LucideIcon
  label: string
  value: ReactNode
  detail?: ReactNode
  tone?: "muted" | "good" | "warn" | "danger" | "info"
  title?: string
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
  const row = (
    <div className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-secondary/35">
      <Icon className={cn("h-3.5 w-3.5 shrink-0", iconClass)} />
      <span className="w-14 shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 flex-1 truncate font-medium text-foreground/90">{value}</span>
      {detail ? <span className="max-w-[45%] shrink-0 truncate text-muted-foreground">{detail}</span> : null}
    </div>
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

function gitSyncLabel(t: ReturnType<typeof useTranslation>["t"], git: WorkspaceGitSnapshot | null): string | null {
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

/** Green → amber → red fill for the session card's context-usage bar (shared thresholds). */
function sessionUsageBarClass(pct: number): string {
  const level = contextUsageLevel(pct)
  return level === "low"
    ? "bg-green-500/70"
    : level === "mid"
      ? "bg-yellow-500/70"
      : "bg-red-500/70"
}

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
  currentAgentId,
  turnActive,
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
  currentAgentId?: string
  turnActive?: boolean
  onCommandAction?: (result: CommandResult) => void
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
}) {
  const { t } = useTranslation()
  const [showMore, setShowMore] = useState(false)
  const [compacting, setCompacting] = useState(false)
  const [copied, setCopied] = useState(false)
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const currentModel = useMemo(
    () =>
      activeModel
        ? ((availableModels ?? []).find(
            (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId,
          ) ?? null)
        : null,
    [activeModel, availableModels],
  )
  const usage = useMemo(
    () => (currentModel ? computeContextUsage(messages, currentModel.contextWindow) : null),
    [currentModel, messages],
  )
  // Cache stats from the latest assistant turn that carries usage (Anthropic).
  const cache = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i]
      if (m.role !== "assistant" || !m.usage) continue
      const u = m.usage
      if (u.cacheCreationInputTokens == null && u.cacheReadInputTokens == null) return null
      return {
        created: u.cacheCreationInputTokens || 0,
        read: u.cacheReadInputTokens || 0,
        lastInput: u.lastInputTokens,
      }
    }
    return null
  }, [messages])

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
    setCompacting(true)
    try {
      const result = await getTransport().call<{
        tierApplied: number
        tokensBefore: number
        tokensAfter: number
        messagesAffected: number
      }>("compact_context_now", { sessionId })
      const saved = result.tokensBefore - result.tokensAfter
      toast.success(
        result.messagesAffected > 0
          ? t("chat.compactDone", { saved, affected: result.messagesAffected })
          : t("chat.compactNoChange"),
      )
    } catch (e) {
      logger.error("ui", "WorkspaceSession::compact", "Compact failed", e)
      toast.error(t("chat.compactFailed"))
    } finally {
      setCompacting(false)
    }
  }, [sessionId, t])

  const handleViewContext = useCallback(async () => {
    if (!sessionId) return
    try {
      const result = await getTransport().call<CommandResult>("execute_slash_command", {
        sessionId,
        agentId: currentAgentId,
        commandText: "/context",
      })
      result._slashCommandText = "/context"
      onCommandAction?.(result)
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
                  sessionUsageBarClass(usage.pct),
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
                className={cn(SESSION_ACTION_BTN, "inline-flex flex-1 items-center justify-center gap-1")}
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
          <EnvRow icon={Bot} label={t("chat.statusAgent")} value={agentName || t("chat.mainAgent")} />
        </div>

        {/* 展开更多 / 收起 */}
        <button
          type="button"
          aria-expanded={showMore}
          onClick={() => setShowMore((v) => !v)}
          className="flex w-full items-center justify-center gap-1 rounded-md py-1 text-[11px] text-muted-foreground transition-colors hover:bg-secondary/45 hover:text-foreground"
        >
          {showMore ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          {showMore ? t("workspace.sessionShowLess", "收起") : t("workspace.sessionShowMore", "展开更多")}
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
}: {
  sessionId?: string | null
  sessionMeta?: SessionMeta | null
  project?: ProjectMeta | null
  effectiveWorkingDir?: string | null
  workingDirSource?: "session" | "project"
  permissionMode?: SessionMode
  planState?: PlanModeState
  turnActive?: boolean
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
  const isLocalRuntime = useMemo(() => getTransport().supportsLocalFileOps(), [])
  const status = resolveWorkspaceEnvironmentStatus(
    env.snapshot,
    effectiveWorkingDir,
    !!env.error,
  )
  const statusLabel = t(status.labelKey, status.fallback)
  const workingDir = env.snapshot?.workingDir.path ?? effectiveWorkingDir ?? null
  const workingDirName = env.snapshot?.workingDir.name ?? (workingDir ? basename(workingDir) : null)
  const source =
    env.snapshot?.workingDir.source ??
    (workingDirSource === "project" ? "project" : workingDirSource === "session" ? "session" : "none")
  const sourceLabel = workingDirSourceLabelKey(source)
  const git = env.snapshot?.git ?? null
  const currentWorktree = git?.worktrees.find((w) => w.isCurrent) ?? null
  const syncLabel = git ? gitSyncLabel(t, git) : null

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
      ? { icon: CalendarClock, value: t("workspace.environment.sourceCron", "定时任务"), detail: null }
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
              detail={git.detached ? t("fileBrowser.gitDetached", "detached") : git.head ?? undefined}
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
                git.status.clean
                  ? t("workspace.environment.clean", "无本地变更")
                  : t("workspace.environment.changedFiles", "{{count}} 个文件", {
                      count: git.status.changedFiles,
                    })
              }
              detail={
                git.status.linesAdded > 0 || git.status.linesRemoved > 0 ? (
                  <span>
                    <span className="text-emerald-600 dark:text-emerald-400">
                      +{git.status.linesAdded}
                    </span>{" "}
                    <span className="text-rose-600 dark:text-rose-400">
                      -{git.status.linesRemoved}
                    </span>
                  </span>
                ) : git.status.conflictedFiles > 0 ? (
                  t("workspace.environment.conflictCount", "{{count}} 个冲突", {
                    count: git.status.conflictedFiles,
                  })
                ) : undefined
              }
              tone={git.status.conflictedFiles > 0 ? "danger" : git.status.clean ? "good" : "warn"}
            />
            {(syncLabel || git.sync.upstream || git.sync.remote) && (
              <EnvRow
                icon={GitPullRequest}
                label={t("workspace.environment.sync", "同步")}
                value={syncLabel ?? git.sync.upstream ?? t("workspace.environment.syncUnknown", "同步状态未知")}
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
 * 右侧「工作台」面板:把本会话的任务进度、碰到的文件、引用来源聚合到一处。
 * 文件 / 来源走 useWorkspaceArtifacts —— 后端读时聚合全会话历史 + 当前轮 live tail
 * 内存合并;输出 / 来源两段各自定高内部滚动,滚到底自动增量渲染(无按钮)。
 */
export default function WorkspacePanel({
  taskSnapshot,
  taskExecutionState = "idle",
  messages,
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
  onCommandAction,
  onViewSystemPrompt,
  systemPromptLoading,
  incognito = false,
  turnActive = false,
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
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
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

      <div className="flex-1 space-y-2 overflow-auto p-2">
        {/* 会话 — 复刻状态悬浮窗的能力(模型 / 上下文 / 动作),核心常驻 + 展开更多。 */}
        <SessionSection
          sessionId={sessionId}
          sessionMeta={sessionMeta}
          agentName={agentName}
          reasoningEffort={reasoningEffort}
          activeModel={activeModel}
          availableModels={availableModels}
          messages={messages}
          currentAgentId={currentAgentId}
          turnActive={turnActive}
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
        />

        {/* 进度 — 复用 TaskProgressPanel(自带「任务 · N/M」折叠头)。 */}
        {taskSnapshot && taskSnapshot.total > 0 ? (
          <TaskProgressPanel snapshot={taskSnapshot} variant="card" executionState={taskExecutionState} />
        ) : (
          <WorkspaceSection title={t("workspace.sectionProgress", "进度")} count={0} icon={LayoutDashboard}>
            <EmptyHint>{t("workspace.emptyProgress", "暂无任务")}</EmptyHint>
          </WorkspaceSection>
        )}

        {/* 输出 — 本会话碰到的文件(读 + 改),定高内部滚动 + 滚动增量渲染。 */}
        <WorkspaceSection title={t("workspace.sectionOutput", "输出")} count={files.length} icon={Files}>
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
        <WorkspaceSection title={t("workspace.sectionSources", "来源")} count={sources.length} icon={Globe}>
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
