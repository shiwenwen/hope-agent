import { Fragment, useRef, useEffect, useLayoutEffect, useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { AnimatedCollapse, AnimatedPresenceBox } from "@/components/ui/animated-presence"
import { FloatingMenu } from "@/components/ui/floating-menu"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import {
  Send,
  Square,
  Slash,
  ClipboardList,
  Pencil,
  Trash2,
  BetweenHorizontalStart,
  ChevronDown,
  ChevronUp,
  X,
  Plus,
  FolderPlus,
  Quote,
  Undo2,
  Target,
  Check,
  GitPullRequest,
  Sparkles,
  Loader2,
  PauseCircle,
  PlayCircle,
  CheckCircle2,
  Radio,
} from "lucide-react"
import type {
  AvailableModel,
  ActiveModel,
  ChatTurnStatus,
  SandboxMode,
  SessionMode,
  PendingFileQuote,
  PendingSendPreview,
  AgentSummaryForSidebar,
} from "@/types/chat"
import type { KbDraftAttachment } from "@/types/knowledge"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { useSlashCommands, type SlashCommandActions } from "../slash-commands/useSlashCommands"
import { useUrlPreview } from "@/hooks/useUrlPreview"
import SlashCommandMenu from "../slash-commands/SlashCommandMenu"
import { useFileMention } from "../file-mention/useFileMention"
import FileMentionMenu from "../file-mention/FileMentionMenu"
import { useNoteMention } from "../note-mention/useNoteMention"
import NoteMentionMenu from "../note-mention/NoteMentionMenu"
import QuickPromptMenu from "../quick-prompts/QuickPromptMenu"
import { useQuickPrompts } from "../quick-prompts/useQuickPrompts"
import UrlPreviewCard from "../UrlPreviewCard"
import type { CommandResult } from "../slash-commands/types"
import { AttachFilesButton, AttachFilesMenuItem, AttachmentPreview } from "./AttachmentBar"
import ModelPicker from "./ModelPicker"
import PermissionModeSwitcher, { type PermissionModeChangeOptions } from "./PermissionModeSwitcher"
import AwarenessToggle from "./AwarenessToggle"
import KnowledgePicker from "./KnowledgePicker"
import WorkingDirectoryButton from "./WorkingDirectoryButton"
import { VoiceRecordButton } from "./VoiceRecordButton"
import { useVoiceInput } from "./useVoiceInput"
import { RecordingBar } from "./RecordingBar"
import { getNextPermissionMode } from "./permissionModes"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import { resolveWorkspaceTaskExecutionState } from "@/components/chat/workspace/taskExecutionState"
import {
  shouldShowTaskProgressPanel,
  type TaskProgressSnapshot,
} from "@/components/chat/tasks/taskProgress"
import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  getChatInputOverflowActionIds,
  getChatInputToolbarFlags,
  resolveChatInputToolbarCollapseLevel,
  type ChatInputOverflowActionId,
  type ChatInputToolbarGroupWidths,
} from "./toolbarOverflow"
import MentionComposerInput from "./MentionComposerInput"
import type { ComposerPasteEvent } from "./MentionComposerInput"
import type { ComposerInputHandle } from "./composerInputHandle"
import {
  createPastedTextAttachment,
  shouldCreatePastedTextAttachment,
} from "./pastedTextAttachment"
import type { ContextUsageInfo } from "../chatUtils"
import { contextUsageBarClass } from "../contextUsageColor"
import type { AgentConfig } from "@/components/settings/types"
import type { QuickPromptItem } from "@/types/quickPrompts"
import type { AutonomyActivity, GoalSnapshot } from "../workspace/useGoal"
import { parseGoalCriteriaDraft, type DraftGoalCriterionKind } from "../workspace/goalCriteriaDraft"
import { parseGoalUpsertSlashCommand } from "../goalSlashCommand"
import { parseLoopCreateSlashCommand } from "../loopSlashCommand"
import type { WorkflowRun, WorkflowRunState } from "../workspace/useWorkflowRuns"

type WorkflowMode = "off" | "on" | "ultracode"
type WorkflowTriggerHint = {
  mode: Exclude<WorkflowMode, "off">
}
export type GoalModeSubmitAction =
  | "create_or_update"
  | "replace"
  | "append_required"
  | "append_optional"
  | "append_follow_up"

const WORKFLOW_MODE_CHANGED_EVENT = "hope-agent:workflow-mode-changed"

const ULTRACODE_TRIGGER_PATTERNS: RegExp[] = [
  /(?:用|使用|开启|打开|启用|切到|进入).{0,12}(?:ultracode|超高|极致|穷尽)/i,
  /(?:ultracode|超高|极致|穷尽).{0,12}(?:模式|跑|执行|做|完成|推进|编排|验证)/i,
  /(?:大规模|全面|完整|彻底).{0,10}(?:交叉验证|并行审查|多代理|multi[-\s]?agent)/i,
]

const WORKFLOW_TRIGGER_PATTERNS: RegExp[] = [
  /(?:用|使用|开启|打开|启用|切到|进入).{0,12}(?:工作流|workflow|动态工作流|多代理|multi[-\s]?agent|subagent)/i,
  /(?:workflow|工作流|动态工作流).{0,16}(?:跑|执行|完成|处理|做|推进|编排|迁移|验证|审查|复核|调研|分析|review|verify|run)/i,
  /(?:多代理|多\s*agent|multi[-\s]?agent|subagent|并行.{0,6}(?:审查|验证|复核)|交叉验证|cross[-\s]?check|parallel review)/i,
  /(?:大规模|完整|全面|彻底).{0,12}(?:迁移|重构|验证|审查|复核|调研|分析|排查|修复)/i,
  /(?:后台|长任务|可恢复|durable|long[-\s]?running).{0,12}(?:执行|运行|跑|推进|orchestrat)/i,
]

function detectWorkflowTriggerHint(raw: string): WorkflowTriggerHint | null {
  const text = raw.replace(/\s+/g, " ").trim()
  if (!text || text.startsWith("/")) return null
  if (ULTRACODE_TRIGGER_PATTERNS.some((pattern) => pattern.test(text))) {
    return { mode: "ultracode" }
  }
  if (WORKFLOW_TRIGGER_PATTERNS.some((pattern) => pattern.test(text))) {
    return { mode: "on" }
  }
  return null
}

function normalizeWorkflowMode(value: unknown): WorkflowMode {
  const raw =
    typeof value === "string"
      ? value
      : typeof value === "object" && value !== null && "mode" in value
        ? (value as { mode?: unknown }).mode
        : null
  return raw === "on" || raw === "ultracode" ? raw : "off"
}

function workflowModeLabel(t: ReturnType<typeof useTranslation>["t"], mode: WorkflowMode): string {
  switch (mode) {
    case "off":
      return t("chat.workflowMode.off", { defaultValue: "关闭" })
    case "on":
      return t("chat.workflowMode.auto", { defaultValue: "自动" })
    case "ultracode":
      return t("chat.workflowMode.ultracode", { defaultValue: "Ultracode" })
  }
}

function workflowModeDescription(
  t: ReturnType<typeof useTranslation>["t"],
  mode: WorkflowMode,
): string {
  switch (mode) {
    case "off":
      return t("chat.workflowMode.offDesc", { defaultValue: "模型不会自动创建工作流运行" })
    case "on":
      return t("chat.workflowMode.autoDesc", {
        defaultValue: "模型按需自主编排可观察、可恢复的工作流",
      })
    case "ultracode":
      return t("chat.workflowMode.ultracodeDesc", {
        defaultValue: "更偏向长任务、深度验证和完整动态编排",
      })
  }
}

function workflowRunStateLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: WorkflowRunState,
): string {
  switch (state) {
    case "draft":
      return t("chat.workflowProgress.stateDraft", "草稿")
    case "awaiting_approval":
      return t("chat.workflowProgress.stateAwaitingApproval", "待审批")
    case "running":
      return t("chat.workflowProgress.stateRunning", "运行中")
    case "awaiting_user":
      return t("chat.workflowProgress.stateAwaitingUser", "等待你")
    case "paused":
      return t("chat.workflowProgress.statePaused", "已暂停")
    case "recovering":
      return t("chat.workflowProgress.stateRecovering", "恢复中")
    case "completed":
      return t("chat.workflowProgress.stateCompleted", "已完成")
    case "failed":
      return t("chat.workflowProgress.stateFailed", "失败")
    case "cancelled":
      return t("chat.workflowProgress.stateCancelled", "已取消")
    case "blocked":
      return t("chat.workflowProgress.stateBlocked", "阻塞")
  }
}

function workflowRunToneClass(state: WorkflowRunState): string {
  switch (state) {
    case "awaiting_approval":
    case "awaiting_user":
    case "blocked":
    case "failed":
      return "border-amber-500/20 bg-amber-500/8 text-amber-700 dark:text-amber-300"
    case "running":
    case "recovering":
      return "border-blue-500/20 bg-blue-500/8 text-blue-700 dark:text-blue-300"
    case "paused":
    case "draft":
      return "border-muted-foreground/20 bg-muted/50 text-muted-foreground"
    case "completed":
      return "border-emerald-500/20 bg-emerald-500/8 text-emerald-700 dark:text-emerald-300"
    case "cancelled":
      return "border-muted-foreground/20 bg-muted/50 text-muted-foreground"
  }
}

function workflowRunIsLive(state: WorkflowRunState): boolean {
  return (
    state === "awaiting_approval" ||
    state === "running" ||
    state === "awaiting_user" ||
    state === "paused" ||
    state === "recovering" ||
    state === "blocked" ||
    state === "failed"
  )
}

interface ChatInputProps {
  input: string
  onInputChange: (value: string) => void
  inputHistory?: string[]
  quickPrompts?: QuickPromptItem[]
  onSend: () => void
  sendDisabled?: boolean
  loading: boolean
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  unavailableModelPreference?: string | null
  reasoningEffort: string
  onModelChange: (key: string, options?: { applyToAgentDefault?: boolean }) => void
  onEffortChange: (effort: string, options?: { applyToAgentDefault?: boolean }) => void
  onEffortReset?: () => void
  attachedFiles: File[]
  onAttachFiles: (files: File[]) => void
  onRemoveFile: (index: number) => void
  onUpdateFile: (index: number, file: File) => void
  pendingQuotes?: PendingFileQuote[]
  onRemoveQuote?: (index: number) => void
  /** Click a staged quote chip to reveal that file in the file browser. */
  onJumpToQuote?: (q: PendingFileQuote) => void
  pendingMessage?: string | null
  pendingSends?: PendingSendPreview[]
  onCancelPending?: () => void
  onDiscardPending?: () => void
  onEditPending?: (id: string, text: string) => Promise<boolean>
  onDiscardPendingItem?: (id: string) => void | Promise<void>
  onSendPending?: (id: string) => void | Promise<void>
  onForceInsertPending?: (id: string) => void
  onCancelForceInsertPending?: (id: string) => void
  onStop?: () => void
  // Slash command support
  currentSessionId?: string | null
  currentAgentId?: string
  /** Materializes a draft conversation before applying session-scoped modes. */
  onEnsureSession?: () => Promise<string | null>
  onCommandAction?: (result: CommandResult) => void
  // Tool permission mode
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode, options?: PermissionModeChangeOptions) => void
  // Sandbox execution mode
  sandboxMode: SandboxMode
  onSandboxModeChange: (mode: SandboxMode) => void
  // Temperature
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (
    temp: number | null,
    options?: { applyToAgentDefault?: boolean },
  ) => void
  // Incognito
  incognitoEnabled?: boolean
  // Knowledge space attach (project context for project-scoped attaches)
  projectId?: string | null
  // Draft KB attaches staged before a session exists (composer draft mode)
  draftKbAttachments?: KbDraftAttachment[]
  onDraftKbAttachChange?: (next: KbDraftAttachment[]) => void
  /** Enable the `[[note]]` picker + the `@` menu's knowledge-notes section.
   *  Off by default so files-only surfaces (QuickChat) keep their behavior. */
  enableNoteMention?: boolean
  /** Enable the `@` menu's built-in **skills** section (`@skill:<name>` →
   *  office / browser / mac control). Off by default; the main chat opts in. */
  enableSkillMention?: boolean
  /** Enable the `@` menu's Agent delegation section (`@agent:<id>`). */
  enableAgentMention?: boolean
  agents?: AgentSummaryForSidebar[]
  // Working directory
  workingDir?: string | null
  /** True when `workingDir` is inherited from the parent project rather than
   *  set on the session itself; used to suppress the "clear" affordance in
   *  WorkingDirectoryButton (clearing a session value that is already null
   *  is a no-op the user can't observe). */
  workingDirInherited?: boolean
  workingDirSaving?: boolean
  onWorkingDirChange?: (workingDir: string | null) => void
  // Plan mode
  planState?: "off" | "planning" | "review" | "executing" | "completed"
  onEnterPlanMode?: () => void
  onExitPlanMode?: () => void
  onTogglePlanPanel?: () => void
  // Draft workflow mode staged before the first message materializes a session.
  draftWorkflowMode?: WorkflowMode
  onDraftWorkflowModeChange?: (mode: WorkflowMode) => void
  // Goal mode
  goalSnapshot?: GoalSnapshot | null
  autonomyActivity?: AutonomyActivity | null
  goalLoading?: boolean
  onGoalModeSubmit?: (objective: string, action?: GoalModeSubmitAction) => Promise<boolean>
  onLoopModeSubmit?: (prompt: string) => Promise<boolean>
  onGoalUpdate?: (objective: string, completionCriteria: string) => Promise<boolean>
  onPauseGoal?: () => Promise<boolean>
  onResumeGoal?: () => Promise<boolean>
  onClearGoal?: () => Promise<boolean>
  onEvaluateGoal?: () => Promise<boolean>
  // Session-scoped Todo progress
  taskProgressSnapshot?: TaskProgressSnapshot | null
  executionState?: ChatTurnStatus | null
  /** 打开右侧工作台面板（状态条点击）。 */
  onOpenWorkspace?: () => void
  /** Most relevant visible workflow run, shown as a compact progress line. */
  workflowProgressRun?: WorkflowRun | null
  /** Total workflow runs relevant to the compact progress line. */
  workflowProgressCount?: number
  /** True when the right-side workspace panel is expanded and showing task detail. */
  workspacePanelVisible?: boolean
  /** Larger centered presentation for a brand-new empty conversation. */
  hero?: boolean
  /** Context-window fullness, rendered as a thin bar fused into the dock's
   *  bottom border (green → amber → red). Null hides the bar. */
  contextUsage?: ContextUsageInfo | null
}

/**
 * Thin context-usage bar fused into the input dock's bottom border. The filled
 * width tracks how full the context window is; color ramps green → yellow → red
 * via the shared `contextUsageBarClass` (dependency-free leaf module, so the
 * input dock stays clear of chatUtils' heavier runtime chain). Sits in the
 * toolbar's `pb-2` padding zone (no buttons there), so its hover target never
 * steals clicks. Inset horizontally so the line starts after the dock's rounded
 * bottom corners instead of bleeding into the curved edge.
 */
function ContextUsageBottomBar({ usage }: { usage: ContextUsageInfo }) {
  const { t } = useTranslation()
  const fill = contextUsageBarClass(usage.pct)
  const width = Math.min(usage.pct, 100)
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div className="absolute inset-x-4 bottom-0 z-10 flex h-2 items-end">
          <div className="h-[2px] w-full overflow-hidden rounded-full">
            <div
              className={cn("h-full rounded-full transition-[width] duration-500", fill)}
              style={{ width: `${width}%` }}
            />
          </div>
        </div>
      </TooltipTrigger>
      <TooltipContent side="top">
        {t("chat.statusContext")} · {usage.usedK}k/{usage.ctxK}k ({usage.pct}%)
      </TooltipContent>
    </Tooltip>
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
    <div className="space-y-1 rounded-md border border-emerald-500/15 bg-emerald-500/5 p-1.5 text-[10px]">
      <div className="flex min-w-0 flex-wrap items-center gap-1 text-emerald-800/80 dark:text-emerald-200/80">
        <span className="shrink-0 font-medium">
          {t("chat.goalMode.criteriaPreview", "标准预览")}
        </span>
        <span>
          {t("chat.goalMode.criteriaRequiredCount", "必须 {{count}}", { count: required })}
        </span>
        <span className="opacity-45">/</span>
        <span>
          {t("chat.goalMode.criteriaOptionalCount", "可选 {{count}}", { count: optional })}
        </span>
        <span className="opacity-45">/</span>
        <span>
          {t("chat.goalMode.criteriaFollowUpCount", "后续 {{count}}", { count: followUp })}
        </span>
      </div>
      <div className="space-y-0.5">
        {items.slice(0, 3).map((item) => (
          <div key={item.id} className="flex min-w-0 items-center gap-1">
            <span className="shrink-0 rounded border border-emerald-500/20 px-1 text-emerald-700 dark:text-emerald-300">
              {chatGoalDraftKindLabel(t, item.kind)}
            </span>
            <span className="min-w-0 flex-1 truncate text-muted-foreground">{item.text}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function chatGoalDraftKindLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: DraftGoalCriterionKind,
): string {
  switch (kind) {
    case "required":
      return t("chat.goalMode.criterionRequired", "必须")
    case "optional":
      return t("chat.goalMode.criterionOptional", "可选")
    case "follow_up":
      return t("chat.goalMode.criterionFollowUp", "后续")
  }
}

function readToolbarItemWidth(el: HTMLElement | null, fallback: number): number {
  if (!el) return fallback
  const rect = el.getBoundingClientRect()
  return rect.width > 0 ? Math.ceil(rect.width) : fallback
}

function visibleToolbarItemRects(container: HTMLElement): DOMRect[] {
  return Array.from(container.children)
    .map((child) => child.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0)
}

function toolbarVisibleWidth(container: HTMLElement): number {
  const rects = visibleToolbarItemRects(container)
  if (rects.length === 0) return 0
  const left = Math.min(...rects.map((rect) => rect.left))
  const right = Math.max(...rects.map((rect) => rect.right))
  return Math.max(0, Math.ceil(right - left))
}

export default function ChatInput({
  input,
  onInputChange,
  inputHistory = [],
  quickPrompts = [],
  onSend,
  sendDisabled = false,
  loading,
  availableModels,
  activeModel,
  unavailableModelPreference,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  onEffortReset,
  attachedFiles,
  onAttachFiles,
  onRemoveFile,
  onUpdateFile,
  pendingQuotes,
  onRemoveQuote,
  onJumpToQuote,
  pendingMessage,
  pendingSends,
  onCancelPending,
  onDiscardPending,
  onEditPending,
  onDiscardPendingItem,
  onSendPending,
  onForceInsertPending,
  onCancelForceInsertPending,
  onStop,
  currentSessionId,
  currentAgentId = DEFAULT_AGENT_ID,
  onEnsureSession,
  onCommandAction,
  permissionMode,
  onPermissionModeChange,
  sandboxMode,
  onSandboxModeChange,
  sessionTemperature,
  onSessionTemperatureChange,
  incognitoEnabled = false,
  projectId,
  draftKbAttachments,
  onDraftKbAttachChange,
  enableNoteMention = false,
  enableSkillMention = false,
  enableAgentMention = false,
  agents = [],
  workingDir,
  workingDirInherited = false,
  workingDirSaving = false,
  onWorkingDirChange,
  planState = "off",
  onEnterPlanMode,
  onExitPlanMode,
  onTogglePlanPanel,
  draftWorkflowMode = "off",
  onDraftWorkflowModeChange,
  goalSnapshot,
  autonomyActivity,
  goalLoading = false,
  onGoalModeSubmit,
  onLoopModeSubmit,
  onGoalUpdate,
  onPauseGoal,
  onResumeGoal,
  onClearGoal,
  onEvaluateGoal,
  taskProgressSnapshot,
  executionState,
  onOpenWorkspace,
  workflowProgressRun,
  workflowProgressCount = 0,
  workspacePanelVisible = false,
  hero = false,
  contextUsage,
}: ChatInputProps) {
  const { t } = useTranslation()
  const inputHandleRef = useRef<ComposerInputHandle>(null)
  const inputShellRef = useRef<HTMLDivElement>(null)
  const toolbarRef = useRef<HTMLDivElement>(null)
  const toolbarLeftRef = useRef<HTMLDivElement>(null)
  const overflowTriggerRef = useRef<HTMLDivElement>(null)
  const addActionsRef = useRef<HTMLDivElement>(null)
  const semanticModesRef = useRef<HTMLDivElement>(null)
  const permissionModeRef = useRef<HTMLDivElement>(null)
  const toolbarGroupWidthsRef = useRef<ChatInputToolbarGroupWidths>({
    ...CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  })
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)
  // 0 = everything inline; 1 = add actions behind "+"; 2 = semantic modes behind
  // "+"; 3 = permission (including sandbox) behind "+". The level is chosen
  // by live DOM measurement instead of fixed container-width breakpoints.
  const [toolbarCollapseLevel, setToolbarCollapseLevel] = useState(0)
  const [toolbarMinHeight, setToolbarMinHeight] = useState<number | null>(null)
  const [pendingExpanded, setPendingExpanded] = useState(false)
  const [editingPendingId, setEditingPendingId] = useState<string | null>(null)
  const [pendingEditValue, setPendingEditValue] = useState("")
  const [pendingEditSaving, setPendingEditSaving] = useState(false)
  const [goalComposerMode, setGoalComposerMode] = useState(false)
  const [loopComposerMode, setLoopComposerMode] = useState(false)
  const [goalComposerAction, setGoalComposerAction] =
    useState<GoalModeSubmitAction>("create_or_update")
  const [goalSubmitting, setGoalSubmitting] = useState(false)
  const [loopSubmitting, setLoopSubmitting] = useState(false)
  const [goalEditOpen, setGoalEditOpen] = useState(false)
  const [goalEditObjective, setGoalEditObjective] = useState("")
  const [goalEditCriteria, setGoalEditCriteria] = useState("")
  const [goalActionPending, setGoalActionPending] = useState<string | null>(null)
  const [workflowMode, setWorkflowMode] = useState<WorkflowMode>("off")
  const [workflowModeLoading, setWorkflowModeLoading] = useState(false)
  const [workflowModeSaving, setWorkflowModeSaving] = useState<WorkflowMode | null>(null)
  const [workflowMenuOpen, setWorkflowMenuOpen] = useState(false)
  const [dismissedWorkflowHintFor, setDismissedWorkflowHintFor] = useState<string | null>(null)
  const { toolbarCompact, toolbarTight, permissionCollapsed } = getChatInputToolbarFlags(
    toolbarCollapseLevel,
  )

  const handlePermissionModeChange = useCallback(
    (mode: SessionMode, options?: PermissionModeChangeOptions) => {
      onPermissionModeChange(mode, options)
      if (!options?.applyToAgentDefault) return

      void (async () => {
        try {
          const transport = getTransport()
          const config = await transport.call<AgentConfig>("get_agent_config", {
            id: currentAgentId,
          })
          await transport.call("save_agent_config_cmd", {
            id: currentAgentId,
            config: {
              ...config,
              capabilities: {
                ...config.capabilities,
                defaultSessionPermissionMode: mode,
              },
            },
          })
        } catch (e) {
          logger.error(
            "chat",
            "ChatInput::setAgentDefaultPermissionMode",
            "Failed to save agent default permission mode",
            e,
          )
          toast.error(t("chat.permissionMode.applyToAgentDefault.failed"))
        }
      })()
    },
    [currentAgentId, onPermissionModeChange, t],
  )

  const [historyIndex, setHistoryIndex] = useState<number | null>(null)
  const historyDraftRef = useRef("")

  const resetHistoryBrowsing = useCallback(() => {
    setHistoryIndex(null)
    historyDraftRef.current = ""
  }, [])

  const setComposerInput = useCallback(
    (value: string) => {
      resetHistoryBrowsing()
      onInputChange(value)
    },
    [onInputChange, resetHistoryBrowsing],
  )

  useEffect(() => {
    resetHistoryBrowsing()
  }, [currentSessionId, resetHistoryBrowsing])

  // Slash commands
  const slashActions: SlashCommandActions = {
    onCommandAction: onCommandAction ?? (() => {}),
    sessionId: currentSessionId ?? null,
    agentId: currentAgentId,
    ensureSession: onEnsureSession,
    bypassLoopCreateOnEnter: !!onLoopModeSubmit,
  }
  const slash = useSlashCommands(input, setComposerInput, slashActions, inputHandleRef)
  const voice = useVoiceInput(currentSessionId)
  const normalToolbarOpen = voice.state !== "recording" && voice.state !== "transcribing"
  // Read the latest `input` when transcription resolves — the user can keep
  // typing during the STT round-trip, and capturing `input` in the closure
  // would overwrite anything typed in the meantime.
  const inputRef = useRef(input)
  useEffect(() => {
    inputRef.current = input
  }, [input])

  const activeGoal = goalSnapshot?.goal ?? null
  useEffect(() => {
    setGoalEditObjective(activeGoal?.objective ?? "")
    setGoalEditCriteria(activeGoal?.completionCriteria ?? "")
    setGoalEditOpen(false)
    setGoalActionPending(null)
  }, [activeGoal?.id, activeGoal?.objective, activeGoal?.completionCriteria])

  useEffect(() => {
    if (!currentSessionId || incognitoEnabled) {
      setWorkflowMode(incognitoEnabled ? "off" : normalizeWorkflowMode(draftWorkflowMode))
      setWorkflowModeLoading(false)
      setWorkflowModeSaving(null)
      return
    }
    let cancelled = false
    setWorkflowModeLoading(true)
    getTransport()
      .call<unknown>("get_workflow_mode", { sessionId: currentSessionId })
      .then((next) => {
        if (cancelled) return
        setWorkflowMode(normalizeWorkflowMode(next))
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("ui", "ChatInput::loadWorkflowMode", "Failed to load workflow mode", e)
      })
      .finally(() => {
        if (!cancelled) setWorkflowModeLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [currentSessionId, draftWorkflowMode, incognitoEnabled])

  useEffect(() => {
    const onWorkflowModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ sessionId?: string | null; mode?: unknown }>).detail
      if (!detail || detail.sessionId !== currentSessionId) return
      setWorkflowMode(normalizeWorkflowMode(detail.mode))
      setWorkflowModeSaving(null)
      setWorkflowModeLoading(false)
    }
    window.addEventListener(WORKFLOW_MODE_CHANGED_EVENT, onWorkflowModeChanged)
    return () => window.removeEventListener(WORKFLOW_MODE_CHANGED_EVENT, onWorkflowModeChanged)
  }, [currentSessionId])

  /**
   * Caret anchor captured at `voice.start()` time. While recording, the
   * transcript (streaming partial OR batch final) is spliced INTO the
   * composer at this position rather than appended to the end. Cleared
   * after stop / cancel.
   */
  const voiceAnchorRef = useRef<{ prefix: string; suffix: string } | null>(null)

  const startVoice = useCallback(async () => {
    const inputHandle = inputHandleRef.current
    const current = inputRef.current
    const selection = inputHandle?.getSelectionRange() ?? {
      start: current.length,
      end: current.length,
    }
    const selStart = selection.start
    const selEnd = selection.end
    voiceAnchorRef.current = {
      prefix: current.slice(0, selStart),
      suffix: current.slice(selEnd),
    }
    await voice.start()
  }, [voice])

  const handleVoiceStop = useCallback(async () => {
    const text = await voice.stopAndTranscribe()
    const anchor = voiceAnchorRef.current
    voiceAnchorRef.current = null
    if (!text) {
      // Failed / empty transcript — restore the surrounding text in case
      // streaming partials had already written something.
      if (anchor) onInputChange(anchor.prefix + anchor.suffix)
      return
    }
    const prefix = anchor?.prefix ?? inputRef.current
    const suffix = anchor?.suffix ?? ""
    onInputChange(prefix + text + suffix)
    // Restore caret to the end of the inserted transcript on the next
    // tick (after React commits the new value to the composer).
    const caret = prefix.length + text.length
    requestAnimationFrame(() => {
      const inputHandle = inputHandleRef.current
      if (!inputHandle) return
      inputHandle.focus()
      inputHandle.setSelectionRange(caret, caret)
    })
  }, [voice, onInputChange])

  const handleVoiceCancel = useCallback(() => {
    const anchor = voiceAnchorRef.current
    voiceAnchorRef.current = null
    voice.cancel()
    // Strip any streaming partial that already landed in the composer.
    if (anchor) onInputChange(anchor.prefix + anchor.suffix)
  }, [voice, onInputChange])

  // Streaming partials arrive at 10-30 Hz from WS providers. Writing
  // them straight into `input` triggers urlPreview / mention / slash
  // re-derivation on every frame, which thrashes the input area.
  // Coalesce to one commit per animation frame: hold the latest partial
  // in a ref, schedule a single rAF, splice once.
  const pendingPartialRef = useRef<string | null>(null)
  const partialRafRef = useRef<number | null>(null)
  useEffect(() => {
    if (voice.state !== "recording") return
    const anchor = voiceAnchorRef.current
    if (!anchor) return
    const partial = voice.partialText
    if (!partial) return
    pendingPartialRef.current = partial
    if (partialRafRef.current !== null) return
    partialRafRef.current = requestAnimationFrame(() => {
      partialRafRef.current = null
      const p = pendingPartialRef.current
      pendingPartialRef.current = null
      if (p == null) return
      const a = voiceAnchorRef.current
      if (!a) return
      onInputChange(a.prefix + p + a.suffix)
    })
  }, [voice.partialText, voice.state, onInputChange])
  useEffect(
    () => () => {
      if (partialRafRef.current !== null) cancelAnimationFrame(partialRafRef.current)
      partialRafRef.current = null
      pendingPartialRef.current = null
    },
    [],
  )

  // Press-to-talk: hold Ctrl+Shift+H anywhere on the page to dictate.
  // Mirrors the inline tooltip on `VoiceRecordButton`. Click-toggle still
  // works via the button itself; this is the keyboard-only path.
  const pttActiveRef = useRef(false)
  const voiceRef = useRef(voice)
  voiceRef.current = voice
  const handleVoiceStopRef = useRef(handleVoiceStop)
  handleVoiceStopRef.current = handleVoiceStop
  const handleVoiceCancelRef = useRef(handleVoiceCancel)
  handleVoiceCancelRef.current = handleVoiceCancel
  const startVoiceRef = useRef(startVoice)
  startVoiceRef.current = startVoice
  useEffect(() => {
    const isPttCombo = (e: KeyboardEvent) =>
      e.code === "KeyH" && e.shiftKey && e.ctrlKey && !e.altKey && !e.metaKey

    const onKeyDown = (e: KeyboardEvent) => {
      if (!isPttCombo(e)) return
      // OS sends repeats while the key is held — only the first edge matters.
      if (e.repeat) return
      e.preventDefault()
      if (pttActiveRef.current) return
      const s = voiceRef.current.state
      if (s !== "idle" && s !== "ready" && s !== "stopped" && s !== "error") return
      pttActiveRef.current = true
      logger.info("voice", "ChatInput::ptt", "start recording (ptt down)")
      void startVoiceRef.current()
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (!isPttCombo(e)) return
      if (!pttActiveRef.current) return
      pttActiveRef.current = false
      e.preventDefault()
      if (voiceRef.current.state === "recording") {
        logger.info("voice", "ChatInput::ptt", "stop recording (ptt up)")
        void handleVoiceStopRef.current()
      }
    }
    // Switching apps or alt-tabbing can swallow the keyup — fall back to
    // cancel so half-captured audio doesn't ride into the next session.
    const onBlur = () => {
      if (!pttActiveRef.current) return
      pttActiveRef.current = false
      if (voiceRef.current.state === "recording") {
        logger.warn("voice", "ChatInput::ptt", "blur during ptt: cancel recording")
        handleVoiceCancelRef.current()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("keyup", onKeyUp)
    window.addEventListener("blur", onBlur)
    return () => {
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("keyup", onKeyUp)
      window.removeEventListener("blur", onBlur)
    }
  }, [])

  // File mention `@` popper — files (working dir) + knowledge notes when enabled.
  const mention = useFileMention(
    input,
    setComposerInput,
    inputHandleRef,
    workingDir ?? null,
    enableNoteMention
      ? {
          sessionId: currentSessionId ?? null,
          projectId: projectId ?? null,
          draftKbAttachments: draftKbAttachments ?? [],
        }
      : undefined,
    enableSkillMention,
    enableAgentMention ? agents : [],
    currentAgentId,
  )
  // `[[note]]` picker — knowledge-space notes reachable from this chat.
  const noteMention = useNoteMention(
    input,
    setComposerInput,
    inputHandleRef,
    currentSessionId ?? null,
    projectId ?? null,
    draftKbAttachments ?? [],
    enableNoteMention,
  )
  // User-global quick prompts (`#` popper).
  const quickPrompt = useQuickPrompts(input, setComposerInput, inputHandleRef, quickPrompts)
  // URL preview
  const { previews: urlPreviews, dismissedUrls, dismiss: dismissUrl } = useUrlPreview(input)
  const hasSendableContent =
    goalComposerMode || loopComposerMode
      ? input.trim().length > 0
      : input.trim().length > 0 || attachedFiles.length > 0 || (pendingQuotes?.length ?? 0) > 0

  // The chat column can shrink when a right-side panel opens while the viewport
  // stays wide, so the overflow affordance follows the actual toolbar layout.
  // Resolve the target collapse tier from measured widths in one pass; otherwise
  // very narrow inputs can visibly crop controls while the UI collapses one tier
  // at a time.
  useLayoutEffect(() => {
    if (!normalToolbarOpen || typeof window === "undefined") return

    const left = toolbarLeftRef.current
    if (!left) return

    let raf: number | null = null

    const updateMeasuredGroupWidths = () => {
      toolbarGroupWidthsRef.current = {
        addActions: readToolbarItemWidth(
          addActionsRef.current,
          toolbarGroupWidthsRef.current.addActions,
        ),
        overflowTrigger: readToolbarItemWidth(
          overflowTriggerRef.current,
          toolbarGroupWidthsRef.current.overflowTrigger,
        ),
        semanticModes: readToolbarItemWidth(
          semanticModesRef.current,
          toolbarGroupWidthsRef.current.semanticModes,
        ),
        permission: readToolbarItemWidth(
          permissionModeRef.current,
          toolbarGroupWidthsRef.current.permission,
        ),
      }
    }

    const update = () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      raf = window.requestAnimationFrame(() => {
        raf = null
        updateMeasuredGroupWidths()
        setToolbarCollapseLevel((level) => {
          const currentLeft = toolbarLeftRef.current
          if (!currentLeft) return level
          return resolveChatInputToolbarCollapseLevel({
            currentLevel: level,
            availableWidth: currentLeft.clientWidth,
            visibleWidth: toolbarVisibleWidth(currentLeft),
            widths: toolbarGroupWidthsRef.current,
          })
        })
      })
    }

    update()

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update)
      return () => {
        if (raf !== null) window.cancelAnimationFrame(raf)
        window.removeEventListener("resize", update)
      }
    }

    const observer = new ResizeObserver(update)
    ;[
      inputShellRef.current,
      toolbarRef.current,
      left,
      overflowTriggerRef.current,
      addActionsRef.current,
      semanticModesRef.current,
      permissionModeRef.current,
    ].forEach((el) => {
      if (el) observer.observe(el)
    })
    return () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      observer.disconnect()
    }
  }, [normalToolbarOpen, toolbarCollapseLevel])

  useEffect(() => {
    if (showOverflowMenu && !toolbarCompact) setShowOverflowMenu(false)
  }, [showOverflowMenu, toolbarCompact])

  useEffect(() => {
    if (!showOverflowMenu) return

    const closeOnOutsideClick = (event: MouseEvent) => {
      if (!overflowTriggerRef.current?.contains(event.target as Node)) {
        setShowOverflowMenu(false)
      }
    }

    document.addEventListener("mousedown", closeOnOutsideClick)
    return () => document.removeEventListener("mousedown", closeOnOutsideClick)
  }, [showOverflowMenu])

  useEffect(() => {
    setToolbarMinHeight(null)
  }, [toolbarCollapseLevel])

  useEffect(() => {
    if (!normalToolbarOpen || typeof window === "undefined") {
      setToolbarMinHeight(null)
      return
    }

    const el = toolbarRef.current
    if (!el) return

    let raf: number | null = null
    const update = () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      raf = window.requestAnimationFrame(() => {
        raf = null
        const next = Math.ceil(el.scrollHeight || el.getBoundingClientRect().height)
        setToolbarMinHeight((prev) => {
          if (prev === null || next > prev) return next
          return prev
        })
      })
    }

    update()

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update)
      return () => {
        if (raf !== null) window.cancelAnimationFrame(raf)
        window.removeEventListener("resize", update)
      }
    }

    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => {
      if (raf !== null) window.cancelAnimationFrame(raf)
      observer.disconnect()
    }
  }, [normalToolbarOpen, toolbarCollapseLevel])

  const handlePaste = useCallback(
    (e: ComposerPasteEvent) => {
      const clipboardData = e.clipboardData
      if (!clipboardData) return
      const items = clipboardData.items
      const files: File[] = []
      if (items) {
        for (let i = 0; i < items.length; i++) {
          const item = items[i]
          if (item.kind === "file") {
            const file = item.getAsFile()
            if (file) files.push(file)
          }
        }
      }
      if (files.length > 0) {
        e.preventDefault()
        onAttachFiles(files)
        return
      }

      const pastedText = clipboardData.getData("text/plain")
      if (shouldCreatePastedTextAttachment(pastedText)) {
        e.preventDefault()
        onAttachFiles([createPastedTextAttachment(pastedText)])

        const selection = inputHandleRef.current?.getSelectionRange()
        if (selection && selection.start !== selection.end) {
          const current = inputRef.current
          const next = current.slice(0, selection.start) + current.slice(selection.end)
          setComposerInput(next)
          requestAnimationFrame(() => {
            const inputHandle = inputHandleRef.current
            if (!inputHandle) return
            inputHandle.focus()
            inputHandle.setSelectionRange(selection.start, selection.start)
          })
        }
      }
    },
    [onAttachFiles, setComposerInput],
  )

  const handleHistoryKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLElement>) => {
      if (
        (e.key !== "ArrowUp" && e.key !== "ArrowDown") ||
        e.shiftKey ||
        e.ctrlKey ||
        e.altKey ||
        e.metaKey ||
        inputHistory.length === 0
      ) {
        return false
      }

      const browsing = historyIndex !== null
      if (!browsing && input.length > 0) return false

      if (e.key === "ArrowDown" && !browsing) return false

      e.preventDefault()
      if (e.key === "ArrowUp") {
        const nextIndex =
          historyIndex == null ? 0 : Math.min(historyIndex + 1, inputHistory.length - 1)
        if (!browsing) historyDraftRef.current = input
        setHistoryIndex(nextIndex)
        onInputChange(inputHistory[nextIndex] ?? "")
        requestAnimationFrame(() => {
          const inputHandle = inputHandleRef.current
          const next = inputHistory[nextIndex] ?? ""
          if (!inputHandle) return
          inputHandle.focus()
          inputHandle.setSelectionRange(next.length, next.length)
        })
        return true
      }

      if (historyIndex == null) return false
      const nextIndex = historyIndex - 1
      if (nextIndex < 0) {
        const draft = historyDraftRef.current
        setHistoryIndex(null)
        historyDraftRef.current = ""
        onInputChange(draft)
        requestAnimationFrame(() => {
          const inputHandle = inputHandleRef.current
          if (!inputHandle) return
          inputHandle.focus()
          inputHandle.setSelectionRange(draft.length, draft.length)
        })
        return true
      }

      setHistoryIndex(nextIndex)
      onInputChange(inputHistory[nextIndex] ?? "")
      requestAnimationFrame(() => {
        const inputHandle = inputHandleRef.current
        const next = inputHistory[nextIndex] ?? ""
        if (!inputHandle) return
        inputHandle.focus()
        inputHandle.setSelectionRange(next.length, next.length)
      })
      return true
    },
    [historyIndex, input, inputHandleRef, inputHistory, onInputChange],
  )

  const sendUnavailable = sendDisabled || !hasSendableContent

  const handleSend = useCallback(() => {
    if (sendUnavailable) return
    resetHistoryBrowsing()
    // Normalize slash-form Goal drafts even when the Goal composer is already
    // active. Pasting a reusable `/goal ...` prompt after clicking the Goal
    // button must not persist the command prefix as part of the objective.
    const directGoalObjective = parseGoalUpsertSlashCommand(input)
    const directLoopPrompt =
      goalComposerMode || !onLoopModeSubmit ? null : parseLoopCreateSlashCommand(input)
    if (goalComposerMode || directGoalObjective) {
      const objective = directGoalObjective ?? input.trim()
      if (!objective || goalSubmitting) return
      if (incognitoEnabled) {
        toast.error(t("chat.goalMode.incognito", "无痕会话不持久化目标"))
        return
      }
      if (!onGoalModeSubmit) return
      const action = activeGoal ? goalComposerAction : undefined
      setGoalSubmitting(true)
      const submit = action ? onGoalModeSubmit(objective, action) : onGoalModeSubmit(objective)
      void submit
        .then((ok) => {
          if (!ok) return
          setComposerInput("")
          setGoalComposerMode(false)
        })
        .finally(() => setGoalSubmitting(false))
      return
    }
    if (loopComposerMode || directLoopPrompt) {
      const prompt = directLoopPrompt ?? input.trim()
      if (!prompt || loopSubmitting) return
      if (incognitoEnabled) {
        toast.error(t("chat.loopMode.incognito", "无痕会话不持久化持续推进"))
        return
      }
      if (!onLoopModeSubmit) return
      setLoopSubmitting(true)
      void onLoopModeSubmit(prompt)
        .then((ok) => {
          if (!ok) return
          setComposerInput("")
          setLoopComposerMode(false)
        })
        .finally(() => setLoopSubmitting(false))
      return
    }
    onSend()
  }, [
    goalComposerMode,
    loopComposerMode,
    goalComposerAction,
    goalSubmitting,
    loopSubmitting,
    incognitoEnabled,
    input,
    activeGoal,
    onGoalModeSubmit,
    onLoopModeSubmit,
    onSend,
    resetHistoryBrowsing,
    sendUnavailable,
    setComposerInput,
    t,
  ])

  function handleKeyDown(e: React.KeyboardEvent<HTMLElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return
    // Slash menu first (owns header `/...` slot), then `[[note]]` picker, then
    // `@` file mention, then `#` quick prompts, then history/send. Each handler self-guards on its own open
    // state, so only the active popper consumes the key.
    if (slash.handleKeyDown(e)) return
    if (noteMention.handleKeyDown(e)) return
    if (mention.handleKeyDown(e)) return
    if (quickPrompt.handleKeyDown(e)) return
    if (handleHistoryKeyDown(e)) return
    if (e.key === "Tab" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault()
      onPermissionModeChange(getNextPermissionMode(permissionMode))
      return
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const currentModelInfo = availableModels.find(
    (m) => m.providerId === activeModel?.providerId && m.modelId === activeModel?.modelId,
  )
  const planToggleLabel = t("planMode.toggleLabel")
  const planToggleTip = (() => {
    switch (planState) {
      case "off":
      case "completed":
        return t("planMode.enter")
      case "planning":
        return t("planMode.indicator")
      case "review":
        return t("planMode.review.badge")
      case "executing":
        return t("planMode.executing")
    }
  })()
  const goalToggleLabel = t("chat.goalMode.label", "目标")
  const goalToggleTip = goalComposerMode
    ? t("chat.goalMode.activeTip", "正在设置目标")
    : t("chat.goalMode.enter", "进入目标模式")
  const loopToggleLabel = t("chat.loopMode.label", "持续推进")
  const loopModeAvailable = !!onLoopModeSubmit
  const loopToggleTip = loopComposerMode
    ? t("chat.loopMode.activeTip", "正在设置持续推进")
    : t("chat.loopMode.enter", "进入持续推进模式")
  const workflowToggleLabel = t("chat.workflowMode.label", "工作流")
  const workflowModeActive = workflowMode !== "off"
  const WorkflowModeIcon = workflowMode === "ultracode" ? Sparkles : GitPullRequest
  const normalizedWorkflowTriggerInput = input.replace(/\s+/g, " ").trim()
  const workflowTriggerHint = useMemo(() => detectWorkflowTriggerHint(input), [input])
  const showWorkflowTriggerHint =
    !!workflowTriggerHint &&
    workflowMode === "off" &&
    !incognitoEnabled &&
    !goalComposerMode &&
    !loopComposerMode &&
    planState !== "planning" &&
    normalizedWorkflowTriggerInput !== dismissedWorkflowHintFor
  const showWorkflowProgressLine =
    !!workflowProgressRun && workflowRunIsLive(workflowProgressRun.state) && !incognitoEnabled
  const workflowProgressExtraCount = Math.max(0, workflowProgressCount - 1)
  const workflowMenuLabel = t("chat.workflowMode.menuTitle", { defaultValue: "工作流模式" })
  const workflowButtonLabel = workflowModeActive
    ? `${workflowToggleLabel} · ${workflowModeLabel(t, workflowMode)}`
    : workflowToggleLabel
  const workflowMenuDisabled = incognitoEnabled || workflowModeLoading || !!workflowModeSaving
  const workflowButtonTone =
    workflowMode === "on"
      ? "bg-blue-500/10 text-blue-600"
      : workflowMode === "ultracode"
        ? "bg-purple-500/10 text-purple-600"
        : "text-muted-foreground hover:text-foreground"
  const activeGoalStateLabel = (() => {
    switch (activeGoal?.state) {
      case "active":
        return t("chat.goalMode.stateActive", "进行中")
      case "paused":
        return t("chat.goalMode.statePaused", "已暂停")
      case "evaluating":
        return t("chat.goalMode.stateEvaluating", "评估中")
      case "blocked":
        return t("chat.goalMode.stateBlocked", "阻塞")
      case "completed":
        return t("chat.goalMode.stateCompleted", "完成")
      case "failed":
        return t("chat.goalMode.stateFailed", "失败")
      case "cancelled":
        return t("chat.goalMode.stateCancelled", "已清除")
      default:
        return ""
    }
  })()
  const activityHeadlineLabel = (() => {
    switch (autonomyActivity?.headlineCode) {
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
        return activeGoalStateLabel
    }
  })()
  const activityDetail = [
    activityHeadlineLabel,
    autonomyActivity?.currentStep,
    autonomyActivity?.waitingOn?.label,
  ]
    .filter(Boolean)
    .join(" · ")
  const activeGoalCriteria = goalSnapshot?.criteria ?? []
  const activeGoalRequiredTotal = activeGoalCriteria.filter(
    (criterion) => (criterion.kind ?? "required") === "required",
  ).length
  const activeGoalRequiredDone = activeGoalCriteria.filter(
    (criterion) =>
      (criterion.kind ?? "required") === "required" && criterion.status === "satisfied",
  ).length
  const activeGoalProgressLabel =
    activeGoalRequiredTotal > 0 ? `${activeGoalRequiredDone}/${activeGoalRequiredTotal}` : null
  const planModeActive = planState !== "off" && planState !== "completed"
  const planComposerBannerOpen = planState === "planning" && !goalComposerMode && !loopComposerMode

  const overflowMenuItemClass =
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-50"

  // Shared by the inline Plan toggle and its "+" overflow-menu counterpart.
  const handlePlanToggle = () => {
    if (planState === "off" || planState === "completed") {
      setGoalComposerMode(false)
      setLoopComposerMode(false)
      onEnterPlanMode?.()
    } else if (planState === "planning") {
      onExitPlanMode?.()
    } else {
      setGoalComposerMode(false)
      setLoopComposerMode(false)
      onTogglePlanPanel?.()
    }
  }

  const handleGoalModeToggle = () => {
    if (incognitoEnabled) {
      toast.error(t("chat.goalMode.incognito", "无痕会话不持久化目标"))
      return
    }
    setGoalComposerMode((value) => {
      const next = !value
      if (next) {
        setLoopComposerMode(false)
        if (planModeActive) void onExitPlanMode?.()
        setGoalComposerAction("create_or_update")
      }
      return next
    })
  }

  const handleLoopModeToggle = () => {
    if (!loopModeAvailable) return
    if (incognitoEnabled) {
      toast.error(t("chat.loopMode.incognito", "无痕会话不持久化持续推进"))
      return
    }
    setLoopComposerMode((value) => {
      const next = !value
      if (next) {
        setGoalComposerMode(false)
        if (planModeActive) void onExitPlanMode?.()
      }
      return next
    })
  }

  const updateWorkflowMode = useCallback(
    async (nextMode: WorkflowMode) => {
      if (incognitoEnabled) {
        toast.error(t("chat.workflowMode.incognito", "无痕会话不启用工作流模式"))
        return
      }
      if (nextMode === workflowMode || workflowModeSaving) return
      if (!currentSessionId) {
        setWorkflowMode(nextMode)
        onDraftWorkflowModeChange?.(nextMode)
        toast.success(
          nextMode === "off"
            ? t("chat.workflowMode.draftOff", "工作流模式已关闭")
            : t("chat.workflowMode.draftSaved", "工作流模式已开启：{{mode}}", {
                mode: workflowModeLabel(t, nextMode),
              }),
        )
        return
      }
      setWorkflowModeSaving(nextMode)
      try {
        const next = await getTransport().call<unknown>("set_workflow_mode", {
          sessionId: currentSessionId,
          mode: nextMode,
        })
        const saved = normalizeWorkflowMode(next)
        setWorkflowMode(saved)
        window.dispatchEvent(
          new CustomEvent(WORKFLOW_MODE_CHANGED_EVENT, {
            detail: { sessionId: currentSessionId, mode: saved },
          }),
        )
        toast.success(
          saved === "off"
            ? t("chat.workflowMode.draftOff", "工作流模式已关闭")
            : t("chat.workflowMode.saved", "工作流模式已开启：{{mode}}", {
                mode: workflowModeLabel(t, saved),
              }),
        )
      } catch (e) {
        logger.error("ui", "ChatInput::updateWorkflowMode", "Failed to update workflow mode", e)
        toast.error(e instanceof Error ? e.message : String(e))
      } finally {
        setWorkflowModeSaving(null)
      }
    },
    [
      currentSessionId,
      incognitoEnabled,
      onDraftWorkflowModeChange,
      t,
      workflowMode,
      workflowModeSaving,
    ],
  )

  const renderWorkflowModeMenuItems = (onPicked?: () => void) => {
    const options: WorkflowMode[] = ["off", "on", "ultracode"]
    return (
      <div className="flex flex-col gap-0.5">
        {options.map((mode) => {
          const selected = workflowMode === mode
          const ModeIcon = mode === "ultracode" ? Sparkles : GitPullRequest
          const savingThis = workflowModeSaving === mode
          return (
            <button
              key={mode}
              type="button"
              className={cn(
                "flex w-full items-start gap-2 rounded-md px-2.5 py-2 text-left transition-all duration-150",
                selected
                  ? "bg-secondary text-foreground font-medium shadow-sm"
                  : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
              )}
              disabled={workflowMenuDisabled}
              onClick={() => {
                onPicked?.()
                void updateWorkflowMode(mode)
              }}
            >
              {savingThis ? (
                <Loader2 className="mt-0.5 h-4 w-4 shrink-0 animate-spin" />
              ) : selected ? (
                <Check className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              ) : (
                <ModeIcon
                  className={cn(
                    "mt-0.5 h-4 w-4 shrink-0",
                    mode === "on" && "text-blue-600",
                    mode === "ultracode" && "text-purple-600",
                  )}
                />
              )}
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="text-[13px]">{workflowModeLabel(t, mode)}</span>
                <span className="text-[11px] font-normal leading-snug text-muted-foreground">
                  {workflowModeDescription(t, mode)}
                </span>
              </span>
            </button>
          )
        })}
        {onOpenWorkspace ? (
          <>
            <div className="my-1 h-px bg-border/60" />
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 transition-all duration-150 hover:bg-secondary/60 hover:text-foreground"
              onClick={() => {
                onPicked?.()
                onOpenWorkspace()
              }}
            >
              <GitPullRequest className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate">
                {t("chat.workflowMode.viewRuns", { defaultValue: "查看工作流运行" })}
              </span>
            </button>
          </>
        ) : null}
      </div>
    )
  }

  const runGoalAction = (key: string, action?: () => Promise<boolean>) => {
    if (!action || goalActionPending) return
    setGoalActionPending(key)
    void action().finally(() => setGoalActionPending(null))
  }

  const saveGoalEdit = () => {
    if (!onGoalUpdate || goalActionPending) return
    const objective = goalEditObjective.trim()
    if (!objective) {
      toast.error(t("chat.goalMode.objectiveRequired", "请输入目标"))
      return
    }
    setGoalActionPending("update")
    void onGoalUpdate(objective, goalEditCriteria)
      .then((ok) => {
        if (ok) setGoalEditOpen(false)
      })
      .finally(() => setGoalActionPending(null))
  }

  const toggleSlashCommandMenu = () => {
    slash.setOpen(!slash.isOpen)
  }

  const taskExecutionState = resolveWorkspaceTaskExecutionState(executionState, loading)
  const visibleTaskProgressSnapshot = shouldShowTaskProgressPanel(taskProgressSnapshot)
    ? taskProgressSnapshot
    : null
  // 任务进度 UI 是否会渲染——决定其下方 Plan Banner 是否需要补顶部圆角。
  const hasVisibleTaskProgress = !!visibleTaskProgressSnapshot
  const pendingQueueItems: PendingSendPreview[] =
    pendingSends && pendingSends.length > 0
      ? pendingSends
      : pendingMessage
        ? [
            {
              id: "__legacy__",
              text: pendingMessage,
              mode: "queue",
              status: "queued",
              canForceInsert: false,
              attachmentCount: 0,
              quoteCount: 0,
            },
          ]
        : []
  const pendingVisibleItems = pendingExpanded ? pendingQueueItems : pendingQueueItems.slice(0, 2)
  const nextSendablePendingId = pendingQueueItems.find(
    (item) => item.status === "queued" || item.status === "fallback_after_reply",
  )?.id
  const hasPendingQueue = pendingQueueItems.length > 0
  const topStripBase =
    !hasVisibleTaskProgress &&
    attachedFiles.length === 0 &&
    !pendingQuotes?.length &&
    !hasPendingQueue
  const workflowTriggerHintIsFirstContent = topStripBase
  const activeGoalStripIsFirstContent = topStripBase && !showWorkflowTriggerHint
  const activeGoalStatusOpen = !!activeGoal && !goalComposerMode
  const effectiveShowWorkflowProgressLine = showWorkflowProgressLine && !activeGoalStatusOpen
  const standaloneActivityStatusOpen =
    !activeGoalStatusOpen &&
    !effectiveShowWorkflowProgressLine &&
    !hasVisibleTaskProgress &&
    !!autonomyActivity &&
    autonomyActivity.state !== "idle" &&
    autonomyActivity.state !== "terminal"
  const workflowModeStatusOpen = workflowModeActive && !incognitoEnabled
  const workflowProgressLineIsFirstContent = activeGoalStripIsFirstContent && !activeGoalStatusOpen
  const standaloneActivityStripIsFirstContent =
    workflowProgressLineIsFirstContent && !effectiveShowWorkflowProgressLine
  const workflowModeStatusIsFirstContent =
    standaloneActivityStripIsFirstContent && !standaloneActivityStatusOpen
  const modeBannerIsFirstContent = workflowModeStatusIsFirstContent && !workflowModeStatusOpen

  const pendingStatusLabel = (item: PendingSendPreview) => {
    switch (item.status) {
      case "saving":
        return t("chat.pendingSaving", "正在保存")
      case "waiting_tool_boundary":
        return t("chat.pendingWaitingToolBoundary", "等待工具完成点")
      case "inserting":
        return t("chat.pendingInserting", "正在插入")
      case "dispatching":
        return t("chat.pendingDispatching", "正在发送")
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReply", "回复后发送")
      case "queued":
      default:
        return t("chat.pendingQueuedShort", "排队中")
    }
  }

  const pendingStatusTip = (item: PendingSendPreview) => {
    switch (item.status) {
      case "saving":
        return t("chat.pendingSavingTip", "正在把消息和附件保存到可恢复队列。")
      case "waiting_tool_boundary":
        return t(
          "chat.pendingWaitingToolBoundaryTip",
          "等待最近一次工具调用完成；如果本轮不再调用工具，将改为回复结束后发送。",
        )
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReplyTip", "未遇到工具完成点，将在当前回复结束后发送。")
      case "inserting":
        return t("chat.pendingInsertingTip", "已进入工具完成边界，暂时不能编辑或删除。")
      case "dispatching":
        return t("chat.pendingDispatchingTip", "正在从持久队列创建新的对话回合。")
      case "queued":
      default:
        return t("chat.pendingQueuedTip", "已加入待发送队列，将在当前回复结束后发送。")
    }
  }

  const renderInlineAddControls = () => (
    <>
      {onWorkingDirChange && (
        <WorkingDirectoryButton
          workingDir={workingDir ?? null}
          inherited={workingDirInherited}
          saving={workingDirSaving}
          onChange={onWorkingDirChange}
        />
      )}
      <AttachFilesButton onAttachFiles={onAttachFiles} />
      <IconTip label={t("slashCommands.buttonTip")}>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 rounded-lg text-muted-foreground hover:text-foreground"
          onClick={toggleSlashCommandMenu}
        >
          <Slash className="h-4 w-4" />
        </Button>
      </IconTip>
    </>
  )

  const handleLoopCreateOpen = () => {
    handleLoopModeToggle()
  }

  const renderOverflowMenuItem = (actionId: ChatInputOverflowActionId) => {
    switch (actionId) {
      case "attach-files":
        return (
          <AttachFilesMenuItem
            onAttachFiles={onAttachFiles}
            onPicked={() => setShowOverflowMenu(false)}
          />
        )
      case "working-dir":
        return onWorkingDirChange ? (
          <WorkingDirectoryButton
            workingDir={workingDir ?? null}
            inherited={workingDirInherited}
            saving={workingDirSaving}
            variant="menu"
            onPicked={() => setShowOverflowMenu(false)}
            onChange={onWorkingDirChange}
          />
        ) : (
          <button type="button" disabled className={overflowMenuItemClass}>
            <FolderPlus className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("chat.addWorkingDirectory")}</span>
          </button>
        )
      case "slash-command":
        return (
          <button
            type="button"
            className={overflowMenuItemClass}
            onClick={() => {
              setShowOverflowMenu(false)
              toggleSlashCommandMenu()
            }}
          >
            <Slash className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("slashCommands.buttonTip")}</span>
          </button>
        )
    }
  }

  // Add-style actions (working dir / attach / slash) live here once the toolbar
  // is compact. Knowledge + Goal + Loop + Workflow + Plan join at the narrower
  // `toolbarTight` tier so semantic mode controls move as one group.
  const renderOverflowMenuItems = () => (
    <>
      {getChatInputOverflowActionIds().map((actionId) => (
        <Fragment key={actionId}>{renderOverflowMenuItem(actionId)}</Fragment>
      ))}
      {toolbarTight && (
        <>
          <KnowledgePicker
            variant="menu"
            sessionId={currentSessionId ?? null}
            projectId={projectId ?? null}
            disabled={incognitoEnabled}
            draftAttachments={draftKbAttachments}
            onDraftAttachChange={onDraftKbAttachChange}
          />
          <button
            type="button"
            aria-label={goalToggleTip}
            className={cn(overflowMenuItemClass, goalComposerMode && "text-emerald-600")}
            disabled={incognitoEnabled}
            onClick={() => {
              setShowOverflowMenu(false)
              handleGoalModeToggle()
            }}
          >
            <Target className="h-4 w-4 shrink-0" />
            <span className="truncate">{goalToggleLabel}</span>
          </button>
          {loopModeAvailable && (
            <button
              type="button"
              className={cn(overflowMenuItemClass, loopComposerMode && "text-sky-600")}
              disabled={incognitoEnabled}
              onClick={() => {
                setShowOverflowMenu(false)
                handleLoopCreateOpen()
              }}
            >
              <Radio className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{loopToggleLabel}</span>
            </button>
          )}
          <div className="rounded-md border border-border/50 bg-background/35 p-1">
            <div className="flex items-center gap-2 px-2 py-1 text-[11px] font-medium text-muted-foreground">
              {workflowModeSaving ? (
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
              ) : (
                <WorkflowModeIcon className="h-3.5 w-3.5 shrink-0" />
              )}
              <span className="truncate">{workflowToggleLabel}</span>
              <span className="ml-auto truncate">{workflowModeLabel(t, workflowMode)}</span>
            </div>
            {renderWorkflowModeMenuItems(() => setShowOverflowMenu(false))}
          </div>
          <button
            type="button"
            aria-label={planToggleTip}
            className={cn(
              overflowMenuItemClass,
              planState === "planning" && "text-blue-600",
              planState === "review" && "text-purple-600",
              planState === "executing" && "text-green-600",
            )}
            onClick={() => {
              setShowOverflowMenu(false)
              handlePlanToggle()
            }}
          >
            <ClipboardList className="h-4 w-4 shrink-0" />
            <span className="truncate">{planToggleLabel}</span>
          </button>
        </>
      )}
      {permissionCollapsed && (
        <PermissionModeSwitcher
          variant="menu"
          permissionMode={permissionMode}
          onPermissionModeChange={handlePermissionModeChange}
          sandboxMode={sandboxMode}
          onSandboxModeChange={onSandboxModeChange}
        />
      )}
    </>
  )

  return (
    <div className={cn("min-w-0 px-3 pb-3 pt-2", hero && "px-0 pb-0 pt-0")}>
      <div
        ref={inputShellRef}
        className={cn(
          "relative min-w-0 overflow-visible rounded-input-dock border border-border-soft bg-surface-floating shadow-input-dock",
          hero && "shadow-floating",
          incognitoEnabled && [
            "[--color-surface-floating:hsl(0_0%_13%)]",
            "[--color-surface-subtle:hsl(0_0%_16%)]",
            "[--color-secondary:hsl(0_0%_17%)]",
            "[--color-foreground:hsl(0_0%_96%)]",
            "[--color-muted-foreground:hsl(0_0%_70%)]",
            "[--color-border:hsl(0_0%_22%)]",
            "[--color-border-soft:hsl(0_0%_22%)]",
            "shadow-[0_18px_52px_hsl(0_0%_4%/0.24)]",
          ],
        )}
      >
        {/* Slash Command Menu */}
        <SlashCommandMenu
          open={slash.isOpen}
          commands={slash.expandedCmd ? [] : slash.filteredCommands}
          selectedIndex={slash.selectedIndex}
          onSelect={slash.executeCommand}
          expandedCmd={slash.expandedCmd}
          filteredOptions={slash.filteredOptions}
          selectedOptionIndex={slash.selectedOptionIndex}
          onSelectOption={slash.executeOption}
        />

        {/* Note Mention Menu (`[[` popper) */}
        <NoteMentionMenu
          isOpen={noteMention.isOpen && !slash.isOpen}
          entries={noteMention.entries}
          selectedIndex={noteMention.selectedIndex}
          loading={noteMention.loading}
          loadErrorDetail={noteMention.loadErrorDetail}
          onSelect={noteMention.applyEntry}
          onHover={noteMention.setSelectedIndex}
        />

        {/* File Mention Menu (`@` popper) */}
        <FileMentionMenu
          isOpen={mention.isOpen && !slash.isOpen && !noteMention.isOpen}
          entries={mention.entries}
          noteEntries={mention.noteEntries}
          notesLoading={mention.notesLoading}
          noteLoadErrorDetail={mention.noteLoadErrorDetail}
          noteCapable={mention.noteCapable}
          skillEntries={mention.skillEntries}
          skillCapable={mention.skillCapable}
          agentEntries={mention.agentEntries}
          agentCapable={mention.agentCapable}
          selectedIndex={mention.selectedIndex}
          mode={mention.mode}
          dirPath={mention.dirPath}
          workingDir={workingDir ?? null}
          loading={mention.loading}
          error={mention.error}
          truncated={mention.truncated}
          hasFileQuery={mention.hasFileQuery}
          onSelect={mention.applyEntry}
          onSelectNote={mention.applyNote}
          onSelectSkill={mention.applySkill}
          onSelectAgent={mention.applyAgent}
          onHover={mention.setSelectedIndex}
        />

        {/* Quick Prompt Menu (`#` popper) */}
        <QuickPromptMenu
          isOpen={quickPrompt.isOpen && !slash.isOpen && !noteMention.isOpen && !mention.isOpen}
          entries={quickPrompt.entries}
          selectedIndex={quickPrompt.selectedIndex}
          query={quickPrompt.query}
          onSelect={quickPrompt.applyEntry}
          onHover={quickPrompt.setSelectedIndex}
        />

        {visibleTaskProgressSnapshot && (
          <TaskProgressPanel
            snapshot={visibleTaskProgressSnapshot}
            executionState={taskExecutionState}
            variant="embedded"
            onOpenWorkspace={onOpenWorkspace}
            workspaceOpen={workspacePanelVisible}
          />
        )}

        {/* Attached files preview (rendered above textarea) */}
        <AttachmentPreview
          attachedFiles={attachedFiles}
          onRemoveFile={onRemoveFile}
          onUpdateFile={onUpdateFile}
        />

        {/* Staged "quote to chat" references */}
        <AnimatedCollapse open={!!pendingQuotes?.length}>
          <div className="flex flex-wrap gap-1.5 px-3 pt-2">
            {pendingQuotes?.map((q, index) => {
              const lines =
                q.startLine === q.endLine ? `${q.startLine}` : `${q.startLine}-${q.endLine}`
              // Code-point-safe truncation so the preview can't split a surrogate
              // pair (emoji / astral CJK) and render a � at the cut.
              const cps = Array.from(q.content)
              const preview = cps.length > 400 ? `${cps.slice(0, 400).join("")}…` : q.content
              return (
                <span
                  key={`${q.path}:${lines}:${index}`}
                  className="inline-flex max-w-[260px] items-center gap-0.5 rounded-md border border-border/60 bg-secondary/40 py-0.5 pl-1 pr-1 text-xs text-foreground/80"
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={() => onJumpToQuote?.(q)}
                        disabled={!onJumpToQuote}
                        className="inline-flex min-w-0 items-center gap-1 rounded px-1 py-0.5 transition-colors hover:bg-background/70 disabled:pointer-events-none"
                      >
                        <Quote className="h-3 w-3 shrink-0 text-muted-foreground" />
                        <span className="truncate">
                          {q.name}
                          <span className="ml-1 text-muted-foreground">L{lines}</span>
                        </span>
                      </button>
                    </TooltipTrigger>
                    {/* Hover preview of the quoted text (click jumps to it). */}
                    <TooltipContent side="top" className="max-w-[340px]">
                      <span className="block max-h-40 overflow-hidden whitespace-pre-wrap text-xs">
                        {preview}
                      </span>
                    </TooltipContent>
                  </Tooltip>
                  {onRemoveQuote && (
                    <button
                      type="button"
                      onClick={() => onRemoveQuote(index)}
                      className="rounded p-0.5 text-muted-foreground hover:bg-background/70 hover:text-foreground"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </span>
              )
            })}
          </div>
        </AnimatedCollapse>

        {/* Pending send queue */}
        <AnimatedCollapse open={hasPendingQueue}>
          <div className="px-3 pt-2.5 pb-0 animate-in fade-in-0 slide-in-from-top-1 duration-200">
            <div className="rounded-lg border border-amber-500/20 bg-amber-500/8 px-2.5 py-2">
              <div className="mb-1.5 flex items-center gap-2">
                <BetweenHorizontalStart className="h-4 w-4 shrink-0 text-amber-500" />
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground/80">
                  {t("chat.pendingQueueTitle", "待发送")} · {pendingQueueItems.length}
                </span>
                {pendingQueueItems.length > 2 && (
                  <IconTip
                    label={
                      pendingExpanded ? t("common.collapse", "收起") : t("common.expand", "展开")
                    }
                  >
                    <button
                      type="button"
                      onClick={() => setPendingExpanded((v) => !v)}
                      className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
                    >
                      {pendingExpanded ? (
                        <ChevronUp className="h-3.5 w-3.5" />
                      ) : (
                        <ChevronDown className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </IconTip>
                )}
              </div>
              <div className="flex flex-col gap-1.5">
                {pendingVisibleItems.map((item) => {
                  const beginEdit = () => {
                    if (item.id === "__legacy__") {
                      onCancelPending?.()
                      return
                    }
                    setEditingPendingId(item.id)
                    setPendingEditValue(item.text)
                  }
                  const saveEdit = async () => {
                    const next = pendingEditValue.trim()
                    if (!next || !onEditPending) return
                    setPendingEditSaving(true)
                    try {
                      const changed = await onEditPending(item.id, next)
                      if (changed) setEditingPendingId(null)
                    } finally {
                      setPendingEditSaving(false)
                    }
                  }
                  const discard = () =>
                    item.id === "__legacy__"
                      ? onDiscardPending?.()
                      : onDiscardPendingItem?.(item.id)
                  const readonly =
                    item.status === "saving" ||
                    item.status === "inserting" ||
                    item.status === "dispatching"
                  const canCancelForce =
                    item.mode === "force_insert" && item.status === "waiting_tool_boundary"
                  const canSendNow =
                    !loading &&
                    item.id === nextSendablePendingId &&
                    (item.status === "queued" || item.status === "fallback_after_reply")
                  const isEditing = editingPendingId === item.id
                  return (
                    <div
                      key={item.id}
                      className="flex min-w-0 items-center gap-1.5 rounded-md bg-background/45 px-2 py-1.5"
                    >
                      <IconTip label={pendingStatusTip(item)}>
                        <span className="shrink-0 rounded-sm bg-amber-500/12 px-1.5 py-0.5 text-[11px] text-amber-700 dark:text-amber-300">
                          {pendingStatusLabel(item)}
                        </span>
                      </IconTip>
                      {isEditing ? (
                        <input
                          autoFocus
                          value={pendingEditValue}
                          disabled={pendingEditSaving}
                          className="h-7 min-w-0 flex-1 rounded border border-border bg-background px-2 text-sm outline-none focus:border-ring"
                          onChange={(event) => setPendingEditValue(event.target.value)}
                          onKeyDown={(event) => {
                            if (event.key === "Enter" && !event.shiftKey) {
                              event.preventDefault()
                              void saveEdit()
                            } else if (event.key === "Escape") {
                              setEditingPendingId(null)
                            }
                          }}
                        />
                      ) : (
                        <span className="min-w-0 flex-1 truncate text-sm text-foreground/90">
                          {item.text}
                          {(item.attachmentCount > 0 || item.quoteCount > 0) && (
                            <span className="ml-1 text-xs text-muted-foreground">
                              +{item.attachmentCount + item.quoteCount}
                            </span>
                          )}
                        </span>
                      )}
                      {isEditing ? (
                        <>
                          <button
                            type="button"
                            disabled={pendingEditSaving || !pendingEditValue.trim()}
                            className="rounded-md p-1 text-emerald-600 hover:bg-emerald-500/10 disabled:opacity-40"
                            onClick={() => void saveEdit()}
                          >
                            {pendingEditSaving ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Check className="h-3.5 w-3.5" />
                            )}
                          </button>
                          <button
                            type="button"
                            className="rounded-md p-1 text-muted-foreground hover:bg-secondary"
                            onClick={() => setEditingPendingId(null)}
                          >
                            <X className="h-3.5 w-3.5" />
                          </button>
                        </>
                      ) : null}
                      {!isEditing && canSendNow && (
                        <IconTip label={t("chat.pendingSendNow", "立即发送")}>
                          <button
                            type="button"
                            className="rounded-md p-1 text-emerald-600 transition-colors hover:bg-emerald-500/10"
                            onClick={() => onSendPending?.(item.id)}
                          >
                            <PlayCircle className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      )}
                      {!isEditing &&
                        (canCancelForce ? (
                          <IconTip label={t("chat.pendingCancelForceInsert", "取消插入本轮")}>
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                              onClick={() => onCancelForceInsertPending?.(item.id)}
                            >
                              <Undo2 className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
                        ) : (
                          loading &&
                          item.canForceInsert && (
                            <IconTip
                              label={t(
                                "chat.pendingForceInsertTip",
                                "会等正在执行的工具完成后插入给模型，不会打断当前工具。",
                              )}
                            >
                              <button
                                type="button"
                                className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                                onClick={() => onForceInsertPending?.(item.id)}
                              >
                                <BetweenHorizontalStart className="h-3.5 w-3.5" />
                              </button>
                            </IconTip>
                          )
                        ))}
                      {!isEditing && !readonly && (
                        <>
                          {item.editable !== false && (
                            <IconTip label={t("chat.pendingEdit")}>
                              <button
                                type="button"
                                className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                                onClick={beginEdit}
                              >
                                <Pencil className="h-3.5 w-3.5" />
                              </button>
                            </IconTip>
                          )}
                          <IconTip label={t("chat.pendingDelete")}>
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                              onClick={discard}
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
                        </>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          </div>
        </AnimatedCollapse>

        {/* Natural workflow trigger hint — suggests mode only, never creates a run itself. */}
        <AnimatedCollapse open={showWorkflowTriggerHint}>
          {workflowTriggerHint ? (
            <div
              className={cn(
                "border-b px-3 py-2 text-xs",
                workflowTriggerHint.mode === "ultracode"
                  ? "border-purple-500/15 bg-purple-500/7 text-purple-700 dark:text-purple-300"
                  : "border-blue-500/15 bg-blue-500/7 text-blue-700 dark:text-blue-300",
                workflowTriggerHintIsFirstContent && "rounded-t-2xl",
              )}
            >
              <div className="flex min-w-0 items-center gap-2">
                {workflowTriggerHint.mode === "ultracode" ? (
                  <Sparkles className="h-3.5 w-3.5 shrink-0" />
                ) : (
                  <GitPullRequest className="h-3.5 w-3.5 shrink-0" />
                )}
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">
                    {t("chat.workflowTriggerHint.title", "这条消息看起来适合工作流")}
                  </div>
                  <div className="truncate text-foreground/65">
                    {workflowTriggerHint.mode === "ultracode"
                      ? t(
                          "chat.workflowTriggerHint.ultracodeDescription",
                          "开启后模型会更偏向深度编排、交叉验证和长任务恢复。",
                        )
                      : t(
                          "chat.workflowTriggerHint.description",
                          "开启后模型可自行判断是否创建可观察、可恢复的后台工作流。",
                        )}
                  </div>
                </div>
                <button
                  type="button"
                  className="shrink-0 rounded-md border border-current/15 bg-background/45 px-2 py-1 text-[11px] font-medium transition-colors hover:bg-background/70 disabled:cursor-not-allowed disabled:opacity-60"
                  disabled={workflowMenuDisabled}
                  onClick={() => {
                    void updateWorkflowMode(workflowTriggerHint.mode).then(() => {
                      setDismissedWorkflowHintFor(normalizedWorkflowTriggerInput)
                    })
                  }}
                >
                  {workflowTriggerHint.mode === "ultracode"
                    ? t("chat.workflowTriggerHint.enableUltracode", "开启 Ultracode")
                    : t("chat.workflowTriggerHint.enable", "开启自动")}
                </button>
                <IconTip label={t("chat.workflowTriggerHint.dismiss", "忽略")}>
                  <button
                    type="button"
                    aria-label={t("chat.workflowTriggerHint.dismiss", "忽略")}
                    className="shrink-0 rounded-md p-1 transition-colors hover:bg-background/60"
                    onClick={() => setDismissedWorkflowHintFor(normalizedWorkflowTriggerInput)}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </IconTip>
              </div>
            </div>
          ) : null}
        </AnimatedCollapse>

        {/* Active Goal status — always visible near the composer while a durable goal is open. */}
        <AnimatedCollapse open={activeGoalStatusOpen}>
          {activeGoal ? (
            <div
              className={cn(
                "border-b border-emerald-500/15 bg-emerald-500/7 px-3 py-2 text-xs text-emerald-700 dark:text-emerald-300",
                activeGoalStripIsFirstContent && "rounded-t-2xl",
              )}
            >
              <div className="flex min-w-0 items-center gap-2">
                <Target className="h-3.5 w-3.5 shrink-0" />
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left font-medium"
                  onClick={onOpenWorkspace}
                >
                  {t("chat.goalMode.activeGoal", "进行中的目标")}{" "}
                  <span className="font-normal text-foreground/75">
                    {activeGoal.objective.replace(/\s+/g, " ")}
                  </span>
                </button>
                {goalLoading ? (
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
                ) : null}
                <span
                  className={cn(
                    "shrink-0 rounded-full border bg-background/45 px-2 py-0.5 text-[11px]",
                    autonomyActivity?.needsUser
                      ? "border-amber-500/30 text-amber-700 dark:text-amber-300"
                      : autonomyActivity?.state === "blocked"
                        ? "border-destructive/30 text-destructive"
                        : "border-emerald-500/20",
                  )}
                  title={activityDetail || undefined}
                >
                  {activityHeadlineLabel}
                </span>
                {activeGoalProgressLabel ? (
                  <span className="shrink-0 rounded-full border border-emerald-500/20 bg-background/45 px-2 py-0.5 text-[11px]">
                    {activeGoalProgressLabel}
                  </span>
                ) : null}
                <IconTip label={t("chat.goalMode.edit", "编辑目标")}>
                  <button
                    type="button"
                    className="rounded-md p-1 text-emerald-700/75 transition-colors hover:bg-background/60 hover:text-emerald-800 dark:text-emerald-300/75 dark:hover:text-emerald-200"
                    onClick={() => setGoalEditOpen((value) => !value)}
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </button>
                </IconTip>
                <IconTip label={t("chat.goalMode.evaluate", "评估目标")}>
                  <button
                    type="button"
                    className="rounded-md p-1 text-emerald-700/75 transition-colors hover:bg-background/60 hover:text-emerald-800 disabled:opacity-50 dark:text-emerald-300/75 dark:hover:text-emerald-200"
                    disabled={!!goalActionPending || activeGoal.state === "evaluating"}
                    onClick={() => runGoalAction("evaluate", onEvaluateGoal)}
                  >
                    {goalActionPending === "evaluate" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <CheckCircle2 className="h-3.5 w-3.5" />
                    )}
                  </button>
                </IconTip>
                {activeGoal.state === "paused" || activeGoal.state === "blocked" ? (
                  <IconTip label={t("chat.goalMode.resume", "恢复目标")}>
                    <button
                      type="button"
                      className="rounded-md p-1 text-emerald-700/75 transition-colors hover:bg-background/60 hover:text-emerald-800 disabled:opacity-50 dark:text-emerald-300/75 dark:hover:text-emerald-200"
                      disabled={!!goalActionPending}
                      onClick={() => runGoalAction("resume", onResumeGoal)}
                    >
                      {goalActionPending === "resume" ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <PlayCircle className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </IconTip>
                ) : (
                  <IconTip label={t("chat.goalMode.pause", "暂停目标")}>
                    <button
                      type="button"
                      className="rounded-md p-1 text-emerald-700/75 transition-colors hover:bg-background/60 hover:text-emerald-800 disabled:opacity-50 dark:text-emerald-300/75 dark:hover:text-emerald-200"
                      disabled={!!goalActionPending || activeGoal.state === "evaluating"}
                      onClick={() => runGoalAction("pause", onPauseGoal)}
                    >
                      {goalActionPending === "pause" ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <PauseCircle className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </IconTip>
                )}
                <IconTip label={t("chat.goalMode.clear", "清除目标")}>
                  <button
                    type="button"
                    className="rounded-md p-1 text-emerald-700/70 transition-colors hover:bg-destructive/10 hover:text-destructive disabled:opacity-50 dark:text-emerald-300/70"
                    disabled={!!goalActionPending}
                    onClick={() => runGoalAction("clear", onClearGoal)}
                  >
                    {goalActionPending === "clear" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Trash2 className="h-3.5 w-3.5" />
                    )}
                  </button>
                </IconTip>
              </div>

              <AnimatedCollapse open={goalEditOpen}>
                <div className="mt-2 space-y-2 rounded-lg border border-emerald-500/20 bg-background/65 p-2 text-foreground">
                  <input
                    value={goalEditObjective}
                    onChange={(event) => setGoalEditObjective(event.target.value)}
                    className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs outline-none focus:border-emerald-500/50"
                    placeholder={t("chat.goalMode.objectivePlaceholder", "目标")}
                  />
                  <textarea
                    value={goalEditCriteria}
                    onChange={(event) => setGoalEditCriteria(event.target.value)}
                    className="min-h-16 w-full resize-y rounded-md border border-border/60 bg-background px-2 py-1.5 text-xs outline-none focus:border-emerald-500/50"
                    placeholder={t(
                      "chat.goalMode.criteriaPlaceholder",
                      "完成标准；可用 [required] / [optional] / [follow-up]",
                    )}
                  />
                  <GoalCriteriaDraftPreview criteriaText={goalEditCriteria} />
                  <div className="flex justify-end gap-1.5">
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2 text-xs"
                      onClick={() => {
                        setGoalEditOpen(false)
                        setGoalEditObjective(activeGoal.objective)
                        setGoalEditCriteria(activeGoal.completionCriteria)
                      }}
                    >
                      {t("common.cancel", "取消")}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      className="h-7 px-2 text-xs"
                      disabled={goalActionPending === "update" || !goalEditObjective.trim()}
                      onClick={saveGoalEdit}
                    >
                      {goalActionPending === "update" ? (
                        <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                      ) : null}
                      {t("common.save", "保存")}
                    </Button>
                  </div>
                </div>
              </AnimatedCollapse>
            </div>
          ) : null}
        </AnimatedCollapse>

        {/* Workflow progress line — compact run status without opening the expert workspace. */}
        <AnimatedCollapse open={effectiveShowWorkflowProgressLine}>
          {workflowProgressRun ? (
            <div
              className={cn(
                "border-b px-3 py-1.5 text-xs",
                workflowRunToneClass(workflowProgressRun.state),
                workflowProgressLineIsFirstContent && "rounded-t-2xl",
              )}
            >
              <div className="flex min-w-0 items-center gap-2">
                <GitPullRequest className="h-3.5 w-3.5 shrink-0" />
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left font-medium"
                  onClick={onOpenWorkspace}
                >
                  {t("chat.workflowProgress.title", "工作流运行")}{" "}
                  <span className="font-normal text-foreground/75">
                    {workflowProgressRun.kind || t("chat.workflowProgress.defaultKind", "通用任务")}
                  </span>
                </button>
                <span className="shrink-0 rounded-full border border-current/15 bg-background/45 px-2 py-0.5 text-[11px]">
                  {workflowRunStateLabel(t, workflowProgressRun.state)}
                </span>
                {workflowProgressRun.cursorSeq > 0 ? (
                  <span className="hidden shrink-0 rounded-full border border-current/15 bg-background/45 px-2 py-0.5 text-[11px] sm:inline-flex">
                    {t("chat.workflowProgress.steps", "{{count}} 步", {
                      count: workflowProgressRun.cursorSeq,
                    })}
                  </span>
                ) : null}
                {workflowProgressExtraCount > 0 ? (
                  <span className="hidden shrink-0 rounded-full border border-current/15 bg-background/45 px-2 py-0.5 text-[11px] sm:inline-flex">
                    {t("chat.workflowProgress.more", "+{{count}} 个", {
                      count: workflowProgressExtraCount,
                    })}
                  </span>
                ) : null}
                {onOpenWorkspace ? (
                  <button
                    type="button"
                    className="shrink-0 rounded-md px-2 py-1 text-[11px] font-medium transition-colors hover:bg-background/60"
                    onClick={onOpenWorkspace}
                  >
                    {t("chat.workflowProgress.view", "查看")}
                  </button>
                ) : null}
              </div>
            </div>
          ) : null}
        </AnimatedCollapse>

        {/* Unified activity fallback for Loop/background states without an active Goal or Workflow. */}
        <AnimatedCollapse open={standaloneActivityStatusOpen}>
          {autonomyActivity ? (
            <div
              className={cn(
                "border-b px-3 py-1.5 text-xs",
                autonomyActivity.needsUser
                  ? "border-amber-500/20 bg-amber-500/8 text-amber-700 dark:text-amber-300"
                  : autonomyActivity.state === "blocked"
                    ? "border-destructive/20 bg-destructive/7 text-destructive"
                    : "border-sky-500/20 bg-sky-500/8 text-sky-700 dark:text-sky-300",
                standaloneActivityStripIsFirstContent && "rounded-t-2xl",
              )}
            >
              <div className="flex min-w-0 items-center gap-2">
                <Radio className="h-3.5 w-3.5 shrink-0" />
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left font-medium"
                  onClick={onOpenWorkspace}
                  title={activityDetail || undefined}
                >
                  {activityHeadlineLabel}
                  {autonomyActivity.currentStep ? (
                    <span className="font-normal text-foreground/70">
                      {" "}
                      {autonomyActivity.currentStep}
                    </span>
                  ) : null}
                </button>
                {autonomyActivity.needsUser ? (
                  <span className="shrink-0 rounded-full border border-current/15 bg-background/45 px-2 py-0.5 text-[11px]">
                    {t("chat.activity.needsUser", "需要你处理")}
                  </span>
                ) : null}
              </div>
            </div>
          ) : null}
        </AnimatedCollapse>

        {/* Workflow Mode status — visible only when autonomous orchestration is enabled. */}
        <AnimatedCollapse open={workflowModeStatusOpen}>
          <div
            className={cn(
              "border-b border-blue-500/15 bg-blue-500/7 px-3 py-1.5 text-xs text-blue-700 dark:text-blue-300",
              workflowModeStatusIsFirstContent && "rounded-t-2xl",
            )}
          >
            <div className="flex min-w-0 items-center gap-2">
              <WorkflowModeIcon className="h-3.5 w-3.5 shrink-0" />
              <button
                type="button"
                className="min-w-0 flex-1 truncate text-left font-medium"
                onClick={onOpenWorkspace}
              >
                {t("chat.workflowMode.active", "工作流模式")}{" "}
                <span className="font-normal text-foreground/75">
                  {workflowMode === "ultracode"
                    ? t("chat.workflowMode.activeUltracodeDetail", "模型会优先使用完整动态编排")
                    : t("chat.workflowMode.activeOnDetail", "模型可按需创建可观察的工作流运行")}
                </span>
              </button>
              <span className="shrink-0 rounded-full border border-blue-500/20 bg-background/45 px-2 py-0.5 text-[11px]">
                {workflowModeLabel(t, workflowMode)}
              </span>
              <IconTip label={t("chat.workflowMode.turnOff", "关闭工作流模式")}>
                <button
                  type="button"
                  className="rounded-md p-1 text-blue-700/75 transition-colors hover:bg-background/60 hover:text-blue-800 disabled:opacity-50 dark:text-blue-300/75 dark:hover:text-blue-200"
                  disabled={!!workflowModeSaving}
                  onClick={() => void updateWorkflowMode("off")}
                >
                  {workflowModeSaving === "off" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <X className="h-3.5 w-3.5" />
                  )}
                </button>
              </IconTip>
            </div>
          </div>
        </AnimatedCollapse>

        {/* Goal Mode Banner */}
        <AnimatedCollapse open={goalComposerMode}>
          <div
            className={cn(
              "space-y-1.5 border-b border-emerald-500/20 bg-emerald-500/10 px-3 py-1.5 text-xs text-emerald-700 animate-in fade-in slide-in-from-top-1 duration-200 dark:text-emerald-300",
              modeBannerIsFirstContent && "rounded-t-2xl",
            )}
          >
            <div className="flex items-center gap-2">
              <Target className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">
                {activeGoal
                  ? t("chat.goalMode.activeRestricted", "目标模式：选择如何更新当前目标")
                  : t("chat.goalMode.restricted", "目标模式：发送后会创建当前会话的持续目标")}
              </span>
              <button
                type="button"
                onClick={() => setGoalComposerMode(false)}
                className="transition-colors hover:text-emerald-900 dark:hover:text-emerald-100"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
            {activeGoal ? (
              <div className="grid grid-cols-2 gap-1 sm:grid-cols-5">
                {(
                  [
                    ["create_or_update", t("chat.goalMode.actionUpdate", "更新目标")],
                    ["replace", t("chat.goalMode.actionReplace", "替代目标")],
                    ["append_required", t("chat.goalMode.actionRequired", "追加必须")],
                    ["append_optional", t("chat.goalMode.actionOptional", "追加可选")],
                    ["append_follow_up", t("chat.goalMode.actionFollowUp", "追加后续")],
                  ] as const
                ).map(([action, label]) => (
                  <button
                    key={action}
                    type="button"
                    className={cn(
                      "h-7 min-w-0 rounded-md border px-2 text-[11px] transition-colors",
                      goalComposerAction === action
                        ? "border-emerald-500/45 bg-background text-emerald-800 dark:text-emerald-100"
                        : "border-emerald-500/15 bg-background/35 text-emerald-700/75 hover:bg-background/70 dark:text-emerald-300/75",
                    )}
                    onClick={() => setGoalComposerAction(action)}
                  >
                    <span className="block truncate">{label}</span>
                  </button>
                ))}
              </div>
            ) : null}
          </div>
        </AnimatedCollapse>

        {/* Loop Mode Banner */}
        <AnimatedCollapse open={loopComposerMode}>
          <div
            className={cn(
              "flex items-center gap-2 border-b border-sky-500/20 bg-sky-500/10 px-3 py-1.5 text-xs text-sky-700 animate-in fade-in slide-in-from-top-1 duration-200 dark:text-sky-300",
              modeBannerIsFirstContent && "rounded-t-2xl",
            )}
          >
            <Radio className="h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0 flex-1 truncate">
              {t("chat.loopMode.restricted", "持续推进模式：发送后会创建可重复触发的推进任务")}
            </span>
            <button
              type="button"
              onClick={() => setLoopComposerMode(false)}
              className="transition-colors hover:text-sky-900 dark:hover:text-sky-100"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </AnimatedCollapse>

        {/* Plan Mode Banner */}
        <AnimatedCollapse open={planComposerBannerOpen}>
          <div
            className={cn(
              "flex items-center gap-2 border-b border-blue-500/20 bg-blue-500/10 px-3 py-1.5 text-xs text-blue-600 animate-in fade-in slide-in-from-top-1 duration-200 dark:text-blue-400",
              modeBannerIsFirstContent && "rounded-t-2xl",
            )}
          >
            <ClipboardList className="h-3.5 w-3.5 shrink-0" />
            <span className="flex-1">{t("planMode.restricted")}</span>
            <button
              onClick={onExitPlanMode}
              className="hover:text-blue-800 dark:hover:text-blue-200 transition-colors"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </AnimatedCollapse>

        {/* Tokenized composer: raw `input` stays plain text, selected mentions
            render as atomic chips inside the editable surface. */}
        <div className="relative">
          <MentionComposerInput
            ref={inputHandleRef}
            placeholder={
              goalComposerMode
                ? t("chat.goalMode.placeholder", "描述你希望持续推进并最终完成的目标")
                : loopComposerMode
                  ? t("chat.loopMode.placeholder", "描述你希望持续推进的任务")
                  : planState === "planning"
                    ? t("planMode.placeholder")
                    : hasPendingQueue
                      ? t("chat.pendingQueued")
                      : t("chat.askAnything")
            }
            value={input}
            onChange={setComposerInput}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            onSelectionChange={() => {
              mention.recheckTrigger()
              noteMention.recheckTrigger()
              quickPrompt.recheckTrigger()
            }}
            workingDir={workingDir ?? null}
            fileEnabled={!!workingDir}
            noteEnabled={enableNoteMention}
            skillEnabled={enableSkillMention}
            agentMentionEnabled={enableAgentMention}
            agents={agents}
            hero={hero}
            readOnly={voice.state === "recording" || voice.state === "transcribing"}
          />
        </div>

        {/* URL Previews */}
        <AnimatedCollapse open={urlPreviews.size > 0}>
          <div className="px-3 pb-1 flex flex-col gap-1.5 max-h-[200px] overflow-y-auto">
            {Array.from(urlPreviews.entries())
              .filter(([url]) => !dismissedUrls.has(url))
              .map(([url, data]) => (
                <UrlPreviewCard
                  key={url}
                  data={data}
                  dismissible
                  onDismiss={() => dismissUrl(url)}
                />
              ))}
          </div>
        </AnimatedCollapse>

        {/* Toolbar — replaced by RecordingBar while voice capture / STT
            is in flight, since the normal toolbar buttons are
            unreachable during recording anyway. */}
        <AnimatedCollapse open={voice.state === "recording" || voice.state === "transcribing"}>
          <AnimatedPresenceBox
            open={voice.state === "recording" || voice.state === "transcribing"}
            className="will-change-[opacity,transform]"
            enterClassName="translate-y-0 opacity-100"
            exitClassName="translate-y-1 opacity-0 pointer-events-none"
          >
            <RecordingBar
              transcribing={voice.state === "transcribing"}
              durationMs={voice.durationMs}
              levels={voice.levels}
              onCancel={handleVoiceCancel}
              onStop={() => void handleVoiceStop()}
            />
          </AnimatedPresenceBox>
        </AnimatedCollapse>
        <div
          style={
            normalToolbarOpen && toolbarMinHeight !== null
              ? { minHeight: toolbarMinHeight }
              : undefined
          }
        >
          <AnimatedCollapse open={normalToolbarOpen} overflow="visible-when-open">
            <div
              ref={toolbarRef}
              // Always two columns so Send/Stop stays pinned in its own column
              // on a single row. The left group never wraps; overflow is a
              // measurement signal that pushes controls into the "+" menu.
              className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2 px-2 pb-2"
            >
              <div
                ref={toolbarLeftRef}
                className="flex min-w-0 flex-nowrap items-center gap-1 overflow-visible"
              >
                <div
                  ref={addActionsRef}
                  className={toolbarCompact ? "hidden" : CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS}
                >
                  {renderInlineAddControls()}
                </div>

                <div
                  ref={overflowTriggerRef}
                  className={
                    toolbarCompact ? "relative block shrink-0" : CHAT_INPUT_OVERFLOW_MENU_CLASS
                  }
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={t("chat.moreActions")}
                        aria-expanded={showOverflowMenu}
                        aria-haspopup="menu"
                        onClick={() => setShowOverflowMenu((open) => !open)}
                        className="h-8 w-8 rounded-lg bg-transparent text-muted-foreground hover:bg-transparent hover:text-foreground focus-visible:ring-0"
                      >
                        <Plus className="h-4 w-4" />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>{t("chat.moreActions")}</TooltipContent>
                  </Tooltip>
                  <FloatingMenu
                    open={showOverflowMenu}
                    className="min-w-[180px] overflow-hidden p-1.5"
                    onEscapeKeyDown={() => setShowOverflowMenu(false)}
                    role="menu"
                  >
                    <div className="flex flex-col gap-0.5">{renderOverflowMenuItems()}</div>
                  </FloatingMenu>
                </div>

                {/* Model / Think / Temperature */}
                <ModelPicker
                  availableModels={availableModels}
                  activeModel={activeModel}
                  reasoningEffort={reasoningEffort}
                  onModelChange={onModelChange}
                  onEffortChange={onEffortChange}
                  onEffortReset={onEffortReset}
                  currentModelInfo={currentModelInfo}
                  unavailablePreference={unavailableModelPreference}
                  sessionTemperature={sessionTemperature}
                  onSessionTemperatureChange={onSessionTemperatureChange}
                />

                <AwarenessToggle sessionId={currentSessionId ?? null} disabled={incognitoEnabled} />

                {/* Knowledge + Goal + Loop + Workflow + Plan — semantic mode controls,
                    kept inline until the measured toolbar would wrap. */}
                {!toolbarTight && (
                  <div ref={semanticModesRef} className="flex shrink-0 items-center gap-1">
                    <KnowledgePicker
                      sessionId={currentSessionId ?? null}
                      projectId={projectId ?? null}
                      disabled={incognitoEnabled}
                      draftAttachments={draftKbAttachments}
                      onDraftAttachChange={onDraftKbAttachChange}
                    />

                    <IconTip label={goalToggleTip}>
                      <button
                        aria-label={goalToggleTip}
                        onClick={handleGoalModeToggle}
                        disabled={incognitoEnabled}
                        className={cn(
                          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50",
                          goalComposerMode
                            ? "text-emerald-600 bg-emerald-500/10"
                            : activeGoal
                              ? "text-emerald-600/90 hover:text-emerald-700"
                              : "text-muted-foreground hover:text-foreground",
                        )}
                      >
                        <Target className="h-4 w-4 shrink-0" />
                        <span>{goalToggleLabel}</span>
                      </button>
                    </IconTip>

                    {loopModeAvailable && (
                      <IconTip label={loopToggleTip}>
                        <button
                          type="button"
                          aria-label={loopToggleTip}
                          onClick={handleLoopCreateOpen}
                          disabled={incognitoEnabled}
                          className={cn(
                            "flex shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded-lg bg-transparent px-2 py-1 text-xs font-medium transition-colors hover:bg-secondary disabled:cursor-not-allowed disabled:opacity-50",
                            loopComposerMode
                              ? "bg-sky-500/10 text-sky-600"
                              : "text-muted-foreground hover:text-foreground",
                          )}
                        >
                          <Radio className="h-4 w-4 shrink-0" />
                          <span>{loopToggleLabel}</span>
                        </button>
                      </IconTip>
                    )}

                    <DropdownMenu open={workflowMenuOpen} onOpenChange={setWorkflowMenuOpen}>
                      <IconTip label={workflowMenuLabel}>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            aria-label={workflowMenuLabel}
                            disabled={workflowMenuDisabled}
                            className={cn(
                              "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50 data-[state=open]:bg-secondary",
                              workflowButtonTone,
                            )}
                          >
                            {workflowModeSaving ? (
                              <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
                            ) : (
                              <WorkflowModeIcon className="h-4 w-4 shrink-0" />
                            )}
                            <span>{workflowButtonLabel}</span>
                            <ChevronDown className="h-3.5 w-3.5 shrink-0 opacity-70" />
                          </button>
                        </DropdownMenuTrigger>
                      </IconTip>
                      <DropdownMenuContent
                        variant="floating"
                        className="min-w-[280px]"
                        side="top"
                        align="start"
                        sideOffset={8}
                      >
                        {renderWorkflowModeMenuItems(() => setWorkflowMenuOpen(false))}
                      </DropdownMenuContent>
                    </DropdownMenu>

                    <IconTip label={planToggleTip}>
                      <button
                        aria-label={planToggleTip}
                        onClick={handlePlanToggle}
                        className={cn(
                          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
                          planState === "planning"
                            ? "text-blue-600 bg-blue-500/10"
                            : planState === "review"
                              ? "text-purple-600 bg-purple-500/10"
                              : planState === "executing"
                                ? "text-green-600 bg-green-500/10"
                                : "text-muted-foreground hover:text-foreground",
                        )}
                      >
                        <ClipboardList className="h-4 w-4 shrink-0" />
                        <span>{planToggleLabel}</span>
                      </button>
                    </IconTip>
                  </div>
                )}

                {/* Permission and sandbox share one menu, which collapses into
                    the "+" overflow only at the narrowest toolbar tier. */}
                {!permissionCollapsed && (
                  <div ref={permissionModeRef} className="shrink-0">
                    <PermissionModeSwitcher
                      permissionMode={permissionMode}
                      onPermissionModeChange={handlePermissionModeChange}
                      sandboxMode={sandboxMode}
                      onSandboxModeChange={onSandboxModeChange}
                    />
                  </div>
                )}
              </div>

              {/* Send & Stop — kept in its own column so toolbar wrapping never
              orphans the send button onto a half-empty row. */}
              <div className="flex min-h-8 min-w-[76px] shrink-0 items-center justify-end gap-1 self-end">
                <VoiceRecordButton
                  state={voice.state}
                  durationMs={voice.durationMs}
                  audioLevel={voice.audioLevel}
                  disabled={false}
                  onStart={() => void startVoice()}
                  onStop={() => void handleVoiceStop()}
                  onCancel={handleVoiceCancel}
                />
                {voice.errorMessage && (
                  <span
                    className="text-xs text-destructive truncate max-w-[180px]"
                    role="status"
                    onAnimationEnd={voice.clearError}
                  >
                    {voice.errorMessage}
                  </span>
                )}
                {loading && (
                  <div className="animate-in fade-in-0 zoom-in-90 duration-150">
                    <IconTip label={t("chat.stopReply")}>
                      <Button
                        size="icon"
                        variant="destructive"
                        className="h-8 w-8 rounded-full shrink-0"
                        onClick={onStop}
                        aria-label={t("chat.stopReply")}
                      >
                        <Square className="h-4 w-4 fill-white stroke-white" />
                      </Button>
                    </IconTip>
                  </div>
                )}

                <IconTip
                  label={
                    loading && hasSendableContent && !sendDisabled
                      ? t("chat.queueMessage")
                      : t("chat.send")
                  }
                >
                  <Button
                    size="icon"
                    className="h-8 w-8 rounded-full shrink-0"
                    onClick={handleSend}
                    disabled={sendUnavailable || goalSubmitting || loopSubmitting}
                    aria-label={
                      loading && hasSendableContent && !sendDisabled
                        ? t("chat.queueMessage")
                        : t("chat.send")
                    }
                  >
                    {goalSubmitting || loopSubmitting ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <Send className="h-4 w-4" />
                    )}
                  </Button>
                </IconTip>
              </div>
            </div>
          </AnimatedCollapse>
        </div>

        {/* Context-usage hairline fused into the dock's bottom border. */}
        {contextUsage ? <ContextUsageBottomBar usage={contextUsage} /> : null}
      </div>
    </div>
  )
}
