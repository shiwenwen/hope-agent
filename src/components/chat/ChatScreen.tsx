import { useState, useRef, useEffect, useLayoutEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import type { SettingsSection } from "@/components/settings/types"
import { BrowserExtensionNudge } from "./BrowserExtensionNudge"
import { useViewportMediaQuery } from "@/hooks/useViewportMediaQuery"
import { cn } from "@/lib/utils"
import {
  Brain,
  ClipboardList,
  Eye,
  FolderOpen,
  GitCompare,
  Globe,
  Layers,
  LayoutDashboard,
  Monitor,
  MousePointer2,
  Users,
  type LucideIcon,
} from "lucide-react"
import type {
  ActiveModel,
  AvailableModel,
  Message,
  SessionMeta,
  SessionMode,
  SandboxMode,
} from "@/types/chat"
import type { QuickPromptAddResult, QuickPromptConfig, QuickPromptItem } from "@/types/quickPrompts"
import { normalizeEffortForModel } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import type { CommandResult } from "./slash-commands/types"
import {
  goalSlashCommandDisplay,
  isGoalUpsertSlashCommand,
  parseGoalObjectiveAndCriteria,
  parseGoalUpsertSlashCommand,
} from "./goalSlashCommand"
import {
  isLoopCreateSlashCommand,
  loopSlashCommandDisplay,
  parseLoopCreateSlashCommand,
} from "./loopSlashCommand"
import type { AgentConfig } from "@/components/settings/types"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import ChatSidebar from "@/components/chat/ChatSidebar"
import ChatInput from "@/components/chat/ChatInput"
import { FileBrowserPanel } from "@/components/chat/FileBrowserPanel"
import type { QuotePayload } from "@/components/chat/project/file-browser/FilePreviewPane"
import type { IncognitoDisabledReason } from "@/components/chat/input/IncognitoToggle"
import ChatTitleBar from "@/components/chat/ChatTitleBar"
import HandoverDialog from "@/components/chat/HandoverDialog"
import MessageList from "@/components/chat/MessageList"
import { ChatWelcomeHero } from "@/components/chat/ChatWelcomeHero"
import CrashRecoveryBanner from "@/components/common/CrashRecoveryBanner"
import CanvasPanel from "@/components/chat/CanvasPanel"
import BrowserPanel from "@/components/chat/BrowserPanel"
import MacControlPanel from "@/components/chat/MacControlPanel"
import { TeamPanel } from "@/components/team/TeamPanel"
import TeamMiniIndicator from "@/components/team/TeamMiniIndicator"
import { useActiveTeam } from "@/components/team/useTeam"
import SessionSearchBar from "@/components/chat/SessionSearchBar"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/components/ui/alert-dialog"
import { useChatSession } from "./useChatSession"
import { useChatStream } from "./useChatStream"
import { useChatStreamReattach } from "./hooks/useChatStreamReattach"
import { usePlanMode } from "./plan-mode/usePlanMode"
import { useTaskProgressSnapshot } from "./tasks/useTaskProgressSnapshot"
import { computeContextUsage, formatContextUsage } from "./chatUtils"
import { recentUserInputHistory } from "./quick-prompts/messageQuickPrompts"
import {
  COMPACT_CONTEXT_UPDATED_EVENT,
  type CompactResult,
  compactContextNow,
  compactResultMessage,
  resolveCurrentModel,
  type CompactContextUpdatedDetail,
} from "./sessionStatus"
import {
  contextCompactionData,
  isContextCompactionPayload,
  isContextCompactionStartPayload,
  parseEventPayload,
  shouldReplaceContextCompactionNotice,
} from "./contextCompactionEvents"
import { useDiffPanel } from "./diff-panel/useDiffPanel"
import { DiffPanel } from "./diff-panel/DiffPanel"
import { useFilePreview } from "./files/useFilePreview"
import FilePreviewPanel from "./files/FilePreviewPanel"
import { FileActionsContext, type FileActionsContextValue } from "./files/fileActionsContext"
import WorkspacePanel from "./workspace/WorkspacePanel"
import BackgroundJobsPanel from "./background-jobs/BackgroundJobsPanel"
import { decideBackgroundJobsAutoOpen } from "./background-jobs/autoOpenPolicy"
import { useBackgroundJobs } from "./background-jobs/useBackgroundJobs"
import { resolveWorkspaceTaskExecutionState } from "./workspace/taskExecutionState"
import { messagesHaveFileActivity } from "./workspace/useSessionFileChanges"
import { messagesHaveUrlActivity } from "./workspace/useSessionUrlSources"
import { messagesHaveKnowledgeActivity } from "./workspace/useSessionKnowledge"
import { useWorkflowRuns, type WorkflowRun } from "./workspace/useWorkflowRuns"
import { useGoal, type GoalSnapshot } from "./workspace/useGoal"
import { messagesHaveBrowserActivity } from "./workspace/useSessionBrowserActivity"
import SubagentSessionDialog from "./SubagentSessionDialog"
import { useModelState } from "./hooks/useModelState"
import SystemPromptDialog from "./SystemPromptDialog"
import { PlanPanel } from "./plan-mode/PlanPanel"
import type { BuiltPlanComment } from "./plan-mode/planCommentMessage"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { useProjects } from "./project/hooks/useProjects"
import ProjectDialog from "./project/ProjectDialog"
import ProjectOverviewDialog from "./project/ProjectOverviewDialog"
import {
  CHAT_DISPLAY_MODE_EVENT,
  normalizeChatDisplayMode,
  readChatDisplayModePreference,
  writeChatDisplayModePreference,
} from "./chatDisplayModePreference"
import {
  COMPLETED_TURN_COLLAPSE_EVENT,
  normalizeCompletedTurnCollapsePreference,
} from "./completedTurnCollapsePreference"
import {
  CHAT_SIDEBAR_DEFAULT_WIDTH,
  CHAT_SIDEBAR_LEGACY_DEFAULT_WIDTH,
  CHAT_SIDEBAR_MAX_WIDTH,
  CHAT_SIDEBAR_MIN_WIDTH,
  CHAT_SIDEBAR_WIDTH_STORAGE_KEY,
} from "./sidebar/types"
import { generateClientId } from "./chatScrollKeys"
import type { Project, ProjectMeta } from "@/types/project"
import type { KbDraftAttachment } from "@/types/knowledge"

function appendGoalCriterionLine(
  existingCriteria: string | null | undefined,
  text: string,
  kind: "required" | "optional",
): string {
  const prefix = kind === "optional" ? "[optional]" : "[required]"
  const current = (existingCriteria ?? "").trim()
  const nextLine = `${prefix} ${text.trim()}`
  return current ? `${current}\n${nextLine}` : nextLine
}

/** A token to append to the chat composer on next render. `attachKbId` (set by the
 *  KnowledgeView "reference in chat" action) is auto-attached read-only so the
 *  `[[note]]` injection isn't dropped by `effective_kb_access` at send time. */
export interface ChatInsert {
  token: string
  attachKbId?: string
}

type SwitchSessionOptions = { targetMessageId?: number; highlightTerms?: string[] }

type IncognitoLeaveIntent =
  | { type: "switchSession"; sessionId: string; opts?: SwitchSessionOptions }
  | { type: "newChat"; agentId: string; opts?: { incognito?: boolean } }
  | { type: "newProjectChat"; projectId: string; defaultAgentId?: string | null }

interface ChatScreenProps {
  onOpenAgentSettings?: (agentId: string) => void
  onCodexReauth?: () => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
  sessionsRefreshTrigger?: number
  onCurrentProjectChange?: (projectId: string | null) => void
  /** Token to append to the chat input on next render (e.g. `@plan:abcd:v0` or a
   *  `[[note]]` ref). */
  pendingChatInsert?: ChatInsert
  /** Called once the insert has been consumed so App can clear the pending slot. */
  onChatInsertConsumed?: () => void
  /** Open the settings view, optionally to a specific section. */
  onOpenSettings?: (section?: SettingsSection) => void
}

interface ManualCompactOverride {
  sessionId: string
  tokensAfter: number
  usageFingerprint: string | null
}

const WORKFLOW_MODE_CHANGED_EVENT = "hope-agent:workflow-mode-changed"

function latestAssistantUsageFingerprint(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i]
    if (msg.role !== "assistant" || !msg.usage) continue
    return JSON.stringify({
      dbId: msg.dbId ?? null,
      timestamp: msg.timestamp ?? null,
      usage: msg.usage,
    })
  }
  return null
}

type ExclusiveRightPanel =
  | "workspace"
  | "diff"
  | "plan"
  | "files"
  | "browser"
  | "mac-control"
  | "canvas"
  | "team"
  | "background-jobs"
  | "preview"
type ExclusiveRightPanelVisibility = Record<ExclusiveRightPanel, boolean>

const EXCLUSIVE_RIGHT_PANEL_ORDER: readonly ExclusiveRightPanel[] = [
  "diff",
  "plan",
  "files",
  "browser",
  "mac-control",
  "canvas",
  "team",
  "background-jobs",
  "workspace",
  "preview",
]

const EMPTY_RIGHT_PANEL_VISIBILITY: ExclusiveRightPanelVisibility = {
  workspace: false,
  diff: false,
  plan: false,
  files: false,
  browser: false,
  "mac-control": false,
  canvas: false,
  team: false,
  "background-jobs": false,
  preview: false,
}

const EXCLUSIVE_RIGHT_PANEL_ICONS: Record<ExclusiveRightPanel, LucideIcon> = {
  workspace: LayoutDashboard,
  diff: GitCompare,
  plan: ClipboardList,
  files: FolderOpen,
  browser: Globe,
  "mac-control": MousePointer2,
  canvas: Monitor,
  team: Users,
  "background-jobs": Layers,
  preview: Eye,
}

const DEFAULT_RIGHT_PANEL_WIDTH = 520
const CHAT_MAIN_MIN_INTERACTIVE_WIDTH = 420
const CHAT_MAIN_COMPACT_MIN_INTERACTIVE_WIDTH = 320
const RIGHT_PANEL_AUTO_COLLAPSE_MIN_WIDTH = 360
const RIGHT_PANEL_AUTO_COLLAPSE_MAX_WIDTH = 640
const SIDEBAR_AUTO_COLLAPSE_GUTTER = 180
const RESPONSIVE_PANEL_HYSTERESIS = 120

interface MacControlFrameOpenHint {
  mediaId?: string | null
  path?: string | null
}

function clampChatSidebarWidth(width: number): number {
  return Math.min(CHAT_SIDEBAR_MAX_WIDTH, Math.max(CHAT_SIDEBAR_MIN_WIDTH, width))
}

function clampResponsiveRightPanelWidth(width: number): number {
  return Math.round(
    Math.min(
      RIGHT_PANEL_AUTO_COLLAPSE_MAX_WIDTH,
      Math.max(RIGHT_PANEL_AUTO_COLLAPSE_MIN_WIDTH, width),
    ),
  )
}

function isSessionMode(value: unknown): value is SessionMode {
  return value === "default" || value === "smart" || value === "yolo"
}

function isSandboxMode(value: unknown): value is SandboxMode {
  return (
    value === "off" ||
    value === "standard" ||
    value === "isolated" ||
    value === "workspace" ||
    value === "trusted"
  )
}

function readActionString(action: object, camelKey: string, snakeKey: string): string | null {
  const record = action as Record<string, unknown>
  const value = record[camelKey] ?? record[snakeKey]
  return typeof value === "string" && value.length > 0 ? value : null
}

type ClientEventMessage = Omit<Message, "role" | "_clientId">

function makeClientEventMessage(message: ClientEventMessage): Message {
  return {
    role: "event",
    _clientId: generateClientId(),
    ...message,
  }
}

function isSameSlashHistoryMessage(a: Message, b: Message): boolean {
  return (
    a.slashEvent?.kind === b.slashEvent?.kind &&
    a.slashEvent?.command === b.slashEvent?.command &&
    a.slashEvent?.displayAs === b.slashEvent?.displayAs &&
    a.slashEvent?.mode === b.slashEvent?.mode &&
    a.content === b.content
  )
}

function appendUniqueSlashHistoryMessages(prev: Message[], additions: Message[]): Message[] {
  if (additions.length === 0) return prev
  const next = [...prev]
  for (const message of additions) {
    if (
      !message.slashEvent ||
      !next.some((existing) => isSameSlashHistoryMessage(existing, message))
    ) {
      next.push(message)
    }
  }
  return next
}

function goalTurnPrompt(visibleGoalText: string): string {
  return [
    "[SYSTEM: The user has just created or updated the durable Goal for this session.",
    "Treat the Active Goal system section as the source of truth, acknowledge briefly, then begin making progress.",
    "Do not expose internal goal ids, revision ids, or slash-command help unless the user asks for status details.]",
    "",
    visibleGoalText,
  ].join("\n")
}

function slashCommandDisplay(commandText: string): {
  content: string
  mode?: "goal" | "loop"
} {
  if (/^\/loop(?=\s|$)/i.test(commandText.trim())) {
    return loopSlashCommandDisplay(commandText)
  }
  return goalSlashCommandDisplay(commandText)
}

type BrowserExtensionRequiredPayload = {
  requirement?: string
  reason?: string
  statusKind?: string
  statusMessage?: string
  nextAction?: string
  sessionId?: string
  source?: string
}

function makeCompactProgressEvent(): Record<string, unknown> {
  return {
    type: "context_compaction_progress",
    data: {
      phase: "preparing",
      kind: "summary",
    },
  }
}

function makeCompactResultEvent(result: CompactResult): Record<string, unknown> {
  return {
    type: "context_compacted",
    data: {
      tier_applied: result.tierApplied,
      tokens_before: result.tokensBefore,
      tokens_after: result.tokensAfter,
      messages_affected: result.messagesAffected,
      description: result.description ?? "no_action_needed",
      kind: result.tierApplied >= 4 ? "emergency" : "summary",
    },
  }
}

function makeCompactFailedEvent(): Record<string, unknown> {
  return {
    type: "context_compaction_progress",
    data: {
      phase: "failed",
      kind: "summary",
    },
  }
}

function makeCompactNoticeMessage(event: Record<string, unknown>, clientId?: string): Message {
  return {
    role: "event",
    content: JSON.stringify(event),
    timestamp: new Date().toISOString(),
    _clientId: clientId ?? generateClientId(),
  }
}

function compactNoticePayload(message: Message): Record<string, unknown> | null {
  if (message.role !== "event") return null
  const payload = parseEventPayload(message.content)
  return isContextCompactionPayload(payload) ? payload : null
}

function isLiveContextCompactionNotice(message: Message): boolean {
  const payload = compactNoticePayload(message)
  if (!payload) return false
  return payload.type === "context_compaction_progress" || isContextCompactionStartPayload(payload)
}

function latestLiveCompactNoticeIndex(messages: Message[]): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (isLiveContextCompactionNotice(messages[i])) return i
  }
  return -1
}

function latestCompactNoticeIndex(messages: Message[]): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (compactNoticePayload(messages[i])) return i
  }
  return -1
}

function upsertManualCompactNotice(
  messages: Message[],
  event: Record<string, unknown>,
  clientId: string,
): Message[] {
  const notice = makeCompactNoticeMessage(event, clientId)
  const sameClientIdx = messages.findIndex((message) => message._clientId === clientId)
  if (sameClientIdx >= 0) {
    const next = [...messages]
    next[sameClientIdx] = notice
    return next
  }

  if (event.type === "context_compaction_progress") {
    const data = contextCompactionData(event)
    if (data.phase !== "failed") return [...messages, notice]
  }

  const liveIdx = latestLiveCompactNoticeIndex(messages)
  if (liveIdx >= 0) {
    const next = [...messages]
    next[liveIdx] = notice
    return next
  }

  const noticeIdx = latestCompactNoticeIndex(messages)
  if (noticeIdx >= 0) {
    const previousPayload = compactNoticePayload(messages[noticeIdx])
    if (previousPayload && shouldReplaceContextCompactionNotice(previousPayload, event)) {
      const next = [...messages]
      next[noticeIdx] = notice
      return next
    }
  }

  return [...messages, notice]
}

export default function ChatScreen({
  onOpenAgentSettings,
  onCodexReauth,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  onOpenDashboardTab,
  sessionsRefreshTrigger,
  onCurrentProjectChange,
  pendingChatInsert,
  onChatInsertConsumed,
  onOpenSettings,
}: ChatScreenProps) {
  const { t } = useTranslation()

  // ── Model State ─────────────────────────────────────────────
  const {
    availableModels,
    setAvailableModels,
    activeModel,
    setActiveModel,
    reasoningEffort,
    setReasoningEffort,
    sessionTemperature,
    setSessionTemperature,
    globalActiveModelRef,
    applyModelForDisplay,
    handleModelChange,
    handleEffortChange,
  } = useModelState()

  // Sidebar panel width
  const [panelWidth, setPanelWidth] = useState(() => {
    if (typeof window === "undefined") return CHAT_SIDEBAR_DEFAULT_WIDTH
    const stored = window.localStorage.getItem(CHAT_SIDEBAR_WIDTH_STORAGE_KEY)
    if (!stored) return CHAT_SIDEBAR_DEFAULT_WIDTH

    const storedWidth = Number(stored)
    if (storedWidth === CHAT_SIDEBAR_LEGACY_DEFAULT_WIDTH) return CHAT_SIDEBAR_DEFAULT_WIDTH

    return Number.isFinite(storedWidth)
      ? clampChatSidebarWidth(storedWidth)
      : CHAT_SIDEBAR_DEFAULT_WIDTH
  })
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    if (typeof window === "undefined") return false
    return window.localStorage.getItem("hope.chatSidebarCollapsed") === "true"
  })
  const autoCollapsedSidebarRef = useRef(false)
  const manualSidebarExpandedOverrideRef = useRef(false)
  const userSidebarCollapsedPreferenceRef = useRef(sidebarCollapsed)

  useEffect(() => {
    if (typeof window === "undefined") return
    window.localStorage.setItem(CHAT_SIDEBAR_WIDTH_STORAGE_KEY, String(panelWidth))
  }, [panelWidth])

  useEffect(() => {
    if (typeof window === "undefined") return
    if (autoCollapsedSidebarRef.current) return
    window.localStorage.setItem("hope.chatSidebarCollapsed", String(sidebarCollapsed))
  }, [sidebarCollapsed])

  const handleSidebarCollapsedChange = useCallback((collapsed: boolean) => {
    autoCollapsedSidebarRef.current = false
    manualSidebarExpandedOverrideRef.current = !collapsed
    userSidebarCollapsedPreferenceRef.current = collapsed
    setSidebarCollapsed(collapsed)
  }, [])

  const [defaultDisplayMode, setDefaultDisplayMode] = useState(() =>
    readChatDisplayModePreference(),
  )
  const [autoCollapseCompletedTurns, setAutoCollapseCompletedTurns] = useState(true)
  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<{ chatDisplayMode?: unknown; autoCollapseCompletedTurns?: unknown }>("get_user_config")
      .then((cfg) => {
        const mode = normalizeChatDisplayMode(cfg.chatDisplayMode)
        if (cancelled) return
        setAutoCollapseCompletedTurns(
          normalizeCompletedTurnCollapsePreference(cfg.autoCollapseCompletedTurns),
        )
        if (mode) {
          setDefaultDisplayMode(mode)
          writeChatDisplayModePreference(mode, { emit: false })
        }
      })
      .catch((e: unknown) =>
        logger.warn(
          "settings",
          "ChatScreen::loadDisplayMode",
          "Failed to load chat display mode",
          e,
        ),
      )

    const handlePreferenceChange = (event: Event) => {
      const mode = normalizeChatDisplayMode((event as CustomEvent).detail?.mode)
      if (mode) setDefaultDisplayMode(mode)
    }
    const handleCompletedTurnCollapseChange = (event: Event) => {
      setAutoCollapseCompletedTurns(
        normalizeCompletedTurnCollapsePreference((event as CustomEvent).detail?.enabled),
      )
    }
    window.addEventListener(CHAT_DISPLAY_MODE_EVENT, handlePreferenceChange)
    window.addEventListener(COMPLETED_TURN_COLLAPSE_EVENT, handleCompletedTurnCollapseChange)
    return () => {
      cancelled = true
      window.removeEventListener(CHAT_DISPLAY_MODE_EVENT, handlePreferenceChange)
      window.removeEventListener(COMPLETED_TURN_COLLAPSE_EVENT, handleCompletedTurnCollapseChange)
    }
  }, [])

  // Right panel width (shared by all switchable right panels)
  const [rightPanelWidth, setRightPanelWidth] = useState(DEFAULT_RIGHT_PANEL_WIDTH)
  const [canvasPanelOpen, setCanvasPanelOpen] = useState(false)

  // Right side diff panel (write/edit/apply_patch metadata viewer)
  const diffPanel = useDiffPanel()

  // Right side file-preview panel (Markdown links / attachments / workspace
  // files → in-app preview). Opened via `onPreviewFile` from the message tree.
  const filePreview = useFilePreview()
  // Fullscreen toggle for the right-side preview panel (its RightPanelShell is
  // owned here, unlike files/canvas which own their own). Reset whenever the
  // preview isn't actively shown so it never reopens stuck-maximized.
  const [filePreviewMaximized, setFilePreviewMaximized] = useState(false)
  useEffect(() => {
    if (!filePreview.showPanel || !filePreview.target) setFilePreviewMaximized(false)
  }, [filePreview.showPanel, filePreview.target])

  // Workspace 面板：聚合任务进度 / 碰到的文件 / 引用来源。首次有内容时自动
  // 展开一次，用户关闭后本会话不再自动弹（dismissedRef 跟踪，仿 browser 面板）。
  const [showWorkspacePanel, setShowWorkspacePanel] = useState(false)
  const workspacePanelDismissedRef = useRef(false)
  const preserveWorkspaceOnSessionSwitchRef = useRef(false)

  // R4 背景任务：会话级在跑/最近作业 + 本地模型任务镜像。新后台任务出现时
  // 自动打开一次；用户关闭后本会话不再抢回焦点。订阅在 ChatScreen 级常驻
  //（见 `session` 定义后的 useBackgroundJobs），喂头部徽标计数 + 面板 + 工作台区块。
  const [showBackgroundJobsPanel, setShowBackgroundJobsPanel] = useState(false)
  const backgroundJobsPanelDismissedRef = useRef(false)
  const suppressNextBackgroundJobsActivationRef = useRef(false)
  const previousBackgroundRunningCountRef = useRef(0)
  const [backgroundJobExpansionOverrides, setBackgroundJobExpansionOverrides] = useState<
    Record<string, boolean>
  >({})

  // Browser live-mirror panel. Auto-opens on the **first** `browser:frame`
  // push of a session. After the user manually closes it, further frames in
  // the same session never re-pop the panel — `browserPanelDismissedRef`
  // tracks the dismissal until a session switch resets it.
  const [showBrowserPanel, setShowBrowserPanel] = useState(false)
  const browserPanelDismissedRef = useRef(false)
  const [showFilesPanel, setShowFilesPanel] = useState(false)
  // Clicking a staged quote chip reveals that file in the browser. The nonce
  // makes each click a fresh signal, even when re-revealing the same path.
  const revealQuoteNonce = useRef(0)
  const [revealFile, setRevealFile] = useState<{
    path: string
    name: string
    startLine: number
    endLine: number
    nonce: number
  } | null>(null)
  const [showMacControlPanel, setShowMacControlPanel] = useState(false)
  const macControlPanelDismissedRef = useRef(false)

  // Context compact state
  const [compacting, setCompacting] = useState(false)
  const compactingRef = useRef(false)
  const [manualCompactOverride, setManualCompactOverride] = useState<ManualCompactOverride | null>(
    null,
  )
  useEffect(() => {
    compactingRef.current = compacting
  }, [compacting])

  // In-session "find in page" search bar state
  const [searchBarOpen, setSearchBarOpen] = useState(false)
  const [searchFocusSignal, setSearchFocusSignal] = useState(0)
  const [handoverSessionId, setHandoverSessionId] = useState<string | null>(null)
  const [subagentPreviewSessionId, setSubagentPreviewSessionId] = useState<string | null>(null)
  const openSessionSearch = useCallback(() => {
    setSearchBarOpen(true)
    setSearchFocusSignal((n) => n + 1)
  }, [])

  // Sidebar global-history search focus tick (Cmd+Shift+F).
  const [globalSearchFocusSignal, setGlobalSearchFocusSignal] = useState(0)
  const focusGlobalSearch = useCallback(() => {
    setGlobalSearchFocusSignal((n) => n + 1)
  }, [])

  // System prompt viewer state
  const [showSystemPrompt, setShowSystemPrompt] = useState(false)
  const [systemPromptContent, setSystemPromptContent] = useState("")
  const [systemPromptLoading, setSystemPromptLoading] = useState(false)
  const [draftIncognito, setDraftIncognito] = useState(false)
  // Draft working dir picked before a session exists. Materialized into the new
  // session by the backend `chat` command on first send, then cleared via the
  // `currentSessionId` transition effect below.
  const [draftWorkingDir, setDraftWorkingDir] = useState<string | null>(null)
  const [workingDirSaving, setWorkingDirSaving] = useState(false)
  // KB attaches staged before a session exists. Replayed onto the new session by
  // useChatStream when `session_created` lands, then cleared via the
  // `currentSessionId` transition effect below (mirrors draftWorkingDir).
  const [draftKbAttachments, setDraftKbAttachments] = useState<KbDraftAttachment[]>([])
  // Project bound to a not-yet-materialized chat (lazy project session). Mirrors
  // draftWorkingDir/draftKbAttachments: set when entering a project draft, ridden
  // into the new session via the `chat` command's `projectId` on first send, then
  // cleared once the real session meta catches up (see the transition effect).
  const [draftProjectId, setDraftProjectId] = useState<string | null>(null)

  // Plan mode state (declared early so useChatStream can access it)
  const [planModeState, setPlanModeState] = useState<
    "off" | "planning" | "review" | "executing" | "completed"
  >("off")

  // Shared stream identity state for dedup across the primary per-call
  // Channel/WS path (useChatStream) and the EventBus reattach path
  // (useChatStreamReattach). Cursors are keyed by session + stream id so a
  // delayed frame from a finished stream cannot mutate the next DB snapshot.
  const streamSeqRef = useRef<Map<string, number>>(new Map())
  const endedStreamIdsRef = useRef<Map<string, string>>(new Map())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

  // ── Projects ────────────────────────────────────────────────
  // Holds the currently-open session id for the project-unread rollup. Lives
  // above the session hook (which feeds this hook's reload callback), so the
  // value is delivered by ref and refreshed via the effect below on switch.
  const activeSessionIdForProjectsRef = useRef<string | null>(initialSessionId ?? null)
  const {
    projects,
    createProject,
    updateProject,
    deleteProject,
    archiveProject,
    reorderProjects,
    moveSessionToProject,
    reloadProjects,
    initialLoading: projectsInitialLoading,
  } = useProjects({
    includeArchived: true,
    activeSessionIdRef: activeSessionIdForProjectsRef,
  })

  const refreshProjectAggregates = useCallback(() => {
    void reloadProjects()
  }, [reloadProjects])

  // ── Session Hook ────────────────────────────────────────────
  const session = useChatSession({
    availableModels,
    setActiveModel,
    globalActiveModelRef,
    handleModelChange,
    applyModelForDisplay,
    initialSessionId,
    onSessionNavigated,
    onUnreadCountChange,
    onSidebarAggregatesChanged: refreshProjectAggregates,
  })

  // R4: live background-jobs subscription (see show-state above) — drives the
  // header badge count, the background-jobs panel, and the workspace section.
  const backgroundJobs = useBackgroundJobs(session.currentSessionId)

  const isCronSession = useMemo(
    () => session.sessions.find((s) => s.id === session.currentSessionId)?.isCron ?? false,
    [session.sessions, session.currentSessionId],
  )
  const isSubagentSession = useMemo(
    () => !!session.sessions.find((s) => s.id === session.currentSessionId)?.parentSessionId,
    [session.sessions, session.currentSessionId],
  )
  const currentSessionMeta = useMemo(
    () =>
      session.currentSessionId
        ? (session.sessions.find((s) => s.id === session.currentSessionId) ?? null)
        : null,
    [session.sessions, session.currentSessionId],
  )
  const incognitoEnabled = session.currentSessionId
    ? (currentSessionMeta?.incognito ?? false)
    : draftIncognito
  // Single source for "which project is this chat in" across draft + materialized
  // states. Prefer the loaded session meta the moment it exists (so switching to a
  // plain session never leaks a stale draft binding); fall back to draftProjectId
  // only while the meta is absent — covers both the pure draft and the brief
  // post-materialization window before the sessions list reloads (no badge flicker).
  const effectiveProjectId = currentSessionMeta
    ? (currentSessionMeta.projectId ?? null)
    : draftProjectId
  const incognitoDisabledReason: IncognitoDisabledReason | undefined = effectiveProjectId
    ? "project"
    : currentSessionMeta?.channelInfo
      ? "channel"
      : undefined
  const reloadSessions = session.reloadSessions
  const currentAgentId = session.currentAgentId
  const handleNewChat = session.handleNewChat
  const currentSessionId = session.currentSessionId
  const chatGoal = useGoal(currentSessionId, { incognito: incognitoEnabled })
  const displayMode = defaultDisplayMode
  const setAgentName = session.setAgentName
  const updateSessionMeta = session.updateSessionMeta
  const rawHandleSwitchSession = session.handleSwitchSession
  const latestMessagesRef = useRef<Message[]>(session.messages)
  const incognitoComposerStateRef = useRef({
    input: "",
    attachedFileCount: 0,
    pendingQuoteCount: 0,
    pendingMessage: "",
    pendingSendCount: 0,
  })
  const clearDisposableIncognitoDraftRef = useRef<() => void>(() => {})
  const confirmedIncognitoLeaveSessionIdsRef = useRef<Set<string>>(new Set())
  const [quickPrompts, setQuickPrompts] = useState<QuickPromptItem[]>([])
  const [incognitoLeaveIntent, setIncognitoLeaveIntent] = useState<IncognitoLeaveIntent | null>(
    null,
  )

  useEffect(() => {
    latestMessagesRef.current = session.messages
  }, [session.messages])

  const inputHistory = useMemo(() => recentUserInputHistory(session.messages), [session.messages])

  const reloadQuickPrompts = useCallback(async () => {
    try {
      const config = await getTransport().call<QuickPromptConfig>("get_quick_prompt_config")
      setQuickPrompts(config.items ?? [])
    } catch (e) {
      logger.error("chat", "ChatScreen::reloadQuickPrompts", "Failed to load quick prompts", e)
    }
  }, [])

  useEffect(() => {
    void reloadQuickPrompts()
  }, [reloadQuickPrompts])

  const handleAddQuickPrompt = useCallback(
    async (content: string) => {
      if (incognitoEnabled) return
      try {
        const result = await getTransport().call<QuickPromptAddResult>("add_quick_prompt", {
          content,
        })
        setQuickPrompts((prev) => {
          if (result.duplicate) {
            return prev.some((item) => item.id === result.item.id) ? prev : [result.item, ...prev]
          }
          return [result.item, ...prev.filter((item) => item.id !== result.item.id)]
        })
        toast.success(
          result.duplicate ? t("chat.quickPrompts.duplicate") : t("chat.quickPrompts.added"),
        )
      } catch (e) {
        logger.error("chat", "ChatScreen::addQuickPrompt", "Failed to add quick prompt", e)
        toast.error(t("chat.quickPrompts.addFailed"))
      }
    },
    [incognitoEnabled, t],
  )

  // Keep the project-unread rollup's active-session exclusion in sync: when the
  // user switches sessions, refresh projects so the newly-active session drops
  // out of its project's badge (and the previously-active one reappears).
  useEffect(() => {
    activeSessionIdForProjectsRef.current = currentSessionId ?? null
    void reloadProjects()
  }, [currentSessionId, reloadProjects])

  useEffect(() => {
    const handleManualCompactUsage = (event: Event) => {
      const detail = (event as CustomEvent<CompactContextUpdatedDetail>).detail
      if (!detail?.sessionId || !detail.result) return
      // Single most-recent override (not a per-session map): the post-compaction
      // number is a transient correction for the session being compacted, so it
      // never needs to accumulate across sessions or outlive the next compaction.
      setManualCompactOverride({
        sessionId: detail.sessionId,
        tokensAfter: detail.result.tokensAfter,
        usageFingerprint: latestAssistantUsageFingerprint(latestMessagesRef.current),
      })
    }

    window.addEventListener(COMPACT_CONTEXT_UPDATED_EVENT, handleManualCompactUsage)
    return () => window.removeEventListener(COMPACT_CONTEXT_UPDATED_EVENT, handleManualCompactUsage)
  }, [])

  // Ambient file-action wiring for the message tree (preview opener + session).
  const fileActionsValue = useMemo<FileActionsContextValue>(
    () => ({ sessionId: currentSessionId, onPreviewFile: filePreview.openPreview }),
    [currentSessionId, filePreview.openPreview],
  )

  const handleSessionEffortChange = useCallback(
    async (effort: string) => {
      const sid = session.currentSessionId
      if (sid) {
        updateSessionMeta(sid, (prev) =>
          prev.reasoningEffort === effort ? prev : { ...prev, reasoningEffort: effort },
        )
      }
      await handleEffortChange(effort, sid, session.currentAgentId)
    },
    [handleEffortChange, session.currentAgentId, session.currentSessionId, updateSessionMeta],
  )

  const hasDisposableIncognitoContent = useCallback(() => {
    const composer = incognitoComposerStateRef.current
    return (
      session.loading ||
      session.messages.length > 0 ||
      composer.input.trim().length > 0 ||
      composer.pendingMessage.trim().length > 0 ||
      composer.attachedFileCount > 0 ||
      composer.pendingQuoteCount > 0 ||
      composer.pendingSendCount > 0
    )
  }, [session.loading, session.messages.length])

  const requestIncognitoLeaveConfirmation = useCallback(
    (intent: IncognitoLeaveIntent) => {
      if (!incognitoEnabled || !hasDisposableIncognitoContent()) return false
      const sessionId = session.currentSessionId
      if (sessionId && confirmedIncognitoLeaveSessionIdsRef.current.has(sessionId)) {
        return false
      }
      setIncognitoLeaveIntent(intent)
      return true
    },
    [hasDisposableIncognitoContent, incognitoEnabled, session.currentSessionId],
  )

  // Enter a project draft (lazy project session): no DB row yet — resolve the
  // project's agent for display, reset draft state, and remember `draftProjectId`.
  // The session materializes inside the project on first send via the `chat`
  // command's `projectId`. Project + incognito are mutually exclusive, so
  // incognito is forced off here (and coerced server-side).
  const startNewChatInProjectNow = useCallback(
    async (projectId: string, defaultAgentId?: string | null) => {
      const project = projects.find((p) => p.id === projectId)
      let agentId = (defaultAgentId && defaultAgentId.trim()) || project?.defaultAgentId || null
      if (!agentId) {
        agentId =
          (await getTransport()
            .call<string | null>("get_default_agent_id")
            .catch(() => null)) || DEFAULT_AGENT_ID
      }
      setDraftIncognito(false)
      setDraftKbAttachments([])
      setDraftWorkingDir(null)
      setDraftProjectId(projectId)
      await handleNewChat(agentId)
    },
    [projects, handleNewChat],
  )

  const startNewChatNow = useCallback(
    async (agentId: string, opts?: { incognito?: boolean }) => {
      setDraftIncognito(opts?.incognito ?? false)
      setDraftKbAttachments([])
      // Leaving any project draft → drop the project / working-dir binding so it
      // can't leak into this plain draft (the currentSessionId transition effect
      // only fires on draft→materialized, not draft→draft).
      setDraftWorkingDir(null)
      setDraftProjectId(null)
      await handleNewChat(agentId)
    },
    [handleNewChat],
  )

  const runIncognitoLeaveIntent = useCallback(
    async (intent: IncognitoLeaveIntent) => {
      switch (intent.type) {
        case "switchSession":
          await rawHandleSwitchSession(intent.sessionId, intent.opts)
          break
        case "newChat":
          await startNewChatNow(intent.agentId, intent.opts)
          break
        case "newProjectChat":
          await startNewChatInProjectNow(intent.projectId, intent.defaultAgentId)
          break
      }
    },
    [rawHandleSwitchSession, startNewChatInProjectNow, startNewChatNow],
  )

  const handleConfirmIncognitoLeave = useCallback(() => {
    const intent = incognitoLeaveIntent
    if (!intent) return
    const sessionId = session.currentSessionId
    clearDisposableIncognitoDraftRef.current()
    if (sessionId) confirmedIncognitoLeaveSessionIdsRef.current.add(sessionId)
    setIncognitoLeaveIntent(null)
    void runIncognitoLeaveIntent(intent)
  }, [incognitoLeaveIntent, runIncognitoLeaveIntent, session.currentSessionId])

  const handleSwitchSession = useCallback(
    async (sessionId: string, opts?: SwitchSessionOptions) => {
      if (!sessionId) return
      if (sessionId === session.currentSessionId) {
        await rawHandleSwitchSession(sessionId, opts)
        return
      }
      if (requestIncognitoLeaveConfirmation({ type: "switchSession", sessionId, opts })) return
      await rawHandleSwitchSession(sessionId, opts)
    },
    [rawHandleSwitchSession, requestIncognitoLeaveConfirmation, session.currentSessionId],
  )

  const handleNewChatInProject = useCallback(
    async (projectId: string, defaultAgentId?: string | null) => {
      if (
        requestIncognitoLeaveConfirmation({
          type: "newProjectChat",
          projectId,
          defaultAgentId,
        })
      ) {
        return
      }
      await startNewChatInProjectNow(projectId, defaultAgentId)
    },
    [requestIncognitoLeaveConfirmation, startNewChatInProjectNow],
  )

  const handleStartNewChat = useCallback(
    async (agentId: string, opts?: { incognito?: boolean }) => {
      if (requestIncognitoLeaveConfirmation({ type: "newChat", agentId, opts })) return
      await startNewChatNow(agentId, opts)
    },
    [requestIncognitoLeaveConfirmation, startNewChatNow],
  )

  const handleStartNewChatFromCurrentContext = useCallback(async () => {
    // Cmd+N from a project (materialized or draft) stays in that project.
    // (handleNewChatInProject resets draft state incl. incognito-off.)
    if (effectiveProjectId) {
      await handleNewChatInProject(effectiveProjectId, currentAgentId)
      return
    }
    await handleStartNewChat(currentAgentId)
  }, [currentAgentId, effectiveProjectId, handleNewChatInProject, handleStartNewChat])

  /**
   * Title-bar agent switch handler. Backend rejects the switch when the
   * session already has user/assistant messages (defense layer); the UI
   * additionally hides the dropdown via `disabled` once messages exist, so
   * we only really get called for empty sessions.
   *
   * Branches:
   *  - Existing session (already materialized) → call backend so the change
   *    is persisted across reloads.
   *  - Draft session (no `currentSessionId` yet) → just update front-end
   *    state; the agent_id is baked in when the first message materializes
   *    the session.
   */
  const handleChangeAgent = useCallback(
    async (agentId: string) => {
      if (!agentId || agentId === session.currentAgentId) return
      const transport = getTransport()
      try {
        if (session.currentSessionId) {
          await transport.call("update_session_agent_cmd", {
            sessionId: session.currentSessionId,
            agentId,
          })
        }
        const agent = session.agents.find((a) => a.id === agentId)
        session.setCurrentAgentId(agentId)
        if (agent) session.setAgentName(agent.name)
        // Apply the new agent's preferred model (best-effort).
        try {
          const cfg = await transport.call<{
            model?: { primary?: string | null }
          }>("get_agent_config", { id: agentId })
          const primary = cfg.model?.primary
          if (primary) {
            const exists = availableModels.some((m) => `${m.providerId}::${m.modelId}` === primary)
            if (exists) {
              const [providerId, modelId] = primary.split("::")
              if (providerId && modelId) {
                setActiveModel({ providerId, modelId })
              }
            }
          }
        } catch {
          /* ignore */
        }
        await session.reloadSessions()
      } catch (err) {
        logger.warn("chat", "ChatScreen::handleChangeAgent", "failed", err)
      }
    },
    [session, availableModels, setActiveModel],
  )

  // ── Team ──────────────────────────────────────────────────
  const activeTeamId = useActiveTeam(currentSessionId ?? null)
  const [showTeamPanel, setShowTeamPanel] = useState(false)

  const refreshRuntimeModelState = useCallback(async () => {
    try {
      const [models, active, settings, agentConfig] = await Promise.all([
        getTransport().call<AvailableModel[]>("get_available_models"),
        getTransport().call<ActiveModel | null>("get_active_model"),
        getTransport().call<{ model: string; reasoning_effort: string }>("get_current_settings"),
        getTransport()
          .call<AgentConfig>("get_agent_config", { id: currentAgentId })
          .catch(() => null),
      ])

      setAvailableModels(models)
      globalActiveModelRef.current = active

      let displayModel = active
      const manualOverride = manualModelOverrideRef.current
      const manualModel = manualOverride
        ? models.find(
            (m) =>
              m.providerId === manualOverride.providerId && m.modelId === manualOverride.modelId,
          )
        : undefined
      if (manualOverride && !manualModel) {
        manualModelOverrideRef.current = null
      }

      if (manualModel && manualOverride) {
        displayModel = manualOverride
      } else if (currentSessionMeta?.providerId && currentSessionMeta?.modelId) {
        const sessionModel = models.find(
          (m) =>
            m.providerId === currentSessionMeta.providerId &&
            m.modelId === currentSessionMeta.modelId,
        )
        if (sessionModel) {
          displayModel = {
            providerId: sessionModel.providerId,
            modelId: sessionModel.modelId,
          }
        }
      } else if (agentConfig?.model?.primary) {
        const [providerId, modelId] = agentConfig.model.primary.split("::")
        const agentModel = models.find((m) => m.providerId === providerId && m.modelId === modelId)
        if (agentModel) {
          displayModel = { providerId, modelId }
        }
      }

      setActiveModel(displayModel)
      const displayModelInfo = displayModel
        ? models.find(
            (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
          )
        : undefined
      const effort =
        currentSessionMeta?.reasoningEffort ??
        agentConfig?.model?.reasoningEffort ??
        settings.reasoning_effort
      setReasoningEffort(normalizeEffortForModel(displayModelInfo, effort, t))

      if (agentConfig?.name) {
        setAgentName(agentConfig.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::refreshRuntimeModelState", "Failed to refresh model state", e)
    }
  }, [
    currentSessionMeta?.modelId,
    currentSessionMeta?.providerId,
    currentSessionMeta?.reasoningEffort,
    currentAgentId,
    globalActiveModelRef,
    setActiveModel,
    setAgentName,
    setAvailableModels,
    setReasoningEffort,
    t,
  ])

  const handleManualModelChange = useCallback(
    async (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      manualModelOverrideRef.current = { providerId, modelId }
      await handleModelChange(key, currentSessionId, session.currentAgentId)
    },
    [handleModelChange, currentSessionId, session.currentAgentId],
  )

  // Auto-show team panel when a team is created
  useEffect(() => {
    if (activeTeamId) setShowTeamPanel(true)
  }, [activeTeamId])

  useEffect(() => {
    manualModelOverrideRef.current = null
  }, [currentSessionId, currentAgentId])

  const sessionWorkingDir = currentSessionMeta?.workingDir ?? null
  const currentProject = useMemo(
    () => (effectiveProjectId ? (projects.find((p) => p.id === effectiveProjectId) ?? null) : null),
    [projects, effectiveProjectId],
  )
  useEffect(() => {
    onCurrentProjectChange?.(effectiveProjectId)
  }, [effectiveProjectId, onCurrentProjectChange])
  const previousSessionIdForProjectDraftRef = useRef<string | null>(session.currentSessionId)
  const materializedProjectDraftSessionIdRef = useRef<string | null>(null)
  // Hygiene: once a project draft (currentSessionId=null) materializes into a real
  // session and that new session's meta is available, the row is the source of
  // truth, so drop the now-redundant draft binding. Do not clear merely because an
  // old session is still mounted while "new chat in project" is resolving.
  useEffect(() => {
    const previousSessionId = previousSessionIdForProjectDraftRef.current
    const nextSessionId = session.currentSessionId
    previousSessionIdForProjectDraftRef.current = nextSessionId

    if (!draftProjectId) {
      materializedProjectDraftSessionIdRef.current = null
      return
    }

    if (previousSessionId === null && nextSessionId) {
      materializedProjectDraftSessionIdRef.current = nextSessionId
    }

    if (
      nextSessionId &&
      currentSessionMeta?.id === nextSessionId &&
      materializedProjectDraftSessionIdRef.current === nextSessionId
    ) {
      setDraftProjectId(null)
      materializedProjectDraftSessionIdRef.current = null
    }
  }, [session.currentSessionId, currentSessionMeta, draftProjectId])
  const projectWorkingDir = useMemo(() => currentProject?.workingDir ?? null, [currentProject])
  const effectiveWorkingDir = sessionWorkingDir ?? projectWorkingDir
  const workingDirSource: "session" | "project" | undefined = sessionWorkingDir
    ? "session"
    : projectWorkingDir
      ? "project"
      : undefined
  const workspaceEffectiveWorkingDir = session.currentSessionId
    ? effectiveWorkingDir
    : (draftWorkingDir ?? projectWorkingDir)
  const workspaceWorkingDirSource: "session" | "project" | undefined = session.currentSessionId
    ? workingDirSource
    : draftWorkingDir
      ? "session"
      : projectWorkingDir
        ? "project"
        : undefined

  const ensureWorkflowSession = useCallback(async (): Promise<string | null> => {
    if (session.currentSessionId) return session.currentSessionId
    if (draftIncognito) {
      toast.error(t("workspace.workflow.incognito", "无痕会话不持久化工作流"))
      return null
    }

    try {
      const transport = getTransport()
      const clearPreserveWorkspaceSoon = () => {
        window.setTimeout(() => {
          preserveWorkspaceOnSessionSwitchRef.current = false
        }, 1000)
      }
      const meta = await transport.call<SessionMeta>("create_session_cmd", {
        agentId: session.currentAgentId,
        projectId: draftProjectId ?? undefined,
        incognito: false,
      })
      preserveWorkspaceOnSessionSwitchRef.current = true
      if (draftWorkingDir) {
        try {
          await transport.call("set_session_working_dir", {
            sessionId: meta.id,
            workingDir: draftWorkingDir,
          })
        } catch (err) {
          await session.handleSwitchSession(meta.id).catch(() => {})
          await reloadSessions().catch(() => {})
          clearPreserveWorkspaceSoon()
          throw err
        }
      }
      await session.handleSwitchSession(meta.id)
      clearPreserveWorkspaceSoon()
      if (draftWorkingDir) {
        session.updateSessionMeta(meta.id, (prev) => ({ ...prev, workingDir: draftWorkingDir }))
      }
      await reloadSessions()
      return meta.id
    } catch (err) {
      logger.error(
        "chat",
        "ChatScreen::ensureWorkflowSession",
        "Failed to materialize workflow session",
        err,
      )
      toast.error(t("workspace.workflow.sessionCreateFailed", "创建工作流会话失败"), {
        description: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }, [draftIncognito, draftProjectId, draftWorkingDir, reloadSessions, session, t])

  // Wrap moveSessionToProject so the sidebar also reloads — otherwise the
  // moved session keeps rendering under the old "Unassigned" group until
  // the user manually refreshes.
  const handleMoveSessionToProject = useCallback(
    async (sessionId: string, projectId: string | null) => {
      await moveSessionToProject(sessionId, projectId)
      await reloadSessions()
    },
    [moveSessionToProject, reloadSessions],
  )

  const refreshUnreadState = useCallback(async () => {
    await Promise.all([reloadSessions(), reloadProjects()])
  }, [reloadSessions, reloadProjects])

  const [projectDialogOpen, setProjectDialogOpen] = useState(false)
  const [projectDialogMode, setProjectDialogMode] = useState<"create" | "edit">("create")
  const [projectDialogInitial, setProjectDialogInitial] = useState<Project | null>(null)

  const [projectOverviewOpen, setProjectOverviewOpen] = useState(false)
  const [projectOverviewTargetId, setProjectOverviewTargetId] = useState<string | null>(null)
  // Derive the live target from the projects list so mutations (rename,
  // archive, file upload) are reflected immediately in the open dialog.
  const projectOverviewTarget = useMemo(
    () =>
      projectOverviewTargetId
        ? (projects.find((p) => p.id === projectOverviewTargetId) ?? null)
        : null,
    [projects, projectOverviewTargetId],
  )

  const [projectDeleteTarget, setProjectDeleteTarget] = useState<Project | null>(null)

  const openCreateProject = useCallback(() => {
    setProjectDialogMode("create")
    setProjectDialogInitial(null)
    setProjectDialogOpen(true)
  }, [])

  const openEditProject = useCallback((project: Project) => {
    setProjectDialogMode("edit")
    setProjectDialogInitial(project)
    setProjectDialogOpen(true)
  }, [])

  const openProjectOverview = useCallback((project: ProjectMeta) => {
    setProjectOverviewTargetId(project.id)
    setProjectOverviewOpen(true)
  }, [])

  const [deletingProject, setDeletingProject] = useState(false)

  const confirmDeleteProject = useCallback(async () => {
    if (!projectDeleteTarget || deletingProject) return
    const projectName = projectDeleteTarget.name
    setDeletingProject(true)
    try {
      const ok = await deleteProject(projectDeleteTarget.id)
      setProjectDeleteTarget(null)
      if (ok) {
        setProjectOverviewOpen(false)
        reloadSessions()
        toast.success(t("common.deleted"), {
          description: projectName,
        })
      } else {
        toast.error(t("common.deleteFailed"), {
          description: projectName,
        })
      }
    } catch {
      toast.error(t("common.deleteFailed"), {
        description: projectName,
      })
    } finally {
      setDeletingProject(false)
    }
  }, [deleteProject, projectDeleteTarget, deletingProject, reloadSessions, t])

  // Rename session handler
  const handleRenameSession = useCallback(
    async (sessionId: string, title: string) => {
      try {
        await getTransport().call("rename_session_cmd", { sessionId, title })
        reloadSessions()
        // Per-project session lists paginate independently and may show sessions
        // older than the global window; a rename bumps neither `updated_at` nor
        // `session_count`, so their refetch triggers don't fire. Nudge them with
        // the new title directly (see useProjectSessions).
        window.dispatchEvent(
          new CustomEvent("hope:session-renamed", { detail: { id: sessionId, title } }),
        )
      } catch (err) {
        logger.error("chat", "ChatScreen::renameSession", "Failed to rename session", err)
      }
    },
    [reloadSessions],
  )

  const handleIncognitoChange = useCallback(
    (enabled: boolean) => {
      if (session.currentSessionId) return
      // Project + incognito are mutually exclusive — a project draft can't go
      // incognito (the toggle is also grayed via incognitoDisabledReason).
      if (draftProjectId) return
      setDraftIncognito(enabled)
      // Incognito = zero KB (D10). Drop any staged attaches so they can't ride
      // into the new incognito session or strand the now-disabled picker badge.
      if (enabled) setDraftKbAttachments([])
    },
    [session.currentSessionId, draftProjectId],
  )

  const handleWorkingDirChange = useCallback(
    async (workingDir: string | null) => {
      const sid = session.currentSessionId
      // No session yet — stash the choice. The backend `chat` command applies
      // it on the auto-create branch when the first message ships.
      if (!sid) {
        setDraftWorkingDir(workingDir)
        return
      }
      const previous = currentSessionMeta?.workingDir ?? null
      if (previous === workingDir) return
      session.updateSessionMeta(sid, (prev) =>
        prev.workingDir === workingDir ? prev : { ...prev, workingDir },
      )
      setWorkingDirSaving(true)
      try {
        await getTransport().call("set_session_working_dir", {
          sessionId: sid,
          workingDir,
        })
      } catch (err) {
        session.updateSessionMeta(sid, (prev) =>
          prev.workingDir === previous ? prev : { ...prev, workingDir: previous },
        )
        logger.error("chat", "ChatScreen::setWorkingDir", "Failed to update working directory", err)
        toast.error(t("chat.workingDir.invalid"), {
          description: err instanceof Error ? err.message : String(err),
        })
      } finally {
        setWorkingDirSaving(false)
      }
    },
    [session, currentSessionMeta?.workingDir, t],
  )

  // Once the auto-created session lands (chat command emits `session_created`),
  // the draft has been materialized server-side — drop the local copy so the
  // sidebar/sessions metadata becomes the single source of truth.
  useEffect(() => {
    if (session.currentSessionId && draftWorkingDir !== null) {
      setDraftWorkingDir(null)
    }
  }, [session.currentSessionId, draftWorkingDir])

  // Drop staged KB attaches once a session exists. On a new-chat first send the
  // attaches were already baked into the `chat` command payload (`kbAttachments`)
  // and applied by the backend on the auto-create branch, so clearing here is
  // pure local cleanup. On switching to an existing session (no send), this just
  // discards the unsent draft — symmetric with draftWorkingDir.
  useEffect(() => {
    if (session.currentSessionId && draftKbAttachments.length > 0) {
      setDraftKbAttachments([])
    }
  }, [session.currentSessionId, draftKbAttachments])

  // Reload sessions when external trigger changes (e.g. mark-all-read from IconSidebar)
  useEffect(() => {
    if (sessionsRefreshTrigger) {
      reloadSessions()
      reloadProjects()
    }
  }, [sessionsRefreshTrigger, reloadSessions, reloadProjects])

  // Close the in-session search bar whenever the active session changes.
  useEffect(() => {
    setSearchBarOpen(false)
  }, [currentSessionId])

  // The diff panel holds change metadata from a specific tool call in the
  // outgoing session — keeping it open across switches would render the
  // previous session's file content alongside a different message list.
  const closeDiff = diffPanel.closeDiff
  useEffect(() => {
    closeDiff()
  }, [currentSessionId, closeDiff])

  // Cmd/Ctrl+F: in-session search; Cmd/Ctrl+Shift+F: global sidebar search.
  // The in-session bar requires an active session (search target is a single
  // session); the global one is always available.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const modKey = e.metaKey || e.ctrlKey
      if (!modKey || e.altKey) return
      if (e.key.toLowerCase() !== "f") return
      // Don't hijack the shortcut while editing inside a genuine document
      // editor (knowledge note / markdown canvas) where Cmd+F means
      // find-in-document. The chat composer is itself a contenteditable
      // (CM6) but has no find-in-page equivalent for chat history, so let
      // the shortcut through there.
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable && !target.closest("[data-chat-composer]")) return

      if (e.shiftKey) {
        e.preventDefault()
        focusGlobalSearch()
        return
      }
      if (!currentSessionId) return
      e.preventDefault()
      openSessionSearch()
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [currentSessionId, openSessionSearch, focusGlobalSearch])

  // Cmd/Ctrl+N: start a fresh chat with the current agent. When the active
  // session belongs to a Project, keep the new session in that Project too.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const modKey = e.metaKey || e.ctrlKey
      if (!modKey || e.altKey || e.shiftKey || e.repeat) return
      if (e.key.toLowerCase() !== "n") return

      // The chat composer is a contenteditable (CM6) but Cmd+N has no
      // document-local meaning there, so only bail inside other editors
      // (knowledge note / markdown canvas).
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable && !target.closest("[data-chat-composer]")) return

      e.preventDefault()
      void handleStartNewChatFromCurrentContext()
    }
    window.addEventListener("keydown", handler)
    return () => window.removeEventListener("keydown", handler)
  }, [handleStartNewChatFromCurrentContext])

  // Listen for tray "new-session" event to trigger new chat
  useEffect(() => {
    return getTransport().listen("new-session", () => {
      void handleStartNewChatFromCurrentContext()
    })
  }, [handleStartNewChatFromCurrentContext])

  // Listen for tray "focus-session" event — emitted when the user clicks an
  // in-progress regular conversation entry inside the system tray dropdown.
  useEffect(() => {
    return getTransport().listen("tray:focus-session", (raw) => {
      const sessionId = (raw as { sessionId?: string } | undefined)?.sessionId
      if (sessionId) void handleSwitchSession(sessionId)
    })
  }, [handleSwitchSession])

  // Listen for channel slash command state-sync events.
  // Deps narrowed to currentSessionId + refreshUnreadState: re-subscribing on
  // every applyModelForDisplay / session.* identity churn would create a
  // listener storm. The other call sites used in the closure are either
  // stable useState setters or only matter for the active session — which is
  // re-keyed by currentSessionId here.
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    // Session model pinned (from IM /model, set_session_model command, or
    // PATCH /api/sessions/{id}/model). The IM channel may target a different
    // session than the one the desktop UI currently has open — gate by
    // sessionId so we don't apply a remote change to the wrong picker.
    unlisteners.push(
      getTransport().listen("session:model_updated", (payload) => {
        const { sessionId, providerId, modelId } = payload as {
          sessionId: string
          providerId: string
          modelId: string
        }
        if (!sessionId || sessionId !== session.currentSessionId) return
        manualModelOverrideRef.current = { providerId, modelId }
        applyModelForDisplay(`${providerId}::${modelId}`)
      }),
    )

    // Effort changed from channel (/thinking)
    unlisteners.push(
      getTransport().listen("slash:effort_changed", (payload) => {
        const data = payload as string | { sessionId?: string; effort?: string }
        const effort = typeof data === "string" ? data : data.effort
        if (!effort) return
        const sid = typeof data === "string" ? undefined : data.sessionId
        if (!sid || sid === session.currentSessionId) {
          setReasoningEffort(effort)
        }
        if (sid) {
          session.updateSessionMeta(sid, (prev) =>
            prev.reasoningEffort === effort ? prev : { ...prev, reasoningEffort: effort },
          )
        }
        session.reloadSessions()
      }),
    )

    // Session cleared from channel (/clear)
    unlisteners.push(
      getTransport().listen("slash:session_cleared", (payload) => {
        const clearedSid = payload as string
        if (clearedSid === session.currentSessionId) {
          session.setMessages([])
        }
        void refreshUnreadState()
      }),
    )

    // Plan state changed from channel (/plan)
    unlisteners.push(
      getTransport().listen("slash:plan_changed", () => {
        void refreshUnreadState()
      }),
    )

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [session.currentSessionId, refreshUnreadState]) // eslint-disable-line react-hooks/exhaustive-deps

  // Fetch models and current settings on mount
  useEffect(() => {
    void refreshRuntimeModelState()
  }, [refreshRuntimeModelState])

  useEffect(() => {
    const offConfig = getTransport().listen("config:changed", () => {
      void refreshRuntimeModelState()
    })
    const offAgents = getTransport().listen("agents:changed", () => {
      void refreshRuntimeModelState()
    })
    const onWindowAgentsChanged = () => {
      void refreshRuntimeModelState()
    }
    window.addEventListener("agents-changed", onWindowAgentsChanged)
    return () => {
      offConfig()
      offAgents()
      window.removeEventListener("agents-changed", onWindowAgentsChanged)
    }
  }, [refreshRuntimeModelState])

  const handleSandboxModeSynced = useCallback(
    (sessionId: string, mode: SandboxMode) => {
      updateSessionMeta(sessionId, (prev) =>
        prev.sandboxMode === mode ? prev : { ...prev, sandboxMode: mode },
      )
    },
    [updateSessionMeta],
  )

  // ── Stream Hook ─────────────────────────────────────────────
  const stream = useChatStream({
    messages: session.messages,
    setMessages: session.setMessages,
    currentSessionId: session.currentSessionId,
    setCurrentSessionId: session.setCurrentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    currentAgentId: session.currentAgentId,
    agentName: session.agentName,
    loading: session.loading,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    capMessagesForSession: session.capMessagesForSession,
    touchSessionCacheLru: session.touchSessionCacheLru,
    sessions: session.sessions,
    agents: session.agents,
    activeModel,
    reloadSessions: refreshUnreadState,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
    reasoningEffort,
    incognitoEnabled,
    draftWorkingDir,
    draftProjectId,
    draftKbAttachments,
    onSandboxModeSynced: handleSandboxModeSynced,
    parentInjectionDeltasViaChatStream: true,
  })

  useEffect(() => {
    incognitoComposerStateRef.current = {
      input: stream.input,
      attachedFileCount: stream.attachedFiles.length,
      pendingQuoteCount: stream.pendingQuotes.length,
      pendingMessage: stream.pendingMessage ?? "",
      pendingSendCount: stream.pendingSends.length,
    }
  }, [
    stream.attachedFiles.length,
    stream.input,
    stream.pendingMessage,
    stream.pendingQuotes.length,
    stream.pendingSends.length,
  ])

  useLayoutEffect(() => {
    clearDisposableIncognitoDraftRef.current = () => {
      stream.setInput("")
      stream.setAttachedFiles([])
      stream.setPendingQuotes([])
      for (const pending of stream.pendingSends) {
        stream.discardPendingSend(pending.id)
      }
    }
  }, [stream])

  useEffect(() => {
    return getTransport().listen("permission:mode_changed", (payload) => {
      const data = payload as { sessionId?: unknown; mode?: unknown }
      if (typeof data.sessionId !== "string" || !isSessionMode(data.mode)) return
      const sessionId = data.sessionId
      const mode = data.mode

      updateSessionMeta(sessionId, (prev) =>
        prev.permissionMode === mode ? prev : { ...prev, permissionMode: mode },
      )
      void reloadSessions()
    })
  }, [reloadSessions, updateSessionMeta])

  useEffect(() => {
    return getTransport().listen("sandbox:mode_changed", (payload) => {
      const data = payload as { sessionId?: unknown; mode?: unknown }
      if (typeof data.sessionId !== "string" || !isSandboxMode(data.mode)) return
      const sessionId = data.sessionId
      const mode = data.mode

      updateSessionMeta(sessionId, (prev) =>
        prev.sandboxMode === mode ? prev : { ...prev, sandboxMode: mode },
      )
      void reloadSessions()
    })
  }, [reloadSessions, updateSessionMeta])

  // Consume a token injection from a global view: `@plan:xxx` from Plans, or a
  // `[[note]]` ref from Knowledge. Append once with a leading space, then notify
  // App so the slot clears. A KB ref also auto-attaches its KB (read-only) so the
  // injection isn't dropped by `effective_kb_access` at send time. Incognito
  // sessions get zero KB access (D10) — skip the attach, never the insert.
  useEffect(() => {
    if (!pendingChatInsert) return
    const { token, attachKbId } = pendingChatInsert
    const run = async () => {
      if (attachKbId && !incognitoEnabled) {
        try {
          if (session.currentSessionId) {
            await getTransport().call("attach_session_kb_cmd", {
              sessionId: session.currentSessionId,
              kbId: attachKbId,
              access: "read",
            })
          } else {
            // New chat: stage the attach; it's baked into the `chat` payload on
            // first send (symmetric with draftWorkingDir / KnowledgePicker).
            setDraftKbAttachments((prev) =>
              prev.some((a) => a.kbId === attachKbId)
                ? prev
                : [...prev, { kbId: attachKbId, access: "read" }],
            )
          }
        } catch (e) {
          // Non-fatal: the token is still inserted; the user can attach manually.
          logger.warn("ui", "ChatScreen::referenceInChat", "auto-attach KB failed", e)
        }
      }
      // Functional updater (not the captured `stream.input`): the `attach` await
      // above is a transport round-trip during which the user may keep typing —
      // reading a stale snapshot here would drop those keystrokes. The updater
      // also composes correctly if two refs are inserted back-to-back.
      stream.setInput((prev) => `${prev}${prev && !prev.endsWith(" ") ? " " : ""}${token} `)
      onChatInsertConsumed?.()
    }
    void run()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingChatInsert])

  // ── Stream Reattach Hook ────────────────────────────────────
  // Rehydrates chat streaming after frontend reload / window reopen / browser
  // refresh via EventBus-backed events, deduplicated against `streamSeqRef`
  // which the primary per-call Channel path also updates.
  useChatStreamReattach({
    currentSessionId: session.currentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    updateSessionMessages: session.updateSessionMessages,
    setShowCodexAuthExpired: stream.setShowCodexAuthExpired,
    setMessages: session.setMessages,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    reloadSessions: refreshUnreadState,
    onTurnStarted: stream.handleTurnStarted,
    onTurnEnded: stream.handleTurnEnded,
  })

  // ── Plan Mode Hook ─────────────────────────────────────────
  const planMode = usePlanMode(session.currentSessionId, planModeState, setPlanModeState)
  const taskProgressSnapshot = useTaskProgressSnapshot(session.currentSessionId, session.messages)
  // Context-window fullness for the input-dock bottom bar. Derived from the
  // active model's window + the latest assistant usage (shared helper, same
  // numbers as the status popover / workspace session card).
  const currentModelForUsage = useMemo(
    () => resolveCurrentModel(activeModel, availableModels),
    [activeModel, availableModels],
  )
  const contextUsage = useMemo(() => {
    if (!currentModelForUsage) return null

    const baseUsage = computeContextUsage(session.messages, currentModelForUsage.contextWindow)
    const currentOverride =
      manualCompactOverride && manualCompactOverride.sessionId === session.currentSessionId
        ? manualCompactOverride
        : null
    if (!currentOverride) return baseUsage

    const latestUsageFingerprint = latestAssistantUsageFingerprint(session.messages)
    if (latestUsageFingerprint !== currentOverride.usageFingerprint) {
      return baseUsage
    }

    return (
      formatContextUsage(currentOverride.tokensAfter, currentModelForUsage.contextWindow) ??
      baseUsage
    )
  }, [currentModelForUsage, manualCompactOverride, session.currentSessionId, session.messages])
  const setPlanState = planMode.setPlanState
  const sendMessage = stream.handleSend
  const [draftWorkflowMode, setDraftWorkflowMode] = useState<"off" | "on" | "ultracode">("off")

  useEffect(() => {
    if (session.currentSessionId) {
      setDraftWorkflowMode("off")
    }
  }, [session.currentSessionId])

  // ── Memory extraction toast ────────────────────────────────
  const [memoryToast, setMemoryToast] = useState<{ count: number } | null>(null)
  const memoryToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    const unlisten = getTransport().listen("memory_extracted", (raw) => {
      const { count, sessionId } = raw as { count: number; sessionId: string }
      // Only show toast for the current session
      if (sessionId === session.currentSessionId && count > 0) {
        setMemoryToast({ count })
        if (memoryToastTimer.current) clearTimeout(memoryToastTimer.current)
        memoryToastTimer.current = setTimeout(() => setMemoryToast(null), 4000)
      }
    })
    return () => {
      unlisten()
      if (memoryToastTimer.current) clearTimeout(memoryToastTimer.current)
    }
  }, [session.currentSessionId])

  // ── Load system prompt ──────────────────────────────────────────
  const loadSystemPrompt = useCallback(async () => {
    setSystemPromptLoading(true)
    try {
      const prompt = await getTransport().call<string>("get_system_prompt", {
        agentId: session.currentAgentId,
      })
      setSystemPromptContent(prompt)
      setShowSystemPrompt(true)
    } catch (e) {
      logger.error("ui", "ChatScreen::loadSystemPrompt", "Failed to load system prompt", e)
    } finally {
      setSystemPromptLoading(false)
    }
  }, [session.currentAgentId])

  const runCompactContextForCurrentSession =
    useCallback(async (): Promise<CompactResult | null> => {
      const sid = session.currentSessionId
      if (!sid || compactingRef.current) return null

      const noticeId = `manual-compact:${sid}:${Date.now()}`
      compactingRef.current = true
      setCompacting(true)
      session.updateSessionMessages(sid, (prev) =>
        upsertManualCompactNotice(prev, makeCompactProgressEvent(), noticeId),
      )

      try {
        const result = await compactContextNow(sid)
        session.updateSessionMessages(sid, (prev) =>
          upsertManualCompactNotice(prev, makeCompactResultEvent(result), noticeId),
        )
        return result
      } catch (e) {
        session.updateSessionMessages(sid, (prev) =>
          upsertManualCompactNotice(prev, makeCompactFailedEvent(), noticeId),
        )
        throw e
      } finally {
        compactingRef.current = false
        setCompacting(false)
      }
    }, [session])

  // ── Slash Command Action Handler ──────────────────────────────
  const handleCommandAction = useCallback(
    async (result: CommandResult) => {
      const action = result.action
      const commandSessionId = result._sessionId ?? session.currentSessionId ?? null

      // Skip history rows for session-spawning commands and skill passThrough
      // (the real user bubble already shows "/skillname args"). Other slash
      // controls are persisted by the backend as event rows; mirror that
      // immediately in the current in-memory timeline.
      const shouldShowSlashHistory =
        action?.type !== "newSession" &&
        action?.type !== "switchAgent" &&
        action?.type !== "passThrough" &&
        !result._isSkillPassThrough
      const actionRendersResult =
        action?.type === "showProjectPicker" ||
        action?.type === "showSessionPicker" ||
        action?.type === "recapCard" ||
        action?.type === "skillFork" ||
        action?.type === "compact"
      const suppressLoopCreateResult = isLoopCreateSlashCommand(result._slashCommandText)
      const shouldAppendResultContent = result.content && !actionRendersResult && !suppressLoopCreateResult
      const slashHistoryMessages: Message[] = []
      if (shouldShowSlashHistory && result._slashCommandText) {
        const now = new Date().toISOString()
        const commandDisplay = slashCommandDisplay(result._slashCommandText)
        slashHistoryMessages.push(
          makeClientEventMessage({
            content: commandDisplay.content,
            timestamp: now,
            slashEvent: {
              kind: "command",
              command: result._slashCommandText,
              displayAs: "user",
              mode: commandDisplay.mode,
            },
          }),
        )
        if (shouldAppendResultContent) {
          slashHistoryMessages.push(
            makeClientEventMessage({
              content: result.content,
              timestamp: now,
              slashEvent: { kind: "result", command: result._slashCommandText },
            }),
          )
        }
      } else if (shouldAppendResultContent && shouldShowSlashHistory) {
        slashHistoryMessages.push(
          makeClientEventMessage({
            content: result.content,
            timestamp: new Date().toISOString(),
          }),
        )
      }
      if (slashHistoryMessages.length > 0) {
        if (commandSessionId) {
          session.updateSessionMessages(commandSessionId, (prev) =>
            appendUniqueSlashHistoryMessages(prev, slashHistoryMessages),
          )
        } else {
          session.setMessages((prev) =>
            appendUniqueSlashHistoryMessages(prev, slashHistoryMessages),
          )
        }
      }

      if (!action) return

      switch (action.type) {
        case "newSession":
          // Behave like the "New Chat" button: clear immediately without showing an empty session
          // in the sidebar. The backend-created session is deleted to avoid DB clutter.
          await handleStartNewChat(session.currentAgentId)
          if (action.sessionId) {
            getTransport()
              .call("delete_session_cmd", { sessionId: action.sessionId })
              .then(() => session.reloadSessions())
              .catch(() => {})
          }
          break
        case "switchModel":
          handleManualModelChange(`${action.providerId}::${action.modelId}`)
          break
        case "setEffort":
          handleSessionEffortChange(action.effort)
          break
        case "switchAgent":
          if (action.sessionId) void handleSwitchSession(action.sessionId)
          break
        case "stopStream":
          stream.handleStop()
          break
        case "compact":
          if (session.currentSessionId) {
            try {
              const result = await runCompactContextForCurrentSession()
              if (result) {
                toast.success(compactResultMessage(t, result))
              }
            } catch (e) {
              logger.error("ui", "ChatScreen::slashCompact", "Compact failed", e)
              toast.error(t("chat.compactFailed"))
            }
          }
          break
        case "sessionCleared":
          session.setMessages(slashHistoryMessages)
          void refreshUnreadState()
          break
        case "passThrough":
          if (result._isSkillPassThrough) {
            // User bubble shows "/skillname args"; LLM gets the expanded prompt.
            await stream.handleSend(action.message, {
              displayText: result._skillCommandText,
            })
          } else if (isGoalUpsertSlashCommand(result._slashCommandText)) {
            const visibleGoal = goalSlashCommandDisplay(result._slashCommandText ?? "").content
            planMode.exitPlanMode()
            void stream.handleSend(goalTurnPrompt(action.message), {
              displayText: visibleGoal,
              goalTrigger: true,
              sessionIdOverride: commandSessionId ?? undefined,
            })
          } else {
            stream.setInput(action.message)
            setTimeout(() => stream.handleSend(), 50)
          }
          break
        case "exportFile":
          try {
            const ext = (action.filename.split(".").pop() ?? "md").toLowerCase()
            const filterName = ext === "json" ? "JSON" : ext === "html" ? "HTML" : "Markdown"
            const filePath = await save({
              defaultPath: action.filename,
              filters: [{ name: filterName, extensions: [ext] }],
            })
            if (filePath) {
              await getTransport().call("write_export_file", {
                path: filePath,
                content: action.content,
              })
            }
          } catch (e) {
            logger.error("ui", "ChatScreen::slashExport", "Export failed", e)
          }
          break
        case "setToolPermission":
          stream.setPermissionModeByUser(action.mode)
          break
        case "setWorkflowMode":
          if (commandSessionId) {
            window.dispatchEvent(
              new CustomEvent(WORKFLOW_MODE_CHANGED_EVENT, {
                detail: { sessionId: commandSessionId, mode: action.mode },
              }),
            )
          }
          break
        case "displayOnly":
          // Already handled above by adding event message
          break
        case "showModelPicker": {
          const pickerMsg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            modelPickerData: {
              models: action.models,
              activeProviderId: action.activeProviderId,
              activeModelId: action.activeModelId,
            },
          })
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterPlanMode":
          planMode.enterPlanMode()
          break
        case "exitPlanMode":
          planMode.exitPlanMode()
          break
        case "approvePlan":
          await planMode.approvePlan()
          stream.handleSend(t("planMode.executeCommand"), {
            planMode: "executing",
            displayText: t("planMode.executionApproved"),
            isPlanTrigger: true,
          })
          break
        case "showPlan":
          planMode.setPlanContent(action.planContent)
          planMode.setShowPanel(true)
          break
        case "viewSystemPrompt":
          loadSystemPrompt()
          break
        case "showContextBreakdown": {
          const contextMsg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            contextBreakdownData: action.breakdown,
          })
          session.setMessages((prev) => [...prev, contextMsg])
          break
        }
        case "showProjectPicker": {
          // Render a markdown list of projects so the user can either click
          // back to /project <name> or visually pick from the sidebar's
          // project tree. A full clickable picker card is a follow-up.
          const lines = [t("project.openProject") + ":"]
          for (const p of action.projects) {
            lines.push(`- **${p.name}** · ${p.sessionCount}`)
          }
          lines.push("")
          lines.push(`> \`/project <${t("project.projectName")}>\``)
          const pickerMsg: Message = makeClientEventMessage({
            content: lines.join("\n"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterProject": {
          void handleNewChatInProject(action.projectId)
          break
        }
        case "assignProject": {
          // IM-mode action — desktop falls back to the "create new chat in
          // project" flow so users still get a usable outcome if they
          // somehow reach this branch from the GUI.
          void handleNewChatInProject(action.projectId)
          break
        }
        case "showSessionPicker": {
          // Markdown list, mirroring the showProjectPicker fallback. Each
          // row carries the short id + title + agent / project / channel
          // chips so users can spot the right session at a glance; the
          // user types `/session <id>` (or clicks in the sidebar) to switch.
          const lines = [t("chat.pickSession") + ":"]
          for (const s of action.sessions) {
            const idShort = s.id.slice(0, 8)
            const chips: string[] = []
            if (s.agentLabel) chips.push(`agent: ${s.agentLabel}`)
            if (s.projectLabel) chips.push(`project: ${s.projectLabel}`)
            if (s.channelLabel) chips.push(s.channelLabel)
            const suffix = chips.length ? ` · _${chips.join(" · ")}_` : ""
            lines.push(`- \`${idShort}\` · ${s.title}${suffix}`)
            if (s.snippet) lines.push(`  > ${s.snippet}`)
          }
          lines.push("")
          lines.push("> `/session <id>` · `/sessions <query>`")
          const pickerMsg: Message = makeClientEventMessage({
            content: lines.join("\n"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, pickerMsg])
          break
        }
        case "enterSession":
        case "attachToSession": {
          // Desktop has no chat-to-session binding; both reduce to "switch
          // to that session". Reuse the sidebar's switch path so history /
          // pagination / agent restore behave identically.
          void handleSwitchSession(action.sessionId)
          break
        }
        case "detachFromSession": {
          // GUI has no IM-attach to release. Surface a hint so /session exit
          // typed from the desktop slash menu doesn't appear silent.
          const msg: Message = makeClientEventMessage({
            content: t("chat.detachOnDesktopNoop"),
            timestamp: new Date().toISOString(),
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
        case "handoverToChannel": {
          // Slash-form handover — mirrors what the title-bar Send icon does
          // through HandoverDialog, but skips the dialog when the user
          // already supplied channel:account:chat[:thread] inline.
          try {
            await getTransport().call<void>("channel_handover_session", {
              sessionId: action.sessionId,
              channelId: action.channelId,
              accountId: action.accountId,
              chatId: action.chatId,
              threadId: action.threadId ?? null,
            })
            const msg: Message = makeClientEventMessage({
              content: t("chat.handover.done"),
              timestamp: new Date().toISOString(),
            })
            session.setMessages((prev) => [...prev, msg])
          } catch (e) {
            const msg: Message = makeClientEventMessage({
              content: t("chat.handover.failed", { error: String(e) }),
              timestamp: new Date().toISOString(),
            })
            session.setMessages((prev) => [...prev, msg])
          }
          break
        }
        case "recapCard": {
          const reportId = readActionString(action, "reportId", "report_id")
          if (!reportId) {
            logger.warn("chat", "ChatScreen::slashRecapCard", "Missing report id", action)
            break
          }
          const msg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            recapCardData: { reportId },
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
        case "openDashboardTab":
          onOpenDashboardTab?.(action.tab)
          break
        case "skillFork": {
          const runId = readActionString(action, "runId", "run_id")
          if (!runId) {
            logger.warn("chat", "ChatScreen::slashSkillFork", "Missing run id", action)
            break
          }
          const skillName =
            readActionString(action, "skillName", "skill_name") ??
            t("skills.defaultName", { defaultValue: "skill" })
          const msg: Message = makeClientEventMessage({
            content: "",
            timestamp: new Date().toISOString(),
            skillForkData: {
              runId,
              skillName,
            },
          })
          session.setMessages((prev) => [...prev, msg])
          break
        }
      }
    },
    [
      session,
      stream,
      handleStartNewChat,
      handleManualModelChange,
      handleSessionEffortChange,
      planMode,
      loadSystemPrompt,
      handleNewChatInProject,
      handleSwitchSession,
      refreshUnreadState,
      onOpenDashboardTab,
      runCompactContextForCurrentSession,
      t,
    ],
  )

  const handleGoalModeSubmit = useCallback(
    async (objective: string, action?: string): Promise<boolean> => {
      const rawObjective = objective.trim()
      const trimmed = parseGoalUpsertSlashCommand(rawObjective) ?? rawObjective
      if (!trimmed) return false
      if (incognitoEnabled || draftIncognito) {
        toast.error(t("chat.goalMode.incognito", "无痕会话不持久化目标"))
        return false
      }
      if (!currentSessionId) {
        const parsed = parseGoalObjectiveAndCriteria(trimmed)
        const initialObjective = parsed.objective || trimmed
        const initialCriteria = parsed.completionCriteria.trim()
        const promptGoalText = initialCriteria
          ? `${initialObjective}\n\nCompletion criteria:\n${initialCriteria}`
          : initialObjective
        void stream.handleSend(goalTurnPrompt(promptGoalText), {
          displayText: trimmed,
          goalTrigger: true,
          initialGoal: {
            objective: initialObjective,
            completionCriteria: initialCriteria || undefined,
          },
        })
        return true
      }
      const sid = await ensureWorkflowSession()
      if (!sid) return false
      const activeGoal = chatGoal.snapshot?.goal ?? null
      if (activeGoal && (action === "append_required" || action === "append_optional")) {
        const nextCriteria = appendGoalCriterionLine(
          activeGoal.completionCriteria,
          trimmed,
          action === "append_optional" ? "optional" : "required",
        )
        try {
          const snapshot = await getTransport().call<GoalSnapshot>("update_goal", {
            goalId: activeGoal.id,
            objective: activeGoal.objective,
            completionCriteria: nextCriteria,
          })
          chatGoal.setSnapshot(snapshot)
          chatGoal.refresh()
          void stream.handleSend(goalTurnPrompt(trimmed), {
            displayText: trimmed,
            goalTrigger: true,
            sessionIdOverride: sid,
          })
          toast.success(t("chat.goalMode.criteriaAdded", "完成标准已追加"))
          return true
        } catch (e) {
          logger.error("ui", "ChatScreen::goalCriteriaAppend", "Failed to append criteria", e)
          toast.error(e instanceof Error ? e.message : String(e))
          return false
        }
      }
      if (activeGoal && action === "append_follow_up") {
        try {
          const snapshot = await getTransport().call<GoalSnapshot>("append_goal_follow_up", {
            goalId: activeGoal.id,
            items: [trimmed],
            source: "composer",
          })
          chatGoal.setSnapshot(snapshot)
          chatGoal.refresh()
          void stream.handleSend(goalTurnPrompt(trimmed), {
            displayText: trimmed,
            goalTrigger: true,
            sessionIdOverride: sid,
          })
          toast.success(t("chat.goalMode.followUpAdded", "后续项已加入目标"))
          return true
        } catch (e) {
          logger.error("ui", "ChatScreen::goalFollowUpAppend", "Failed to append follow-up", e)
          toast.error(e instanceof Error ? e.message : String(e))
          return false
        }
      }
      if (activeGoal && action === "replace") {
        try {
          await getTransport().call<GoalSnapshot>("close_goal", {
            goalId: activeGoal.id,
            decision: "superseded",
            reason: t("chat.goalMode.supersededReason", "用户用目标模式创建了替代目标"),
            followUpItems: [],
          })
          chatGoal.setSnapshot(null)
        } catch (e) {
          logger.error("ui", "ChatScreen::goalReplace", "Failed to supersede current goal", e)
          toast.error(e instanceof Error ? e.message : String(e))
          return false
        }
      }
      const commandText = `/goal ${trimmed}`
      try {
        const result = await getTransport().call<CommandResult>("execute_slash_command", {
          sessionId: sid,
          agentId: session.currentAgentId,
          commandText,
        })
        result._slashCommandText = commandText
        result._sessionId = sid
        await handleCommandAction(result)
        chatGoal.refresh()
        return true
      } catch (e) {
        logger.error("ui", "ChatScreen::goalModeSubmit", "Failed to create goal from composer", e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      }
    },
    [
      chatGoal,
      draftIncognito,
      currentSessionId,
      ensureWorkflowSession,
      handleCommandAction,
      incognitoEnabled,
      session.currentAgentId,
      stream,
      t,
    ],
  )

  const handleLoopModeSubmit = useCallback(
    async (prompt: string): Promise<boolean> => {
      const rawPrompt = prompt.trim()
      const trimmed = parseLoopCreateSlashCommand(rawPrompt) ?? rawPrompt
      if (!trimmed) return false
      if (incognitoEnabled || draftIncognito) {
        toast.error(t("chat.loopMode.incognito", "无痕会话不持久化持续推进"))
        return false
      }
      const sid = await ensureWorkflowSession()
      if (!sid) return false
      const commandText = `/loop ${trimmed}`
      const commandDisplay = slashCommandDisplay(commandText)
      const optimisticLoopMessage = makeClientEventMessage({
        content: commandDisplay.content,
        timestamp: new Date().toISOString(),
        slashEvent: {
          kind: "command",
          command: commandText,
          displayAs: "user",
          mode: commandDisplay.mode,
        },
      })
      session.updateSessionMessages(sid, (prev) =>
        appendUniqueSlashHistoryMessages(prev, [optimisticLoopMessage]),
      )
      try {
        const result = await getTransport().call<CommandResult>("execute_slash_command", {
          sessionId: sid,
          agentId: session.currentAgentId,
          commandText,
        })
        result._slashCommandText = commandText
        result._sessionId = sid
        await handleCommandAction(result)
        return true
      } catch (e) {
        session.updateSessionMessages(sid, (prev) =>
          prev.filter((message) => message._clientId !== optimisticLoopMessage._clientId),
        )
        logger.error("ui", "ChatScreen::loopModeSubmit", "Failed to create loop from composer", e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      }
    },
    [
      draftIncognito,
      ensureWorkflowSession,
      handleCommandAction,
      incognitoEnabled,
      session.currentAgentId,
      t,
    ],
  )

  const handleGoalUpdate = useCallback(
    async (objective: string, completionCriteria: string): Promise<boolean> => {
      const goalId = chatGoal.snapshot?.goal.id
      const trimmedObjective = objective.trim()
      if (!goalId || !trimmedObjective) return false
      try {
        const snapshot = await getTransport().call<GoalSnapshot>("update_goal", {
          goalId,
          objective: trimmedObjective,
          completionCriteria: completionCriteria.trim(),
        })
        chatGoal.setSnapshot(snapshot)
        toast.success(t("chat.goalMode.updated", "目标已更新"))
        chatGoal.refresh()
        return true
      } catch (e) {
        logger.error("ui", "ChatScreen::goalUpdate", "Failed to update goal", e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      }
    },
    [chatGoal, t],
  )

  const runGoalControlAction = useCallback(
    async (command: "pause_goal" | "resume_goal" | "clear_goal" | "evaluate_goal") => {
      const goalId = chatGoal.snapshot?.goal.id
      if (!goalId) return false
      try {
        const snapshot = await getTransport().call<GoalSnapshot>(command, { goalId })
        chatGoal.setSnapshot(command === "clear_goal" ? null : snapshot)
        chatGoal.refresh()
        return true
      } catch (e) {
        logger.error("ui", "ChatScreen::goalControl", `Goal action failed: ${command}`, e)
        toast.error(e instanceof Error ? e.message : String(e))
        return false
      }
    },
    [chatGoal],
  )

  // ── Plan Approve Handler ───────────────────────────────────────
  const handlePlanApprove = useCallback(async () => {
    await planMode.approvePlan()
    // Send a short trigger — the full plan is already in the system prompt (Executing state)
    stream.handleSend(t("planMode.executeCommand"), {
      planMode: "executing",
      displayText: t("planMode.executionApproved"),
      isPlanTrigger: true,
    })
  }, [planMode, stream, t])

  const handlePlanContinue = useCallback(async () => {
    stream.handleSend(t("planMode.executeCommand"), {
      planMode: "executing",
      displayText: t("planMode.executionResumed"),
      isPlanTrigger: true,
    })
  }, [stream, t])

  const handleMessageSwitchModel = useCallback(
    (providerId: string, modelId: string) => {
      void handleManualModelChange(`${providerId}::${modelId}`)
    },
    [handleManualModelChange],
  )

  // ── Plan Request Changes Handler ──────────────────────────────
  // See `planCommentMessage.ts` for the prompt vs displayText vs payload split.
  const handleRequestChanges = useCallback(
    ({ prompt, displayText, payload }: BuiltPlanComment) => {
      setPlanState("planning")
      if (currentSessionId) {
        getTransport()
          .call("set_plan_mode", { sessionId: currentSessionId, state: "planning" })
          .catch(() => {})
      }
      sendMessage(prompt, { displayText, planComment: payload })
    },
    [setPlanState, sendMessage, currentSessionId],
  )

  const shouldShowPlanPanel =
    planMode.showPanel &&
    planMode.planState !== "off" &&
    (planMode.planState === "planning" || planMode.planContent.trim().length > 0)
  const isDiffPanelVisible = diffPanel.showPanel && diffPanel.activeChanges.length > 0
  const isFilePreviewVisible = filePreview.showPanel && !!filePreview.target
  const [activeExclusiveRightPanel, setActiveExclusiveRightPanel] =
    useState<ExclusiveRightPanel | null>(null)
  const [rightPanelCollapsed, setRightPanelCollapsed] = useState(false)
  const [manualRightPanelExpandedOverride, setManualRightPanelExpandedOverride] = useState(false)
  const autoCollapsedRightPanelRef = useRef(false)
  const rightPanelVisibility = useMemo<ExclusiveRightPanelVisibility>(
    () => ({
      workspace: showWorkspacePanel,
      diff: isDiffPanelVisible,
      plan: shouldShowPlanPanel,
      files: showFilesPanel && !!effectiveWorkingDir,
      browser: showBrowserPanel,
      "mac-control": showMacControlPanel,
      canvas: canvasPanelOpen,
      team: !!activeTeamId && showTeamPanel,
      "background-jobs": showBackgroundJobsPanel,
      preview: isFilePreviewVisible,
    }),
    [
      activeTeamId,
      canvasPanelOpen,
      effectiveWorkingDir,
      isDiffPanelVisible,
      isFilePreviewVisible,
      shouldShowPlanPanel,
      showBackgroundJobsPanel,
      showBrowserPanel,
      showFilesPanel,
      showMacControlPanel,
      showTeamPanel,
      showWorkspacePanel,
    ],
  )
  const openExclusiveRightPanels = useMemo(
    () => EXCLUSIVE_RIGHT_PANEL_ORDER.filter((panel) => rightPanelVisibility[panel]),
    [rightPanelVisibility],
  )
  const hasOpenExclusiveRightPanel = openExclusiveRightPanels.length > 0
  const renderedExclusiveRightPanel =
    activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
      ? activeExclusiveRightPanel
      : (openExclusiveRightPanels[0] ?? null)
  const shouldRenderRightPanelContent = !!renderedExclusiveRightPanel
  const getRightPanelLabel = useCallback(
    (panel: ExclusiveRightPanel) => {
      switch (panel) {
        case "workspace":
          return t("workspace.panelTitle", "工作台")
        case "diff":
          return t("diffPanel.title", "Diff")
        case "plan":
          return t("planMode.panelTitle", "Plan")
        case "files":
          return t("fileBrowser.panelTitle", "Files")
        case "browser":
          return t("browser.panelTitle", "Browser")
        case "mac-control":
          return t("macControl.panelTitle", "Mac Control")
        case "canvas":
          return t("canvas.panelTitle", "Canvas")
        case "team":
          return t("team.panelTitle", "Team")
        case "background-jobs":
          return t("backgroundJobs.panelTitle", "后台任务")
        case "preview":
          return t("filePreview.panelTitle", "Preview")
      }
    },
    [t],
  )
  const titleBarRightPanels = useMemo(
    () =>
      openExclusiveRightPanels.map((panel) => ({
        id: panel,
        label: getRightPanelLabel(panel),
        icon: EXCLUSIVE_RIGHT_PANEL_ICONS[panel],
      })),
    [getRightPanelLabel, openExclusiveRightPanels],
  )
  const handleSelectRightPanel = useCallback((panelId: string) => {
    if (!EXCLUSIVE_RIGHT_PANEL_ORDER.includes(panelId as ExclusiveRightPanel)) return
    setActiveExclusiveRightPanel(panelId as ExclusiveRightPanel)
    autoCollapsedRightPanelRef.current = false
    setManualRightPanelExpandedOverride(true)
    setRightPanelCollapsed(false)
  }, [])

  const showRightPanelByUser = useCallback((panel: ExclusiveRightPanel) => {
    setActiveExclusiveRightPanel(panel)
    autoCollapsedRightPanelRef.current = false
    setManualRightPanelExpandedOverride(true)
    setRightPanelCollapsed(false)
  }, [])

  // Stage a "quote to chat" reference as a removable chip above the composer.
  // On send it becomes a quote attachment: the model sees a <file_reference>
  // block, the user only ever sees a friendly quote card.
  const handleFileQuote = useCallback(
    (q: QuotePayload) => {
      stream.setPendingQuotes((prev) => [...prev, q])
    },
    [stream],
  )
  // Reveal a quoted file in the browser: open the files panel + signal target.
  const handleQuoteJump = useCallback(
    (q: QuotePayload) => {
      setShowFilesPanel(true)
      showRightPanelByUser("files")
      revealQuoteNonce.current += 1
      setRevealFile({
        path: q.path,
        name: q.name,
        startLine: q.startLine,
        endLine: q.endLine,
        nonce: revealQuoteNonce.current,
      })
    },
    [showRightPanelByUser],
  )

  // 打开并激活 Workspace 面板（状态条点击 / 重新打开）。
  const openWorkspacePanel = useCallback(() => {
    workspacePanelDismissedRef.current = false
    setShowWorkspacePanel(true)
    showRightPanelByUser("workspace")
  }, [showRightPanelByUser])

  const openBrowserPanel = useCallback(() => {
    browserPanelDismissedRef.current = false
    setShowBrowserPanel(true)
    showRightPanelByUser("browser")
  }, [showRightPanelByUser])

  const openBackgroundJobsPanel = useCallback(
    (opts?: { activate?: boolean }) => {
      backgroundJobsPanelDismissedRef.current = false
      const activate = opts?.activate ?? true
      suppressNextBackgroundJobsActivationRef.current = !activate
      setShowBackgroundJobsPanel(true)
      if (activate) showRightPanelByUser("background-jobs")
    },
    [showRightPanelByUser],
  )

  const closeBackgroundJobsPanel = useCallback(() => {
    backgroundJobsPanelDismissedRef.current = true
    suppressNextBackgroundJobsActivationRef.current = false
    setShowBackgroundJobsPanel(false)
  }, [])

  const handleBackgroundJobExpandedChange = useCallback((jobId: string, expanded: boolean) => {
    setBackgroundJobExpansionOverrides((prev) =>
      prev[jobId] === expanded ? prev : { ...prev, [jobId]: expanded },
    )
  }, [])

  useEffect(() => {
    if (!hasOpenExclusiveRightPanel && rightPanelCollapsed) {
      autoCollapsedRightPanelRef.current = false
      setManualRightPanelExpandedOverride(false)
      setRightPanelCollapsed(false)
    }
  }, [hasOpenExclusiveRightPanel, rightPanelCollapsed])

  const preferredSidebarWidthForResponsive = userSidebarCollapsedPreferenceRef.current
    ? 0
    : panelWidth
  const responsiveRightPanelWidth = clampResponsiveRightPanelWidth(rightPanelWidth)
  const rightPanelCollapseAt =
    preferredSidebarWidthForResponsive + CHAT_MAIN_MIN_INTERACTIVE_WIDTH + responsiveRightPanelWidth
  const rightPanelExpandAt = rightPanelCollapseAt + RESPONSIVE_PANEL_HYSTERESIS
  const sidebarCollapseAt =
    panelWidth + CHAT_MAIN_MIN_INTERACTIVE_WIDTH + SIDEBAR_AUTO_COLLAPSE_GUTTER
  const sidebarExpandAt = sidebarCollapseAt + RESPONSIVE_PANEL_HYSTERESIS
  const shouldAutoCollapseRightPanel = useViewportMediaQuery(
    `(max-width: ${rightPanelCollapseAt}px)`,
  )
  const shouldAutoExpandRightPanel = useViewportMediaQuery(`(min-width: ${rightPanelExpandAt}px)`)
  const rightPanelOverlay =
    hasOpenExclusiveRightPanel && manualRightPanelExpandedOverride && shouldAutoCollapseRightPanel
  const shouldAutoCollapseSidebar = useViewportMediaQuery(`(max-width: ${sidebarCollapseAt}px)`)
  const shouldAutoExpandSidebar = useViewportMediaQuery(`(min-width: ${sidebarExpandAt}px)`)

  useEffect(() => {
    if (shouldAutoExpandRightPanel && manualRightPanelExpandedOverride) {
      setManualRightPanelExpandedOverride(false)
    }

    if (shouldAutoExpandSidebar) {
      manualSidebarExpandedOverrideRef.current = false
    }

    if (hasOpenExclusiveRightPanel) {
      if (
        shouldAutoCollapseRightPanel &&
        !rightPanelCollapsed &&
        !manualRightPanelExpandedOverride
      ) {
        autoCollapsedRightPanelRef.current = true
        setRightPanelCollapsed(true)
      } else if (
        shouldAutoExpandRightPanel &&
        rightPanelCollapsed &&
        autoCollapsedRightPanelRef.current
      ) {
        autoCollapsedRightPanelRef.current = false
        setRightPanelCollapsed(false)
      }
    } else {
      autoCollapsedRightPanelRef.current = false
      if (manualRightPanelExpandedOverride) {
        setManualRightPanelExpandedOverride(false)
      }
    }

    if (
      shouldAutoCollapseSidebar &&
      !sidebarCollapsed &&
      !userSidebarCollapsedPreferenceRef.current &&
      !manualSidebarExpandedOverrideRef.current
    ) {
      autoCollapsedSidebarRef.current = true
      setSidebarCollapsed(true)
    } else if (
      shouldAutoExpandSidebar &&
      sidebarCollapsed &&
      autoCollapsedSidebarRef.current &&
      !userSidebarCollapsedPreferenceRef.current
    ) {
      autoCollapsedSidebarRef.current = false
      setSidebarCollapsed(false)
    }
  }, [
    hasOpenExclusiveRightPanel,
    manualRightPanelExpandedOverride,
    rightPanelCollapsed,
    sidebarCollapsed,
    shouldAutoCollapseRightPanel,
    shouldAutoCollapseSidebar,
    shouldAutoExpandRightPanel,
    shouldAutoExpandSidebar,
  ])

  // Plan / Diff / Browser / Mac Control / Canvas / Team share the same right
  // rail. Track rising edges so the panel that just opened wins while the
  // others remain open in the background and can be switched back to.
  const previousRightPanelVisibilityRef = useRef<ExclusiveRightPanelVisibility>(
    EMPTY_RIGHT_PANEL_VISIBILITY,
  )
  // Monotonic open-nonces capture a user re-opening an ALREADY-visible panel
  // (clicking a file while preview is open / re-opening a diff): there's no
  // visibility rising edge to latch onto (showPanel stayed true), so the nonce's
  // rising edge force-claims the active slot.
  const previousPreviewOpenNonceRef = useRef(filePreview.openNonce)
  const previousDiffOpenNonceRef = useRef(diffPanel.openNonce)
  useLayoutEffect(() => {
    const previous = previousRightPanelVisibilityRef.current
    const rawNewlyOpened =
      EXCLUSIVE_RIGHT_PANEL_ORDER.find(
        (panel) => rightPanelVisibility[panel] && !previous[panel],
      ) ?? null
    const newlyOpened =
      rawNewlyOpened === "background-jobs" && suppressNextBackgroundJobsActivationRef.current
        ? null
        : rawNewlyOpened
    if (rawNewlyOpened === "background-jobs") {
      suppressNextBackgroundJobsActivationRef.current = false
    }
    let forced: ExclusiveRightPanel | null = null
    if (filePreview.openNonce !== previousPreviewOpenNonceRef.current) {
      forced = "preview"
    } else if (diffPanel.openNonce !== previousDiffOpenNonceRef.current) {
      forced = "diff"
    }
    previousPreviewOpenNonceRef.current = filePreview.openNonce
    previousDiffOpenNonceRef.current = diffPanel.openNonce
    const stillActive =
      activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
        ? activeExclusiveRightPanel
        : null
    const active = forced ?? newlyOpened ?? stillActive ?? openExclusiveRightPanels[0] ?? null

    previousRightPanelVisibilityRef.current = rightPanelVisibility
    if (activeExclusiveRightPanel !== active) {
      setActiveExclusiveRightPanel(active)
    }
  }, [
    activeExclusiveRightPanel,
    openExclusiveRightPanels,
    rightPanelVisibility,
    filePreview.openNonce,
    diffPanel.openNonce,
  ])

  // Reset dismissal flags (and any open panel state) on session switch so each
  // session gets a fresh chance to auto-open live mirror panels. Bind the stable
  // `closePreview` callback locally so this effect depends on it (not the
  // per-render `filePreview` object, which would reset every panel on every
  // preview toggle).
  const closeFilePreview = filePreview.closePreview
  useEffect(() => {
    const preserveWorkspace = preserveWorkspaceOnSessionSwitchRef.current
    browserPanelDismissedRef.current = false
    macControlPanelDismissedRef.current = false
    workspacePanelDismissedRef.current = false
    backgroundJobsPanelDismissedRef.current = false
    suppressNextBackgroundJobsActivationRef.current = false
    previousBackgroundRunningCountRef.current = 0
    setShowBrowserPanel(false)
    setShowMacControlPanel(false)
    setShowWorkspacePanel(preserveWorkspace)
    setShowBackgroundJobsPanel(false)
    setBackgroundJobExpansionOverrides({})
    closeFilePreview()
    if (preserveWorkspace) {
      preserveWorkspaceOnSessionSwitchRef.current = false
    }
  }, [session.currentSessionId, closeFilePreview])

  // Auto-open the BrowserPanel only on the first `browser:frame` of a session
  // and only if the user hasn't already dismissed it.
  useEffect(() => {
    const unlisten = getTransport().listen("browser:frame", (raw) => {
      const payload = parsePayload<{ sessionId?: string | null }>(raw)
      if (payload?.sessionId && payload.sessionId !== session.currentSessionId) return
      if (browserPanelDismissedRef.current) return
      setShowBrowserPanel((prev) => (prev ? prev : true))
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [session.currentSessionId])

  useEffect(() => {
    const unlisten = getTransport().listen("browser:extension_required", (raw) => {
      const payload = parsePayload<BrowserExtensionRequiredPayload>(raw)
      if (!payload) return
      if (payload.sessionId && payload.sessionId !== session.currentSessionId) return
      const reason = payload.reason || payload.statusMessage
      const next = payload.nextAction
        ? t("chat.browserExtensionRequired.nextAction", {
            defaultValue: "Next action: {{nextAction}}",
            nextAction: payload.nextAction,
          })
        : t("chat.browserExtensionRequired.openSettings", {
            defaultValue: "Open Settings > Browser to install or enable the extension.",
          })
      toast(
        t("chat.browserExtensionRequired.title", { defaultValue: "Chrome extension required" }),
        {
          id: "browser-extension-required",
          description: [reason, next].filter(Boolean).join("\n"),
        },
      )
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [session.currentSessionId, t])

  useEffect(() => {
    const unlisten = getTransport().listen("mac_control:frame", (raw) => {
      if (macControlPanelDismissedRef.current) return
      const payload = parsePayload<MacControlFrameOpenHint>(raw)
      const isToolScreenshotFrame = !!(payload?.mediaId || payload?.path)
      setShowMacControlPanel((prev) => (prev ? prev : isToolScreenshotFrame))
    })
    return () => {
      try {
        unlisten?.()
      } catch {
        // ignore
      }
    }
  }, [])

  // 首次有任务/文件/来源时自动展开 Workspace 面板一次；用户关闭后本会话不再
  // 自动弹（仿 browser/mac-control 的 dismissed 模型）。用便宜的存在性检查(短路)，
  // 完整聚合在 WorkspacePanel 内部、面板打开时才进行。
  const hasWorkspaceContent =
    (taskProgressSnapshot?.total ?? 0) > 0 ||
    messagesHaveFileActivity(session.messages) ||
    messagesHaveUrlActivity(session.messages) ||
    messagesHaveBrowserActivity(session.messages) ||
    messagesHaveKnowledgeActivity(session.messages)
  // 依赖里带 currentSessionId：切到「已有内容」的旧会话时 hasWorkspaceContent 不发生
  // false→true 跳变，靠 session 变化触发本 effect 重跑(配合 session-reset 复位
  // dismissedRef)，否则旧会话切回来面板不会自动展开。
  useEffect(() => {
    if (!hasWorkspaceContent || workspacePanelDismissedRef.current) return
    setShowWorkspacePanel((prev) => (prev ? prev : true))
  }, [hasWorkspaceContent, session.currentSessionId])

  useEffect(() => {
    const previousRunningCount = previousBackgroundRunningCountRef.current
    const runningCount = backgroundJobs.runningCount
    previousBackgroundRunningCountRef.current = runningCount
    const action = decideBackgroundJobsAutoOpen({
      runningCount,
      previousRunningCount,
      dismissed: backgroundJobsPanelDismissedRef.current,
      activePanel: renderedExclusiveRightPanel,
    })

    if (action === "activate") openBackgroundJobsPanel({ activate: true })
    if (action === "open-in-background") openBackgroundJobsPanel({ activate: false })
  }, [backgroundJobs.runningCount, openBackgroundJobsPanel, renderedExclusiveRightPanel])

  const workspaceTaskExecutionState = resolveWorkspaceTaskExecutionState(
    session.currentSessionId
      ? stream.executionStateBySession.get(session.currentSessionId)
      : undefined,
    session.loading,
  )
  const workflowTitleBarRuns = useWorkflowRuns(session.currentSessionId, {
    incognito: incognitoEnabled,
    turnActive:
      workspaceTaskExecutionState === "running" || workspaceTaskExecutionState === "cancelling",
  })
  const workflowTitleBarStatus = useMemo(() => {
    const needsAttention = (run: WorkflowRun) =>
      run.state === "awaiting_approval" ||
      run.state === "awaiting_user" ||
      run.state === "blocked" ||
      run.state === "failed"
    const isRunning = (run: WorkflowRun) => run.state === "running" || run.state === "recovering"
    return {
      activeCount: workflowTitleBarRuns.activeCount,
      attentionCount: workflowTitleBarRuns.runs.filter(needsAttention).length,
      runningCount: workflowTitleBarRuns.runs.filter(isRunning).length,
    }
  }, [workflowTitleBarRuns.activeCount, workflowTitleBarRuns.runs])
  const workflowInputProgress = useMemo(() => {
	    const visibleStates = new Set<WorkflowRun["state"]>([
	      "awaiting_approval",
	      "awaiting_user",
	      "blocked",
	      "failed",
	      "running",
	      "recovering",
	      "paused",
	    ])
    const priority = (run: WorkflowRun) => {
      switch (run.state) {
        case "awaiting_approval":
        case "awaiting_user":
        case "blocked":
        case "failed":
          return 0
	        case "running":
	        case "recovering":
	          return 1
	        case "paused":
	          return 2
	        default:
	          return 9
	      }
    }
    const candidates = workflowTitleBarRuns.runs
      .filter((run) => visibleStates.has(run.state))
      .slice()
      .sort((a, b) => {
        const byPriority = priority(a) - priority(b)
        if (byPriority !== 0) return byPriority
        return Date.parse(b.updatedAt || b.createdAt) - Date.parse(a.updatedAt || a.createdAt)
      })
    return {
      run: candidates[0] ?? null,
      count: candidates.length,
    }
  }, [workflowTitleBarRuns.runs])

  const handleToggleFilesPanel = useCallback(() => {
    if (showFilesPanel) {
      setShowFilesPanel(false)
      return
    }
    setShowFilesPanel(true)
    showRightPanelByUser("files")
  }, [showFilesPanel, showRightPanelByUser])

  const rightPanelReservedMainWidth =
    manualRightPanelExpandedOverride && !rightPanelCollapsed
      ? CHAT_MAIN_COMPACT_MIN_INTERACTIVE_WIDTH
      : CHAT_MAIN_MIN_INTERACTIVE_WIDTH
  const chatMainMinWidth = `min(100%, ${rightPanelReservedMainWidth}px)`
  const workspacePanelVisibleInRightPanel =
    showWorkspacePanel && renderedExclusiveRightPanel === "workspace" && !rightPanelCollapsed

  const emptySessionInputHero =
    session.messages.length === 0 &&
    !session.historyLoading &&
    !session.loading &&
    !planMode.pendingQuestionGroup &&
    !planMode.planCardInfo &&
    !planMode.planSubagentRunning &&
    !searchBarOpen
  // The hero greeting (logo + slogan) is rendered above the centered composer
  // only when that composer is actually mounted (cron / subagent sessions have
  // no input box). When true, MessageList must suppress its own empty greeting
  // so the two don't overlap.
  const heroComposerActive = emptySessionInputHero && !isCronSession && !isSubagentSession

  return (
    <>
      {/* Sidebar */}
      <ChatSidebar
        sessions={session.sessions}
        agents={session.agents}
        projects={projects}
        projectsLoading={projectsInitialLoading}
        currentSessionId={session.currentSessionId}
        loadingSessionIds={session.loadingSessionIds}
        sessionsLoading={session.sessionsLoading}
        panelWidth={panelWidth}
        sidebarCollapsed={sidebarCollapsed}
        onPanelWidthChange={setPanelWidth}
        onSidebarCollapsedChange={handleSidebarCollapsedChange}
        onSwitchSession={handleSwitchSession}
        onNewChat={handleStartNewChat}
        onDeleteSession={session.handleDeleteSession}
        onEditAgent={onOpenAgentSettings}
        onToggleSessionPinned={session.handleToggleSessionPinned}
        onReorderAgents={session.handleReorderAgents}
        onMarkAllRead={refreshUnreadState}
        onRenameSession={handleRenameSession}
        hasMoreSessions={session.hasMoreSessions}
        loadingMoreSessions={session.loadingMoreSessions}
        onLoadMoreSessions={session.handleLoadMoreSessions}
        onOpenProjectSettings={openProjectOverview}
        onAddProject={openCreateProject}
        onNewChatInProject={(projectId) => {
          // Enter a project draft (lazy creation). Agent resolution, draft reset
          // and incognito-off (project + incognito are mutually exclusive) all
          // live in handleNewChatInProject.
          void handleNewChatInProject(projectId)
        }}
        onArchiveProject={(projectId, archived) => {
          void archiveProject(projectId, archived)
        }}
        onReorderProjects={(projectIds) => {
          void reorderProjects(projectIds)
        }}
        onMoveSessionToProject={handleMoveSessionToProject}
        searchFocusSignal={globalSearchFocusSignal}
      />

      {/* Project create/edit dialog */}
      <ProjectDialog
        open={projectDialogOpen}
        mode={projectDialogMode}
        initialProject={projectDialogInitial}
        agents={session.agents}
        onOpenChange={setProjectDialogOpen}
        onCreate={createProject}
        onUpdate={updateProject}
      />

      {/* Project overview dialog (tabs: overview/sessions/files/instructions) */}
      <ProjectOverviewDialog
        open={projectOverviewOpen}
        project={projectOverviewTarget}
        onOpenChange={setProjectOverviewOpen}
        onEdit={(p) => {
          setProjectOverviewOpen(false)
          openEditProject(p)
        }}
        onDelete={(p) => setProjectDeleteTarget(p)}
        onArchive={async (p, archived) => {
          await archiveProject(p.id, archived)
          // Close the dialog since archived projects vanish from the sidebar
          if (archived) setProjectOverviewOpen(false)
        }}
        onNewSessionInProject={(projectId, defaultAgentId) => {
          void handleNewChatInProject(projectId, defaultAgentId)
        }}
        onOpenSession={(sid) => void handleSwitchSession(sid)}
        onUpdateProject={updateProject}
      />

      {/* Project delete confirmation */}
      <AlertDialog
        open={!!projectDeleteTarget}
        onOpenChange={(o) => !o && setProjectDeleteTarget(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("project.deleteConfirm.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("project.deleteConfirm.body")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={confirmDeleteProject}
              disabled={deletingProject}
            >
              {deletingProject ? t("common.saving") : t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Incognito leave confirmation */}
      <AlertDialog
        open={!!incognitoLeaveIntent}
        onOpenChange={(open) => {
          if (!open) setIncognitoLeaveIntent(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("chat.incognitoLeaveConfirmTitle", {
                defaultValue: "Leave incognito chat?",
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("chat.incognitoLeaveConfirmBody", {
                defaultValue:
                  "After you leave, this chat will be deleted from this device and can't be restored from history.",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t("chat.incognitoLeaveConfirmCancel", {
                defaultValue: "Stay here",
              })}
            </AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={handleConfirmIncognitoLeave}
            >
              {t("chat.incognitoLeaveConfirmAction", {
                defaultValue: "Delete and leave",
              })}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Command Approval Dialog */}
      <ApprovalDialog
        requests={stream.approvalRequests}
        onRespond={stream.handleApprovalResponse}
      />

      {/* Codex Auth Expired Dialog */}
      <AlertDialog open={stream.showCodexAuthExpired} onOpenChange={stream.setShowCodexAuthExpired}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("codexAuth.expiredTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("codexAuth.expiredDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            {onCodexReauth && (
              <AlertDialogAction
                onClick={() => {
                  stream.setShowCodexAuthExpired(false)
                  onCodexReauth()
                }}
              >
                {t("codexAuth.reauth")}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* System Prompt Viewer Dialog */}
      <SystemPromptDialog
        open={showSystemPrompt}
        onOpenChange={setShowSystemPrompt}
        content={systemPromptContent}
      />

      {/* Conversation workspace */}
      <div className="flex-1 flex flex-col min-w-0 bg-background">
        <ChatTitleBar
          agentName={session.agentName}
          currentAgentId={session.currentAgentId}
          currentSessionId={session.currentSessionId}
          sessions={session.sessions}
          messages={session.messages}
          contextUsageOverride={contextUsage}
          activeModel={activeModel}
          availableModels={availableModels}
          reasoningEffort={reasoningEffort}
          loading={session.loading}
          compacting={compacting}
          onCompactContext={runCompactContextForCurrentSession}
          onRenameSession={handleRenameSession}
          onViewSystemPrompt={loadSystemPrompt}
          systemPromptLoading={systemPromptLoading}
          onCommandAction={handleCommandAction}
          onOpenSearch={openSessionSearch}
          searchOpen={searchBarOpen}
          effectiveWorkingDir={effectiveWorkingDir}
          workingDirSource={workingDirSource}
          project={currentProject}
          onOpenProjectSettings={openProjectOverview}
          onOpenHandover={(sid) => setHandoverSessionId(sid)}
          agents={session.agents}
          onChangeAgent={handleChangeAgent}
          sidebarCollapsed={sidebarCollapsed}
          onExpandSidebar={() => handleSidebarCollapsedChange(false)}
          incognitoEnabled={incognitoEnabled}
          incognitoDisabledReason={incognitoDisabledReason}
          onIncognitoChange={handleIncognitoChange}
          onToggleFilesPanel={effectiveWorkingDir ? handleToggleFilesPanel : undefined}
          filesPanelOpen={showFilesPanel}
          onToggleWorkspacePanel={() => {
            if (showWorkspacePanel) {
              workspacePanelDismissedRef.current = true
              setShowWorkspacePanel(false)
            } else {
              openWorkspacePanel()
            }
          }}
          workspacePanelOpen={showWorkspacePanel}
          workspaceWorkflowStatus={workflowTitleBarStatus}
          onToggleBackgroundJobsPanel={() => {
            if (showBackgroundJobsPanel) {
              closeBackgroundJobsPanel()
            } else {
              openBackgroundJobsPanel()
            }
          }}
          backgroundJobsPanelOpen={showBackgroundJobsPanel}
          backgroundJobsRunningCount={backgroundJobs.runningCount}
          rightPanels={titleBarRightPanels}
          activeRightPanelId={renderedExclusiveRightPanel}
          rightPanelCollapsed={rightPanelCollapsed}
          onSelectRightPanel={handleSelectRightPanel}
          onToggleRightPanelCollapsed={
            hasOpenExclusiveRightPanel
              ? () => {
                  const nextCollapsed = !rightPanelCollapsed
                  autoCollapsedRightPanelRef.current = false
                  setManualRightPanelExpandedOverride(!nextCollapsed)
                  setRightPanelCollapsed(nextCollapsed)
                }
              : undefined
          }
        />

        <BrowserExtensionNudge
          sessionId={session.currentSessionId}
          onOpenSettings={onOpenSettings}
        />

        <div className="flex-1 flex min-h-0 overflow-hidden">
          <div
            className="relative flex-1 flex flex-col min-w-0"
            style={{ minWidth: chatMainMinWidth }}
          >
            {activeTeamId && !showTeamPanel && (
              <div className="px-3 py-1 border-b border-border">
                <TeamMiniIndicator teamId={activeTeamId} onClick={() => setShowTeamPanel(true)} />
              </div>
            )}

            {searchBarOpen && session.currentSessionId && (
              <SessionSearchBar
                sessionId={session.currentSessionId}
                onJumpTo={session.jumpToMessage}
                onClose={() => setSearchBarOpen(false)}
                focusSignal={searchFocusSignal}
              />
            )}

            <CrashRecoveryBanner />

            <FileActionsContext.Provider value={fileActionsValue}>
              <MessageList
                messages={session.messages}
                historyLoading={session.historyLoading}
                loading={session.loading}
                executionState={
                  session.currentSessionId
                    ? (stream.executionStateBySession.get(session.currentSessionId) ?? null)
                    : null
                }
                agents={session.agents}
                hasMore={session.hasMore}
                loadingMore={session.loadingMore}
                onLoadMore={session.handleLoadMore}
                hasMoreAfter={session.hasMoreAfter}
                loadingMoreAfter={session.loadingMoreAfter}
                onLoadMoreAfter={session.handleLoadMoreAfter}
                onResetToLatest={session.resetToLatest}
                sessionId={session.currentSessionId}
                incognito={incognitoEnabled}
                heroComposer={heroComposerActive}
                pendingScrollIntent={session.pendingScrollIntent}
                onScrollTargetHandled={session.clearPendingScrollIntent}
                pendingQuestionGroup={planMode.pendingQuestionGroup}
                onQuestionSubmitted={() => planMode.setPendingQuestionGroup(null)}
                planCardData={planMode.planCardInfo ? { title: planMode.planCardInfo.title } : null}
                planState={planMode.planState}
                onOpenPlanPanel={planMode.openPlanPanel}
                onApprovePlan={handlePlanApprove}
                onExitPlan={planMode.exitPlanMode}
                planSubagentRunning={planMode.planSubagentRunning}
                onSwitchModel={handleMessageSwitchModel}
                onViewSystemPrompt={loadSystemPrompt}
                compacting={compacting}
                onCompactContext={runCompactContextForCurrentSession}
                onOpenDashboardTab={onOpenDashboardTab}
                onViewChildSession={(sid) => {
                  setSubagentPreviewSessionId(sid)
                }}
                onOpenDiff={diffPanel.openDiff}
                onResume={(message) => {
                  void stream.handleSend(message)
                }}
                onAddQuickPrompt={incognitoEnabled ? undefined : handleAddQuickPrompt}
                displayMode={displayMode}
                autoCollapseCompletedTurns={autoCollapseCompletedTurns}
              />

              {/* Memory extraction toast — absolute-positioned above ChatInput
               * so it doesn't shrink the MessageList scroll container when it
               * appears/disappears. */}
              {!isCronSession && !isSubagentSession && (
                <div
                  className={cn(
                    "relative",
                    emptySessionInputHero &&
                      "absolute inset-x-0 top-[48%] z-20 flex -translate-y-1/2 justify-center px-5 sm:px-8",
                  )}
                >
                  {memoryToast && (
                    <div
                      className={cn(
                        "absolute bottom-full mb-2 flex items-center gap-2 px-3 py-1.5 rounded-lg bg-secondary/50 text-xs text-muted-foreground animate-in fade-in slide-in-from-bottom-2 duration-300 z-10",
                        emptySessionInputHero
                          ? "inset-x-5 mx-auto max-w-[880px] sm:inset-x-8"
                          : "inset-x-3 mx-auto max-w-[880px]",
                      )}
                    >
                      <Brain className="h-3.5 w-3.5 shrink-0" />
                      <span>
                        {t("settings.memoryExtractedToast", { count: memoryToast.count })}
                      </span>
                      <button
                        onClick={() => setMemoryToast(null)}
                        className="ml-auto text-muted-foreground/60 hover:text-muted-foreground"
                      >
                        ×
                      </button>
                    </div>
                  )}

                  <div
                    className={cn(
                      "mx-auto w-full max-w-[880px]",
                      emptySessionInputHero && "flex flex-col",
                    )}
                  >
                    {heroComposerActive && (
                      <div className="mb-5 sm:mb-6">
                        <ChatWelcomeHero incognito={incognitoEnabled} />
                      </div>
                    )}
                    <ChatInput
                      input={stream.input}
                      onInputChange={stream.setInput}
                      inputHistory={inputHistory}
                      quickPrompts={quickPrompts}
                      onSend={() =>
                        stream.handleSend(
                          undefined,
                          !session.currentSessionId && draftWorkflowMode !== "off"
                            ? { workflowMode: draftWorkflowMode }
                            : undefined,
                        )
                      }
                      sendDisabled={session.historyLoading}
                      loading={session.loading}
                      availableModels={availableModels}
                      activeModel={activeModel}
                      reasoningEffort={reasoningEffort}
                      onModelChange={handleManualModelChange}
                      onEffortChange={handleSessionEffortChange}
                      attachedFiles={stream.attachedFiles}
                      onAttachFiles={(files) =>
                        stream.setAttachedFiles((prev) => [...prev, ...files])
                      }
                      onRemoveFile={(index) =>
                        stream.setAttachedFiles((prev) => prev.filter((_, i) => i !== index))
                      }
                      pendingQuotes={stream.pendingQuotes}
                      onRemoveQuote={(index) => {
                        stream.setPendingQuotes((prev) => prev.filter((_, i) => i !== index))
                        setRevealFile(null) // dropping a quote clears its reveal highlight
                      }}
                      onJumpToQuote={handleQuoteJump}
                      pendingMessage={stream.pendingMessage}
                      pendingSends={stream.pendingSends}
                      onCancelPending={() => {
                        stream.setInput(stream.pendingMessage || "")
                        stream.setPendingMessage(null)
                      }}
                      onDiscardPending={() => {
                        stream.setPendingMessage(null)
                      }}
                      onEditPending={stream.editPendingSend}
                      onDiscardPendingItem={stream.discardPendingSend}
                      onForceInsertPending={stream.forceInsertPendingSend}
                      onCancelForceInsertPending={stream.cancelForceInsertPendingSend}
                      onStop={stream.handleStop}
                      currentSessionId={session.currentSessionId}
                      currentAgentId={session.currentAgentId}
                      onEnsureSession={ensureWorkflowSession}
                      onCommandAction={handleCommandAction}
                      permissionMode={stream.permissionMode}
                      onPermissionModeChange={stream.setPermissionModeByUser}
                      sandboxMode={stream.sandboxMode}
                      onSandboxModeChange={stream.setSandboxModeByUser}
                      sessionTemperature={sessionTemperature}
                      onSessionTemperatureChange={setSessionTemperature}
                      incognitoEnabled={incognitoEnabled}
                      projectId={effectiveProjectId}
                      draftKbAttachments={draftKbAttachments}
                      onDraftKbAttachChange={setDraftKbAttachments}
                      enableNoteMention
                      enableSkillMention
                      enableAgentMention
                      agents={session.agents}
                      workingDir={
                        session.currentSessionId
                          ? effectiveWorkingDir
                          : (draftWorkingDir ?? projectWorkingDir)
                      }
                      workingDirInherited={
                        session.currentSessionId
                          ? workingDirSource === "project"
                          : draftWorkingDir
                            ? false
                            : !!projectWorkingDir
                      }
                      workingDirSaving={workingDirSaving}
                      onWorkingDirChange={effectiveProjectId ? undefined : handleWorkingDirChange}
                      planState={planMode.planState}
                      onEnterPlanMode={planMode.enterPlanMode}
                      onExitPlanMode={planMode.exitPlanMode}
                      onTogglePlanPanel={() => planMode.setShowPanel((p) => !p)}
                      draftWorkflowMode={draftWorkflowMode}
                      onDraftWorkflowModeChange={setDraftWorkflowMode}
                      goalSnapshot={chatGoal.snapshot}
                      autonomyActivity={chatGoal.activity}
                      goalLoading={chatGoal.loading}
                      onGoalModeSubmit={handleGoalModeSubmit}
                      onLoopModeSubmit={handleLoopModeSubmit}
                      onGoalUpdate={handleGoalUpdate}
                      onPauseGoal={() => runGoalControlAction("pause_goal")}
                      onResumeGoal={() => runGoalControlAction("resume_goal")}
                      onClearGoal={() => runGoalControlAction("clear_goal")}
                      onEvaluateGoal={() => runGoalControlAction("evaluate_goal")}
                      taskProgressSnapshot={taskProgressSnapshot}
                      onOpenWorkspace={openWorkspacePanel}
                      workflowProgressRun={workflowInputProgress.run}
                      workflowProgressCount={workflowInputProgress.count}
                      workspacePanelVisible={workspacePanelVisibleInRightPanel}
                      executionState={
                        session.currentSessionId
                          ? (stream.executionStateBySession.get(session.currentSessionId) ?? null)
                          : null
                      }
                      hero={emptySessionInputHero}
                      contextUsage={contextUsage}
                    />
                  </div>
                </div>
              )}
            </FileActionsContext.Provider>
          </div>

          {/* Diff panel (right side, selected from the title-bar panel switcher) */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "diff" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("diffPanel.resizePanel", "Resize diff panel")}
              maxWidth={860}
              reservedMainWidth={rightPanelReservedMainWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              contentKey="diff"
            >
              <DiffPanel
                changes={diffPanel.activeChanges}
                activeIndex={diffPanel.activeIndex}
                openNonce={diffPanel.openNonce}
                onActiveIndexChange={diffPanel.setActiveIndex}
                onClose={diffPanel.closeDiff}
                onPreviewFile={filePreview.openPreview}
                embedded
              />
            </RightPanelShell>
          )}

          {/* Plan workspace (right side, integrated under the shared title bar) */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "plan" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("planMode.resizePanel", "Resize plan panel")}
              maxWidth={860}
              reservedMainWidth={rightPanelReservedMainWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              contentKey="plan"
            >
              <PlanPanel
                planState={planMode.planState}
                planContent={planMode.planContent}
                sessionId={session.currentSessionId}
                onApprove={handlePlanApprove}
                onExit={planMode.exitPlanMode}
                onClose={() => planMode.setShowPanel(false)}
                onContinue={handlePlanContinue}
                isExecutionActive={session.loading && planMode.planState === "executing"}
                onRequestChanges={handleRequestChanges}
                embedded
              />
            </RightPanelShell>
          )}

          {/* Project file browser (right side, scoped to the working dir) */}
          {/* File browser panel — permanently mounted (like CanvasPanel) and
              toggled via `visible`, so a popped-out window survives panel
              switches / collapses. */}
          <FileBrowserPanel
            scope={!session.currentSessionId && currentProject ? "project" : "session"}
            scopeId={
              !session.currentSessionId && currentProject
                ? currentProject.id
                : session.currentSessionId
            }
            rootPath={effectiveWorkingDir}
            sessionId={session.currentSessionId}
            visible={shouldRenderRightPanelContent && renderedExclusiveRightPanel === "files"}
            collapsed={rightPanelCollapsed}
            overlay={rightPanelOverlay}
            panelWidth={rightPanelWidth}
            onPanelWidthChange={setRightPanelWidth}
            reservedMainWidth={rightPanelReservedMainWidth}
            onQuote={handleFileQuote}
            revealFile={revealFile}
            onClose={() => setShowFilesPanel(false)}
          />

          {/* Canvas Preview Panel */}
          <CanvasPanel
            panelWidth={rightPanelWidth}
            onPanelWidthChange={setRightPanelWidth}
            currentSessionId={currentSessionId}
            onOpenChange={setCanvasPanelOpen}
            collapsed={rightPanelCollapsed}
            overlay={rightPanelOverlay}
            reservedMainWidth={rightPanelReservedMainWidth}
            visible={shouldRenderRightPanelContent && renderedExclusiveRightPanel === "canvas"}
          />

          {/* Browser live-mirror panel — open on first `browser:frame` push,
              close-only by user, then switchable from the title bar. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "browser" && (
            <BrowserPanel
              sessionId={session.currentSessionId}
              panelWidth={rightPanelWidth}
              onPanelWidthChange={setRightPanelWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              reservedMainWidth={rightPanelReservedMainWidth}
              onClose={() => {
                browserPanelDismissedRef.current = true
                setShowBrowserPanel(false)
              }}
            />
          )}

          {/* Mac Control live-mirror panel — opens on tool-produced managed
              screenshot frames; panel polling frames only refresh an already
              open panel and must not re-open after a session switch. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "mac-control" && (
            <MacControlPanel
              panelWidth={rightPanelWidth}
              onPanelWidthChange={setRightPanelWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              reservedMainWidth={rightPanelReservedMainWidth}
              onClose={() => {
                macControlPanelDismissedRef.current = true
                setShowMacControlPanel(false)
              }}
            />
          )}

          {/* Team Panel */}
          {shouldRenderRightPanelContent &&
            renderedExclusiveRightPanel === "team" &&
            activeTeamId && (
              <TeamPanel
                teamId={activeTeamId}
                panelWidth={rightPanelWidth}
                onPanelWidthChange={setRightPanelWidth}
                collapsed={rightPanelCollapsed}
                overlay={rightPanelOverlay}
                reservedMainWidth={rightPanelReservedMainWidth}
                onClose={() => setShowTeamPanel(false)}
                onViewSession={setSubagentPreviewSessionId}
              />
            )}

          {/* Workspace 面板 — 聚合任务进度 / 碰到的文件 / 引用来源 */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "workspace" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("workspace.resizePanel", "Resize workspace panel")}
              maxWidth={860}
              reservedMainWidth={rightPanelReservedMainWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              contentKey="workspace"
            >
              <WorkspacePanel
                taskSnapshot={taskProgressSnapshot}
                taskExecutionState={workspaceTaskExecutionState}
                messages={session.messages}
                contextUsageOverride={contextUsage}
                onOpenDiff={diffPanel.openDiff}
                onPreviewFile={filePreview.openPreview}
                sessionId={session.currentSessionId}
                sessionMeta={currentSessionMeta}
                project={currentProject}
                effectiveWorkingDir={workspaceEffectiveWorkingDir}
                workingDirSource={workspaceWorkingDirSource}
                permissionMode={stream.permissionMode}
                planState={planMode.planState}
                activeModel={activeModel}
                agentName={session.agentName}
                reasoningEffort={reasoningEffort}
                availableModels={availableModels}
                currentAgentId={session.currentAgentId}
                compacting={compacting}
                onCompactContext={runCompactContextForCurrentSession}
                onCommandAction={handleCommandAction}
                onViewSystemPrompt={loadSystemPrompt}
                systemPromptLoading={systemPromptLoading}
                incognito={incognitoEnabled}
                turnActive={
                  workspaceTaskExecutionState === "running" ||
                  workspaceTaskExecutionState === "cancelling"
                }
                workflowRunsState={workflowTitleBarRuns}
                backgroundJobs={backgroundJobs.jobs}
                backgroundJobExpansionOverrides={backgroundJobExpansionOverrides}
                onBackgroundJobExpandedChange={handleBackgroundJobExpandedChange}
                onOpenBackgroundJobs={openBackgroundJobsPanel}
                onOpenBrowserPanel={openBrowserPanel}
                onViewSubagentSession={setSubagentPreviewSessionId}
                onEnsureSession={ensureWorkflowSession}
                draftWorkflowMode={draftWorkflowMode}
                onDraftWorkflowModeChange={setDraftWorkflowMode}
                onClose={() => {
                  workspacePanelDismissedRef.current = true
                  setShowWorkspacePanel(false)
                }}
              />
            </RightPanelShell>
          )}

          {/* Background-jobs panel (R4) — session jobs (cancellable) + a
              read-only mirror of global local-model jobs. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "background-jobs" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("backgroundJobs.resizePanel", "Resize background jobs panel")}
              maxWidth={860}
              reservedMainWidth={rightPanelReservedMainWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              contentKey="background-jobs"
            >
              <BackgroundJobsPanel
                jobs={backgroundJobs.jobs}
                jobExpansionOverrides={backgroundJobExpansionOverrides}
                onJobExpandedChange={handleBackgroundJobExpandedChange}
                onClose={closeBackgroundJobsPanel}
                onViewSubagentSession={setSubagentPreviewSessionId}
              />
            </RightPanelShell>
          )}

          {/* File preview panel — single-file viewer opened from Markdown
              links / attachments / the workspace panel (file-operations
              unification). Reuses the file-browser FilePreviewPane. */}
          {shouldRenderRightPanelContent && renderedExclusiveRightPanel === "preview" && (
            <RightPanelShell
              width={rightPanelWidth}
              onWidthChange={setRightPanelWidth}
              resizeLabel={t("filePreview.resizePanel", "Resize preview panel")}
              maxWidth={860}
              maximized={filePreviewMaximized}
              reservedMainWidth={rightPanelReservedMainWidth}
              collapsed={rightPanelCollapsed}
              overlay={rightPanelOverlay}
              contentKey="preview"
            >
              <FilePreviewPanel
                target={filePreview.target}
                sessionId={session.currentSessionId}
                maximized={filePreviewMaximized}
                onToggleMaximize={() => setFilePreviewMaximized((v) => !v)}
                onClose={() => {
                  setFilePreviewMaximized(false)
                  filePreview.closePreview()
                }}
              />
            </RightPanelShell>
          )}
        </div>
      </div>

      <HandoverDialog
        open={!!handoverSessionId}
        onOpenChange={(o) => {
          if (!o) setHandoverSessionId(null)
        }}
        sessionId={handoverSessionId}
      />

      <SubagentSessionDialog
        sessionId={subagentPreviewSessionId}
        agents={session.agents}
        onOpenChange={(open) => {
          if (!open) setSubagentPreviewSessionId(null)
        }}
        onOpenNestedSession={setSubagentPreviewSessionId}
      />
    </>
  )
}
