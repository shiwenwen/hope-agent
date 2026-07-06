import { Fragment, useRef, useEffect, useLayoutEffect, useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { AnimatedCollapse, AnimatedPresenceBox } from "@/components/ui/animated-presence"
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
} from "lucide-react"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
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
import SandboxModeSwitcher from "./SandboxModeSwitcher"
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
  CHAT_INPUT_TOOLBAR_EXPAND_BUFFER_PX,
  CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL,
  getChatInputOverflowActionIds,
  getChatInputToolbarFlags,
  type ChatInputOverflowActionId,
} from "./toolbarOverflow"
import MentionComposerInput from "./MentionComposerInput"
import type { ComposerInputHandle } from "./composerInputHandle"
import type { ContextUsageInfo } from "../chatUtils"
import { contextUsageBarClass } from "../contextUsageColor"
import type { AgentConfig } from "@/components/settings/types"
import type { QuickPromptItem } from "@/types/quickPrompts"
import type { GoalSnapshot } from "../workspace/useGoal"
import { parseGoalCriteriaDraft, type DraftGoalCriterionKind } from "../workspace/goalCriteriaDraft"

type WorkflowMode = "off" | "on" | "ultracode"
export type GoalModeSubmitAction =
  | "create_or_update"
  | "replace"
  | "append_required"
  | "append_optional"
  | "append_follow_up"

const WORKFLOW_MODE_CHANGED_EVENT = "hope-agent:workflow-mode-changed"

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

interface ChatInputProps {
  input: string
  onInputChange: (value: string) => void
  inputHistory?: string[]
  quickPrompts?: QuickPromptItem[]
  onSend: () => void
  loading: boolean
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  onModelChange: (key: string) => void
  onEffortChange: (effort: string) => void
  attachedFiles: File[]
  onAttachFiles: (files: File[]) => void
  onRemoveFile: (index: number) => void
  pendingQuotes?: PendingFileQuote[]
  onRemoveQuote?: (index: number) => void
  /** Click a staged quote chip to reveal that file in the file browser. */
  onJumpToQuote?: (q: PendingFileQuote) => void
  pendingMessage?: string | null
  pendingSends?: PendingSendPreview[]
  onCancelPending?: () => void
  onDiscardPending?: () => void
  onEditPending?: (id: string) => void
  onDiscardPendingItem?: (id: string) => void
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
  onSessionTemperatureChange?: (temp: number | null) => void
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
  // Goal mode
  goalSnapshot?: GoalSnapshot | null
  goalLoading?: boolean
  onGoalModeSubmit?: (objective: string, action?: GoalModeSubmitAction) => Promise<boolean>
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

type ToolbarGroupKey = keyof typeof CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS
type ToolbarGroupWidths = Record<ToolbarGroupKey, number>

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

function toolbarHasWrappedItems(container: HTMLElement): boolean {
  const rects = visibleToolbarItemRects(container)
  if (rects.length <= 1) return false
  const firstTop = Math.min(...rects.map((rect) => rect.top))
  return rects.some((rect) => rect.top > firstTop + 2)
}

function toolbarHasHorizontalOverflow(container: HTMLElement): boolean {
  return container.scrollWidth > container.clientWidth + 2
}

function toolbarNeedsMoreCollapse(container: HTMLElement): boolean {
  return toolbarHasHorizontalOverflow(container) || toolbarHasWrappedItems(container)
}

function toolbarFirstLineFreeSpace(container: HTMLElement): number {
  if (toolbarHasHorizontalOverflow(container)) return 0
  const rects = visibleToolbarItemRects(container)
  if (rects.length === 0) return 0
  const containerRect = container.getBoundingClientRect()
  const firstTop = Math.min(...rects.map((rect) => rect.top))
  const firstLineRight = Math.max(
    ...rects.filter((rect) => rect.top <= firstTop + 2).map((rect) => rect.right),
  )
  return Math.max(0, Math.floor(containerRect.right - firstLineRight))
}

function widthNeededToExpandToolbar(level: number, widths: ToolbarGroupWidths): number {
  switch (level) {
    case 1:
      return Math.max(0, widths.addActions - widths.overflowTrigger)
    case 2:
      return widths.semanticModes
    case 3:
      return widths.sandbox
    case 4:
      return widths.permission
    default:
      return Number.POSITIVE_INFINITY
  }
}

export default function ChatInput({
  input,
  onInputChange,
  inputHistory = [],
  quickPrompts = [],
  onSend,
  loading,
  availableModels,
  activeModel,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  attachedFiles,
  onAttachFiles,
  onRemoveFile,
  pendingQuotes,
  onRemoveQuote,
  onJumpToQuote,
  pendingMessage,
  pendingSends,
  onCancelPending,
  onDiscardPending,
  onEditPending,
  onDiscardPendingItem,
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
  goalSnapshot,
  goalLoading = false,
  onGoalModeSubmit,
  onGoalUpdate,
  onPauseGoal,
  onResumeGoal,
  onClearGoal,
  onEvaluateGoal,
  taskProgressSnapshot,
  executionState,
  onOpenWorkspace,
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
  const sandboxModeRef = useRef<HTMLDivElement>(null)
  const permissionModeRef = useRef<HTMLDivElement>(null)
  const toolbarGroupWidthsRef = useRef<ToolbarGroupWidths>({
    ...CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  })
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)
  // 0 = everything inline; 1 = add actions behind "+"; 2 = semantic modes behind
  // "+"; 3 = sandbox behind "+"; 4 = permission behind "+". The level is chosen
  // by live DOM measurement instead of fixed container-width breakpoints.
  const [toolbarCollapseLevel, setToolbarCollapseLevel] = useState(0)
  const [toolbarMinHeight, setToolbarMinHeight] = useState<number | null>(null)
  const [pendingExpanded, setPendingExpanded] = useState(false)
  const [goalComposerMode, setGoalComposerMode] = useState(false)
  const [goalComposerAction, setGoalComposerAction] =
    useState<GoalModeSubmitAction>("create_or_update")
  const [goalSubmitting, setGoalSubmitting] = useState(false)
  const [goalEditOpen, setGoalEditOpen] = useState(false)
  const [goalEditObjective, setGoalEditObjective] = useState("")
  const [goalEditCriteria, setGoalEditCriteria] = useState("")
  const [goalActionPending, setGoalActionPending] = useState<string | null>(null)
  const [workflowMode, setWorkflowMode] = useState<WorkflowMode>("off")
  const [workflowModeLoading, setWorkflowModeLoading] = useState(false)
  const [workflowModeSaving, setWorkflowModeSaving] = useState<WorkflowMode | null>(null)
  const [workflowMenuOpen, setWorkflowMenuOpen] = useState(false)
  const { toolbarCompact, toolbarTight, sandboxCollapsed, permissionCollapsed } =
    getChatInputToolbarFlags(toolbarCollapseLevel)

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
  }
  const slash = useSlashCommands(input, setComposerInput, slashActions, inputHandleRef)
  const voice = useVoiceInput()
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
      setWorkflowMode("off")
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
  }, [currentSessionId, incognitoEnabled])

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
  const hasSendableContent = goalComposerMode
    ? input.trim().length > 0
    : input.trim().length > 0 || attachedFiles.length > 0 || (pendingQuotes?.length ?? 0) > 0

  // The chat column can shrink when a right-side panel opens while the viewport
  // stays wide, so the overflow affordance follows the actual toolbar layout.
  // Instead of static width thresholds, we measure horizontal overflow and free
  // first-line space, then collapse or expand one tier at a time. The toolbar
  // itself is nowrap, so "too much content" never turns into a visible second
  // row while the measurement catches up.
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
        sandbox: readToolbarItemWidth(sandboxModeRef.current, toolbarGroupWidthsRef.current.sandbox),
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
          if (toolbarNeedsMoreCollapse(currentLeft)) {
            return Math.min(CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL, level + 1)
          }
          if (level <= 0) return level
          const freeSpace = toolbarFirstLineFreeSpace(currentLeft)
          const needed =
            widthNeededToExpandToolbar(level, toolbarGroupWidthsRef.current) +
            CHAT_INPUT_TOOLBAR_EXPAND_BUFFER_PX
          return freeSpace >= needed ? level - 1 : level
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
      sandboxModeRef.current,
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
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items
      if (!items) return
      const files: File[] = []
      for (let i = 0; i < items.length; i++) {
        const item = items[i]
        if (item.kind === "file") {
          const file = item.getAsFile()
          if (file) files.push(file)
        }
      }
      if (files.length > 0) {
        e.preventDefault()
        onAttachFiles(files)
      }
    },
    [onAttachFiles],
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

  const handleSend = useCallback(() => {
    resetHistoryBrowsing()
    if (goalComposerMode) {
      const objective = input.trim()
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
    onSend()
  }, [
    goalComposerMode,
    goalComposerAction,
    goalSubmitting,
    incognitoEnabled,
    input,
    activeGoal,
    onGoalModeSubmit,
    onSend,
    resetHistoryBrowsing,
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
  const workflowToggleLabel = t("chat.workflowMode.label", "工作流")
  const workflowModeActive = workflowMode !== "off"
  const WorkflowModeIcon = workflowMode === "ultracode" ? Sparkles : GitPullRequest
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
  const activeGoalCriteria = goalSnapshot?.criteria ?? []
  const activeGoalRequiredTotal = activeGoalCriteria.filter(
    (criterion) => (criterion.kind ?? "required") === "required",
  ).length
  const activeGoalRequiredDone = activeGoalCriteria.filter(
    (criterion) =>
      (criterion.kind ?? "required") === "required" && criterion.status === "satisfied",
  ).length
  const activeGoalProgressLabel =
    activeGoalRequiredTotal > 0
      ? `${activeGoalRequiredDone}/${activeGoalRequiredTotal}`
      : `r${activeGoal?.revision ?? 1}`

  const overflowMenuItemClass =
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-foreground/80 outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-50"

  // Shared by the inline Plan toggle and its "+" overflow-menu counterpart.
  const handlePlanToggle = () => {
    if (planState === "off" || planState === "completed") {
      onEnterPlanMode?.()
    } else if (planState === "planning") {
      onExitPlanMode?.()
    } else {
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
      if (next) setGoalComposerAction("create_or_update")
      return next
    })
  }

  const updateWorkflowMode = useCallback(
    async (nextMode: WorkflowMode) => {
      if (incognitoEnabled) {
        toast.error(t("chat.workflowMode.incognito", "无痕会话不启用工作流模式"))
        return
      }
      const targetSessionId = currentSessionId ?? (await onEnsureSession?.())
      if (!targetSessionId) {
        if (!onEnsureSession) {
          toast.error(t("chat.workflowMode.sessionRequired", "先创建会话后再开启工作流模式"))
        }
        return
      }
      if (nextMode === workflowMode || workflowModeSaving) return
      setWorkflowModeSaving(nextMode)
      try {
        const next = await getTransport().call<unknown>("set_workflow_mode", {
          sessionId: targetSessionId,
          mode: nextMode,
        })
        const saved = normalizeWorkflowMode(next)
        setWorkflowMode(saved)
        window.dispatchEvent(
          new CustomEvent(WORKFLOW_MODE_CHANGED_EVENT, {
            detail: { sessionId: targetSessionId, mode: saved },
          }),
        )
        toast.success(
          t("chat.workflowMode.saved", "工作流模式已切换为 {{mode}}，模型下一轮会感知", {
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
    [currentSessionId, incognitoEnabled, onEnsureSession, t, workflowMode, workflowModeSaving],
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
  const hasPendingQueue = loading && pendingQueueItems.length > 0

  const pendingStatusLabel = (item: PendingSendPreview) => {
    switch (item.status) {
      case "waiting_tool_boundary":
        return t("chat.pendingWaitingToolBoundary", "等待工具完成点")
      case "inserted":
        return t("chat.pendingInserted", "已插入")
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReply", "回复后发送")
      case "queued":
      default:
        return t("chat.pendingQueuedShort", "排队中")
    }
  }

  const pendingStatusTip = (item: PendingSendPreview) => {
    switch (item.status) {
      case "waiting_tool_boundary":
        return t(
          "chat.pendingWaitingToolBoundaryTip",
          "等待最近一次工具调用完成；如果本轮不再调用工具，将改为回复结束后发送。",
        )
      case "fallback_after_reply":
        return t("chat.pendingFallbackAfterReplyTip", "未遇到工具完成点，将在当前回复结束后发送。")
      case "inserted":
        return t("chat.pendingInsertedTip", "已在本轮工具完成后插入给模型。")
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
  // is compact. Knowledge + Goal + Workflow + Plan join at the narrower
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
      {sandboxCollapsed && (
        <SandboxModeSwitcher
          variant="menu"
          sandboxMode={sandboxMode}
          onSandboxModeChange={onSandboxModeChange}
        />
      )}
      {permissionCollapsed && (
        <PermissionModeSwitcher
          variant="menu"
          permissionMode={permissionMode}
          onPermissionModeChange={handlePermissionModeChange}
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
          onSelect={noteMention.applyEntry}
          onHover={noteMention.setSelectedIndex}
        />

        {/* File Mention Menu (`@` popper) */}
        <FileMentionMenu
          isOpen={mention.isOpen && !slash.isOpen && !noteMention.isOpen}
          entries={mention.entries}
          noteEntries={mention.noteEntries}
          notesLoading={mention.notesLoading}
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
        <AttachmentPreview attachedFiles={attachedFiles} onRemoveFile={onRemoveFile} />

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
                  const edit = () =>
                    item.id === "__legacy__" ? onCancelPending?.() : onEditPending?.(item.id)
                  const discard = () =>
                    item.id === "__legacy__"
                      ? onDiscardPending?.()
                      : onDiscardPendingItem?.(item.id)
                  const readonly = item.status === "inserted"
                  const canCancelForce =
                    item.mode === "force_insert" && item.status === "waiting_tool_boundary"
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
                      <span className="min-w-0 flex-1 truncate text-sm text-foreground/90">
                        {item.text}
                        {(item.attachmentCount > 0 || item.quoteCount > 0) && (
                          <span className="ml-1 text-xs text-muted-foreground">
                            +{item.attachmentCount + item.quoteCount}
                          </span>
                        )}
                      </span>
                      {canCancelForce ? (
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
                      )}
                      {!readonly && (
                        <>
                          <IconTip label={t("chat.pendingEdit")}>
                            <button
                              type="button"
                              className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
                              onClick={edit}
                            >
                              <Pencil className="h-3.5 w-3.5" />
                            </button>
                          </IconTip>
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

        {/* Active Goal status — always visible near the composer while a durable goal is open. */}
        <AnimatedCollapse open={!!activeGoal && !goalComposerMode}>
          {activeGoal ? (
            <div className="border-b border-emerald-500/15 bg-emerald-500/7 px-3 py-2 text-xs text-emerald-700 dark:text-emerald-300">
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
                <span className="shrink-0 rounded-full border border-emerald-500/20 bg-background/45 px-2 py-0.5 text-[11px]">
                  {activeGoalStateLabel}
                </span>
                <span className="shrink-0 rounded-full border border-emerald-500/20 bg-background/45 px-2 py-0.5 text-[11px]">
                  {activeGoalProgressLabel}
                </span>
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

        {/* Workflow Mode status — visible only when autonomous orchestration is enabled. */}
        <AnimatedCollapse open={workflowModeActive && !incognitoEnabled}>
          <div className="border-b border-blue-500/15 bg-blue-500/7 px-3 py-1.5 text-xs text-blue-700 dark:text-blue-300">
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
          <div className="space-y-1.5 border-b border-emerald-500/20 bg-emerald-500/10 px-3 py-1.5 text-xs text-emerald-700 animate-in fade-in slide-in-from-top-1 duration-200 dark:text-emerald-300">
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
                    [
                      "create_or_update",
                      t("chat.goalMode.actionUpdate", "更新目标"),
                    ],
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

        {/* Plan Mode Banner */}
        <AnimatedCollapse open={planState === "planning"}>
          <div
            className={`flex items-center gap-2 px-3 py-1.5 bg-blue-500/10 border-b border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs animate-in fade-in slide-in-from-top-1 duration-200${!hasVisibleTaskProgress && attachedFiles.length === 0 && !hasPendingQueue ? " rounded-t-2xl" : ""}`}
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
                className="flex min-w-0 flex-nowrap items-center gap-1 overflow-hidden"
              >
                <div
                  ref={addActionsRef}
                  className={toolbarCompact ? "hidden" : CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS}
                >
                  {renderInlineAddControls()}
                </div>

                <div
                  ref={overflowTriggerRef}
                  className={toolbarCompact ? "block shrink-0" : CHAT_INPUT_OVERFLOW_MENU_CLASS}
                >
                  <DropdownMenu.Root open={showOverflowMenu} onOpenChange={setShowOverflowMenu}>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <DropdownMenu.Trigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={t("chat.moreActions")}
                            className="h-8 w-8 rounded-lg bg-transparent text-muted-foreground hover:bg-transparent hover:text-foreground focus-visible:ring-0 data-[state=open]:bg-transparent"
                          >
                            <Plus className="h-4 w-4" />
                          </Button>
                        </DropdownMenu.Trigger>
                      </TooltipTrigger>
                      <TooltipContent>{t("chat.moreActions")}</TooltipContent>
                    </Tooltip>
                    <DropdownMenu.Portal>
                      <DropdownMenu.Content
                        className="z-50 min-w-[180px] overflow-hidden rounded-floating border border-border-soft bg-surface-floating/95 p-1.5 text-popover-foreground shadow-floating backdrop-blur-xl animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
                        side="top"
                        align="start"
                        sideOffset={8}
                      >
                        <div className="flex flex-col gap-0.5">{renderOverflowMenuItems()}</div>
                      </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                  </DropdownMenu.Root>
                </div>

                {/* Model / Think / Temperature */}
                <ModelPicker
                  availableModels={availableModels}
                  activeModel={activeModel}
                  reasoningEffort={reasoningEffort}
                  onModelChange={onModelChange}
                  onEffortChange={onEffortChange}
                  currentModelInfo={currentModelInfo}
                  sessionTemperature={sessionTemperature}
                  onSessionTemperatureChange={onSessionTemperatureChange}
                />

                <AwarenessToggle sessionId={currentSessionId ?? null} disabled={incognitoEnabled} />

                {/* Knowledge + Goal + Workflow + Plan — semantic mode controls,
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

                    <DropdownMenu.Root open={workflowMenuOpen} onOpenChange={setWorkflowMenuOpen}>
                      <IconTip label={workflowMenuLabel}>
                        <DropdownMenu.Trigger asChild>
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
                        </DropdownMenu.Trigger>
                      </IconTip>
                      <DropdownMenu.Portal>
                        <DropdownMenu.Content
                          className="z-50 min-w-[280px] overflow-hidden rounded-floating border border-border-soft bg-surface-floating/95 p-1.5 text-popover-foreground shadow-floating backdrop-blur-xl animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
                          side="top"
                          align="start"
                          sideOffset={8}
                        >
                          {renderWorkflowModeMenuItems(() => setWorkflowMenuOpen(false))}
                        </DropdownMenu.Content>
                      </DropdownMenu.Portal>
                    </DropdownMenu.Root>

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

                {/* Tool Permission Mode — collapses into the "+" menu last, at
                    the narrowest tier (kept inline longer than Sandbox). */}
                {!permissionCollapsed && (
                  <div ref={permissionModeRef} className="shrink-0">
                    <PermissionModeSwitcher
                      permissionMode={permissionMode}
                      onPermissionModeChange={handlePermissionModeChange}
                    />
                  </div>
                )}
                {!sandboxCollapsed && (
                  <div ref={sandboxModeRef} className="shrink-0">
                    <SandboxModeSwitcher
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
                  label={loading && hasSendableContent ? t("chat.queueMessage") : t("chat.send")}
                >
                  <Button
                    size="icon"
                    className="h-8 w-8 rounded-full shrink-0"
                    onClick={handleSend}
                    disabled={!hasSendableContent || goalSubmitting}
                    aria-label={
                      loading && hasSendableContent ? t("chat.queueMessage") : t("chat.send")
                    }
                  >
                    {goalSubmitting ? (
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
