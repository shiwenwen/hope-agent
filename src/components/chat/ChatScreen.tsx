import { useState, useRef, useEffect, useLayoutEffect, useCallback, useMemo } from "react"
import { toast } from "sonner"
import { getTransport, useTransport } from "@/lib/transport-provider"
import { parsePayload, TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { subscribeAskAiQuotes } from "@/lib/manual/askAi"
import type { SettingsSection } from "@/components/settings/types"
import type { KnowledgeFocusTarget } from "@/components/knowledge/knowledgeFocus"
import { requestMemoryFocus } from "@/components/settings/memory-panel/memoryFocus"
import { memorySourceLabel } from "./message/memoryTraceFormat"
import {
  forkComposerDraftForMessage,
  forkSessionRequestForMessage,
  resendComposerDraftForMessage,
  type ForkComposerDraft,
} from "./message/messageFork"
import { BrowserExtensionNudge } from "./BrowserExtensionNudge"
import { useViewportMediaQuery } from "@/hooks/useViewportMediaQuery"
import { useViewportWidth } from "@/hooks/useViewportWidth"
import { useReadableSurface } from "@/hooks/useReadableSurface"
import { cn } from "@/lib/utils"
import {
  Bot,
  Brain,
  ClipboardList,
  Eye,
  FolderOpen,
  GitCompare,
  GitFork,
  GitPullRequest,
  Globe,
  Layers,
  LayoutDashboard,
  Monitor,
  MousePointer2,
  Users,
  type LucideIcon,
} from "lucide-react"
import type {
  ActiveMemoryRecall,
  ActiveMemoryRecallEvent,
  ActiveModel,
  AvailableModel,
  ChatRuntimeDefaults,
  ForkSessionResult,
  Message,
  PendingFileQuote,
  PendingMessageQuote,
  SessionMessage,
  SessionMeta,
  SessionMode,
  SandboxMode,
} from "@/types/chat"
import type { QuickPromptAddResult, QuickPromptConfig, QuickPromptItem } from "@/types/quickPrompts"
import { normalizeEffortForModel } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import type { CommandResult } from "./slash-commands/types"
import {
  newMentionId,
  slashSkillMentionBinding,
  type ComposerMentionKind,
} from "./mentions/typedMentions"
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
import SideChatPanel, { type SideChatSeed } from "@/components/chat/SideChatPanel"
import { SideChatTray } from "@/components/chat/SideChatTray"
import { FileBrowserPanel } from "@/components/chat/FileBrowserPanel"
import type { QuotePayload } from "@/components/chat/project/file-browser/FilePreviewPane"
import type { FileBrowserDirectoryReveal } from "@/components/chat/project/file-browser/FileBrowserView"
import type { ProjectFolderIdentity } from "@/components/chat/project/fileQuoteTarget"
import { projectSourceFoldersForSession } from "@/components/chat/project/projectSourceFolders"
import type { IncognitoDisabledReason } from "@/components/chat/input/IncognitoToggle"
import ChatTitleBar from "@/components/chat/ChatTitleBar"
import HandoverDialog from "@/components/chat/HandoverDialog"
import CronJobForm from "@/components/cron/CronJobForm"
import MessageList, { CHAT_CONTENT_MAX_WIDTH_PX } from "@/components/chat/MessageList"
import { environmentInsetWidth } from "@/components/chat/environmentInset"
import { ChatWelcomeHero } from "@/components/chat/ChatWelcomeHero"
import CrashRecoveryBanner from "@/components/common/CrashRecoveryBanner"
import CanvasPanel from "@/components/chat/CanvasPanel"
import BrowserPanel from "@/components/chat/BrowserPanel"
import MacControlPanel from "@/components/chat/MacControlPanel"
import { FloatingPanelLayer } from "@/components/chat/right-panel/FloatingPanelLayer"
import { useFloatingPanels, type FloatablePanel } from "@/hooks/useFloatingPanels"
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
import {
  activeMemoryRecallToUsedRefs,
  computeContextUsage,
  formatContextUsage,
  shouldSendDraftWorkflowMode,
} from "./chatUtils"
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
import { previewTargetMime, useFilePreview, type PreviewTarget } from "./files/useFilePreview"
import { useFileTabs, type FileTabFileReveal } from "./files/useFileTabs"
import FilePreviewPanel from "./files/FilePreviewPanel"
import { FileActionsContext, type FileActionsContextValue } from "./files/fileActionsContext"
import WorkspacePanel, { type WorkspaceFocusRequest } from "./workspace/WorkspacePanel"
import { confirmDiscardDirtyFileEditors } from "./files/fileDirtyRegistry"
import { PullRequestPanel } from "./workspace/PullRequestPanel"
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
import SubagentPanel, { type SubagentPanelSelectRequest } from "./subagent/SubagentPanel"
import { useSubagentRuns } from "./subagent/useSubagentRuns"
import type { SubagentOpenTarget } from "./subagent/subagentRunModel"
import { useModelState } from "./hooks/useModelState"
import SystemPromptDialog from "./SystemPromptDialog"
import { PlanPanel } from "./plan-mode/PlanPanel"
import type { BuiltPlanComment } from "./plan-mode/planCommentMessage"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { WorkbenchResizeHandle } from "./workbench/WorkbenchResizeHandle"
import { WorkbenchSurface } from "./workbench/WorkbenchSurface"
import { CHAT_IDEAL_MIN, useWorkbenchSizing } from "./workbench/useWorkbenchSizing"
import type { WorkbenchPanelId, WorkbenchTabItem } from "./workbench/types"
import { TerminalPanel } from "./terminal/TerminalPanel"
import { useProjects } from "./project/hooks/useProjects"
import { useProjectWorkingDir } from "./project/hooks/useProjectWorkingDir"
import {
  projectFocusLoadErrorToast,
  projectFocusMissingToast,
} from "./project/projectFocusFeedback"
import { chatKnowledgeReferenceAttachErrorToast } from "./chatKnowledgeReferenceFeedback"
import ProjectDialog from "./project/ProjectDialog"
import ProjectOverviewDialog from "./project/ProjectOverviewDialog"
import { useChatDisplayPreferences } from "./hooks/useChatDisplayPreferences"
import { ProjectSessionDraftBar } from "./project/ProjectSessionDraftBar"
import {
  createLocalProjectRuntimeDraft,
  type ProjectRuntimeDraft,
} from "./project/projectRuntimeDraft"
import {
  CHAT_SIDEBAR_DEFAULT_WIDTH,
  CHAT_SIDEBAR_LEGACY_DEFAULT_WIDTH,
  CHAT_SIDEBAR_MAX_WIDTH,
  CHAT_SIDEBAR_MIN_WIDTH,
  CHAT_SIDEBAR_WIDTH_STORAGE_KEY,
} from "./sidebar/types"
import { generateClientId, getLatestUserTurnKey } from "./chatScrollKeys"
import type { Project, ProjectMeta } from "@/types/project"
import type {
  ProjectBootstrapProgressEvent,
  ProjectBootstrapRun,
  ProjectSessionBootstrapInput,
} from "@/lib/transport"
import type { KbDraftAttachment } from "@/types/knowledge"
import type { ChatFocusTarget } from "@/components/chat/chatFocus"
import {
  chatFocusLoadErrorToast,
  chatFocusMissingMessageToast,
  chatFocusMissingSessionToast,
} from "./chatFocusFeedback"

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

/** A token to append to the chat composer on next render. `mention` preserves a
 * first-party selection as typed provenance; lookalike text without it remains
 * inert. `attachKbId` auto-attaches a referenced KB read-only when applicable. */
export interface ChatInsert {
  token: string
  attachKbId?: string
  mention?: {
    kind: ComposerMentionKind
    targetId: string
    displayLabel: string
  }
}

type SwitchSessionOptions = { targetMessageId?: number; highlightTerms?: string[] }
type SwitchSessionResult = "switched" | "confirmation_pending" | "cancelled"
type SideChatFocusContinuation = { sideSessionId: string; nonce: number }

type IncognitoLeaveIntent =
  | {
      type: "switchSession"
      sessionId: string
      opts?: SwitchSessionOptions
      sideChatFocus?: SideChatFocusContinuation
    }
  | { type: "newChat"; agentId: string; opts?: { incognito?: boolean } }
  | { type: "newProjectChat"; projectId: string; defaultAgentId?: string | null }

interface ChatScreenProps {
  /** Chat stays mounted across App views; only a selected view can read messages. */
  isViewVisible: boolean
  onOpenAgentSettings?: (agentId: string) => void
  onCodexReauth?: () => void
  initialSessionId?: string
  onSessionNavigated?: () => void
  onUnreadCountChange?: (count: number) => void
  /** Incremented when the already-active Conversations icon is clicked. */
  unreadFocusSignal?: number
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
  sessionsRefreshTrigger?: number
  onCurrentProjectChange?: (projectId: string | null) => void
  externalChatFocus?: (ChatFocusTarget & { nonce: number }) | null
  onExternalChatFocusHandled?: (nonce: number) => void
  /** A cross-space excerpt to stage in the currently active composer. */
  externalFileQuote?: { quote: PendingFileQuote; nonce: number } | null
  onExternalFileQuoteHandled?: (nonce: number) => void
  externalProjectFocus?: { projectId: string; nonce: number } | null
  onExternalProjectFocusHandled?: (nonce: number) => void
  /** Token and optional typed provenance to append on next render. */
  pendingChatInsert?: ChatInsert
  /** Called once the insert has been consumed so App can clear the pending slot. */
  onChatInsertConsumed?: () => void
  /** 设计空间「实现到代码」：目标会话就绪后把 message 经正常发送路径发出（一次性）。 */
  pendingAutoSend?: { sessionId: string; message: string; nonce: number }
  /** Called once the auto-send has fired so App can clear the pending slot. */
  onAutoSendConsumed?: (nonce: number) => void
  /** Open the settings view, optionally to a specific section. */
  onOpenSettings?: (section?: SettingsSection, modelTab?: string) => void
  /** Open the Knowledge Space view. */
  onOpenKnowledge?: (target?: KnowledgeFocusTarget) => void
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

type ExclusiveRightPanel = WorkbenchPanelId
type ExclusiveRightPanelVisibility = Record<ExclusiveRightPanel, boolean>

const EXCLUSIVE_RIGHT_PANEL_ORDER: readonly ExclusiveRightPanel[] = [
  "diff",
  "pull-request",
  "plan",
  "files",
  "browser",
  "mac-control",
  "canvas",
  "team",
  "background-jobs",
  "subagent",
  "workspace",
  "preview",
]

function isExclusiveRightPanel(value: string): value is ExclusiveRightPanel {
  return EXCLUSIVE_RIGHT_PANEL_ORDER.includes(value as ExclusiveRightPanel)
}

const EMPTY_RIGHT_PANEL_VISIBILITY: ExclusiveRightPanelVisibility = {
  workspace: false,
  "pull-request": false,
  diff: false,
  plan: false,
  files: false,
  browser: false,
  "mac-control": false,
  canvas: false,
  team: false,
  "background-jobs": false,
  subagent: false,
  preview: false,
}

const EXCLUSIVE_RIGHT_PANEL_ICONS: Record<ExclusiveRightPanel, LucideIcon> = {
  workspace: LayoutDashboard,
  "pull-request": GitPullRequest,
  diff: GitCompare,
  plan: ClipboardList,
  files: FolderOpen,
  browser: Globe,
  "mac-control": MousePointer2,
  canvas: Monitor,
  team: Users,
  "background-jobs": Layers,
  subagent: Bot,
  preview: Eye,
}

const EXCLUSIVE_RIGHT_PANEL_LABEL_KEYS: Record<ExclusiveRightPanel, string> = {
  workspace: "workspace.panelTitle",
  "pull-request": "workspace.git.pullRequestPanelTitle",
  diff: "diffPanel.title",
  plan: "planMode.panelTitle",
  files: "fileBrowser.panelTitle",
  browser: "browser.panelTitle",
  "mac-control": "macControl.panelTitle",
  canvas: "canvas.panelTitle",
  team: "team.panelTitle",
  "background-jobs": "backgroundJobs.panelTitle",
  subagent: "subagentPanel.title",
  preview: "filePreview.panelTitle",
}

const PERSISTENT_RIGHT_PANEL_ORDER: readonly ExclusiveRightPanel[] = [
  "workspace",
  "files",
  "background-jobs",
  "subagent",
]

const CHAT_MAIN_MIN_INTERACTIVE_WIDTH = 420
/** From the column's right edge: 316 card + 16 card offset + 16 gutter. */
const ENVIRONMENT_CARD_LANE_PX = 348
const RESPONSIVE_PANEL_HYSTERESIS = 120
/** Workbench / file-tab scope for a chat that has not been persisted yet. The
 *  first message renames this scope to the new session id (`sessionPromotedRef`)
 *  so the draft's open tabs follow it. */
const DRAFT_WORKBENCH_KEY = "__draft__"
const EMPTY_WORKBENCH_ORDER: string[] = []

interface MacControlFrameOpenHint {
  mediaId?: string | null
  path?: string | null
}

interface RestorableWorkbenchSessionState {
  workspace: boolean
  browser: boolean
  macControl: boolean
  backgroundJobs: boolean
  subagent: boolean
  activePanel: ExclusiveRightPanel | null
  collapsed: boolean
  dismissed: {
    workspace: boolean
    browser: boolean
    macControl: boolean
    backgroundJobs: boolean
  }
}

function clampChatSidebarWidth(width: number): number {
  return Math.min(CHAT_SIDEBAR_MAX_WIDTH, Math.max(CHAT_SIDEBAR_MIN_WIDTH, width))
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
    "I just created or updated the durable Goal for this session.",
    "Use the current Active Goal context as the source of truth, acknowledge briefly, then begin making progress.",
    "Do not expose internal goal ids, revision ids, or slash-command help unless I ask for status details.",
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

function attachActiveMemoryToLatestAssistant(
  messages: Message[],
  turnKey: string | null,
  recall: ActiveMemoryRecall,
): Message[] {
  if (!turnKey) return messages
  if (getLatestUserTurnKey(messages) !== turnKey) return messages

  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i]
    if (msg.role === "user") break
    if (msg.role !== "assistant") continue
    const usedMemoryRefs = activeMemoryRecallToUsedRefs(recall)
    if (msg.activeMemory === recall && msg.usedMemoryRefs?.length === usedMemoryRefs.length) {
      return messages
    }
    const next = messages.slice()
    next[i] = {
      ...msg,
      activeMemory: recall,
      ...(usedMemoryRefs.length > 0 ? { usedMemoryRefs } : {}),
    }
    return next
  }

  return messages
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
  isViewVisible,
  onOpenAgentSettings,
  onCodexReauth,
  initialSessionId,
  onSessionNavigated,
  onUnreadCountChange,
  unreadFocusSignal,
  onOpenDashboardTab,
  sessionsRefreshTrigger,
  onCurrentProjectChange,
  externalChatFocus,
  onExternalChatFocusHandled,
  externalFileQuote,
  onExternalFileQuoteHandled,
  externalProjectFocus,
  onExternalProjectFocusHandled,
  pendingChatInsert,
  onChatInsertConsumed,
  pendingAutoSend,
  onAutoSendConsumed,
  onOpenSettings,
  onOpenKnowledge,
}: ChatScreenProps) {
  const { t } = useTranslation()
  const transport = useTransport()
  const [messageTailVisible, setMessageTailVisible] = useState(true)
  const surfaceReadable = useReadableSurface(isViewVisible)
  const transcriptSurfaceReadable = surfaceReadable && messageTailVisible
  const activeSessionReadableRef = useRef(false)

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
    handleTemperatureChange,
    resetSessionEffort,
    resetSessionTemperature,
  } = useModelState()
  const [unavailableModelPreference, setUnavailableModelPreference] = useState<string | null>(null)

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
  // Only an auto-collapse may be auto-undone, and re-opening the workbench by
  // hand at a narrow width must survive the next resize tick.
  const autoCollapsedRightPanelRef = useRef(false)
  const manualRightPanelExpandedOverrideRef = useRef(false)

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

  const { displayMode: defaultDisplayMode, autoCollapseCompletedTurns } =
    useChatDisplayPreferences()

  const [canvasPanelOpen, setCanvasPanelOpen] = useState(false)

  // Right side diff panel (write/edit/apply_patch metadata viewer)
  const diffPanel = useDiffPanel()

  // Tabs are created only on explicit request, never by picking a file.
  const fileTabs = useFileTabs()
  // Bare previews, for resources the tree cannot address.
  const filePreview = useFilePreview()
  // Workspace 面板：聚合任务进度 / 碰到的文件 / 引用来源。首次有内容时自动
  // 展开一次，用户关闭后本会话不再自动弹（dismissedRef 跟踪，仿 browser 面板）。
  const [showWorkspacePanel, setShowWorkspacePanel] = useState(false)
  const [workspaceFocusRequest, setWorkspaceFocusRequest] = useState<WorkspaceFocusRequest | null>(
    null,
  )
  const [pendingControlFocus, setPendingControlFocus] = useState<{
    sessionId: string
    kind: string
    itemId?: string
    nonce: number
  } | null>(null)
  const [showPullRequestPanel, setShowPullRequestPanel] = useState(false)
  const [pullRequestExpectedUrl, setPullRequestExpectedUrl] = useState<string | null>(null)
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

  // Sub-agent panel: a right-side exclusive panel listing this session's
  // sub-agent runs + the selected run's live transcript. Opened by clicking an
  // inline sub-agent chip (via `subagentPanelSelectRequest`) or the title-bar
  // button. Only appears once the session has spawned at least one sub-agent.
  const [showSubagentPanel, setShowSubagentPanel] = useState(false)
  const [subagentPanelSelectRequest, setSubagentPanelSelectRequest] =
    useState<SubagentPanelSelectRequest | null>(null)
  const subagentSelectNonceRef = useRef(0)

  // Browser live-mirror panel. Auto-opens on the **first** `browser:frame`
  // push of a session. After the user manually closes it, further frames in
  // the same session never re-pop the panel — `browserPanelDismissedRef`
  // tracks the dismissal until a session switch resets it.
  const [showBrowserPanel, setShowBrowserPanel] = useState(false)
  const browserPanelDismissedRef = useRef(false)
  const [composerFocusSignal, setComposerFocusSignal] = useState<number | undefined>(undefined)
  const pendingForkComposerRef = useRef<{
    sessionId: string
    draft: ForkComposerDraft
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
  const [scheduleSessionTarget, setScheduleSessionTarget] = useState<{
    id: string
    title: string
  } | null>(null)
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
  const [draftProjectRuntime, setDraftProjectRuntime] = useState<ProjectRuntimeDraft>(() =>
    createLocalProjectRuntimeDraft(),
  )
  const [projectBootstrapProgress, setProjectBootstrapProgress] = useState<{
    stage: string
    error: string | null
  } | null>(null)

  // Plan mode state (declared early so useChatStream can access it)
  const [planModeState, setPlanModeState] = useState<
    "off" | "planning" | "review" | "executing" | "completed"
  >("off")

  // Shared stream identity state for dedup across the primary per-call
  // Channel/WS path (useChatStream) and the EventBus reattach path
  // (useChatStreamReattach). Cursors are keyed by session + stream id so a
  // delayed frame from a finished stream cannot mutate the next DB snapshot.
  const streamSeqRef = useRef<Map<string, number>>(new Map())
  const endedStreamIdsRef = useRef<Map<string, Set<string>>>(new Map())
  const manualModelOverrideRef = useRef<ActiveModel | null>(null)

  // ── Projects ────────────────────────────────────────────────
  // Holds the actually-readable session id for the project-unread rollup. Lives
  // above the session hook (which feeds this hook's reload callback), so the
  // value is delivered by ref and refreshed via the effect below on switch.
  const activeSessionIdForProjectsRef = useRef<string | null>(null)
  const {
    projects,
    loading: projectsLoading,
    loaded: projectsLoaded,
    error: projectsError,
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
    applyModelForDisplay,
    initialSessionId,
    onSessionNavigated,
    onUnreadCountChange,
    onSidebarAggregatesChanged: refreshProjectAggregates,
    activeSessionReadable: transcriptSurfaceReadable,
    activeSessionReadableRef,
  })
  const activeSessionReadable = transcriptSurfaceReadable && session.currentSessionContentReady
  activeSessionReadableRef.current = activeSessionReadable

  // R4: live background-jobs subscription (see show-state above) — drives the
  // header badge count, the background-jobs panel, and the workspace section.
  const backgroundJobs = useBackgroundJobs(session.currentSessionId)

  // Live sub-agent runs for the current session — drives the panel list, the
  // title-bar running badge, and chip → run resolution.
  const subagentRuns = useSubagentRuns(session.currentSessionId)

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
  const [sideChats, setSideChats] = useState<SessionMeta[]>([])
  const [sideChatStateSourceId, setSideChatStateSourceId] = useState<string | null>(null)
  const [activeSideChatId, setActiveSideChatId] = useState<string | null>(null)
  const [sideChatPanelOpen, setSideChatPanelOpen] = useState(false)
  const [sideChatCreatingSourceId, setSideChatCreatingSourceId] = useState<string | null>(null)
  const [sideChatSeed, setSideChatSeed] = useState<SideChatSeed | null>(null)
  const [pendingSideChatFocus, setPendingSideChatFocus] = useState<{
    sourceSessionId: string
    sideSessionId: string
    nonce: number
  } | null>(null)
  const sideChatSeedNonceRef = useRef(0)
  const canUseSideChat =
    !!currentSessionMeta &&
    (currentSessionMeta.kind ?? "regular") === "regular" &&
    !currentSessionMeta.incognito &&
    !currentSessionMeta.channelInfo &&
    !currentSessionMeta.parentSessionId &&
    !currentSessionMeta.isCron
  const sideChatSourceId = canUseSideChat ? session.currentSessionId : null
  const sideChatSourceIdRef = useRef<string | null>(sideChatSourceId)
  sideChatSourceIdRef.current = sideChatSourceId
  const sideChatStateIsCurrent = sideChatStateSourceId === sideChatSourceId
  const visibleSideChats = sideChatStateIsCurrent ? sideChats : []
  const visibleActiveSideChatId = sideChatStateIsCurrent ? activeSideChatId : null
  const visibleActiveSideChat = visibleActiveSideChatId
    ? (visibleSideChats.find((chat) => chat.id === visibleActiveSideChatId) ?? null)
    : null
  const visibleSideChatPanelOpen = sideChatStateIsCurrent && sideChatPanelOpen
  const sideChatCreating = sideChatCreatingSourceId === sideChatSourceId

  const refreshSideChats = useCallback(async (): Promise<SessionMeta[]> => {
    const sourceSessionId = sideChatSourceIdRef.current
    if (!sourceSessionId) return []
    try {
      const chats = await getTransport().call<SessionMeta[]>("list_side_chats_cmd", {
        sessionId: sourceSessionId,
      })
      if (sideChatSourceIdRef.current !== sourceSessionId) return []
      setSideChatStateSourceId(sourceSessionId)
      setSideChats(chats)
      return chats
    } catch (error) {
      logger.error("ui", "ChatScreen::listSideChats", "Failed to list side chats", error)
      return []
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const sourceSessionId = sideChatSourceId
    setSideChatStateSourceId(sourceSessionId)
    setSideChats([])
    setActiveSideChatId(null)
    setSideChatPanelOpen(false)
    setSideChatSeed(null)
    if (!sourceSessionId) {
      return
    }
    getTransport()
      .call<SessionMeta[]>("list_side_chats_cmd", { sessionId: sourceSessionId })
      .then((chats) => {
        if (cancelled || sideChatSourceIdRef.current !== sourceSessionId) return
        setSideChats(chats)
      })
      .catch((error) => {
        if (!cancelled) {
          logger.error("ui", "ChatScreen::loadSideChats", "Failed to load side chats", error)
        }
      })
    return () => {
      cancelled = true
    }
  }, [sideChatSourceId])

  const revealSideChat = useCallback(
    async (
      sessionId: string,
      seed?: Omit<SideChatSeed, "nonce">,
      expectedSourceSessionId = sideChatSourceIdRef.current,
    ) => {
      if (!expectedSourceSessionId || sideChatSourceIdRef.current !== expectedSourceSessionId)
        return false
      let chat =
        sideChatStateSourceId === expectedSourceSessionId
          ? sideChats.find((item) => item.id === sessionId)
          : undefined
      if (!chat) {
        try {
          chat =
            (await getTransport().call<SessionMeta | null>("get_session_cmd", {
              sessionId,
            })) ?? undefined
        } catch (error) {
          logger.error("ui", "ChatScreen::revealSideChat", "Failed to load side chat", error)
          return false
        }
      }
      if (!chat) return false
      if (
        sideChatSourceIdRef.current !== expectedSourceSessionId ||
        chat.kind !== "side" ||
        chat.forkedFromSessionId !== expectedSourceSessionId
      ) {
        return false
      }
      setSideChatStateSourceId(expectedSourceSessionId)
      setSideChats((current) =>
        current.some((item) => item.id === chat.id) ? current : [...current, chat],
      )
      setActiveSideChatId(sessionId)
      setSideChatPanelOpen(true)
      if (seed) {
        sideChatSeedNonceRef.current += 1
        setSideChatSeed({ ...seed, nonce: sideChatSeedNonceRef.current })
      } else {
        setSideChatSeed(null)
      }
      return true
    },
    [sideChatStateSourceId, sideChats],
  )

  useEffect(() => {
    if (!pendingSideChatFocus) return
    if (sideChatSourceId !== pendingSideChatFocus.sourceSessionId) return
    let cancelled = false
    const request = pendingSideChatFocus
    void revealSideChat(request.sideSessionId, undefined, request.sourceSessionId).then(
      (opened) => {
        if (cancelled) return
        setPendingSideChatFocus((current) => (current?.nonce === request.nonce ? null : current))
        if (!opened) {
          const failureToast = chatFocusMissingSessionToast(t)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
        }
        onExternalChatFocusHandled?.(request.nonce)
      },
    )
    return () => {
      cancelled = true
    }
  }, [onExternalChatFocusHandled, pendingSideChatFocus, revealSideChat, sideChatSourceId, t])

  const createSideChat = useCallback(
    async (seed?: Omit<SideChatSeed, "nonce">) => {
      const sourceSessionId = sideChatSourceIdRef.current
      if (!sourceSessionId || sideChatCreatingSourceId === sourceSessionId) return
      setSideChatCreatingSourceId(sourceSessionId)
      try {
        const chat = await getTransport().call<SessionMeta>("create_side_chat_cmd", {
          sessionId: sourceSessionId,
        })
        if (sideChatSourceIdRef.current === sourceSessionId) {
          await revealSideChat(chat.id, seed, sourceSessionId)
        }
      } catch (error) {
        logger.error("ui", "ChatScreen::createSideChat", "Failed to create side chat", error)
        toast.error(t("chat.sideChat.createFailed", "无法创建侧聊"))
      } finally {
        setSideChatCreatingSourceId((current) => (current === sourceSessionId ? null : current))
      }
    },
    [revealSideChat, sideChatCreatingSourceId, t],
  )
  const handleSideChatDeleted = useCallback((sessionId: string) => {
    setSideChats((current) => current.filter((chat) => chat.id !== sessionId))
    setActiveSideChatId(null)
    setSideChatPanelOpen(false)
    setSideChatSeed(null)
  }, [])
  const canScheduleCurrentSession =
    !!currentSessionMeta &&
    (currentSessionMeta.kind ?? "regular") === "regular" &&
    !currentSessionMeta.incognito &&
    !currentSessionMeta.channelInfo &&
    !currentSessionMeta.parentSessionId &&
    !currentSessionMeta.isCron
  const openScheduleSession = useCallback(
    (sessionId: string) => {
      if (!canScheduleCurrentSession || currentSessionMeta?.id !== sessionId) return
      setScheduleSessionTarget({
        id: sessionId,
        title: currentSessionMeta.title?.trim() || t("chat.newChat"),
      })
    },
    [canScheduleCurrentSession, currentSessionMeta, t],
  )
  const forkSourceSession = useMemo(() => {
    const sourceId = currentSessionMeta?.forkedFromSessionId
    if (!sourceId) return null
    const live = session.sessions.find((s) => s.id === sourceId)
    return {
      id: sourceId,
      title:
        live?.title ||
        currentSessionMeta?.forkedFromSessionTitle ||
        t("chat.fork.sourceFallback", "原会话"),
    }
  }, [
    currentSessionMeta?.forkedFromSessionId,
    currentSessionMeta?.forkedFromSessionTitle,
    session.sessions,
    t,
  ])
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
  const currentSessionMessages = session.messages
  const updateSessionMessages = session.updateSessionMessages
  const chatGoal = useGoal(currentSessionId, { incognito: incognitoEnabled })
  const displayMode = defaultDisplayMode
  const setAgentName = session.setAgentName
  const updateSessionMeta = session.updateSessionMeta
  const rawHandleSwitchSession = session.handleSwitchSession
  const lastExternalChatFocusNonceRef = useRef<number | null>(null)
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

  // Keep the project rollup aligned with the same real-reading predicate used
  // by the global badge. A mounted-but-hidden or background window must not
  // suppress unread state for its current session.
  useEffect(() => {
    activeSessionIdForProjectsRef.current = activeSessionReadable
      ? (currentSessionId ?? null)
      : null
    void reloadProjects()
  }, [activeSessionReadable, currentSessionId, reloadProjects])

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

  const handleSessionEffortChange = useCallback(
    async (effort: string, options?: { applyToAgentDefault?: boolean }) => {
      const sid = session.currentSessionId
      if (sid) {
        updateSessionMeta(sid, (prev) =>
          prev.reasoningEffort === effort ? prev : { ...prev, reasoningEffort: effort },
        )
      }
      await handleEffortChange(effort, sid, session.currentAgentId, options)
    },
    [handleEffortChange, session.currentAgentId, session.currentSessionId, updateSessionMeta],
  )

  const handleSessionEffortReset = useCallback(async () => {
    const sid = session.currentSessionId
    if (sid) {
      await resetSessionEffort(sid)
      return
    }
    const defaults = await getTransport().call<ChatRuntimeDefaults>("get_chat_runtime_defaults", {
      agentId: session.currentAgentId,
    })
    setReasoningEffort(defaults.reasoningEffort)
  }, [resetSessionEffort, session.currentAgentId, session.currentSessionId, setReasoningEffort])

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
      if (
        !confirmDiscardDirtyFileEditors(
          t("fileEditor.unsavedBody", "Discard the current edits before leaving this file?"),
        )
      ) {
        return
      }
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
      setDraftProjectRuntime(createLocalProjectRuntimeDraft())
      setProjectBootstrapProgress(null)
      await handleNewChat(agentId)
    },
    [projects, handleNewChat, t],
  )

  const startNewChatNow = useCallback(
    async (agentId: string, opts?: { incognito?: boolean }) => {
      if (
        !confirmDiscardDirtyFileEditors(
          t("fileEditor.unsavedBody", "Discard the current edits before leaving this file?"),
        )
      ) {
        return
      }
      setDraftIncognito(opts?.incognito ?? false)
      setDraftKbAttachments([])
      // Leaving any project draft → drop the project / working-dir binding so it
      // can't leak into this plain draft (the currentSessionId transition effect
      // only fires on draft→materialized, not draft→draft).
      setDraftWorkingDir(null)
      setDraftProjectId(null)
      setDraftProjectRuntime(createLocalProjectRuntimeDraft())
      setProjectBootstrapProgress(null)
      await handleNewChat(agentId)
    },
    [handleNewChat, t],
  )

  const runIncognitoLeaveIntent = useCallback(
    async (intent: IncognitoLeaveIntent) => {
      switch (intent.type) {
        case "switchSession":
          if (await rawHandleSwitchSession(intent.sessionId, intent.opts)) {
            if (intent.sideChatFocus) {
              setPendingSideChatFocus({
                sourceSessionId: intent.sessionId,
                sideSessionId: intent.sideChatFocus.sideSessionId,
                nonce: intent.sideChatFocus.nonce,
              })
            }
          } else if (intent.sideChatFocus) {
            onExternalChatFocusHandled?.(intent.sideChatFocus.nonce)
          }
          break
        case "newChat":
          await startNewChatNow(intent.agentId, intent.opts)
          break
        case "newProjectChat":
          await startNewChatInProjectNow(intent.projectId, intent.defaultAgentId)
          break
      }
    },
    [onExternalChatFocusHandled, rawHandleSwitchSession, startNewChatInProjectNow, startNewChatNow],
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

  const handleIncognitoLeaveOpenChange = useCallback(
    (open: boolean) => {
      if (open) return
      const focusNonce =
        incognitoLeaveIntent?.type === "switchSession"
          ? incognitoLeaveIntent.sideChatFocus?.nonce
          : undefined
      setIncognitoLeaveIntent(null)
      if (focusNonce !== undefined) onExternalChatFocusHandled?.(focusNonce)
    },
    [incognitoLeaveIntent, onExternalChatFocusHandled],
  )

  const handleSwitchSession = useCallback(
    async (
      sessionId: string,
      opts?: SwitchSessionOptions,
      sideChatFocus?: SideChatFocusContinuation,
    ): Promise<SwitchSessionResult> => {
      if (!sessionId) return "cancelled"
      if (sessionId === session.currentSessionId) {
        return (await rawHandleSwitchSession(sessionId, opts)) ? "switched" : "cancelled"
      }
      if (
        requestIncognitoLeaveConfirmation({
          type: "switchSession",
          sessionId,
          opts,
          sideChatFocus,
        })
      ) {
        return "confirmation_pending"
      }
      return (await rawHandleSwitchSession(sessionId, opts)) ? "switched" : "cancelled"
    },
    [rawHandleSwitchSession, requestIncognitoLeaveConfirmation, session.currentSessionId],
  )

  useEffect(() => {
    if (!externalChatFocus) return
    if (lastExternalChatFocusNonceRef.current === externalChatFocus.nonce) return
    lastExternalChatFocusNonceRef.current = externalChatFocus.nonce
    setPendingSideChatFocus(null)
    ;(async () => {
      try {
        await reloadSessions()
        const sourceSession = await getTransport().call<SessionMeta | null>("get_session_cmd", {
          sessionId: externalChatFocus.sessionId,
        })
        if (!sourceSession) {
          const failureToast = chatFocusMissingSessionToast(t)
          toast.error(
            failureToast.title,
            failureToast.description ? { description: failureToast.description } : undefined,
          )
          onExternalChatFocusHandled?.(externalChatFocus.nonce)
          return
        }
        if (externalChatFocus.targetMessageId !== undefined) {
          const [messages] = await getTransport().call<
            [SessionMessage[], number, boolean, boolean]
          >("load_session_messages_around_cmd", {
            sessionId: externalChatFocus.sessionId,
            targetMessageId: externalChatFocus.targetMessageId,
            before: 1,
            after: 1,
          })
          if (!messages.some((message) => message.id === externalChatFocus.targetMessageId)) {
            const failureToast = chatFocusMissingMessageToast(t)
            toast.error(
              failureToast.title,
              failureToast.description ? { description: failureToast.description } : undefined,
            )
            await handleSwitchSession(externalChatFocus.sessionId)
            onExternalChatFocusHandled?.(externalChatFocus.nonce)
            return
          }
        }
        const sideChatFocus = externalChatFocus.sideSessionId
          ? { sideSessionId: externalChatFocus.sideSessionId, nonce: externalChatFocus.nonce }
          : undefined
        const switchResult = await handleSwitchSession(
          externalChatFocus.sessionId,
          { targetMessageId: externalChatFocus.targetMessageId },
          sideChatFocus,
        )
        if (switchResult === "confirmation_pending") return
        if (switchResult !== "switched") {
          onExternalChatFocusHandled?.(externalChatFocus.nonce)
          return
        }
        if (sideChatFocus) {
          setPendingSideChatFocus({
            sourceSessionId: externalChatFocus.sessionId,
            ...sideChatFocus,
          })
          return
        }
        if (externalChatFocus.controlTarget) {
          setPendingControlFocus({
            sessionId: externalChatFocus.sessionId,
            kind: externalChatFocus.controlTarget.kind,
            itemId: externalChatFocus.controlTarget.itemId,
            nonce: externalChatFocus.nonce,
          })
        }
        onExternalChatFocusHandled?.(externalChatFocus.nonce)
      } catch (error) {
        logger.warn("chat", "ChatScreen::externalChatFocus", "Failed to open source chat", error)
        const failureToast = chatFocusLoadErrorToast(t, error)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
        onExternalChatFocusHandled?.(externalChatFocus.nonce)
      }
    })()
  }, [externalChatFocus, handleSwitchSession, onExternalChatFocusHandled, reloadSessions, t])

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

  const handleProjectRuntimeDraftChange = useCallback((next: ProjectRuntimeDraft) => {
    setDraftProjectRuntime((previous) => ({
      ...next,
      requestId: next.baseRef ? next.requestId || previous.requestId || generateClientId() : "",
    }))
    setProjectBootstrapProgress(null)
  }, [])

  const draftProjectBootstrap = useMemo<ProjectSessionBootstrapInput | null>(() => {
    if (!draftProjectId || !draftProjectRuntime.baseRef || !draftProjectRuntime.requestId) {
      return null
    }
    return {
      requestId: draftProjectRuntime.requestId,
      launchMode: draftProjectRuntime.launchMode,
      baseRef: draftProjectRuntime.baseRef,
      includeLocalChanges: draftProjectRuntime.includeLocalChanges,
    }
  }, [draftProjectId, draftProjectRuntime])

  useEffect(() => {
    if (!draftProjectRuntime.requestId) return
    const requestId = draftProjectRuntime.requestId
    const transport = getTransport()
    const applyProgress = (event: ProjectBootstrapProgressEvent) => {
      if (event.requestId !== requestId) return
      const failed =
        event.status === "failed" || event.status === "cancelled" || event.status === "interrupted"
      setProjectBootstrapProgress({
        stage: event.stage,
        error: failed
          ? event.message || t("chat.projectRuntime.prepareFailed", "工作树准备失败")
          : null,
      })
      if (failed) {
        // A failed request id is durable and cannot be replayed. Keep the
        // selected branch but mint a fresh id for the user's next Send.
        setDraftProjectRuntime((current) =>
          current.requestId === requestId ? { ...current, requestId: generateClientId() } : current,
        )
      }
    }
    const unlisten = transport.listen("project:bootstrap_progress", (raw) => {
      const event = parsePayload<ProjectBootstrapProgressEvent>(raw)
      if (event) applyProgress(event)
    })
    const recover = () => {
      void transport
        .call<ProjectBootstrapRun | null>("get_project_bootstrap_run", { requestId })
        .then((run) => {
          if (!run) return
          applyProgress({
            requestId: run.id,
            status: run.status,
            stage: run.stage,
            sessionId: run.sessionId,
            worktreeId: run.worktreeId,
            message: run.errorMessage,
            errorCode: run.errorCode,
          })
        })
        .catch(() => undefined)
    }
    // The HTTP event socket can reconnect after progress frames were missed.
    // Re-read the durable run both now and whenever that socket reconnects.
    recover()
    const unlistenReconnect = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, recover)
    return () => {
      unlisten()
      unlistenReconnect()
    }
  }, [draftProjectRuntime.requestId, t])

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
      const [models, active, agentConfig, runtimeDefaults] = await Promise.all([
        getTransport().call<AvailableModel[]>("get_available_models"),
        getTransport().call<ActiveModel | null>("get_active_model"),
        getTransport()
          .call<AgentConfig>("get_agent_config", { id: currentAgentId })
          .catch(() => null),
        getTransport().call<ChatRuntimeDefaults>("get_chat_runtime_defaults", {
          ...(currentSessionId ? { sessionId: currentSessionId } : {}),
          agentId: currentAgentId,
        }),
      ])

      setAvailableModels(models)
      globalActiveModelRef.current = active

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

      const displayModel =
        manualModel && manualOverride ? manualOverride : (runtimeDefaults.model ?? null)

      setActiveModel(displayModel)
      const displayModelInfo = displayModel
        ? models.find(
            (m) => m.providerId === displayModel.providerId && m.modelId === displayModel.modelId,
          )
        : undefined
      const effort = runtimeDefaults.reasoningEffort
      setReasoningEffort(normalizeEffortForModel(displayModelInfo, effort, t))
      setSessionTemperature(runtimeDefaults.temperature ?? null)
      setUnavailableModelPreference(
        !runtimeDefaults.preferredModelAvailable && runtimeDefaults.preferredModel
          ? `${runtimeDefaults.preferredModel.providerId}::${runtimeDefaults.preferredModel.modelId}`
          : null,
      )

      if (agentConfig?.name) {
        setAgentName(agentConfig.name)
      }
    } catch (e) {
      logger.error("ui", "ChatScreen::refreshRuntimeModelState", "Failed to refresh model state", e)
    }
  }, [
    currentSessionId,
    currentAgentId,
    globalActiveModelRef,
    setActiveModel,
    setAgentName,
    setAvailableModels,
    setReasoningEffort,
    setSessionTemperature,
    t,
  ])

  const handleManualModelChange = useCallback(
    async (key: string, options?: { applyToAgentDefault?: boolean }) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      setUnavailableModelPreference(null)
      manualModelOverrideRef.current = currentSessionId ? null : { providerId, modelId }
      await handleModelChange(key, currentSessionId, session.currentAgentId, options)
    },
    [handleModelChange, currentSessionId, session.currentAgentId],
  )

  const handleSessionTemperatureChange = useCallback(
    async (temperature: number | null, options?: { applyToAgentDefault?: boolean }) => {
      const sid = session.currentSessionId
      if (temperature == null) {
        if (sid) await resetSessionTemperature(sid)
        else setSessionTemperature(null)
        return
      }
      await handleTemperatureChange(temperature, sid, session.currentAgentId, options)
      if (sid) {
        updateSessionMeta(sid, (previous) => ({ ...previous, temperature }))
      }
    },
    [
      handleTemperatureChange,
      resetSessionTemperature,
      session.currentAgentId,
      session.currentSessionId,
      setSessionTemperature,
      updateSessionMeta,
    ],
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
      setDraftProjectRuntime(createLocalProjectRuntimeDraft())
      setProjectBootstrapProgress(null)
      materializedProjectDraftSessionIdRef.current = null
    }
  }, [session.currentSessionId, currentSessionMeta, draftProjectId])
  const projectWorkingDir = useProjectWorkingDir(
    transport,
    effectiveProjectId,
    currentProject?.workingDir ?? null,
  )
  const activeSideChatProject = useMemo(
    () =>
      visibleActiveSideChat?.projectId
        ? (projects.find((project) => project.id === visibleActiveSideChat.projectId) ?? null)
        : null,
    [projects, visibleActiveSideChat?.projectId],
  )
  const activeSideChatProjectWorkingDir = useProjectWorkingDir(
    transport,
    visibleActiveSideChat?.projectId ?? null,
    activeSideChatProject?.workingDir ?? null,
  )
  const activeSideChatWorkingDir =
    visibleActiveSideChat?.workingDir ?? activeSideChatProjectWorkingDir
  const effectiveWorkingDir = sessionWorkingDir ?? projectWorkingDir
  const projectFileBrowserRoots = useMemo(
    () =>
      projectSourceFoldersForSession(
        currentProject?.linkedDirs ?? [],
        sessionWorkingDir,
        projectWorkingDir,
      ),
    [currentProject?.linkedDirs, projectWorkingDir, sessionWorkingDir],
  )
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

  useEffect(() => {
    if (!externalProjectFocus) return
    const project = projects.find((candidate) => candidate.id === externalProjectFocus.projectId)
    if (project) {
      setProjectOverviewTargetId(project.id)
      setProjectOverviewOpen(true)
      onExternalProjectFocusHandled?.(externalProjectFocus.nonce)
      return
    }
    if (projectsLoading || !projectsLoaded) return
    const failureToast = projectsError
      ? projectFocusLoadErrorToast(t, projectsError)
      : projectFocusMissingToast(t)
    toast.error(
      failureToast.title,
      failureToast.description ? { description: failureToast.description } : undefined,
    )
    onExternalProjectFocusHandled?.(externalProjectFocus.nonce)
  }, [
    externalProjectFocus,
    onExternalProjectFocusHandled,
    projects,
    projectsError,
    projectsLoaded,
    projectsLoading,
    t,
  ])

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

  // Settings can restore an archived conversation while ChatScreen remains
  // mounted behind the settings view. Refresh both session rows and project
  // counts so the restored conversation is visible immediately on return.
  useEffect(() => {
    const handleArchiveChanged = () => {
      void Promise.all([reloadSessions(), reloadProjects()])
    }
    window.addEventListener("hope:session-archive-changed", handleArchiveChanged)
    return () => window.removeEventListener("hope:session-archive-changed", handleArchiveChanged)
  }, [reloadProjects, reloadSessions])

  const handleIncognitoChange = useCallback(
    (enabled: boolean) => {
      if (session.currentSessionId) return
      // Project + incognito are mutually exclusive — a project draft can't go
      // incognito (the title-bar toggle is hidden via incognitoDisabledReason).
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

  // Assigned further down, once the workbench scope refs exist; the indirection
  // keeps the stream option stable across renders.
  const sessionPromotedRef = useRef<(sessionId: string) => void>(() => {})
  const handleSessionPromoted = useCallback((sessionId: string) => {
    sessionPromotedRef.current(sessionId)
  }, [])

  // ── Stream Hook ─────────────────────────────────────────────
  const stream = useChatStream({
    uiSurface: "main_chat",
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
    onSessionPromoted: handleSessionPromoted,
    sessions: session.sessions,
    agents: session.agents,
    manualModelOverrideRef,
    reloadSessions: refreshUnreadState,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    planMode: planModeState,
    temperatureOverride: sessionTemperature,
    reasoningEffort,
    incognitoEnabled,
    draftWorkingDir,
    mentionWorkingDir: workspaceEffectiveWorkingDir,
    draftProjectId,
    draftProjectBootstrap,
    onProjectBootstrapFailure: (message) => {
      setProjectBootstrapProgress({ stage: "failed", error: message })
      setDraftProjectRuntime((current) =>
        current.baseRef ? { ...current, requestId: generateClientId() } : current,
      )
    },
    draftKbAttachments,
    onSandboxModeSynced: handleSandboxModeSynced,
    parentInjectionDeltasViaChatStream: true,
  })

  const stageExternalFileQuote = stream.setPendingQuotes
  const restoreForkInput = stream.setInput
  const restoreForkAttachedFiles = stream.setAttachedFiles
  const restoreForkMessageQuotes = stream.setPendingMessageQuotes
  const lastExternalFileQuoteNonceRef = useRef<number | null>(null)
  useEffect(() => {
    if (!externalFileQuote) return
    if (lastExternalFileQuoteNonceRef.current === externalFileQuote.nonce) return
    lastExternalFileQuoteNonceRef.current = externalFileQuote.nonce
    stageExternalFileQuote((current) => [...current, externalFileQuote.quote])
    setComposerFocusSignal((current) => (current ?? 0) + 1)
    onExternalFileQuoteHandled?.(externalFileQuote.nonce)
  }, [externalFileQuote, onExternalFileQuoteHandled, stageExternalFileQuote])

  useEffect(() => {
    const pending = pendingForkComposerRef.current
    if (!pending || session.currentSessionId !== pending.sessionId) return
    pendingForkComposerRef.current = null
    restoreForkInput(pending.draft.text)
    restoreForkAttachedFiles(pending.draft.attachedFiles)
    stageExternalFileQuote(pending.draft.pendingQuotes)
    restoreForkMessageQuotes(pending.draft.pendingMessageQuotes)
    setComposerFocusSignal((value) => (value ?? 0) + 1)
  }, [
    restoreForkAttachedFiles,
    restoreForkInput,
    restoreForkMessageQuotes,
    session.currentSessionId,
    stageExternalFileQuote,
  ])

  // Ambient file-action wiring for persisted resources and renderer-only drafts.
  const setAttachedFiles = stream.setAttachedFiles
  const replaceDraftAttachment = useCallback(
    (draftId: string, file: File) =>
      setAttachedFiles((drafts) =>
        drafts.map((draft) =>
          draft.id === draftId ? { ...draft, file, status: "ready", error: undefined } : draft,
        ),
      ),
    [setAttachedFiles],
  )

  const setProjectWelcomeInput = stream.setInput
  const handleProjectWelcomeSuggestion = useCallback(
    (prompt: string) => {
      setProjectWelcomeInput(prompt)
    },
    [setProjectWelcomeInput],
  )

  useEffect(() => {
    incognitoComposerStateRef.current = {
      input: stream.input,
      attachedFileCount: stream.attachedFiles.length,
      pendingQuoteCount: stream.pendingQuotes.length + stream.pendingMessageQuotes.length,
      pendingMessage: stream.pendingMessage ?? "",
      pendingSendCount: stream.pendingSends.length,
    }
  }, [
    stream.attachedFiles.length,
    stream.input,
    stream.pendingMessage,
    stream.pendingQuotes.length,
    stream.pendingMessageQuotes.length,
    stream.pendingSends.length,
  ])

  useLayoutEffect(() => {
    clearDisposableIncognitoDraftRef.current = () => {
      stream.setInput("")
      stream.setAttachedFiles([])
      stream.setPendingQuotes([])
      stream.setPendingMessageQuotes([])
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
    const { token, attachKbId, mention } = pendingChatInsert
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
          const failure = chatKnowledgeReferenceAttachErrorToast(t, e)
          toast.error(
            failure.title,
            failure.description ? { description: failure.description } : undefined,
          )
        }
      }
      if (mention) {
        // The KB attach above may await a transport round-trip. Append against
        // the latest state so intervening typing and back-to-back refs survive.
        stream.appendInputMention(
          token,
          {
            id: newMentionId(),
            kind: mention.kind,
            targetId: mention.targetId,
            displayLabel: mention.displayLabel,
          },
          true,
        )
      } else {
        stream.setInput((prev) => `${prev}${prev && !prev.endsWith(" ") ? " " : ""}${token} `)
      }
      onChatInsertConsumed?.()
    }
    void run()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingChatInsert])

  // 设计空间「实现到代码」：把 handoff pack 作首条消息发进实现会话（流式 / 审批 /
  // DiffPanel 全复用）。用 handleSend 的 sessionIdOverride **原子**切到目标会话并发送
  // （directText 形态，不带任何 staged 附件 / quote），**不依赖** currentSessionId 会合、
  // 立即消费——无悬挂窗口、无被抢占后几小时误发（review F2）。nonce ref 防 StrictMode 重投。
  const autoSendFiredNonceRef = useRef<number | null>(null)
  useEffect(() => {
    if (!pendingAutoSend) return
    if (autoSendFiredNonceRef.current === pendingAutoSend.nonce) return
    autoSendFiredNonceRef.current = pendingAutoSend.nonce
    void stream.handleSend(pendingAutoSend.message, {
      sessionIdOverride: pendingAutoSend.sessionId,
    })
    onAutoSendConsumed?.(pendingAutoSend.nonce)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingAutoSend])

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
  const [activeMemoryToast, setActiveMemoryToast] = useState<ActiveMemoryRecall | null>(null)
  const memoryToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const activeMemoryToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const messagesRef = useRef<Message[]>(session.messages)
  const pendingActiveMemoryBySession = useRef<
    Map<string, { turnKey: string | null; recall: ActiveMemoryRecall }>
  >(new Map())

  useEffect(() => {
    messagesRef.current = currentSessionMessages
  }, [currentSessionMessages])

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

  useEffect(() => {
    const unlisten = getTransport().listen("memory:active_recall", (raw) => {
      const event = raw as ActiveMemoryRecallEvent
      if (event.sessionId !== currentSessionId || !event.recall?.summary) return
      const turnKey = getLatestUserTurnKey(messagesRef.current)
      pendingActiveMemoryBySession.current.set(event.sessionId, { turnKey, recall: event.recall })
      updateSessionMessages(event.sessionId, (prev) =>
        attachActiveMemoryToLatestAssistant(prev, turnKey, event.recall),
      )
      setActiveMemoryToast(event.recall)
      if (activeMemoryToastTimer.current) clearTimeout(activeMemoryToastTimer.current)
      activeMemoryToastTimer.current = setTimeout(() => setActiveMemoryToast(null), 6500)
    })
    return () => {
      unlisten()
      if (activeMemoryToastTimer.current) clearTimeout(activeMemoryToastTimer.current)
    }
  }, [currentSessionId, updateSessionMessages])

  useEffect(() => {
    const sid = currentSessionId
    if (!sid) return
    const pending = pendingActiveMemoryBySession.current.get(sid)
    if (!pending) return
    const next = attachActiveMemoryToLatestAssistant(
      currentSessionMessages,
      pending.turnKey,
      pending.recall,
    )
    if (next !== currentSessionMessages) {
      updateSessionMessages(sid, () => next)
    }
  }, [currentSessionId, currentSessionMessages, updateSessionMessages])

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
      const sid = currentSessionId
      if (!sid || compactingRef.current) return null

      const noticeId = `manual-compact:${sid}:${Date.now()}`
      compactingRef.current = true
      setCompacting(true)
      updateSessionMessages(sid, (prev) =>
        upsertManualCompactNotice(prev, makeCompactProgressEvent(), noticeId),
      )

      try {
        const result = await compactContextNow(sid)
        updateSessionMessages(sid, (prev) =>
          upsertManualCompactNotice(prev, makeCompactResultEvent(result), noticeId),
        )
        return result
      } catch (e) {
        updateSessionMessages(sid, (prev) =>
          upsertManualCompactNotice(prev, makeCompactFailedEvent(), noticeId),
        )
        throw e
      } finally {
        compactingRef.current = false
        setCompacting(false)
      }
    }, [currentSessionId, updateSessionMessages])

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
        action?.type !== "forkSession" &&
        action?.type !== "openSideChat" &&
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
      const shouldAppendResultContent =
        result.content && !actionRendersResult && !suppressLoopCreateResult
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
        case "forkSession":
          try {
            await reloadSessions()
            const switched = await rawHandleSwitchSession(action.sessionId)
            if (!switched) {
              toast.warning(
                t("chat.fork.createdButNotOpened", {
                  id: action.sessionId.slice(0, 8),
                  defaultValue: "分支 `{{id}}` 已创建，但未能打开。请从会话列表中选择它继续。",
                }),
              )
              break
            }
            toast.success(
              t("chat.fork.created", {
                defaultValue: "已在新会话中继续",
              }),
            )
          } catch (e) {
            logger.error("ui", "ChatScreen::slashForkSession", "Failed to open forked session", e)
            toast.error(
              e instanceof Error
                ? e.message
                : t("chat.fork.failed", { defaultValue: "无法在新会话中继续" }),
            )
          }
          break
        case "openSideChat":
          await revealSideChat(
            action.sessionId,
            action.initialPrompt ? { prompt: action.initialPrompt } : undefined,
          )
          void refreshSideChats()
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
            const commandText = result._skillCommandText ?? result._slashCommandText
            const activation = action.skillActivation
            if (commandText && activation) {
              // The model receives the user's command plus a backend-resolved,
              // user-level skill binding. The legacy expanded action message is
              // retained for transports that do not support typed sidecars.
              await stream.handleSend(commandText, {
                displayText: commandText,
                structuredMentions: [
                  slashSkillMentionBinding(
                    commandText,
                    activation.skillName,
                    activation.commandName,
                  ),
                ],
              })
            } else {
              await stream.handleSend(action.message, {
                displayText: result._skillCommandText,
              })
            }
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
      rawHandleSwitchSession,
      reloadSessions,
      refreshUnreadState,
      refreshSideChats,
      revealSideChat,
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
          agentId: currentAgentId,
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
      currentAgentId,
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
      updateSessionMessages(sid, (prev) =>
        appendUniqueSlashHistoryMessages(prev, [optimisticLoopMessage]),
      )
      try {
        const result = await getTransport().call<CommandResult>("execute_slash_command", {
          sessionId: sid,
          agentId: currentAgentId,
          commandText,
        })
        result._slashCommandText = commandText
        result._sessionId = sid
        await handleCommandAction(result)
        return true
      } catch (e) {
        updateSessionMessages(sid, (prev) =>
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
      currentAgentId,
      t,
      updateSessionMessages,
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

  const handleForkFromMessage = useCallback(
    async (message: Message) => {
      const sourceSessionId = session.currentSessionId
      if (!sourceSessionId) return
      const request = forkSessionRequestForMessage(sourceSessionId, message)
      if (!request) return
      try {
        const forked = await getTransport().call<ForkSessionResult>("fork_session_cmd", {
          ...request,
        })
        const composerDraft = await forkComposerDraftForMessage(message, forked)
        await reloadSessions()
        pendingForkComposerRef.current =
          composerDraft == null ? null : { sessionId: forked.id, draft: composerDraft }
        await rawHandleSwitchSession(forked.id)
        toast.success(
          t("chat.fork.created", {
            defaultValue: "已在新会话中继续",
          }),
        )
      } catch (e) {
        pendingForkComposerRef.current = null
        logger.error("ui", "ChatScreen::forkSession", "Failed to fork session", e)
        toast.error(
          e instanceof Error
            ? e.message
            : t("chat.fork.failed", { defaultValue: "无法在新会话中继续" }),
        )
      }
    },
    [rawHandleSwitchSession, reloadSessions, session.currentSessionId, t],
  )

  const handleEditAndResend = useCallback(
    async (message: Message, content: string) => {
      const sessionId = session.currentSessionId
      const messageId = message.dbId
      if (!sessionId || typeof messageId !== "number") return

      try {
        // Restore source-owned files before the rewind. If any attachment is
        // unavailable, keep the original transcript intact rather than
        // silently resending an incomplete prompt.
        const draft = await resendComposerDraftForMessage(message)
        await new Promise<void>((resolve, reject) => {
          let dispatchSettled = false
          const rejectDispatch = (error: unknown) => {
            if (dispatchSettled) return
            dispatchSettled = true
            reject(error)
          }
          const acceptDispatch = () => {
            if (dispatchSettled) return
            dispatchSettled = true
            resolve()
          }
          const goalEdit = message.isGoalTrigger === true
          void stream
            .handleSend(goalEdit ? goalTurnPrompt(content) : content, {
              draftOverride: {
                attachedFiles: draft.attachedFiles,
                pendingQuotes: draft.pendingQuotes,
                pendingMessageQuotes: draft.pendingMessageQuotes,
              },
              editMessageId: messageId,
              displayText: goalEdit ? content : undefined,
              goalTrigger: goalEdit || undefined,
              onDispatchAccepted: acceptDispatch,
              onPreparationError: rejectDispatch,
            })
            .catch(rejectDispatch)
        })
      } catch (error) {
        logger.error("ui", "ChatScreen::editMessage", "Failed to edit and resend message", error)
        toast.error(
          t("chat.editMessage.failed", {
            defaultValue: "Could not edit and resend this message",
          }),
          { description: error instanceof Error ? error.message : String(error) },
        )
        throw error
      }
    },
    [session.currentSessionId, stream, t],
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
  const [workbenchResizing, setWorkbenchResizing] = useState(false)
  const [workbenchMaximized, setWorkbenchMaximized] = useState(false)
  const [terminalOpen, setTerminalOpen] = useState(false)
  const [environmentPopoverOpen, setEnvironmentPopoverOpen] = useState(false)
  const [workbenchTabOrders, setWorkbenchTabOrders] = useState<Record<string, string[]>>({})
  const [incognitoWorkbenchTabOrder, setIncognitoWorkbenchTabOrder] = useState<string[]>([])

  useEffect(() => {
    const toggleTerminal = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return
      if (event.key.toLowerCase() !== "j") return
      event.preventDefault()
      setTerminalOpen((value) => !value)
    }
    window.addEventListener("keydown", toggleTerminal)
    return () => window.removeEventListener("keydown", toggleTerminal)
  }, [])
  // In-app floating windows for the browser / mac-control mirrors. A floating
  // panel leaves the exclusive right-panel slot (visibility below) so the slot
  // falls through to the next open panel; docking re-enters it.
  const {
    floatingPanels: floatingPanelList,
    isFloating: isPanelFloating,
    zIndexOf: floatingPanelZIndex,
    float: floatPanel,
    dock: dockFloatingPanel,
    closeFloating: closeFloatingPanel,
    focusFloating: focusFloatingPanel,
  } = useFloatingPanels()
  const rightPanelVisibility = useMemo<ExclusiveRightPanelVisibility>(
    () => ({
      workspace: showWorkspacePanel,
      "pull-request": showPullRequestPanel && !!session.currentSessionId,
      diff: isDiffPanelVisible,
      plan: shouldShowPlanPanel,
      files: fileTabs.showPanel && !!effectiveWorkingDir,
      browser: showBrowserPanel && !isPanelFloating("browser"),
      "mac-control": showMacControlPanel && !isPanelFloating("mac-control"),
      canvas: canvasPanelOpen,
      team: !!activeTeamId && showTeamPanel,
      "background-jobs": showBackgroundJobsPanel,
      subagent: showSubagentPanel,
      preview: isFilePreviewVisible,
    }),
    [
      activeTeamId,
      canvasPanelOpen,
      effectiveWorkingDir,
      fileTabs.showPanel,
      isDiffPanelVisible,
      isFilePreviewVisible,
      isPanelFloating,
      shouldShowPlanPanel,
      showBackgroundJobsPanel,
      showSubagentPanel,
      showBrowserPanel,
      showMacControlPanel,
      showPullRequestPanel,
      showTeamPanel,
      showWorkspacePanel,
      session.currentSessionId,
    ],
  )
  const openExclusiveRightPanels = useMemo(
    () => EXCLUSIVE_RIGHT_PANEL_ORDER.filter((panel) => rightPanelVisibility[panel]),
    [rightPanelVisibility],
  )
  const hasOpenExclusiveRightPanel = openExclusiveRightPanels.length > 0

  useEffect(() => {
    if (!hasOpenExclusiveRightPanel && workbenchMaximized) {
      setWorkbenchMaximized(false)
    }
  }, [hasOpenExclusiveRightPanel, workbenchMaximized])

  useEffect(() => {
    if (!workbenchMaximized) return
    const restoreWorkbench = (event: KeyboardEvent) => {
      // A dialog / dropdown that already consumed Escape owns that press.
      if (event.key !== "Escape" || event.defaultPrevented) return
      event.preventDefault()
      setWorkbenchMaximized(false)
    }
    window.addEventListener("keydown", restoreWorkbench)
    return () => window.removeEventListener("keydown", restoreWorkbench)
  }, [workbenchMaximized])
  const workbenchSessionKey = session.currentSessionId ?? DRAFT_WORKBENCH_KEY
  // Stable empty identity: a fresh `[]` every render would invalidate the memos
  // and layout effects below in the (very common) no-stored-order state.
  const storedWorkbenchOrder = incognitoEnabled
    ? incognitoWorkbenchTabOrder
    : (workbenchTabOrders[workbenchSessionKey] ?? EMPTY_WORKBENCH_ORDER)
  const orderedOpenExclusiveRightPanels = useMemo(
    () => [
      ...storedWorkbenchOrder.filter(
        (panel): panel is ExclusiveRightPanel =>
          isExclusiveRightPanel(panel) && rightPanelVisibility[panel],
      ),
      ...openExclusiveRightPanels.filter((panel) => !storedWorkbenchOrder.includes(panel)),
    ],
    [openExclusiveRightPanels, rightPanelVisibility, storedWorkbenchOrder],
  )
  const workbenchOpen = hasOpenExclusiveRightPanel && !rightPanelCollapsed
  const {
    containerRef: workbenchContainerRef,
    availableWidth: workbenchAvailableWidth,
    width: rightPanelWidth,
    layoutMode: workbenchLayoutMode,
    shouldCollapse: workbenchOutOfRoom,
    collapseThreshold: workbenchCollapseAt,
    setManualWidth: setRightPanelWidth,
    commitManualWidth: commitRightPanelWidth,
    resetAutomaticWidth: resetRightPanelWidth,
  } = useWorkbenchSizing(workbenchOpen, ENVIRONMENT_CARD_LANE_PX, environmentPopoverOpen)
  const previousHasOpenRightPanelRef = useRef(false)
  const animateRightPanelOnMount =
    hasOpenExclusiveRightPanel && !previousHasOpenRightPanelRef.current
  useLayoutEffect(() => {
    previousHasOpenRightPanelRef.current = hasOpenExclusiveRightPanel
  }, [hasOpenExclusiveRightPanel])
  const renderedExclusiveRightPanel =
    activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
      ? activeExclusiveRightPanel
      : (orderedOpenExclusiveRightPanels[0] ?? null)
  const handleSelectRightPanel = useCallback((panelId: string) => {
    if (!isExclusiveRightPanel(panelId)) return
    setActiveExclusiveRightPanel(panelId)
    manualRightPanelExpandedOverrideRef.current = true
    setRightPanelCollapsed(false)
  }, [])

  const showRightPanelByUser = useCallback((panel: ExclusiveRightPanel) => {
    setActiveExclusiveRightPanel(panel)
    manualRightPanelExpandedOverrideRef.current = true
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
  const handleMessageQuote = useCallback(
    (quote: PendingMessageQuote) => {
      stream.setPendingMessageQuotes((prev) => [...prev, quote])
      setComposerFocusSignal((prev) => (prev ?? 0) + 1)
    },
    [stream],
  )
  const handleAskInSideChat = useCallback(
    (quote: PendingMessageQuote) => {
      void createSideChat({ quote })
    },
    [createSideChat],
  )
  // Help window "Ask AI" — manual excerpts arrive through a module-level
  // queue (App switches to the chat view first; the subscription drains
  // anything that arrived before this screen mounted).
  useEffect(
    () =>
      subscribeAskAiQuotes((text) => {
        handleMessageQuote({ role: "assistant", content: text })
      }),
    [handleMessageQuote],
  )
  // 文件浏览器面板的作用域（与下方 FileBrowserPanel 渲染处共用，避免两处漂移）。
  const fileBrowserScope: "session" | "project" =
    !session.currentSessionId && currentProject ? "project" : "session"
  const fileBrowserScopeId =
    !session.currentSessionId && currentProject ? currentProject.id : session.currentSessionId

  /** Absolute path → a browsable root. Longest match wins; `null` = outside. */
  const resolveBrowsablePath = useCallback(
    (absPath: string): { relPath: string; projectRoot: ProjectFolderIdentity | null } | null => {
      const path = absPath.replace(/\\/g, "/").replace(/\/+$/, "")
      const roots: { path: string | null; projectRoot: ProjectFolderIdentity | null }[] = [
        { path: effectiveWorkingDir, projectRoot: null },
        ...projectFileBrowserRoots.map((rootPath, index) => ({
          path: rootPath,
          projectRoot: { index, path: rootPath },
        })),
      ]
      let resolved: { relPath: string; projectRoot: ProjectFolderIdentity | null } | null = null
      let matchedLength = -1
      for (const root of roots) {
        if (!root.path) continue
        const rootDir = root.path.replace(/\\/g, "/").replace(/\/+$/, "")
        if (!rootDir || rootDir.length <= matchedLength) continue
        if (path !== rootDir && !path.startsWith(`${rootDir}/`)) continue
        matchedLength = rootDir.length
        resolved = {
          relPath: path.slice(rootDir.length).replace(/^\/+/, ""),
          projectRoot: root.projectRoot,
        }
      }
      return resolved
    },
    [effectiveWorkingDir, projectFileBrowserRoots],
  )

  /** A file the browser can reveal, or `null` when it lives outside every root.
   *  Carries the quoted range through: a "jump to this line" that lands in a
   *  file tab must still scroll to it and highlight it. */
  const resolveBrowsableTarget = useCallback(
    (target: PreviewTarget): FileTabFileReveal | null => {
      if (target.kind === "workspace") {
        // Workspace targets already speak the browser's relative language — but
        // only while they address the very scope this panel is rooted at.
        if (target.scope !== fileBrowserScope || target.scopeId !== (fileBrowserScopeId ?? "")) {
          return null
        }
        if (target.isDirectory) return null
        return {
          path: target.relPath,
          name: target.name,
          projectRoot: null,
          startLine: target.revealLines?.start,
          endLine: target.revealLines?.end,
        }
      }
      if (target.kind !== "sessionPath") return null
      const resolved = resolveBrowsablePath(target.path)
      if (!resolved || !resolved.relPath) return null
      return {
        path: resolved.relPath,
        name: target.name,
        projectRoot: resolved.projectRoot,
        startLine: target.revealLines?.start,
        endLine: target.revealLines?.end,
      }
    },
    [fileBrowserScope, fileBrowserScopeId, resolveBrowsablePath],
  )

  const revealFileInActiveTab = fileTabs.revealFileInActiveTab
  const openFileTab = fileTabs.openTab
  const openFilePreview = filePreview.openPreview

  /** Tree-addressable files go to the active browser tab; the rest get a preview tab. */
  const openFileTarget = useCallback(
    (target: PreviewTarget) => {
      const browsable = resolveBrowsableTarget(target)
      if (browsable) {
        revealFileInActiveTab(browsable)
        showRightPanelByUser("files")
        return
      }
      openFilePreview(target)
    },
    [openFilePreview, resolveBrowsableTarget, revealFileInActiveTab, showRightPanelByUser],
  )

  /** Side-chat file reads must carry the side session that authorized the path. */
  const openSideChatFileTarget = useCallback(
    (sideSessionId: string, target: PreviewTarget) => {
      openFilePreview(target, sideSessionId)
    },
    [openFilePreview],
  )

  /** Explicit "open in a new tab": same addressing rules, same preview fallback,
   *  so a file under a linked folder or worktree root is never a dead click. */
  const openFileTargetInNewTab = useCallback(
    (target: PreviewTarget) => {
      const browsable = resolveBrowsableTarget(target)
      if (browsable) {
        openFileTab(browsable)
        showRightPanelByUser("files")
        return
      }
      openFilePreview(target)
    },
    [openFilePreview, openFileTab, resolveBrowsableTarget, showRightPanelByUser],
  )

  // Placed after `openFileTarget` so it can close over it.
  const fileActionsValue = useMemo<FileActionsContextValue>(
    () => ({
      sessionId: currentSessionId,
      onPreviewFile: openFileTarget,
      onReplaceDraft: replaceDraftAttachment,
    }),
    [currentSessionId, openFileTarget, replaceDraftAttachment],
  )

  // Reveal a quoted file in the browser: focus a file tab + signal the target.
  const handleQuoteJump = useCallback(
    (q: QuotePayload) => {
      if (q.revealable === false) return
      fileTabs.revealFileInActiveTab({
        path: q.path,
        name: q.name,
        projectRoot: q.projectRoot ?? null,
        worktreeRoot: q.worktreeRoot ?? null,
        startLine: q.startLine,
        endLine: q.endLine,
      })
      showRightPanelByUser("files")
    },
    [fileTabs, showRightPanelByUser],
  )

  // 预览头面包屑 → 文件浏览器。只有浏览器真能寻址的目录才可点，其余渲染成纯文本
  // 而不是死链接。
  const resolvePreviewDirectory = useCallback(
    (target: PreviewTarget, dirPath: string): Omit<FileBrowserDirectoryReveal, "nonce"> | null => {
      if (target.kind === "workspace") {
        if (target.scope !== fileBrowserScope || target.scopeId !== (fileBrowserScopeId ?? "")) {
          return null
        }
        return { relPath: dirPath.replace(/\\/g, "/").replace(/^\/+|\/+$/g, ""), projectRoot: null }
      }
      return resolveBrowsablePath(dirPath)
    },
    [fileBrowserScope, fileBrowserScopeId, resolveBrowsablePath],
  )
  const canRevealPreviewDirectory = useCallback(
    (target: PreviewTarget, dirPath: string) => !!resolvePreviewDirectory(target, dirPath),
    [resolvePreviewDirectory],
  )
  const revealPreviewDirectory = useCallback(
    (target: PreviewTarget, dirPath: string) => {
      const resolved = resolvePreviewDirectory(target, dirPath)
      if (!resolved) return
      fileTabs.revealDirectoryInActiveTab(resolved.relPath, resolved.projectRoot)
      showRightPanelByUser("files")
    },
    [fileTabs, resolvePreviewDirectory, showRightPanelByUser],
  )

  // 打开并激活 Workspace 面板（状态条点击 / 重新打开）。
  const openWorkspacePanel = useCallback(() => {
    workspacePanelDismissedRef.current = false
    setShowWorkspacePanel(true)
    showRightPanelByUser("workspace")
  }, [showRightPanelByUser])

  useEffect(() => {
    if (!pendingControlFocus || session.currentSessionId !== pendingControlFocus.sessionId) return
    if (pendingControlFocus.kind === "plan") {
      planMode.openPlanPanel()
      setPendingControlFocus(null)
      return
    }
    const section: WorkspaceFocusRequest["section"] =
      pendingControlFocus.kind === "goal"
        ? "goal"
        : pendingControlFocus.kind === "workflow"
          ? "workflow"
          : pendingControlFocus.kind === "loop"
            ? "loop"
            : "progress"
    openWorkspacePanel()
    setWorkspaceFocusRequest({
      sessionId: pendingControlFocus.sessionId,
      section,
      itemId: pendingControlFocus.itemId,
      nonce: pendingControlFocus.nonce,
    })
    setPendingControlFocus(null)
  }, [openWorkspacePanel, pendingControlFocus, planMode, session.currentSessionId])

  const handleWorkspaceFocusRequestHandled = useCallback((nonce: number) => {
    setWorkspaceFocusRequest((current) => (current?.nonce === nonce ? null : current))
  }, [])

  const openPullRequestPanel = useCallback(
    (expectedUrl?: string | null) => {
      setPullRequestExpectedUrl(expectedUrl ?? null)
      setShowPullRequestPanel(true)
      showRightPanelByUser("pull-request")
    },
    [showRightPanelByUser],
  )

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

  const openSubagentPanel = useCallback(
    (target?: { runId?: string; childSessionId?: string }) => {
      if (target?.runId || target?.childSessionId) {
        subagentSelectNonceRef.current += 1
        setSubagentPanelSelectRequest({
          runId: target.runId,
          childSessionId: target.childSessionId,
          nonce: subagentSelectNonceRef.current,
        })
      } else {
        // Opened from the title-bar button (no target) — drop any stale request
        // so the panel auto-selects the newest run instead of re-selecting the
        // last chip-clicked one.
        setSubagentPanelSelectRequest(null)
      }
      setShowSubagentPanel(true)
      showRightPanelByUser("subagent")
    },
    [showRightPanelByUser],
  )

  const closeSubagentPanel = useCallback(() => {
    setShowSubagentPanel(false)
    setSubagentPanelSelectRequest(null)
  }, [])

  const handleOpenSubagentRun = useCallback(
    (target: SubagentOpenTarget) => {
      openSubagentPanel(
        target.runId
          ? { runId: target.runId }
          : target.childSessionId
            ? { childSessionId: target.childSessionId }
            : undefined,
      )
    },
    [openSubagentPanel],
  )

  const handleBackgroundJobExpandedChange = useCallback((jobId: string, expanded: boolean) => {
    setBackgroundJobExpansionOverrides((prev) =>
      prev[jobId] === expanded ? prev : { ...prev, [jobId]: expanded },
    )
  }, [])

  useEffect(() => {
    if (!hasOpenExclusiveRightPanel && rightPanelCollapsed) {
      setRightPanelCollapsed(false)
    }
  }, [hasOpenExclusiveRightPanel, rightPanelCollapsed])

  // Give-up order as the window narrows (steps 1-3 live in useWorkbenchSizing):
  //   1. conversation + workbench shrink together, down to their ideal minimums
  //   2. the card's lane goes first, handing its space back to both columns
  //   3. the workbench shrinks alone, then collapses
  //   4. the sidebar squeezes to its minimum, then collapses
  const shouldAutoCollapseWorkbench = hasOpenExclusiveRightPanel && workbenchOutOfRoom
  const shouldAutoExpandWorkbench =
    workbenchAvailableWidth >= workbenchCollapseAt + RESPONSIVE_PANEL_HYSTERESIS

  useEffect(() => {
    if (!hasOpenExclusiveRightPanel) {
      autoCollapsedRightPanelRef.current = false
      manualRightPanelExpandedOverrideRef.current = false
      return
    }
    if (shouldAutoExpandWorkbench) manualRightPanelExpandedOverrideRef.current = false
    if (
      shouldAutoCollapseWorkbench &&
      !rightPanelCollapsed &&
      !manualRightPanelExpandedOverrideRef.current
    ) {
      autoCollapsedRightPanelRef.current = true
      // Collapsing always leaves maximize first — a collapsed-but-maximized
      // workbench comes back as a full-window overlay over the transcript.
      setWorkbenchMaximized(false)
      setRightPanelCollapsed(true)
    } else if (
      shouldAutoExpandWorkbench &&
      rightPanelCollapsed &&
      autoCollapsedRightPanelRef.current
    ) {
      autoCollapsedRightPanelRef.current = false
      setRightPanelCollapsed(false)
    }
  }, [
    hasOpenExclusiveRightPanel,
    rightPanelCollapsed,
    shouldAutoCollapseWorkbench,
    shouldAutoExpandWorkbench,
  ])

  // Last step of the cascade, measured on the window since the sidebar hugs its
  // edge: squeeze it for as long as it has slack, and only then give it up.
  const viewportWidth = useViewportWidth()
  const sidebarSqueezedWidth = Math.round(
    Math.min(panelWidth, Math.max(CHAT_SIDEBAR_MIN_WIDTH, viewportWidth - CHAT_IDEAL_MIN)),
  )
  const sidebarCollapseAt = CHAT_SIDEBAR_MIN_WIDTH + CHAT_MAIN_MIN_INTERACTIVE_WIDTH
  const shouldAutoCollapseSidebar = useViewportMediaQuery(`(max-width: ${sidebarCollapseAt}px)`)
  const shouldAutoExpandSidebar = useViewportMediaQuery(
    `(min-width: ${sidebarCollapseAt + RESPONSIVE_PANEL_HYSTERESIS}px)`,
  )

  useEffect(() => {
    if (shouldAutoExpandSidebar) {
      manualSidebarExpandedOverrideRef.current = false
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
  }, [sidebarCollapsed, shouldAutoCollapseSidebar, shouldAutoExpandSidebar])

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
  const previousFileTabsOpenNonceRef = useRef(fileTabs.openNonce)
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
    } else if (fileTabs.openNonce !== previousFileTabsOpenNonceRef.current) {
      forced = "files"
    } else if (diffPanel.openNonce !== previousDiffOpenNonceRef.current) {
      forced = "diff"
    }
    previousPreviewOpenNonceRef.current = filePreview.openNonce
    previousFileTabsOpenNonceRef.current = fileTabs.openNonce
    previousDiffOpenNonceRef.current = diffPanel.openNonce
    const stillActive =
      activeExclusiveRightPanel && rightPanelVisibility[activeExclusiveRightPanel]
        ? activeExclusiveRightPanel
        : null
    const active =
      forced ?? newlyOpened ?? stillActive ?? orderedOpenExclusiveRightPanels[0] ?? null

    previousRightPanelVisibilityRef.current = rightPanelVisibility
    if (activeExclusiveRightPanel !== active) {
      setActiveExclusiveRightPanel(active)
    }
  }, [
    activeExclusiveRightPanel,
    orderedOpenExclusiveRightPanels,
    rightPanelVisibility,
    fileTabs.openNonce,
    filePreview.openNonce,
    diffPanel.openNonce,
  ])

  // Workbench sets are session-scoped. Returning to a normal session restores
  // its safe tabs/order/active selection; incognito sets are never cached.
  const workbenchSessionCacheRef = useRef(new Map<string, RestorableWorkbenchSessionState>())
  const activeWorkbenchSessionRef = useRef(DRAFT_WORKBENCH_KEY)
  const activeWorkbenchIncognitoRef = useRef(false)
  const suppressWorkbenchOrderWriteRef = useRef(false)
  const switchFilePreviewScope = filePreview.switchScope
  const switchFileTabsScope = fileTabs.switchScope
  const renameFilePreviewScope = filePreview.renameScope
  const renameFileTabsScope = fileTabs.renameScope

  // A draft becomes a real session on its first message. Re-address the draft's
  // workbench scope rather than let the switch effect below treat it as leaving:
  // the open file tabs / previews belong to this very conversation, and the
  // orphaned `__draft__` bucket would otherwise be restored into the next new
  // chat — pointing at the previous chat's working dir.
  sessionPromotedRef.current = (sessionId: string) => {
    const previousKey = activeWorkbenchSessionRef.current
    if (previousKey === sessionId) return
    activeWorkbenchSessionRef.current = sessionId
    const cached = workbenchSessionCacheRef.current.get(previousKey)
    workbenchSessionCacheRef.current.delete(previousKey)
    if (cached) workbenchSessionCacheRef.current.set(sessionId, cached)
    setWorkbenchTabOrders((current) => {
      const order = current[previousKey]
      if (!order) return current
      const next = { ...current, [sessionId]: order }
      delete next[previousKey]
      return next
    })
    const prefix = incognitoEnabled ? "incognito:" : ""
    renameFileTabsScope(`${prefix}${previousKey}`, `${prefix}${sessionId}`)
    renameFilePreviewScope(`${prefix}${previousKey}`, `${prefix}${sessionId}`)
  }
  useLayoutEffect(() => {
    const nextKey = session.currentSessionId ?? DRAFT_WORKBENCH_KEY
    const previousKey = activeWorkbenchSessionRef.current
    const leavingIncognito = activeWorkbenchIncognitoRef.current
    if (previousKey === nextKey && leavingIncognito === incognitoEnabled) return
    suppressWorkbenchOrderWriteRef.current = true
    const preserveWorkspace = preserveWorkspaceOnSessionSwitchRef.current
    if (!leavingIncognito) {
      workbenchSessionCacheRef.current.set(previousKey, {
        workspace: showWorkspacePanel,
        browser: showBrowserPanel,
        macControl: showMacControlPanel,
        backgroundJobs: showBackgroundJobsPanel,
        subagent: showSubagentPanel,
        activePanel: activeExclusiveRightPanel,
        collapsed: rightPanelCollapsed,
        dismissed: {
          workspace: workspacePanelDismissedRef.current,
          browser: browserPanelDismissedRef.current,
          macControl: macControlPanelDismissedRef.current,
          backgroundJobs: backgroundJobsPanelDismissedRef.current,
        },
      })
    }
    if (leavingIncognito || incognitoEnabled) setIncognitoWorkbenchTabOrder([])
    const restore = !incognitoEnabled ? workbenchSessionCacheRef.current.get(nextKey) : undefined
    activeWorkbenchSessionRef.current = nextKey
    activeWorkbenchIncognitoRef.current = incognitoEnabled
    workspacePanelDismissedRef.current = restore?.dismissed.workspace ?? false
    browserPanelDismissedRef.current = restore?.dismissed.browser ?? false
    macControlPanelDismissedRef.current = restore?.dismissed.macControl ?? false
    backgroundJobsPanelDismissedRef.current = restore?.dismissed.backgroundJobs ?? false
    suppressNextBackgroundJobsActivationRef.current = false
    previousBackgroundRunningCountRef.current = 0
    // Frames are session-scoped — close floating mirrors on session switch.
    closeFloatingPanel("browser")
    closeFloatingPanel("mac-control")
    setShowBrowserPanel(restore?.browser ?? false)
    setShowMacControlPanel(restore?.macControl ?? false)
    setShowWorkspacePanel(preserveWorkspace || (restore?.workspace ?? false))
    setShowPullRequestPanel(false)
    setPullRequestExpectedUrl(null)
    setShowBackgroundJobsPanel(restore?.backgroundJobs ?? false)
    setBackgroundJobExpansionOverrides({})
    setShowSubagentPanel(restore?.subagent ?? false)
    setSubagentPanelSelectRequest(null)
    setActiveExclusiveRightPanel(restore?.activePanel ?? null)
    setRightPanelCollapsed(restore?.collapsed ?? false)
    setWorkbenchMaximized(false)
    const scopeSwitch = {
      restore: !incognitoEnabled,
      cacheCurrent: !leavingIncognito,
    }
    switchFilePreviewScope(incognitoEnabled ? `incognito:${nextKey}` : nextKey, scopeSwitch)
    switchFileTabsScope(incognitoEnabled ? `incognito:${nextKey}` : nextKey, scopeSwitch)
    if (preserveWorkspace) {
      preserveWorkspaceOnSessionSwitchRef.current = false
    }
  }, [
    activeExclusiveRightPanel,
    closeFloatingPanel,
    incognitoEnabled,
    rightPanelCollapsed,
    session.currentSessionId,
    showBackgroundJobsPanel,
    showBrowserPanel,
    showMacControlPanel,
    showSubagentPanel,
    showWorkspacePanel,
    switchFilePreviewScope,
    switchFileTabsScope,
  ])

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
  const titleBarRightPanels = useMemo<WorkbenchTabItem[]>(() => {
    const persistentPanels = PERSISTENT_RIGHT_PANEL_ORDER.filter((panel) => {
      if (panel === "files") return !!effectiveWorkingDir
      // Only surface the sub-agent entry once the session has spawned one.
      if (panel === "subagent") return subagentRuns.runs.length > 0 || showSubagentPanel
      return true
    })
    const persistentSet = new Set<ExclusiveRightPanel>(persistentPanels)
    // Floating mirrors left the exclusive slot but stay listed (open) so the
    // user can find them — clicking their button docks them back.
    const isFloatingPanelId = (panel: ExclusiveRightPanel) =>
      (panel === "browser" || panel === "mac-control") && isPanelFloating(panel)
    const transientPanels = EXCLUSIVE_RIGHT_PANEL_ORDER.filter(
      (panel) =>
        !persistentSet.has(panel) && (rightPanelVisibility[panel] || isFloatingPanelId(panel)),
    )
    const workflowBadgeCount =
      workflowTitleBarStatus.attentionCount || workflowTitleBarStatus.activeCount

    return [...persistentPanels, ...transientPanels].map((panel) => {
      const base = {
        id: panel,
        panelId: panel,
        labelKey: EXCLUSIVE_RIGHT_PANEL_LABEL_KEYS[panel],
        icon: EXCLUSIVE_RIGHT_PANEL_ICONS[panel],
        open: rightPanelVisibility[panel] || isFloatingPanelId(panel),
        windowMode:
          panel === "browser" || panel === "mac-control"
            ? isPanelFloating(panel)
              ? ("floating" as const)
              : ("docked" as const)
            : undefined,
      }
      if (panel === "workspace" && workflowBadgeCount > 0) {
        return {
          ...base,
          badge: {
            count: workflowBadgeCount,
            labelKey:
              workflowTitleBarStatus.attentionCount > 0
                ? "chat.rightPanel.workflowAttentionCount"
                : "chat.rightPanel.workflowActiveCount",
            tone:
              workflowTitleBarStatus.attentionCount > 0
                ? ("attention" as const)
                : workflowTitleBarStatus.runningCount > 0
                  ? ("running" as const)
                  : ("neutral" as const),
          },
        }
      }
      if (panel === "background-jobs" && backgroundJobs.runningCount > 0) {
        return {
          ...base,
          badge: {
            count: backgroundJobs.runningCount,
            labelKey: "chat.rightPanel.backgroundRunningCount",
            tone: "attention" as const,
          },
        }
      }
      if (panel === "subagent" && subagentRuns.runningCount > 0) {
        return {
          ...base,
          badge: {
            count: subagentRuns.runningCount,
            labelKey: "chat.rightPanel.subagentRunningCount",
            tone: "running" as const,
          },
        }
      }
      return base
    })
  }, [
    backgroundJobs.runningCount,
    effectiveWorkingDir,
    isPanelFloating,
    rightPanelVisibility,
    subagentRuns.runningCount,
    subagentRuns.runs.length,
    showSubagentPanel,
    workflowTitleBarStatus,
  ])
  const workbenchTabs = useMemo<WorkbenchTabItem[]>(() => {
    const byId = new Map(titleBarRightPanels.map((item) => [item.id, item]))
    const unordered: WorkbenchTabItem[] = []
    for (const panel of orderedOpenExclusiveRightPanels) {
      if (panel === "preview") {
        for (const entry of filePreview.entries) {
          unordered.push({
            id: entry.id,
            panelId: "preview",
            label: entry.title,
            icon: EXCLUSIVE_RIGHT_PANEL_ICONS.preview,
            fileIcon: { name: entry.title, mime: previewTargetMime(entry.target) },
            open: true,
          })
        }
        continue
      }
      if (panel === "files") {
        for (const tab of fileTabs.tabs) {
          unordered.push({
            id: tab.id,
            panelId: "files",
            label: tab.selection?.name ?? t("fileBrowser.panelTitle", "Files"),
            icon: EXCLUSIVE_RIGHT_PANEL_ICONS.files,
            fileIcon: tab.selection ? { name: tab.selection.name } : undefined,
            open: true,
          })
        }
        continue
      }
      const item = byId.get(panel)
      if (item) unordered.push(item)
    }
    for (const item of titleBarRightPanels) {
      // Both expand into per-resource tabs above.
      if (item.panelId === "preview" || item.panelId === "files") continue
      if (item.open && !unordered.some((tab) => tab.id === item.id)) unordered.push(item)
    }
    const byTabId = new Map(unordered.map((item) => [item.id, item]))
    return [
      ...storedWorkbenchOrder
        .map((id) => byTabId.get(id))
        .filter((item): item is WorkbenchTabItem => !!item),
      ...unordered.filter((item) => !storedWorkbenchOrder.includes(item.id)),
    ]
  }, [
    fileTabs.tabs,
    filePreview.entries,
    orderedOpenExclusiveRightPanels,
    storedWorkbenchOrder,
    t,
    titleBarRightPanels,
  ])

  useLayoutEffect(() => {
    if (suppressWorkbenchOrderWriteRef.current) {
      suppressWorkbenchOrderWriteRef.current = false
      return
    }
    const nextOrder = workbenchTabs.map((tab) => tab.id)
    if (incognitoEnabled) {
      setIncognitoWorkbenchTabOrder((current) =>
        current.length === nextOrder.length &&
        current.every((tabId, index) => tabId === nextOrder[index])
          ? current
          : nextOrder,
      )
      return
    }
    setWorkbenchTabOrders((current) => {
      const previous = current[workbenchSessionKey] ?? []
      if (
        previous.length === nextOrder.length &&
        previous.every((tabId, index) => tabId === nextOrder[index])
      ) {
        return current
      }
      return { ...current, [workbenchSessionKey]: nextOrder }
    })
  }, [incognitoEnabled, workbenchSessionKey, workbenchTabs])

  const handleReorderWorkbenchTabs = useCallback(
    (source: string, target: string) => {
      const reorder = (currentOrder: string[]) => {
        const order = [...currentOrder]
        const sourceIndex = order.indexOf(source)
        const targetIndex = order.indexOf(target)
        if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return currentOrder
        order.splice(sourceIndex, 1)
        order.splice(targetIndex, 0, source)
        return order
      }
      if (incognitoEnabled) {
        setIncognitoWorkbenchTabOrder((current) =>
          reorder(current.length > 0 ? current : workbenchTabs.map((tab) => tab.id)),
        )
      } else {
        setWorkbenchTabOrders((current) => {
          const currentOrder = current[workbenchSessionKey] ?? workbenchTabs.map((tab) => tab.id)
          const order = reorder(currentOrder)
          return order === currentOrder ? current : { ...current, [workbenchSessionKey]: order }
        })
      }
      if (source.startsWith("preview:") && target.startsWith("preview:")) {
        filePreview.reorderPreviews(source, target)
      }
      if (source.startsWith("files:") && target.startsWith("files:")) {
        fileTabs.reorderTabs(source, target)
      }
    },
    [fileTabs, filePreview, incognitoEnabled, workbenchSessionKey, workbenchTabs],
  )
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

  const handleOpenWorkbenchPanel = useCallback(
    (panel: ExclusiveRightPanel) => {
      if ((panel === "browser" || panel === "mac-control") && isPanelFloating(panel)) {
        dockFloatingPanel(panel)
        showRightPanelByUser(panel)
        return
      }
      if (panel === "workspace") {
        openWorkspacePanel()
        return
      }
      if (panel === "files") {
        // The `+` menu always means "another browser", never re-focus.
        fileTabs.openTab()
        showRightPanelByUser("files")
        return
      }
      if (panel === "background-jobs") {
        openBackgroundJobsPanel()
        return
      }
      if (panel === "subagent") {
        openSubagentPanel()
        return
      }
      if (panel === "plan") {
        planMode.openPlanPanel()
        return
      }
      if (panel === "browser") {
        openBrowserPanel()
        return
      }
      if (panel === "mac-control") {
        macControlPanelDismissedRef.current = false
        setShowMacControlPanel(true)
        showRightPanelByUser("mac-control")
        return
      }
      if (panel === "canvas") {
        setCanvasPanelOpen(true)
        showRightPanelByUser("canvas")
        return
      }
      if (panel === "team" && activeTeamId) {
        setShowTeamPanel(true)
        showRightPanelByUser("team")
        return
      }
      if (panel === "pull-request" && session.currentSessionId) {
        setShowPullRequestPanel(true)
        showRightPanelByUser("pull-request")
        return
      }
      handleSelectRightPanel(panel)
    },
    [
      activeTeamId,
      dockFloatingPanel,
      fileTabs,
      handleSelectRightPanel,
      isPanelFloating,
      openBackgroundJobsPanel,
      openBrowserPanel,
      openSubagentPanel,
      openWorkspacePanel,
      planMode,
      session.currentSessionId,
      showRightPanelByUser,
    ],
  )

  const handleSelectWorkbenchTab = useCallback(
    (tabId: string) => {
      if (tabId.startsWith("preview:")) {
        filePreview.selectPreview(tabId)
        showRightPanelByUser("preview")
        return
      }
      if (tabId.startsWith("files:")) {
        fileTabs.selectTab(tabId)
        showRightPanelByUser("files")
        return
      }
      // A floating mirror's tab has to dock it back first, or selecting it
      // would just activate whichever docked panel happens to be first.
      if ((tabId === "browser" || tabId === "mac-control") && isPanelFloating(tabId)) {
        dockFloatingPanel(tabId)
        showRightPanelByUser(tabId)
        return
      }
      handleSelectRightPanel(tabId)
    },
    [
      dockFloatingPanel,
      fileTabs,
      filePreview,
      handleSelectRightPanel,
      isPanelFloating,
      showRightPanelByUser,
    ],
  )

  const handleToggleWorkbenchTabWindow = useCallback(
    (tabId: string) => {
      if (tabId !== "browser" && tabId !== "mac-control") return
      if (isPanelFloating(tabId)) {
        dockFloatingPanel(tabId)
        showRightPanelByUser(tabId)
        return
      }
      floatPanel(tabId)
    },
    [dockFloatingPanel, floatPanel, isPanelFloating, showRightPanelByUser],
  )

  // `ownerId` = the file tab being closed. Without it, closing one tab would
  // discard every other open tab's unsaved buffer too.
  const confirmFilesPanelClose = useCallback(
    (ownerId?: string) =>
      confirmDiscardDirtyFileEditors(
        t("fileEditor.unsavedBody", "Discard the current edits before leaving this file?"),
        ownerId,
      ),
    [t],
  )

  // Already a stable callback from `useFilePreview`; wrapping it only created a
  // new identity every render.
  const closeFilePreview = filePreview.closePreview

  const handleCloseWorkbenchTab = useCallback(
    (tabId: string) => {
      if (tabId.startsWith("preview:")) {
        closeFilePreview(tabId)
        return
      }
      if (tabId.startsWith("files:")) {
        if (!confirmFilesPanelClose(tabId)) return
        fileTabs.closeTab(tabId)
        return
      }
      if (!isExclusiveRightPanel(tabId)) return
      const panel = tabId
      if (panel === "workspace") {
        workspacePanelDismissedRef.current = true
        setWorkspaceFocusRequest(null)
        setShowWorkspacePanel(false)
      } else if (panel === "pull-request") {
        setShowPullRequestPanel(false)
        setPullRequestExpectedUrl(null)
      } else if (panel === "diff") {
        diffPanel.closeDiff()
      } else if (panel === "plan") {
        planMode.setShowPanel(false)
      } else if (panel === "files") {
        if (!confirmFilesPanelClose()) return
        fileTabs.closeAllTabs()
      } else if (panel === "browser") {
        browserPanelDismissedRef.current = true
        closeFloatingPanel("browser")
        setShowBrowserPanel(false)
      } else if (panel === "mac-control") {
        macControlPanelDismissedRef.current = true
        closeFloatingPanel("mac-control")
        setShowMacControlPanel(false)
      } else if (panel === "canvas") {
        window.dispatchEvent(new CustomEvent("hope-agent:close-canvas"))
        setCanvasPanelOpen(false)
      } else if (panel === "team") {
        setShowTeamPanel(false)
      } else if (panel === "background-jobs") {
        closeBackgroundJobsPanel()
      } else if (panel === "subagent") {
        closeSubagentPanel()
      } else if (panel === "preview") {
        filePreview.closeAllPreviews()
      }
    },
    [
      closeBackgroundJobsPanel,
      closeFilePreview,
      closeFloatingPanel,
      closeSubagentPanel,
      confirmFilesPanelClose,
      diffPanel,
      fileTabs,
      filePreview,
      planMode,
    ],
  )

  const handleCollapseWorkbench = useCallback(() => {
    setWorkbenchMaximized(false)
    autoCollapsedRightPanelRef.current = false
    manualRightPanelExpandedOverrideRef.current = false
    setRightPanelCollapsed(true)
  }, [])

  const handleToggleWorkbenchMaximize = useCallback(() => {
    // Maximizing is an explicit "keep it open" — without this the next resize
    // tick can auto-collapse the workbench out from under a manual maximize.
    manualRightPanelExpandedOverrideRef.current = true
    setRightPanelCollapsed(false)
    setWorkbenchMaximized((value) => !value)
  }, [])

  const handleExpandWorkbench = useCallback(() => {
    if (hasOpenExclusiveRightPanel) {
      manualRightPanelExpandedOverrideRef.current = true
      setRightPanelCollapsed(false)
      if (!renderedExclusiveRightPanel) {
        setActiveExclusiveRightPanel(orderedOpenExclusiveRightPanels[0] ?? null)
      }
      return
    }
    openWorkspacePanel()
  }, [
    hasOpenExclusiveRightPanel,
    openWorkspacePanel,
    orderedOpenExclusiveRightPanels,
    renderedExclusiveRightPanel,
  ])

  const workspacePanelVisibleInRightPanel =
    showWorkspacePanel && renderedExclusiveRightPanel === "workspace" && !rightPanelCollapsed
  const conversationAvailableWidth =
    workbenchOpen && workbenchLayoutMode === "docked"
      ? workbenchAvailableWidth - rightPanelWidth
      : workbenchAvailableWidth
  // The environment card floats over the right edge of the conversation. Shift
  // the transcript + composer out from under it only when the centred column
  // would actually collide, and stop once the remaining lane gets too narrow to
  // read in — from there the card is allowed to overlap.
  const environmentInsetPx = environmentPopoverOpen
    ? environmentInsetWidth({
        available: conversationAvailableWidth,
        lane: ENVIRONMENT_CARD_LANE_PX,
        contentMaxWidth: CHAT_CONTENT_MAX_WIDTH_PX,
        minContentWidth: CHAT_MAIN_MIN_INTERACTIVE_WIDTH,
      })
    : 0
  const activeWorkbenchTabId =
    renderedExclusiveRightPanel === "preview"
      ? filePreview.activeId
      : renderedExclusiveRightPanel === "files"
        ? fileTabs.activeId
        : renderedExclusiveRightPanel

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
        readableSessionId={activeSessionReadable ? session.currentSessionId : null}
        loadingSessionIds={session.loadingSessionIds}
        sessionsLoading={session.sessionsLoading}
        totalUnreadCount={session.totalUnreadCount}
        panelWidth={panelWidth}
        renderedWidth={sidebarSqueezedWidth}
        sidebarCollapsed={sidebarCollapsed}
        onPanelWidthChange={setPanelWidth}
        onSidebarCollapsedChange={handleSidebarCollapsedChange}
        onSwitchSession={handleSwitchSession}
        onNewChat={handleStartNewChat}
        onArchiveSession={session.handleArchiveSession}
        onEditAgent={onOpenAgentSettings}
        onToggleSessionPinned={session.handleToggleSessionPinned}
        onReorderAgents={session.handleReorderAgents}
        onMarkAllRead={refreshUnreadState}
        onRenameSession={handleRenameSession}
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
        unreadFocusSignal={unreadFocusSignal}
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
        onQuote={handleFileQuote}
        onOpenStructuredMemory={(projectId) => {
          setProjectOverviewOpen(false)
          requestMemoryFocus(
            {
              kind: "claims",
              statusFilter: "active",
              scopeType: "project",
              scopeId: projectId,
            },
            { updateUrl: false },
          )
          onOpenSettings?.("memory")
        }}
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
      <AlertDialog open={!!incognitoLeaveIntent} onOpenChange={handleIncognitoLeaveOpenChange}>
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
      <div
        ref={workbenchContainerRef}
        className="relative flex min-w-0 flex-1 flex-col bg-background"
      >
        {/* Divider anchor — excludes the Terminal dock so the line stops above
            it. No `overflow-hidden`: the split row below already clips. */}
        <div className="relative flex min-h-0 flex-1 flex-col">
          {workbenchOpen && workbenchLayoutMode === "docked" && !workbenchMaximized && (
            <WorkbenchResizeHandle
              width={rightPanelWidth}
              availableWidth={workbenchAvailableWidth}
              resizeLabel={t("chat.rightPanel.resize", "Resize workbench")}
              onWidthChange={setRightPanelWidth}
              onWidthCommit={commitRightPanelWidth}
              onResetWidth={resetRightPanelWidth}
              onResizingChange={setWorkbenchResizing}
            />
          )}
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
            onScheduleSession={canScheduleCurrentSession ? openScheduleSession : undefined}
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
            workbenchWidth={rightPanelWidth}
            workbenchLayoutMode={workbenchLayoutMode}
            workbenchTabs={workbenchTabs}
            workbenchLaunchItems={titleBarRightPanels}
            activeWorkbenchTabId={activeWorkbenchTabId}
            workbenchCollapsed={rightPanelCollapsed}
            workbenchDocked={hasOpenExclusiveRightPanel}
            workbenchResizing={workbenchResizing}
            workbenchMaximized={workbenchMaximized}
            onSelectWorkbenchTab={handleSelectWorkbenchTab}
            onOpenWorkbenchPanel={handleOpenWorkbenchPanel}
            onCloseWorkbenchTab={handleCloseWorkbenchTab}
            onReorderWorkbenchTabs={handleReorderWorkbenchTabs}
            onToggleWorkbenchTabWindow={handleToggleWorkbenchTabWindow}
            onToggleWorkbenchMaximize={handleToggleWorkbenchMaximize}
            onCollapseWorkbench={handleCollapseWorkbench}
            onExpandWorkbench={handleExpandWorkbench}
            onStatusOpenChange={setEnvironmentPopoverOpen}
            onOpenWorkspace={openWorkspacePanel}
            terminalOpen={terminalOpen}
            onToggleTerminal={() => setTerminalOpen((value) => !value)}
          />

          {scheduleSessionTarget && (
            <CronJobForm
              sessionTarget={scheduleSessionTarget}
              onSave={() => setScheduleSessionTarget(null)}
              onCancel={() => setScheduleSessionTarget(null)}
            />
          )}

          <BrowserExtensionNudge
            sessionId={session.currentSessionId}
            onOpenSettings={onOpenSettings}
          />

          <div className="flex min-h-0 flex-1 overflow-hidden">
            <div
              className={cn(
                "relative min-w-0 flex-1 flex-col",
                workbenchOpen && workbenchLayoutMode === "stage" ? "hidden" : "flex",
              )}
              inert={workbenchOpen && workbenchLayoutMode === "stage" ? true : undefined}
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

              {forkSourceSession && (
                <div className="border-b border-border/60 px-4 py-2">
                  <button
                    type="button"
                    onClick={() => void rawHandleSwitchSession(forkSourceSession.id)}
                    className="mx-auto flex max-w-[880px] items-center gap-2 rounded-lg px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground"
                  >
                    <GitFork className="h-3.5 w-3.5 shrink-0" />
                    <span className="shrink-0">
                      {t("chat.fork.continuedFrom", {
                        defaultValue: "接续自",
                      })}
                    </span>
                    <span className="min-w-0 truncate font-medium text-foreground/80">
                      {forkSourceSession.title}
                    </span>
                  </button>
                </div>
              )}

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
                  anchorLatestTurn
                  heroComposer={heroComposerActive}
                  projectName={currentProject?.name ?? null}
                  onProjectSuggestion={currentProject ? handleProjectWelcomeSuggestion : undefined}
                  pendingScrollIntent={session.pendingScrollIntent}
                  onScrollTargetHandled={session.clearPendingScrollIntent}
                  pendingQuestionGroup={planMode.pendingQuestionGroup}
                  onQuestionSubmitted={() => {
                    planMode.setPendingQuestionGroup(null)
                    void planMode.refreshPendingQuestion()
                  }}
                  planCardData={
                    planMode.planCardInfo ? { title: planMode.planCardInfo.title } : null
                  }
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
                  onOpenSubagentRun={handleOpenSubagentRun}
                  subagentRunsSnapshot={subagentRuns}
                  bottomInset={isCronSession || isSubagentSession}
                  onOpenDiff={diffPanel.openDiff}
                  onResume={(message) => {
                    void stream.handleSend(message)
                  }}
                  onForkFromMessage={handleForkFromMessage}
                  onEditAndResend={
                    !isCronSession && !isSubagentSession && stream.pendingSends.length === 0
                      ? handleEditAndResend
                      : undefined
                  }
                  onOpenMemorySettings={onOpenSettings ? () => onOpenSettings("memory") : undefined}
                  onConfigureVisionBridge={
                    onOpenSettings ? () => onOpenSettings("modelConfig", "visionBridge") : undefined
                  }
                  onOpenKnowledge={onOpenKnowledge}
                  onAddQuickPrompt={incognitoEnabled ? undefined : handleAddQuickPrompt}
                  onAddMessageQuote={handleMessageQuote}
                  onAskInSideChat={canUseSideChat ? handleAskInSideChat : undefined}
                  displayMode={displayMode}
                  autoCollapseCompletedTurns={autoCollapseCompletedTurns}
                  onAtBottomChange={setMessageTailVisible}
                  environmentInsetPx={environmentInsetPx}
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
                    // Same box as the transcript scroller (its `px-4` left) so
                    // both centre on the identical axis while inset.
                    style={
                      environmentInsetPx
                        ? { paddingLeft: 16, paddingRight: environmentInsetPx }
                        : undefined
                    }
                  >
                    {(activeMemoryToast || memoryToast) && (
                      <div
                        className={cn(
                          "absolute bottom-full mb-2 flex flex-col gap-2 animate-in fade-in slide-in-from-bottom-2 duration-300 z-10",
                          emptySessionInputHero
                            ? "inset-x-5 mx-auto max-w-[880px] sm:inset-x-8"
                            : "inset-x-3 mx-auto max-w-[880px]",
                        )}
                      >
                        {activeMemoryToast && (
                          <div className="flex items-center gap-2 rounded-lg border border-primary/15 bg-primary/8 px-3 py-1.5 text-xs text-foreground shadow-sm">
                            <Brain className="h-3.5 w-3.5 shrink-0 text-primary" />
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-1.5">
                                <span className="font-medium">
                                  {t("memory.activeRecallToastTitle", "已引用记忆")}
                                </span>
                                {activeMemoryToast.selected && (
                                  <span className="truncate text-[10px] text-muted-foreground">
                                    {memorySourceLabel(activeMemoryToast.selected, t)}
                                  </span>
                                )}
                              </div>
                              <div className="truncate text-muted-foreground">
                                {activeMemoryToast.summary}
                              </div>
                            </div>
                            <button
                              onClick={() => setActiveMemoryToast(null)}
                              className="ml-auto text-muted-foreground/60 hover:text-muted-foreground"
                              aria-label={t("common.close", "关闭")}
                            >
                              ×
                            </button>
                          </div>
                        )}
                        {memoryToast && (
                          <div className="flex items-center gap-2 rounded-lg bg-secondary/50 px-3 py-1.5 text-xs text-muted-foreground">
                            <Brain className="h-3.5 w-3.5 shrink-0" />
                            <span>
                              {t("settings.memoryExtractedToast", { count: memoryToast.count })}
                            </span>
                            <button
                              onClick={() => setMemoryToast(null)}
                              className="ml-auto text-muted-foreground/60 hover:text-muted-foreground"
                              aria-label={t("common.close", "关闭")}
                            >
                              ×
                            </button>
                          </div>
                        )}
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
                          <ChatWelcomeHero
                            incognito={incognitoEnabled}
                            projectName={currentProject?.name ?? null}
                            onProjectSuggestion={
                              currentProject ? handleProjectWelcomeSuggestion : undefined
                            }
                          />
                        </div>
                      )}
                      <ChatInput
                        topAccessory={
                          canUseSideChat ? (
                            <SideChatTray
                              chats={visibleSideChats}
                              activeId={visibleActiveSideChatId}
                              panelOpen={visibleSideChatPanelOpen}
                              creating={sideChatCreating}
                              onCreate={() => void createSideChat()}
                              onSelect={(sessionId) => void revealSideChat(sessionId)}
                              onClosePanel={() => setSideChatPanelOpen(false)}
                            />
                          ) : !session.currentSessionId && !incognitoEnabled ? (
                            <ProjectSessionDraftBar
                              project={currentProject}
                              projects={projects}
                              draft={draftProjectRuntime}
                              disabled={session.loading}
                              progressStage={projectBootstrapProgress?.stage ?? null}
                              progressError={projectBootstrapProgress?.error ?? null}
                              onDraftChange={handleProjectRuntimeDraftChange}
                              onSelectProject={(projectId, defaultAgentId) => {
                                void handleNewChatInProject(projectId, defaultAgentId)
                              }}
                              onRemoveProject={() => {
                                void handleStartNewChat(currentAgentId)
                              }}
                              onRetry={() => {
                                setProjectBootstrapProgress(null)
                                window.setTimeout(() => {
                                  void stream.handleSend()
                                }, 0)
                              }}
                              onUseLocal={() => {
                                handleProjectRuntimeDraftChange({
                                  ...draftProjectRuntime,
                                  launchMode: "local",
                                })
                              }}
                            />
                          ) : undefined
                        }
                        input={stream.input}
                        structuredMentions={stream.typedMentions}
                        onInputChange={stream.setInput}
                        onInputChangeWithMention={stream.setInputWithMention}
                        inputHistory={inputHistory}
                        quickPrompts={quickPrompts}
                        onSend={() =>
                          stream.handleSend(
                            undefined,
                            shouldSendDraftWorkflowMode(
                              session.currentSessionId,
                              incognitoEnabled,
                              draftWorkflowMode,
                            )
                              ? { workflowMode: draftWorkflowMode }
                              : undefined,
                          )
                        }
                        sendDisabled={
                          session.historyLoading ||
                          (draftProjectRuntime.launchMode === "worktree" &&
                            (!draftProjectBootstrap || session.loading))
                        }
                        loading={session.loading}
                        availableModels={availableModels}
                        activeModel={activeModel}
                        unavailableModelPreference={unavailableModelPreference}
                        reasoningEffort={reasoningEffort}
                        onModelChange={handleManualModelChange}
                        onEffortChange={handleSessionEffortChange}
                        onEffortReset={handleSessionEffortReset}
                        attachedFiles={stream.attachedFiles}
                        maxAttachmentBytes={stream.maxChatAttachmentBytes}
                        onAttachFiles={(files) =>
                          stream.setAttachedFiles((prev) => [...prev, ...files])
                        }
                        onRemoveFile={(index) =>
                          stream.setAttachedFiles((prev) => prev.filter((_, i) => i !== index))
                        }
                        onUpdateFile={(index, file) =>
                          stream.setAttachedFiles((prev) =>
                            prev.map((existing, i) =>
                              i === index
                                ? { ...existing, file, status: "ready", error: undefined }
                                : existing,
                            ),
                          )
                        }
                        pendingQuotes={stream.pendingQuotes}
                        onRemoveQuote={(index) => {
                          stream.setPendingQuotes((prev) => prev.filter((_, i) => i !== index))
                          fileTabs.clearReveals() // dropping a quote clears its reveal highlight
                        }}
                        onJumpToQuote={handleQuoteJump}
                        pendingMessageQuotes={stream.pendingMessageQuotes}
                        onRemoveMessageQuote={(index) =>
                          stream.setPendingMessageQuotes((prev) =>
                            prev.filter((_, i) => i !== index),
                          )
                        }
                        focusSignal={composerFocusSignal}
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
                        onSendPending={stream.sendPendingSend}
                        onForceInsertPending={stream.forceInsertPendingSend}
                        onCancelForceInsertPending={stream.cancelForceInsertPendingSend}
                        onStop={stream.handleStop}
                        stopPending={stream.stopPendingSessions.has(
                          session.currentSessionId ?? "__pending__",
                        )}
                        autonomyPaused={currentSessionMeta?.autonomyPaused ?? false}
                        onContinue={stream.handleContinue}
                        currentSessionId={session.currentSessionId}
                        currentAgentId={session.currentAgentId}
                        onEnsureSession={ensureWorkflowSession}
                        onCommandAction={handleCommandAction}
                        enableSideChatCommand={canUseSideChat}
                        permissionMode={stream.permissionMode}
                        onPermissionModeChange={stream.setPermissionModeByUser}
                        sandboxMode={stream.sandboxMode}
                        onSandboxModeChange={stream.setSandboxModeByUser}
                        sessionTemperature={sessionTemperature}
                        onSessionTemperatureChange={handleSessionTemperatureChange}
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

              {visibleSideChatPanelOpen && visibleActiveSideChatId ? (
                <SideChatPanel
                  key={visibleActiveSideChatId}
                  sessionId={visibleActiveSideChatId}
                  isViewVisible={isViewVisible}
                  title={visibleActiveSideChat?.title}
                  workingDir={activeSideChatWorkingDir}
                  seed={sideChatSeed}
                  onClose={() => setSideChatPanelOpen(false)}
                  onActivity={() => void refreshSideChats()}
                  onCodexReauth={onCodexReauth}
                  onDeleted={handleSideChatDeleted}
                  onPreviewFile={(target) =>
                    openSideChatFileTarget(visibleActiveSideChatId, target)
                  }
                />
              ) : null}
            </div>

            {/* Always mounted, `empty` when nothing is open: the Canvas panel
                owns the only `canvas_show` listener and the session's canvas
                restore, so unmounting it makes the canvas unreachable (and
                remounting it re-adopts a canvas the user just closed). */}
            <WorkbenchSurface
              width={rightPanelWidth}
              layoutMode={workbenchLayoutMode}
              collapsed={rightPanelCollapsed}
              resizing={workbenchResizing}
              maximized={workbenchMaximized}
              empty={!hasOpenExclusiveRightPanel}
            >
              {/* Diff panel (right side, selected from the title-bar panel switcher) */}
              {rightPanelVisibility.diff && (
                <RightPanelShell
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "diff"}
                  animateOnMount={animateRightPanelOnMount}
                  contentKey="diff"
                >
                  <DiffPanel
                    changes={diffPanel.activeChanges}
                    activeIndex={diffPanel.activeIndex}
                    openNonce={diffPanel.openNonce}
                    onActiveIndexChange={diffPanel.setActiveIndex}
                    onClose={diffPanel.closeDiff}
                    onPreviewFile={openFileTarget}
                    gitContext={diffPanel.gitContext}
                    onGitSnapshotChange={diffPanel.replaceGitDiff}
                    embedded
                  />
                </RightPanelShell>
              )}

              {rightPanelVisibility["pull-request"] && session.currentSessionId && (
                <RightPanelShell
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "pull-request"}
                  animateOnMount={animateRightPanelOnMount}
                  contentKey={`pull-request:${session.currentSessionId}`}
                >
                  <PullRequestPanel
                    sessionId={session.currentSessionId}
                    expectedUrl={pullRequestExpectedUrl}
                    onFillInput={stream.setInput}
                    integrated
                    onClose={() => {
                      setShowPullRequestPanel(false)
                      setPullRequestExpectedUrl(null)
                    }}
                  />
                </RightPanelShell>
              )}

              {/* Plan workspace (right side, integrated under the shared title bar) */}
              {rightPanelVisibility.plan && (
                <RightPanelShell
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "plan"}
                  animateOnMount={animateRightPanelOnMount}
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
              {fileTabs.tabs.map((tab) => (
                <FileBrowserPanel
                  key={tab.id}
                  scope={fileBrowserScope}
                  scopeId={fileBrowserScopeId}
                  rootPath={effectiveWorkingDir}
                  linkedRootPaths={projectFileBrowserRoots}
                  sessionId={session.currentSessionId}
                  instanceKey={tab.id}
                  visible={rightPanelVisibility.files}
                  collapsed={
                    rightPanelCollapsed ||
                    renderedExclusiveRightPanel !== "files" ||
                    fileTabs.activeId !== tab.id
                  }
                  animateOnMount={animateRightPanelOnMount}
                  onQuote={handleFileQuote}
                  onOpenInNewTab={openFileTargetInNewTab}
                  onSelectionChange={(selection) => fileTabs.setTabSelection(tab.id, selection)}
                  revealFile={tab.revealFile}
                  revealDirectory={tab.revealDirectory}
                  onClose={() => {
                    if (!confirmFilesPanelClose(tab.id)) return false
                    fileTabs.closeTab(tab.id)
                    return true
                  }}
                />
              ))}

              {/* Canvas Preview Panel */}
              <CanvasPanel
                currentSessionId={currentSessionId}
                onOpenChange={setCanvasPanelOpen}
                onQuote={handleFileQuote}
                collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "canvas"}
                animateOnMount={animateRightPanelOnMount}
                visible={rightPanelVisibility.canvas}
              />

              {/* Browser live-mirror panel — open on first `browser:frame` push,
              close-only by user, then switchable from the title bar. */}
              {rightPanelVisibility.browser && (
                <BrowserPanel
                  sessionId={session.currentSessionId}
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "browser"}
                  animateOnMount={animateRightPanelOnMount}
                  onClose={() => {
                    browserPanelDismissedRef.current = true
                    setShowBrowserPanel(false)
                  }}
                  onFloat={() => floatPanel("browser")}
                />
              )}

              {/* Mac Control live-mirror panel — opens on tool-produced managed
              screenshot frames; panel polling frames only refresh an already
              open panel and must not re-open after a session switch. */}
              {rightPanelVisibility["mac-control"] && (
                <MacControlPanel
                  sessionId={session.currentSessionId}
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "mac-control"}
                  animateOnMount={animateRightPanelOnMount}
                  onClose={() => {
                    macControlPanelDismissedRef.current = true
                    setShowMacControlPanel(false)
                  }}
                  onFloat={() => floatPanel("mac-control")}
                />
              )}

              {/* Team Panel */}
              {rightPanelVisibility.team && activeTeamId && (
                <TeamPanel
                  teamId={activeTeamId}
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "team"}
                  animateOnMount={animateRightPanelOnMount}
                  onClose={() => setShowTeamPanel(false)}
                  onViewSession={setSubagentPreviewSessionId}
                />
              )}

              {/* Workspace 面板 — 聚合任务进度 / 碰到的文件 / 引用来源 */}
              {rightPanelVisibility.workspace && (
                <RightPanelShell
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "workspace"}
                  animateOnMount={animateRightPanelOnMount}
                  contentKey="workspace"
                >
                  <WorkspacePanel
                    taskSnapshot={taskProgressSnapshot}
                    taskExecutionState={workspaceTaskExecutionState}
                    messages={session.messages}
                    contextUsageOverride={contextUsage}
                    onOpenDiff={diffPanel.openDiff}
                    onOpenGitDiff={diffPanel.openGitDiff}
                    onFillInput={stream.setInput}
                    onOpenPullRequest={openPullRequestPanel}
                    onPreviewFile={openFileTarget}
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
                    onViewSubagentSession={(sid) => openSubagentPanel({ childSessionId: sid })}
                    subagentRunsState={subagentRuns}
                    focusRequest={workspaceFocusRequest}
                    onFocusRequestHandled={handleWorkspaceFocusRequestHandled}
                    onEnsureSession={ensureWorkflowSession}
                    draftWorkflowMode={draftWorkflowMode}
                    onDraftWorkflowModeChange={setDraftWorkflowMode}
                    integrated
                    onClose={() => {
                      workspacePanelDismissedRef.current = true
                      setWorkspaceFocusRequest(null)
                      setShowWorkspacePanel(false)
                    }}
                  />
                </RightPanelShell>
              )}

              {/* Background-jobs panel (R4) — session jobs (cancellable) + a
              read-only mirror of global local-model jobs. */}
              {rightPanelVisibility["background-jobs"] && (
                <RightPanelShell
                  collapsed={
                    rightPanelCollapsed || renderedExclusiveRightPanel !== "background-jobs"
                  }
                  animateOnMount={animateRightPanelOnMount}
                  contentKey="background-jobs"
                >
                  <BackgroundJobsPanel
                    jobs={backgroundJobs.jobs}
                    jobExpansionOverrides={backgroundJobExpansionOverrides}
                    onJobExpandedChange={handleBackgroundJobExpandedChange}
                    onClose={closeBackgroundJobsPanel}
                    onViewSubagentSession={(sid) => openSubagentPanel({ childSessionId: sid })}
                    integrated
                  />
                </RightPanelShell>
              )}

              {/* Sub-agent panel — this session's sub-agent runs + the selected
              run's live child-session transcript. Opened from inline chips. */}
              {rightPanelVisibility.subagent && (
                <RightPanelShell
                  collapsed={rightPanelCollapsed || renderedExclusiveRightPanel !== "subagent"}
                  animateOnMount={animateRightPanelOnMount}
                  contentKey={`subagent:${session.currentSessionId ?? ""}`}
                >
                  <SubagentPanel
                    sessionId={session.currentSessionId}
                    runsState={subagentRuns}
                    agents={session.agents}
                    selectRequest={subagentPanelSelectRequest}
                    onClose={closeSubagentPanel}
                    integrated
                  />
                </RightPanelShell>
              )}

              {/* Each preview tab stays mounted so switching files preserves its
              renderer, scroll position and local selection. */}
              {filePreview.entries.map((entry) => (
                <RightPanelShell
                  key={entry.id}
                  collapsed={
                    rightPanelCollapsed ||
                    renderedExclusiveRightPanel !== "preview" ||
                    filePreview.activeId !== entry.id
                  }
                  animateOnMount={animateRightPanelOnMount}
                  contentKey={entry.id}
                >
                  <FilePreviewPanel
                    target={entry.target}
                    sessionId={entry.authorizationSessionId ?? session.currentSessionId}
                    onReplaceDraft={replaceDraftAttachment}
                    onQuote={handleFileQuote}
                    onClose={() => closeFilePreview(entry.id)}
                    integrated
                    onNavigateDirectory={(dirPath) => revealPreviewDirectory(entry.target, dirPath)}
                    canNavigateDirectory={(dirPath) =>
                      canRevealPreviewDirectory(entry.target, dirPath)
                    }
                  />
                </RightPanelShell>
              ))}
            </WorkbenchSurface>

            {/* In-app floating control-panel windows stay alive when the workbench is collapsed. */}
            <FloatingPanelLayer
              floatingPanels={floatingPanelList}
              zIndexOf={floatingPanelZIndex}
              onDock={(panel: FloatablePanel) => {
                dockFloatingPanel(panel)
                showRightPanelByUser(panel)
              }}
              onClose={(panel: FloatablePanel) => {
                closeFloatingPanel(panel)
                if (panel === "browser") {
                  browserPanelDismissedRef.current = true
                  setShowBrowserPanel(false)
                } else {
                  macControlPanelDismissedRef.current = true
                  setShowMacControlPanel(false)
                }
              }}
              onFocus={focusFloatingPanel}
              sessionId={session.currentSessionId}
            />
          </div>
        </div>
        <TerminalPanel
          open={terminalOpen}
          workingDir={effectiveWorkingDir}
          onOpenChange={setTerminalOpen}
        />
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
