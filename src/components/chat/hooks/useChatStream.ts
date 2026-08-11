import { useState, useRef, useEffect, useCallback, useLayoutEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import {
  type ChatAttachment,
  type ProjectSessionBootstrapInput,
  type Transport,
} from "@/lib/transport"
import {
  maxChatAttachmentBytes as attachmentBytesForConfig,
  readFilesystemConfig,
  useFilesystemConfig,
} from "@/lib/filesystemConfig"
import { toast } from "sonner"
import type { DraftAttachment } from "@/components/chat/files/types"
import type { KbDraftAttachment } from "@/types/knowledge"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import {
  getCachedConfig,
  loadNotificationConfig,
  isAgentNotifyEnabled,
  notify,
  notifyIfBackground,
} from "@/lib/notifications"
import type {
  Message,
  MessageAttachment,
  PendingFileQuote,
  PendingMessageQuote,
  PendingSendPreview,
  ActiveModel,
  AgentSummaryForSidebar,
  SandboxMode,
  SessionMeta,
  SessionMessage,
  SessionMode,
  ChatTurnStatus,
  ChatTurnInterruptReason,
} from "@/types/chat"
import { parseSessionMessages } from "../chatUtils"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import {
  createStreamDeltaBuffers,
  discardAllPendingStreamDeltas,
  discardPendingStreamDeltas,
  handleStreamEvent,
  isStreamEnded,
  markStreamActive,
  markStreamEnded,
  streamCursorKey,
  streamIdFromEvent,
  streamIdFromPayload,
  type EndedStreamIds,
} from "./useStreamEventHandler"
import { useApprovals } from "./useApprovals"
import { generateClientId } from "@/components/chat/chatScrollKeys"
import { expandMentionsToAttachments } from "@/components/chat/file-mention/expandMentions"
import { expandPlanMentionsToAttachments } from "@/components/chat/plan-mention/expandPlanMentions"
import {
  buildIncomingTurnWire,
  mergeTypedMentionDrafts,
  reconcileTypedMentions,
  reconcileTypedMentionsForChange,
  trimTextWithTypedMentions,
  type ComposerMentionBinding,
  type ComposerMentionTextChange,
} from "@/components/chat/mentions/typedMentions"
import { useNotificationListeners } from "./useNotificationListeners"
import type { SessionStreamState } from "./useChatStreamReattach"
import { modelOverrideFromManualSelection } from "../modelSelection"
import {
  hasSendableChatPayload,
  nextDispatchablePending,
  shouldApplyPendingQueueSnapshot,
  shouldReplayNextPending,
} from "./pendingQueue"
import {
  AUTO_SEND_PENDING_EVENT,
  normalizeAutoSendPendingPreference,
} from "../autoSendPendingPreference"
import {
  awaitUnlessAborted,
  beginChatBackendHandoff,
  deferActiveTurnRelease,
  ChatPreparationCancelledError,
  discardChatAttachmentUploads,
  isChatPreparationCancelled,
  loadingStateAfterPreparationRelease,
  shouldRollbackNonPersistedStoppedSend,
  validateChatAttachmentCount,
} from "./chatPreparation"

const ACTIVE_STREAM_ERROR_CODE = "active_stream"
const QUEUED_MESSAGE_UNAVAILABLE_ERROR_CODE = "queued_message_unavailable"
const CHAT_CANCELLED_DURING_PREFLIGHT_CODE = "chat_cancelled_during_preflight"
const CHAT_NOTIFICATION_PREVIEW_MAX_CHARS = 220

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === "string") return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

function isActiveStreamError(error: unknown): boolean {
  return errorText(error).includes(ACTIVE_STREAM_ERROR_CODE)
}

function isQueuedMessageUnavailableError(error: unknown): boolean {
  const text = errorText(error)
  return (
    text.includes(QUEUED_MESSAGE_UNAVAILABLE_ERROR_CODE) ||
    text.includes("Queued message is no longer available")
  )
}

function isPreflightStopError(error: unknown): boolean {
  return errorText(error).includes(CHAT_CANCELLED_DURING_PREFLIGHT_CODE)
}

function normalizeNotificationText(text: string): string {
  return text.replace(/\s+/g, " ").trim()
}

function truncateNotificationText(text: string): string {
  if (text.length <= CHAT_NOTIFICATION_PREVIEW_MAX_CHARS) return text
  return `${text.slice(0, CHAT_NOTIFICATION_PREVIEW_MAX_CHARS - 3).trimEnd()}...`
}

function assistantNotificationText(message: Message): string {
  const blockText = message.contentBlocks
    ?.filter((block) => block.type === "text")
    .map((block) => block.content)
    .join("\n\n")
  return truncateNotificationText(normalizeNotificationText(blockText || message.content || ""))
}

function userNotificationText(message: Message): string {
  const blockText = message.contentBlocks
    ?.filter((block) => block.type === "text")
    .map((block) => block.content)
    .join("\n\n")
  return truncateNotificationText(normalizeNotificationText(blockText || message.content || ""))
}

function latestAssistantNotificationPreview(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (message.role !== "assistant") continue
    const text = assistantNotificationText(message)
    if (text) return text
  }
  return null
}

function latestUserNotificationPreview(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (message.role !== "user") continue
    const text = userNotificationText(message)
    if (text) return text
  }
  return null
}

function chatCompletionNotificationPayload(
  sessionTitle: string,
  messages: Message[],
  showChatContent: boolean,
  genericTitle: string,
  genericBody: string,
): { title: string; body: string } {
  if (!showChatContent) return { title: genericTitle, body: genericBody }
  const preview = latestAssistantNotificationPreview(messages)
  return { title: sessionTitle, body: preview || genericBody }
}

function optimisticAttachmentForFile(file: File): MessageAttachment {
  const mimeType = file.type || "application/octet-stream"
  return {
    name: file.name,
    mimeType,
    sizeBytes: file.size,
    kind: mimeType.toLowerCase().startsWith("image/") ? "image" : "file",
    ...(mimeType.toLowerCase().startsWith("image/")
      ? { previewUrl: URL.createObjectURL(file) }
      : {}),
  }
}

interface SendOptions {
  displayText?: string
  /** First-party structured bindings for a direct send such as `/skill`.
   * The backend revalidates their source anchors and resolves live content. */
  structuredMentions?: ComposerMentionBinding[]
  planMode?: string
  workflowMode?: "off" | "on" | "ultracode" | string
  isPlanTrigger?: boolean
  goalTrigger?: boolean
  initialGoal?: {
    objective: string
    completionCriteria?: string
  }
  sessionIdOverride?: string
  queuedRequestId?: string
  /** Existing latest persisted user message being replaced atomically by the
   * chat request. Never valid for durable queued sends. */
  editMessageId?: number
  /** One-shot attachment/quote payload used by inline message edit + resend.
   * It bypasses the main composer so an unrelated draft is preserved. */
  draftOverride?: {
    attachedFiles: DraftAttachment[]
    pendingQuotes: PendingFileQuote[]
    pendingMessageQuotes: PendingMessageQuote[]
  }
  /** Resolves inline editing as soon as the replacement turn is durable. */
  onDispatchAccepted?: () => void
  onPreparationError?: (error: unknown) => void
  /** Routed through the chat command into `attachments_meta.plan_comment`
   *  so the desktop GUI can render PlanCommentBubble with structured
   *  selection + comment fields. IM channels ignore this and use displayText. */
  planComment?: { selectedText: string; comment: string }
}

interface PendingSend {
  id: string
  sessionId: string
  createdAt: number
  text: string
  mode: "queue" | "force_insert"
  status:
    | "saving"
    | "queued"
    | "waiting_tool_boundary"
    | "inserting"
    | "dispatching"
    | "fallback_after_reply"
    | "held_after_stop"
  options?: SendOptions
  attachedFiles?: DraftAttachment[]
  quotes?: PendingFileQuote[]
  messageQuotes?: PendingMessageQuote[]
  attachmentCount?: number
  quoteCount?: number
  isPlanTrigger?: boolean
  goalTrigger?: boolean
  planComment?: { selectedText: string; comment: string }
  /** Authoritative backend projection; absent for the local `saving` row. */
  canForceInsert?: boolean
  editable?: boolean
  managedBy?: "channel"
}

interface QueuedTurnMessageView {
  requestId: string
  sessionId: string
  turnId?: string
  message: string
  displayText?: string
  attachmentCount: number
  quoteCount: number
  isPlanTrigger: boolean
  goalTrigger: boolean
  planComment?: { selectedText: string; comment: string }
  planMode?: string
  workflowMode?: string
  managedBy?: "channel"
  canForceInsert: boolean
  mode: "queue" | "force_insert"
  status:
    | "queued"
    | "waiting_tool_boundary"
    | "inserting"
    | "dispatching"
    | "fallback_after_reply"
    | "held_after_stop"
  createdAt: string
  updatedAt: string
}

interface QueueTurnUserMessageResult {
  queued: boolean
  requestId: string
  reason?: string
  item?: QueuedTurnMessageView
}

interface CancelQueuedTurnUserMessageResult {
  cancelled: boolean
  reason?: string
}

interface InputDraft {
  input: string
  typedMentions: ComposerMentionBinding[]
  attachedFiles: DraftAttachment[]
  pendingQuotes: PendingFileQuote[]
  pendingMessageQuotes: PendingMessageQuote[]
}

function inputDraftKey(sessionId: string | null): string {
  return sessionId ? `session:${sessionId}` : "draft"
}

export interface UseChatStreamOptions {
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  currentSessionId: string | null
  setCurrentSessionId: React.Dispatch<React.SetStateAction<string | null>>
  currentSessionIdRef: React.MutableRefObject<string | null>
  currentAgentId: string
  agentName: string
  loading: boolean
  setLoading: React.Dispatch<React.SetStateAction<boolean>>
  loadingSessionsRef: React.MutableRefObject<Set<string>>
  setLoadingSessionIds: React.Dispatch<React.SetStateAction<Set<string>>>
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
  /** Bound the messages array post-append. Returns `msgs` unchanged when
   *  under cap. Optional — QuickChat / Window paths don't participate in
   *  the main-app LRU/cap and can omit. */
  capMessagesForSession?: (sessionId: string, msgs: Message[]) => Message[]
  /** Bumps the LRU position of `sessionId`. Called on cache writes that
   *  don't otherwise route through `handleSwitchSession` (new-session
   *  rename in particular). */
  touchSessionCacheLru?: (sessionId: string) => void
  sessions: Pick<SessionMeta, "id" | "title" | "workingDir" | "permissionMode" | "sandboxMode">[]
  agents: AgentSummaryForSidebar[]
  /** Display-only compatibility input; never converted into a strict override. */
  activeModel?: ActiveModel | null
  /** Explicit picker intent. Display-only model restoration must never populate this ref. */
  manualModelOverrideRef?: React.MutableRefObject<ActiveModel | null>
  reloadSessions: () => Promise<void>
  updateSessionMessages: (sessionId: string, updater: (prev: Message[]) => Message[]) => void
  /**
   * Per-session seq cursor shared with `useChatStreamReattach`. Primary-path
   * `onmessage` bumps it so redundant EventBus events are dropped.
   */
  lastSeqRef: React.MutableRefObject<Map<string, number>>
  /** Latest stream id that has ended for each session. Used to drop delayed
   *  primary frames that arrive after DB reconciliation. */
  endedStreamIdsRef: React.MutableRefObject<EndedStreamIds>
  /** Current plan mode state, passed to backend chat() for reliable sync */
  planMode?: string
  /** Session-level temperature override (0.0–2.0). Overrides agent and global settings. */
  temperatureOverride?: number | null
  /** Session-level Think / reasoning effort. */
  reasoningEffort?: string | null
  /** New-chat preset; only applied when the backend auto-creates a session. */
  incognitoEnabled?: boolean
  /**
   * Draft working dir picked before the session was materialized. Sent to the
   * `chat` command only when no `sessionId` is set yet — the backend applies it
   * on the auto-create branch.
   */
  draftWorkingDir?: string | null
  /**
   * Project bound to a not-yet-materialized chat (lazy project session). Like
   * draftWorkingDir, it rides on the `chat` command payload (`projectId`) as a
   * send-time snapshot and the backend binds the auto-created session to it —
   * sent only when no `sessionId` is set yet.
   */
  draftProjectId?: string | null
  /** Worktree launch configuration staged alongside a lazy project session. */
  draftProjectBootstrap?: ProjectSessionBootstrapInput | null
  /** Surface pre-session bootstrap failures back to the draft control bar. */
  onProjectBootstrapFailure?: (message: string) => void
  /**
   * KB attaches staged before the session existed (composer draft mode). Like
   * draftWorkingDir, they ride on the `chat` command payload (`kbAttachments`)
   * as a send-time snapshot and the backend applies them on the auto-create
   * branch via `apply_draft_attachments` — sent only when no `sessionId` is set
   * yet and not incognito.
   */
  draftKbAttachments?: KbDraftAttachment[]
  /**
   * Knowledge-space sidebar chat: the note open when the conversation started.
   * Sent only on the auto-create send so the backend promotes the new session
   * into a KB chat thread anchored to it (history / default-load key).
   */
  draftKbAnchorNote?: string | null
  /**
   * Tool-visibility scope forwarded to the `chat` command. The knowledge-space
   * sidebar passes `"knowledge"`, the design-space per-project chat passes
   * `"design"`, to trim the injected tool set; omitted for the main / quick chat
   * (full tools).
   */
  toolScope?: "knowledge" | "design"
  /** First-party message-list + composer identity for pet activity routing.
   * Internal callers and side queries omit this metadata. */
  uiSurface?: "main_chat" | "quick_chat" | "knowledge_chat" | "design_chat" | "pet_chat"
  /**
   * Design-space per-project chat: the design project open when the conversation
   * started. Sent only on the auto-create send (with `toolScope === "design"`)
   * so the backend promotes the new session into a design chat thread anchored
   * to this project (history / default-load key). Mirrors `draftKbAnchorNote`.
   */
  draftDesignProjectId?: string | null
  /**
   * Per-turn extra attachments merged at send time, AFTER the visible composer
   * quotes/files. The knowledge panel uses this to inject the currently-open
   * note as an invisible `quote` so the assistant always sees "the document I'm
   * looking at" without the user manually quoting it. Returns `[]` for callers
   * that don't need it.
   */
  getExtraAttachments?: () => ChatAttachment[]
  /** Called after a persisted session sandbox-mode update succeeds. */
  onSandboxModeSynced?: (sessionId: string, mode: SandboxMode) => void
  /**
   * When true, this surface has `useChatStreamReattach` mounted and should let
   * ParentInjection deltas arrive through `chat:stream_delta` instead of the
   * legacy `parent_agent_stream` delta side channel.
   */
  parentInjectionDeltasViaChatStream?: boolean
}

export interface UseChatStreamReturn {
  input: string
  typedMentions: ComposerMentionBinding[]
  setInput: React.Dispatch<React.SetStateAction<string>>
  setInputWithMention: (
    value: string,
    mention: ComposerMentionBinding,
    change?: ComposerMentionTextChange,
  ) => void
  appendInputMention: (
    token: string,
    mention: Omit<ComposerMentionBinding, "raw" | "start" | "end">,
    trailingSpace?: boolean,
  ) => void
  attachedFiles: DraftAttachment[]
  setAttachedFiles: React.Dispatch<React.SetStateAction<DraftAttachment[]>>
  maxChatAttachmentBytes: number
  pendingQuotes: PendingFileQuote[]
  setPendingQuotes: React.Dispatch<React.SetStateAction<PendingFileQuote[]>>
  pendingMessageQuotes: PendingMessageQuote[]
  setPendingMessageQuotes: React.Dispatch<React.SetStateAction<PendingMessageQuote[]>>
  pendingMessage: string | null
  setPendingMessage: React.Dispatch<React.SetStateAction<string | null>>
  pendingSends: PendingSendPreview[]
  editPendingSend: (id: string, text: string) => Promise<boolean>
  discardPendingSend: (id: string) => Promise<void>
  sendPendingSend: (id: string) => Promise<void>
  forceInsertPendingSend: (id: string) => Promise<void>
  cancelForceInsertPendingSend: (id: string) => Promise<void>
  approvalRequests: ApprovalRequest[]
  showCodexAuthExpired: boolean
  setShowCodexAuthExpired: React.Dispatch<React.SetStateAction<boolean>>
  permissionMode: SessionMode
  setPermissionMode: React.Dispatch<React.SetStateAction<SessionMode>>
  /** User-initiated permission-mode change (switcher / `/permission`). Marks the
   *  draft dirty so a new session's first send carries the chosen mode. */
  setPermissionModeByUser: React.Dispatch<React.SetStateAction<SessionMode>>
  sandboxMode: SandboxMode
  setSandboxMode: React.Dispatch<React.SetStateAction<SandboxMode>>
  /** User-initiated sandbox-mode change. Mirrors permission mode draft behavior. */
  setSandboxModeByUser: React.Dispatch<React.SetStateAction<SandboxMode>>
  handleSend: (directText?: string, options?: SendOptions) => Promise<void>
  handleStop: () => Promise<void>
  handleApprovalResponse: (
    requestId: string,
    response: "allow_once" | "allow_always" | "deny",
  ) => Promise<void>
  handleTurnStarted: (sessionId: string, turnId: string) => void
  handleTurnEnded: (
    sessionId: string,
    status?: ChatTurnStatus | null,
    interruptReason?: ChatTurnInterruptReason | null,
    turnId?: string | null,
  ) => boolean
  executionStateBySession: Map<string, ChatTurnStatus>
}

export function useChatStream({
  messages,
  setMessages,
  currentSessionId,
  setCurrentSessionId,
  currentSessionIdRef,
  currentAgentId,
  agentName,
  loading,
  setLoading,
  loadingSessionsRef,
  setLoadingSessionIds,
  sessionCacheRef,
  capMessagesForSession,
  touchSessionCacheLru,
  sessions,
  agents,
  manualModelOverrideRef,
  reloadSessions,
  updateSessionMessages,
  lastSeqRef,
  endedStreamIdsRef,
  planMode,
  temperatureOverride,
  reasoningEffort,
  incognitoEnabled = false,
  draftWorkingDir = null,
  draftProjectId = null,
  draftProjectBootstrap = null,
  onProjectBootstrapFailure,
  draftKbAttachments = [],
  draftKbAnchorNote = null,
  toolScope,
  uiSurface,
  draftDesignProjectId = null,
  getExtraAttachments,
  onSandboxModeSynced,
  parentInjectionDeltasViaChatStream = false,
}: UseChatStreamOptions): UseChatStreamReturn {
  // Latest draft attaches, snapshotted into the startChat payload at send time
  // (mirrors how draftWorkingDir is baked into the create call) so a later
  // New Chat / session switch can't redirect them onto the wrong session.
  const draftKbAttachmentsRef = useRef(draftKbAttachments)
  // Latest draft project binding, snapshotted at send time so a mid-send project
  // switch can't materialize the session under the wrong project.
  const draftProjectIdRef = useRef(draftProjectId)
  draftProjectIdRef.current = draftProjectId
  draftKbAttachmentsRef.current = draftKbAttachments
  const { t } = useTranslation()
  const { config: filesystemConfig } = useFilesystemConfig()
  const [input, setInputState] = useState("")
  const [attachedFiles, setAttachedFilesState] = useState<DraftAttachment[]>([])
  const maxChatAttachmentBytes = attachmentBytesForConfig(filesystemConfig)
  const [pendingQuotes, setPendingQuotesState] = useState<PendingFileQuote[]>([])
  const [pendingMessageQuotes, setPendingMessageQuotesState] = useState<PendingMessageQuote[]>([])
  const inputRef = useRef(input)
  const typedMentionsRef = useRef<ComposerMentionBinding[]>([])
  const attachedFilesRef = useRef(attachedFiles)
  const pendingQuotesRef = useRef(pendingQuotes)
  const pendingMessageQuotesRef = useRef(pendingMessageQuotes)
  const inputDraftsRef = useRef<Map<string, InputDraft>>(new Map())
  const activeInputDraftKeyRef = useRef(inputDraftKey(currentSessionId))

  const saveInputDraft = useCallback((key: string, draft: InputDraft) => {
    if (
      !draft.input &&
      draft.typedMentions.length === 0 &&
      draft.attachedFiles.length === 0 &&
      draft.pendingQuotes.length === 0 &&
      draft.pendingMessageQuotes.length === 0
    ) {
      inputDraftsRef.current.delete(key)
      return
    }
    inputDraftsRef.current.set(key, draft)
  }, [])

  const setInput = useCallback<React.Dispatch<React.SetStateAction<string>>>(
    (value) => {
      setInputState((prev) => {
        const next = typeof value === "function" ? (value as (p: string) => string)(prev) : value
        const nextMentions = reconcileTypedMentions(prev, next, typedMentionsRef.current)
        inputRef.current = next
        typedMentionsRef.current = nextMentions
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: next,
          typedMentions: nextMentions,
          attachedFiles: attachedFilesRef.current,
          pendingQuotes: pendingQuotesRef.current,
          pendingMessageQuotes: pendingMessageQuotesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  const setInputWithMention = useCallback(
    (next: string, mention: ComposerMentionBinding, change?: ComposerMentionTextChange) => {
      setInputState((prev) => {
        const reconciled = change
          ? reconcileTypedMentionsForChange(prev, next, typedMentionsRef.current, change)
          : reconcileTypedMentions(prev, next, typedMentionsRef.current)
        const nextMentions = [
          ...reconciled.filter((existing) => existing.id !== mention.id),
          mention,
        ].sort((a, b) => a.start - b.start)
        inputRef.current = next
        typedMentionsRef.current = nextMentions
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: next,
          typedMentions: nextMentions,
          attachedFiles: attachedFilesRef.current,
          pendingQuotes: pendingQuotesRef.current,
          pendingMessageQuotes: pendingMessageQuotesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  /** Append a provenance-bearing token against the latest state snapshot.
   * Global pickers may await a transport call before insertion, so deriving
   * offsets from a render-captured `input` would overwrite intervening typing. */
  const appendInputMention = useCallback(
    (
      token: string,
      mentionBase: Omit<ComposerMentionBinding, "raw" | "start" | "end">,
      trailingSpace = false,
    ) => {
      setInputState((prev) => {
        const separator = prev && !prev.endsWith(" ") ? " " : ""
        const suffix = trailingSpace ? " " : ""
        const start = prev.length + separator.length
        const next = `${prev}${separator}${token}${suffix}`
        const mention: ComposerMentionBinding = {
          ...mentionBase,
          raw: token,
          start,
          end: start + token.length,
        }
        const reconciled = reconcileTypedMentionsForChange(prev, next, typedMentionsRef.current, {
          oldStart: prev.length,
          oldEnd: prev.length,
          newEnd: next.length,
        })
        const nextMentions = [
          ...reconciled.filter((existing) => existing.id !== mention.id),
          mention,
        ].sort((a, b) => a.start - b.start)
        inputRef.current = next
        typedMentionsRef.current = nextMentions
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: next,
          typedMentions: nextMentions,
          attachedFiles: attachedFilesRef.current,
          pendingQuotes: pendingQuotesRef.current,
          pendingMessageQuotes: pendingMessageQuotesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  const replaceInputWithMentions = useCallback(
    (next: string, mentions: ComposerMentionBinding[]) => {
      const valid = mentions.filter(
        (mention) => next.slice(mention.start, mention.end) === mention.raw,
      )
      inputRef.current = next
      typedMentionsRef.current = valid
      setInputState(next)
      saveInputDraft(activeInputDraftKeyRef.current, {
        input: next,
        typedMentions: valid,
        attachedFiles: attachedFilesRef.current,
        pendingQuotes: pendingQuotesRef.current,
        pendingMessageQuotes: pendingMessageQuotesRef.current,
      })
    },
    [saveInputDraft],
  )

  const setAttachedFiles = useCallback<React.Dispatch<React.SetStateAction<DraftAttachment[]>>>(
    (value) => {
      setAttachedFilesState((prev) => {
        const next =
          typeof value === "function"
            ? (value as (p: DraftAttachment[]) => DraftAttachment[])(prev)
            : value
        attachedFilesRef.current = next
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: inputRef.current,
          typedMentions: typedMentionsRef.current,
          attachedFiles: next,
          pendingQuotes: pendingQuotesRef.current,
          pendingMessageQuotes: pendingMessageQuotesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  const setPendingQuotes = useCallback<React.Dispatch<React.SetStateAction<PendingFileQuote[]>>>(
    (value) => {
      setPendingQuotesState((prev) => {
        const next =
          typeof value === "function"
            ? (value as (p: PendingFileQuote[]) => PendingFileQuote[])(prev)
            : value
        pendingQuotesRef.current = next
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: inputRef.current,
          typedMentions: typedMentionsRef.current,
          attachedFiles: attachedFilesRef.current,
          pendingQuotes: next,
          pendingMessageQuotes: pendingMessageQuotesRef.current,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  const setPendingMessageQuotes = useCallback<
    React.Dispatch<React.SetStateAction<PendingMessageQuote[]>>
  >(
    (value) => {
      setPendingMessageQuotesState((prev) => {
        const next =
          typeof value === "function"
            ? (value as (p: PendingMessageQuote[]) => PendingMessageQuote[])(prev)
            : value
        pendingMessageQuotesRef.current = next
        saveInputDraft(activeInputDraftKeyRef.current, {
          input: inputRef.current,
          typedMentions: typedMentionsRef.current,
          attachedFiles: attachedFilesRef.current,
          pendingQuotes: pendingQuotesRef.current,
          pendingMessageQuotes: next,
        })
        return next
      })
    },
    [saveInputDraft],
  )

  useLayoutEffect(() => {
    const nextKey = inputDraftKey(currentSessionId)
    const previousKey = activeInputDraftKeyRef.current
    if (previousKey === nextKey) return

    saveInputDraft(previousKey, {
      input: inputRef.current,
      typedMentions: typedMentionsRef.current,
      attachedFiles: attachedFilesRef.current,
      pendingQuotes: pendingQuotesRef.current,
      pendingMessageQuotes: pendingMessageQuotesRef.current,
    })

    activeInputDraftKeyRef.current = nextKey
    const nextDraft = inputDraftsRef.current.get(nextKey) ??
      (previousKey === "draft" ? inputDraftsRef.current.get(previousKey) : undefined) ?? {
        input: "",
        typedMentions: [],
        attachedFiles: [],
        pendingQuotes: [],
        pendingMessageQuotes: [],
      }
    if (previousKey === "draft" && !inputDraftsRef.current.has(nextKey)) {
      saveInputDraft(nextKey, nextDraft)
      inputDraftsRef.current.delete(previousKey)
    }
    inputRef.current = nextDraft.input
    typedMentionsRef.current = nextDraft.typedMentions
    attachedFilesRef.current = nextDraft.attachedFiles
    pendingQuotesRef.current = nextDraft.pendingQuotes
    pendingMessageQuotesRef.current = nextDraft.pendingMessageQuotes
    setInputState(nextDraft.input)
    setAttachedFilesState(nextDraft.attachedFiles)
    setPendingQuotesState(nextDraft.pendingQuotes)
    setPendingMessageQuotesState(nextDraft.pendingMessageQuotes)
  }, [currentSessionId, saveInputDraft])

  // Pending sends queued while a response is streaming. Stores the LLM-bound
  // `text` plus the original `options` (displayText / planMode / isPlanTrigger)
  // so replay preserves metadata. User-typed sends can additionally request
  // insertion at the next safe tool boundary.
  const [pendingSendsState, setPendingSendsState] = useState<PendingSend[]>([])
  const pendingSendsRef = useRef<PendingSend[]>([])
  const pendingSyncSeqRef = useRef(0)
  const updatePendingSends = useCallback((updater: React.SetStateAction<PendingSend[]>) => {
    setPendingSendsState((prev) => {
      const next =
        typeof updater === "function"
          ? (updater as (p: PendingSend[]) => PendingSend[])(prev)
          : updater
      pendingSendsRef.current = next
      return next
    })
  }, [])
  const pendingFromView = useCallback(
    (item: QueuedTurnMessageView): PendingSend => ({
      id: item.requestId,
      sessionId: item.sessionId,
      createdAt: Date.parse(item.createdAt) || Date.now(),
      text: item.message,
      mode: item.mode,
      status: item.status,
      options: {
        ...(item.displayText ? { displayText: item.displayText } : {}),
        ...(item.isPlanTrigger ? { isPlanTrigger: true } : {}),
        ...(item.goalTrigger ? { goalTrigger: true } : {}),
        ...(item.planComment ? { planComment: item.planComment } : {}),
        ...(item.planMode ? { planMode: item.planMode } : {}),
        ...(item.workflowMode ? { workflowMode: item.workflowMode } : {}),
      },
      attachmentCount: item.attachmentCount,
      quoteCount: item.quoteCount,
      isPlanTrigger: item.isPlanTrigger,
      goalTrigger: item.goalTrigger,
      planComment: item.planComment,
      managedBy: item.managedBy,
      canForceInsert: item.canForceInsert,
      editable:
        item.managedBy !== "channel" &&
        !item.displayText &&
        !item.isPlanTrigger &&
        !item.goalTrigger &&
        !item.planComment,
    }),
    [],
  )
  const syncPendingSends = useCallback(
    async (sessionId: string): Promise<PendingSend[]> => {
      const syncSeq = ++pendingSyncSeqRef.current
      const items = await getTransport().call<QueuedTurnMessageView[]>(
        "list_queued_turn_user_messages",
        { sessionId },
      )
      const mapped = items.map(pendingFromView)
      if (
        syncSeq === pendingSyncSeqRef.current &&
        shouldApplyPendingQueueSnapshot(currentSessionIdRef.current, sessionId)
      ) {
        updatePendingSends(mapped)
      }
      return mapped
    },
    [currentSessionIdRef, pendingFromView, updatePendingSends],
  )
  const pendingDisplayText = useCallback(
    (pending: PendingSend): string => {
      const explicit = pending.options?.displayText?.trim() || pending.text
      if (explicit) return explicit
      const quote = pending.messageQuotes?.[0]
      if (quote) {
        return quote.role === "user"
          ? t("chat.messageQuote.yourMessage", "你的消息")
          : t("chat.messageQuote.assistantMessage", "助手消息")
      }
      return pending.attachedFiles?.length || pending.attachmentCount || pending.quoteCount
        ? t("chat.attachPhotosAndFiles")
        : ""
    },
    [t],
  )
  const canForceInsertPending = useCallback(
    (pending: PendingSend): boolean =>
      !pending.isPlanTrigger &&
      !pending.goalTrigger &&
      !pending.planComment &&
      pending.managedBy !== "channel" &&
      pending.canForceInsert === true &&
      (pending.status === "queued" || pending.status === "fallback_after_reply") &&
      pending.mode !== "force_insert",
    [],
  )
  // External views: keep the original `pendingMessage: string | null` API for
  // ChatScreen / QuickChat, derived from the first queued item.
  const pendingMessage = pendingSendsState[0] ? pendingDisplayText(pendingSendsState[0]) : null
  const pendingSends: PendingSendPreview[] = pendingSendsState.map((pending) => ({
    id: pending.id,
    text: pendingDisplayText(pending),
    mode: pending.mode,
    status: pending.status,
    canForceInsert: canForceInsertPending(pending),
    attachmentCount: pending.attachmentCount ?? pending.attachedFiles?.length ?? 0,
    quoteCount:
      pending.quoteCount ?? (pending.quotes?.length ?? 0) + (pending.messageQuotes?.length ?? 0),
    sessionId: pending.sessionId,
    isPlanTrigger: pending.isPlanTrigger,
    goalTrigger: pending.goalTrigger,
    editable: pending.editable,
    managedBy: pending.managedBy,
  }))
  const setPendingMessage = useCallback<React.Dispatch<React.SetStateAction<string | null>>>(
    (value) => {
      updatePendingSends((prev) => {
        const first = prev[0]
        const next =
          typeof value === "function"
            ? (value as (p: string | null) => string | null)(
                first ? pendingDisplayText(first) : null,
              )
            : value
        if (next === null) return prev.slice(1)
        return [
          {
            id: generateClientId(),
            sessionId: currentSessionId ?? "",
            createdAt: Date.now(),
            text: next,
            mode: "queue",
            status: "saving",
          },
        ]
      })
    },
    [currentSessionId, pendingDisplayText, updatePendingSends],
  )
  const [showCodexAuthExpired, setShowCodexAuthExpired] = useState(false)
  const [permissionMode, setPermissionModeState] = useState<SessionMode>("default")
  const permissionModeRef = useRef<SessionMode>("default")
  const [sandboxMode, setSandboxModeState] = useState<SandboxMode>("off")
  const sandboxModeRef = useRef<SandboxMode>("off")
  // Whether the user explicitly changed the permission mode in the current draft
  // (via the switcher / `/permission`), vs. it being seeded from the agent
  // default or set programmatically (restore / events). For a NEW session we send
  // `permissionMode` only when this is true; otherwise we omit it so the backend's
  // create-time agent default (create_session_full) stays authoritative. Set only
  // through `setPermissionModeByUser`; reset on each fresh draft. Independent of
  // seeding timing, so a user override made (or a config fetch that fails) before
  // seeding settles is still honored.
  const permissionModeDirtyRef = useRef(false)
  const sandboxModeDirtyRef = useRef(false)
  const restoredModesRef = useRef<{
    sessionId: string
    permissionMode: SessionMode
    sandboxMode: SandboxMode
  } | null>(null)
  const lastModeSessionIdRef = useRef<string | null | undefined>(undefined)
  const [executionStateBySession, setExecutionStateBySession] = useState<
    Map<string, ChatTurnStatus>
  >(() => new Map())
  const activeTurnBySessionRef = useRef<Map<string, string>>(new Map())
  // A stop watchdog may durably finish and release a turn before the original
  // transport promise unwinds.  Track which request currently owns each
  // session's optimistic/loading lifecycle so a late `finally` from the old
  // request cannot remove the new turn's placeholder or clear its loading bit.
  const chatRequestOwnerBySessionRef = useRef<Map<string, string>>(new Map())
  const userStoppedRequestIdsRef = useRef<Set<string>>(new Set())
  // A request id is only useful to the backend after startChat has begun. A
  // Stop during local attachment preparation should stay local; sending it to
  // the backend would create a never-consumed pre-registration cancel latch.
  const backendStartedRequestIdsRef = useRef<Set<string>>(new Set())
  const preparationAbortByRequestRef = useRef<Map<string, AbortController>>(new Map())
  const lastTurnStatusBySessionRef = useRef<
    Map<string, { status: ChatTurnStatus; interruptReason?: ChatTurnInterruptReason | null }>
  >(new Map())

  // Persist the new mode to the session row whenever the title-bar switcher
  // changes it. Backend re-reads the column at the start of each tool round,
  // so in-flight loops pick up the change without a separate global snapshot.
  // Without a session id the choice is local-only until the first send.
  const setPermissionMode = useCallback<React.Dispatch<React.SetStateAction<SessionMode>>>(
    (value) => {
      setPermissionModeState((prev) => {
        const next =
          typeof value === "function" ? (value as (p: SessionMode) => SessionMode)(prev) : value
        if (next !== prev) {
          const sid = currentSessionIdRef.current
          if (sid) {
            getTransport()
              .call("set_permission_mode", { sessionId: sid, mode: next })
              .catch((e) => {
                logger.error(
                  "chat",
                  "setPermissionMode",
                  "Failed to sync session permission mode",
                  e,
                )
              })
          }
        }
        return next
      })
    },
    [currentSessionIdRef],
  )

  // User-initiated permission-mode change (switcher / `/permission`). Marks the
  // draft "dirty" so a new session's first send carries the chosen mode, then
  // delegates to the persist/state logic. Programmatic callers (seeding, restore,
  // backend events) must use `setPermissionMode` so they don't mark the draft.
  const setPermissionModeByUser = useCallback<React.Dispatch<React.SetStateAction<SessionMode>>>(
    (value) => {
      permissionModeDirtyRef.current = true
      setPermissionMode(value)
    },
    [setPermissionMode],
  )

  const setSandboxMode = useCallback<React.Dispatch<React.SetStateAction<SandboxMode>>>(
    (value) => {
      setSandboxModeState((prev) => {
        const next =
          typeof value === "function" ? (value as (p: SandboxMode) => SandboxMode)(prev) : value
        if (next !== prev) {
          const sid = currentSessionIdRef.current
          if (sid) {
            getTransport()
              .call("set_sandbox_mode", { sessionId: sid, mode: next })
              .then(() => onSandboxModeSynced?.(sid, next))
              .catch((e) => {
                logger.error("chat", "setSandboxMode", "Failed to sync session sandbox mode", e)
              })
          }
        }
        return next
      })
    },
    [currentSessionIdRef, onSandboxModeSynced],
  )

  const setSandboxModeByUser = useCallback<React.Dispatch<React.SetStateAction<SandboxMode>>>(
    (value) => {
      sandboxModeDirtyRef.current = true
      setSandboxMode(value)
    },
    [setSandboxMode],
  )

  // Auto-send pending messages setting
  const autoSendPendingRef = useRef(true)
  const autoSendRef = useRef(false)
  // Holds a programmatic queued send (Plan Mode approve, slash-skill expansion)
  // so the auto-send effect can replay it with the original options instead of
  // rerouting through the input box. User-typed drafts go via `setInput` and
  // leave this ref null.
  const queuedReplayRef = useRef<PendingSend | null>(null)

  // Delta batch buffer
  const deltaBuffersRef = useRef(createStreamDeltaBuffers())

  useEffect(() => {
    const unlisten = getTransport().listen("chat:stream_end", (raw) => {
      const payload = raw as {
        sessionId?: string
        turnId?: string | null
        status?: ChatTurnStatus | null
        interruptReason?: ChatTurnInterruptReason | null
      } | null
      const sid = payload?.sessionId
      if (!sid) return
      const streamId = streamIdFromPayload(raw)
      const currentTurnId = activeTurnBySessionRef.current.get(sid)
      if (payload.turnId && !currentTurnId && chatRequestOwnerBySessionRef.current.has(sid)) {
        // A new request already owns the session but has not received its
        // turn_started yet. The backend guarantees started-before-terminal
        // for that request, so this terminal event can only belong to the old
        // turn and must not occupy the new request's empty turn-id slot.
        markStreamEnded(endedStreamIdsRef.current, sid, streamId)
        if (streamId) discardPendingStreamDeltas(sid, deltaBuffersRef, streamId)
        return
      }
      if (payload?.turnId && currentTurnId && currentTurnId !== payload.turnId) {
        // A delayed terminal event from the watchdog/transport belongs to an
        // older turn.  Never let it overwrite a newer turn's running state.
        markStreamEnded(endedStreamIdsRef.current, sid, streamId)
        if (streamId) discardPendingStreamDeltas(sid, deltaBuffersRef, streamId)
        return
      }
      if (payload?.turnId) {
        // Keep the identity until every listener for this event has run. The
        // reattach listener owns loading teardown and calls handleTurnEnded;
        // deleting synchronously here would make it reject the same terminal
        // event as stale while the HTTP request owner is still present.
        deferActiveTurnRelease(activeTurnBySessionRef.current, sid, payload.turnId)
      }
      if (payload?.status) {
        lastTurnStatusBySessionRef.current.set(sid, {
          status: payload.status,
          interruptReason: payload.interruptReason ?? null,
        })
        setExecutionStateBySession((prev) => new Map(prev).set(sid, payload.status!))
      }
      markStreamEnded(endedStreamIdsRef.current, sid, streamId)
      discardPendingStreamDeltas(sid, deltaBuffersRef, streamId ?? null)
    })
    return () => {
      unlisten()
      discardAllPendingStreamDeltas(deltaBuffersRef)
    }
  }, [endedStreamIdsRef])

  useEffect(() => {
    const unlisten = getTransport().listen("chat:turn_status", (raw) => {
      const payload = raw as {
        sessionId?: string
        turnId?: string | null
        status?: ChatTurnStatus | null
        interruptReason?: ChatTurnInterruptReason | null
      } | null
      const sid = payload?.sessionId
      if (!sid || !payload?.status) return
      const currentTurnId = activeTurnBySessionRef.current.get(sid)
      if (payload.turnId && !currentTurnId && chatRequestOwnerBySessionRef.current.has(sid)) {
        return
      }
      if (payload.turnId && currentTurnId && currentTurnId !== payload.turnId) return
      if (payload.turnId) activeTurnBySessionRef.current.set(sid, payload.turnId)
      lastTurnStatusBySessionRef.current.set(sid, {
        status: payload.status,
        interruptReason: payload.interruptReason ?? null,
      })
      setExecutionStateBySession((prev) => new Map(prev).set(sid, payload.status!))
    })
    return unlisten
  }, [])

  // SQLite is authoritative. Reconcile on session switch, startup/reload, and
  // every backend mutation event (including another desktop/web client).
  useEffect(() => {
    const sid = currentSessionId
    if (!sid) {
      pendingSyncSeqRef.current += 1
      updatePendingSends([])
      return
    }
    updatePendingSends([])
    void syncPendingSends(sid).catch((error) => {
      logger.warn("chat", "useChatStream::queueSync", "Failed to load pending messages", error)
    })
  }, [currentSessionId, syncPendingSends, updatePendingSends])

  useEffect(() => {
    return getTransport().listen("chat:turn_queue_changed", (raw) => {
      const payload = raw as { sessionId?: unknown } | null
      const sid = typeof payload?.sessionId === "string" ? payload.sessionId : null
      if (!sid || currentSessionIdRef.current !== sid) return
      void syncPendingSends(sid).catch((error) => {
        logger.warn("chat", "useChatStream::queueEvent", "Failed to reconcile queue", error)
      })
    })
  }, [currentSessionIdRef, syncPendingSends])

  // Compose sub-hooks
  const { approvalRequests, handleApprovalResponse } = useApprovals(currentSessionId)

  useNotificationListeners({
    currentSessionIdRef,
    setMessages,
    setLoading,
    loadingSessionsRef,
    setLoadingSessionIds,
    sessionCacheRef,
    reloadSessions,
    consumeParentStreamDeltas: !parentInjectionDeltasViaChatStream,
  })

  // Keep refs in sync
  useEffect(() => {
    pendingSendsRef.current = pendingSendsState
  }, [pendingSendsState])
  useEffect(() => {
    permissionModeRef.current = permissionMode
  }, [permissionMode])
  useEffect(() => {
    sandboxModeRef.current = sandboxMode
  }, [sandboxMode])

  // Restore per-session modes from SessionMeta for every surface that uses
  // this hook (main chat, quick chat, knowledge chat). Programmatic restore is
  // local-only: persisting happens only from the explicit user setters.
  useEffect(() => {
    const sid = currentSessionId
    if (!sid) {
      restoredModesRef.current = null
      if (messages.length === 0 && lastModeSessionIdRef.current !== null) {
        setPermissionModeState("default")
        permissionModeRef.current = "default"
        setSandboxModeState("off")
        sandboxModeRef.current = "off"
      }
      lastModeSessionIdRef.current = null
      return
    }

    const meta = sessions.find((session) => session.id === sid)
    if (!meta) return

    const nextPermissionMode: SessionMode = meta.permissionMode ?? "default"
    const nextSandboxMode: SandboxMode = meta.sandboxMode ?? "off"
    const restored = restoredModesRef.current
    if (
      restored?.sessionId === sid &&
      restored.permissionMode === nextPermissionMode &&
      restored.sandboxMode === nextSandboxMode
    ) {
      return
    }

    restoredModesRef.current = {
      sessionId: sid,
      permissionMode: nextPermissionMode,
      sandboxMode: nextSandboxMode,
    }
    lastModeSessionIdRef.current = sid
    setPermissionModeState(nextPermissionMode)
    permissionModeRef.current = nextPermissionMode
    setSandboxModeState(nextSandboxMode)
    sandboxModeRef.current = nextSandboxMode
  }, [currentSessionId, messages.length, sessions])

  // Seed `permissionMode` / `sandboxMode` from the agent defaults
  // whenever the user is sitting on a fresh chat (no session row yet, no
  // messages). Once the first message lands the session row owns the mode
  // and the title-bar switcher updates the row directly.
  //
  // Skipping when there is already a session id keeps the user's manual
  // choice intact across navigation — only "new chat" or agent swap re-seeds.
  useEffect(() => {
    if (currentSessionId || messages.length > 0 || !currentAgentId) return
    // Fresh draft (or agent swap): the user hasn't chosen modes yet.
    permissionModeDirtyRef.current = false
    sandboxModeDirtyRef.current = false
    let cancelled = false
    void (async () => {
      try {
        const config = await getTransport().call<{
          capabilities?: {
            sandbox?: boolean
            defaultSessionPermissionMode?: SessionMode | null
            defaultSandboxMode?: SandboxMode | null
          }
        }>("get_agent_config", { id: currentAgentId })
        // Don't clobber a mode the user changed while the fetch was in flight.
        if (!cancelled && !permissionModeDirtyRef.current) {
          const fallback =
            (config?.capabilities?.defaultSessionPermissionMode as SessionMode | undefined) ??
            "default"
          setPermissionModeState(fallback)
        }
        if (!cancelled && !sandboxModeDirtyRef.current) {
          const fallback =
            (config?.capabilities?.defaultSandboxMode as SandboxMode | undefined) ??
            (config?.capabilities?.sandbox ? "standard" : "off")
          setSandboxModeState(fallback)
        }
      } catch (e) {
        logger.error(
          "chat",
          "useChatStream",
          "Failed to seed session modes from agent capabilities",
          e,
        )
      }
    })()
    return () => {
      cancelled = true
    }
  }, [currentAgentId, currentSessionId, messages.length])

  // Load config on mount
  useEffect(() => {
    getTransport()
      .call<{ autoSendPending?: boolean }>("get_user_config")
      .then((cfg) => {
        autoSendPendingRef.current = normalizeAutoSendPendingPreference(cfg.autoSendPending)
      })
      .catch(() => {})
    loadNotificationConfig().catch(() => {})

    const handleAutoSendPendingChange = (event: Event) => {
      autoSendPendingRef.current = normalizeAutoSendPendingPreference(
        (event as CustomEvent).detail?.enabled,
      )
    }
    window.addEventListener(AUTO_SEND_PENDING_EVENT, handleAutoSendPendingChange)
    return () => {
      window.removeEventListener(AUTO_SEND_PENDING_EVENT, handleAutoSendPendingChange)
    }
  }, [])

  async function handleStop() {
    const sid = currentSessionIdRef.current ?? currentSessionId ?? null
    if (!sid) {
      const pendingRequestOwner = chatRequestOwnerBySessionRef.current.get("__pending__")
      if (pendingRequestOwner) {
        userStoppedRequestIdsRef.current.add(pendingRequestOwner)
        preparationAbortByRequestRef.current.get(pendingRequestOwner)?.abort()
      }
      if (draftProjectBootstrap) {
        try {
          await getTransport().call("cancel_project_bootstrap", {
            requestId: draftProjectBootstrap.requestId,
          })
        } catch (e) {
          logger.error("ui", "ChatScreen::stopBootstrap", "Failed to stop project task setup", e)
        }
      }
      if (pendingRequestOwner) {
        if (!backendStartedRequestIdsRef.current.has(pendingRequestOwner)) return
        try {
          await getTransport().call("stop_chat", {
            sessionId: null,
            turnId: null,
            clientRequestId: pendingRequestOwner,
          })
        } catch (e) {
          logger.error("ui", "ChatScreen::stopPending", "Failed to stop pending chat", e)
        }
        return
      }
      const active = Array.from(activeTurnBySessionRef.current.entries()).at(-1)
      if (!active) return
      const [activeSid, activeTurnId] = active
      const requestOwner =
        chatRequestOwnerBySessionRef.current.get(activeSid) ??
        chatRequestOwnerBySessionRef.current.get("__pending__")
      if (requestOwner) {
        userStoppedRequestIdsRef.current.add(requestOwner)
        preparationAbortByRequestRef.current.get(requestOwner)?.abort()
      }
      setExecutionStateBySession((prev) => new Map(prev).set(activeSid, "cancelling"))
      try {
        await getTransport().call("stop_chat", {
          sessionId: activeSid,
          turnId: activeTurnId,
          clientRequestId: requestOwner,
        })
      } catch (e) {
        logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
      }
      return
    }
    const activeTurnId = activeTurnBySessionRef.current.get(sid) ?? null
    const requestOwner = chatRequestOwnerBySessionRef.current.get(sid)
    if (requestOwner) {
      userStoppedRequestIdsRef.current.add(requestOwner)
      preparationAbortByRequestRef.current.get(requestOwner)?.abort()
    }
    if (requestOwner && !activeTurnId && !backendStartedRequestIdsRef.current.has(requestOwner)) {
      return
    }
    setExecutionStateBySession((prev) => new Map(prev).set(sid, "cancelling"))
    try {
      await getTransport().call("stop_chat", {
        sessionId: sid,
        turnId: activeTurnId,
        clientRequestId: requestOwner,
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::stop", "Failed to stop chat", e)
    }
  }

  const handleTurnStarted = useCallback((sessionId: string, turnId: string) => {
    activeTurnBySessionRef.current.set(sessionId, turnId)
    lastTurnStatusBySessionRef.current.set(sessionId, { status: "running" })
    setExecutionStateBySession((prev) => new Map(prev).set(sessionId, "running"))
  }, [])

  const handleTurnEnded = useCallback(
    (
      sessionId: string,
      status?: ChatTurnStatus | null,
      interruptReason?: ChatTurnInterruptReason | null,
      turnId?: string | null,
    ) => {
      const currentTurnId = activeTurnBySessionRef.current.get(sessionId)
      if (turnId && !currentTurnId && chatRequestOwnerBySessionRef.current.has(sessionId)) {
        return false
      }
      if (turnId && currentTurnId && currentTurnId !== turnId) return false
      activeTurnBySessionRef.current.delete(sessionId)
      if (status) {
        lastTurnStatusBySessionRef.current.set(sessionId, {
          status,
          interruptReason: interruptReason ?? null,
        })
        setExecutionStateBySession((prev) => new Map(prev).set(sessionId, status))
      }
      return true
    },
    [],
  )

  const quoteLineLabel = useCallback(
    (q: PendingFileQuote) =>
      q.startLine === q.endLine ? `${q.startLine}` : `${q.startLine}-${q.endLine}`,
    [],
  )

  const ensureAttachmentCount = useCallback(
    (attachments: ChatAttachment[], transport: Transport, signal?: AbortSignal) =>
      validateChatAttachmentCount(
        attachments,
        transport,
        t("attachments.tooMany", "A message can contain at most 64 files"),
        signal,
      ),
    [t],
  )

  const buildChatAttachments = useCallback(
    async (
      text: string,
      filesToSend: DraftAttachment[],
      quotesToSend: PendingFileQuote[],
      messageQuotesToSend: PendingMessageQuote[],
      targetSessionId: string | null,
      transport: Transport,
      mentionBindings: ComposerMentionBinding[],
      signal?: AbortSignal,
    ): Promise<ChatAttachment[]> => {
      const attachments: ChatAttachment[] = []

      if (signal?.aborted) throw new ChatPreparationCancelledError()

      const sessionWorkingDir = sessions.find((s) => s.id === targetSessionId)?.workingDir ?? null
      const resolvedWorkingDir = targetSessionId ? sessionWorkingDir : draftWorkingDir
      const mentionAttachments = expandMentionsToAttachments(
        text,
        resolvedWorkingDir ?? null,
        mentionBindings,
      )
      for (const m of mentionAttachments) {
        attachments.push(m)
      }

      const planAttachments = await awaitUnlessAborted(
        expandPlanMentionsToAttachments(text, mentionBindings),
        signal,
      )
      for (const p of planAttachments) {
        attachments.push(p)
      }

      if (filesToSend.length > 64)
        throw new Error(t("attachments.tooMany", "A message can contain at most 64 files"))
      const unavailable = filesToSend.find((draft) => draft.status === "error")
      if (unavailable) {
        throw new Error(unavailable.error || t("attachments.uploadFailedShort", "Upload failed"))
      }
      if (filesToSend.length > 0) {
        let filesystemConfig: Awaited<ReturnType<typeof readFilesystemConfig>> | null = null
        try {
          filesystemConfig = await awaitUnlessAborted(readFilesystemConfig(transport), signal)
        } catch (error) {
          if (isChatPreparationCancelled(error)) throw error
        }
        if (filesystemConfig) {
          const configuredMaxBytes = attachmentBytesForConfig(filesystemConfig)
          const oversized = filesToSend.find((draft) => draft.file.size > configuredMaxBytes)
          if (oversized) {
            throw new Error(
              t("attachments.tooLarge", "{{name}} exceeds the {{limit}} MB limit", {
                name: oversized.file.name,
                limit: filesystemConfig.maxChatAttachmentMb,
              }),
            )
          }
        }
      }

      const leases: Array<Awaited<ReturnType<Transport["stageChatAttachment"]>> | undefined> =
        new Array(filesToSend.length)
      let cursor = 0
      let failure: unknown = null
      const worker = async () => {
        while (!failure && !signal?.aborted) {
          const index = cursor
          cursor += 1
          if (index >= filesToSend.length) return
          try {
            leases[index] = await awaitUnlessAborted(
              transport.stageChatAttachment(filesToSend[index].file),
              signal,
              (lease) => transport.discardChatAttachmentUpload(lease.uploadId),
            )
          } catch (error) {
            failure = error
          }
        }
      }
      await Promise.all(Array.from({ length: Math.min(3, filesToSend.length) }, () => worker()))
      if (signal?.aborted && !failure) failure = new ChatPreparationCancelledError()
      if (failure) {
        const cleanup = Promise.allSettled(
          leases
            .filter((lease): lease is NonNullable<typeof lease> => !!lease)
            .map((lease) => transport.discardChatAttachmentUpload(lease.uploadId)),
        )
        if (isChatPreparationCancelled(failure)) void cleanup
        else await cleanup
        throw failure
      }
      for (let index = 0; index < filesToSend.length; index += 1) {
        const draft = filesToSend[index]
        const file = draft.file
        const lease = leases[index]
        if (!lease) throw new Error(`attachment upload missing: ${file.name}`)
        attachments.push({
          name: file.name,
          mime_type: file.type || "application/octet-stream",
          source: draft.semanticSource,
          upload_id: lease.uploadId,
        })
      }

      for (const q of quotesToSend) {
        attachments.push({
          name: q.name,
          mime_type: "text/plain",
          source: "quote",
          data: q.content,
          file_path: q.path,
          quote_lines: quoteLineLabel(q),
        })
      }

      for (const q of messageQuotesToSend) {
        attachments.push({
          name: "message-quote",
          mime_type: "text/plain",
          source: "message_quote",
          data: q.content,
          quote_role: q.role,
        })
      }

      await ensureAttachmentCount(attachments, transport, signal)
      return attachments
    },
    [draftWorkingDir, ensureAttachmentCount, quoteLineLabel, sessions, t],
  )

  /**
   * Send a message. If `directText` is provided, use it directly instead of the input box.
   * This avoids flashing text in the input (used by Plan Mode approve).
   */
  async function handleSend(directText?: string, options?: SendOptions) {
    const rawText = directText ?? input
    const usesComposerDraft = directText === undefined && !options?.draftOverride
    const typedMentionsForSend = options?.structuredMentions
      ? [...options.structuredMentions]
      : usesComposerDraft
        ? [...typedMentionsRef.current]
        : []
    const canonical = trimTextWithTypedMentions(rawText, typedMentionsForSend)
    const canonicalText = canonical.text
    const canonicalTypedMentions = canonical.mentions
    const draftFiles =
      options?.draftOverride?.attachedFiles ?? (usesComposerDraft ? attachedFiles : [])
    const draftQuotes =
      options?.draftOverride?.pendingQuotes ?? (usesComposerDraft ? pendingQuotes : [])
    const draftMessageQuotes =
      options?.draftOverride?.pendingMessageQuotes ??
      (usesComposerDraft ? pendingMessageQuotes : [])
    const hasAttachedFiles = draftFiles.length > 0
    const hasQuotes = draftQuotes.length > 0
    const hasMessageQuotes = draftMessageQuotes.length > 0
    if (
      !hasMessageQuotes &&
      !hasSendableChatPayload(rawText, hasAttachedFiles, hasQuotes, options?.queuedRequestId)
    ) {
      options?.onPreparationError?.(new Error("Message is empty"))
      return
    }

    // If currently loading, queue the message as pending. Capture the
    // LLM-bound text, the original options, and any staged files/quotes so the
    // replay below resends with identical content + metadata (Plan Mode
    // triggers carry `isPlanTrigger`, slash-skill expansions carry
    // `displayText`, etc.).
    if (loading) {
      if (options?.editMessageId != null) {
        options.onPreparationError?.(
          new Error("The assistant is already responding; wait for it to finish before editing"),
        )
        return
      }
      const queueSessionId =
        options?.sessionIdOverride ?? currentSessionIdRef.current ?? currentSessionId
      if (!queueSessionId) {
        logger.warn(
          "chat",
          "useChatStream::queue",
          "Session is still being created; pending message was kept in the composer",
        )
        return
      }
      const queuedFiles = [...draftFiles]
      const queuedQuotes = [...draftQuotes]
      const queuedMessageQuotes = [...draftMessageQuotes]
      const queueTransport = getTransport()
      let durableAttachments: ChatAttachment[] = []
      const requestId = generateClientId()
      updatePendingSends((prev) => [
        ...prev,
        {
          id: requestId,
          sessionId: queueSessionId,
          createdAt: Date.now(),
          text: rawText.trim(),
          mode: "queue",
          status: "saving",
          options,
          ...(queuedFiles.length > 0 && { attachedFiles: queuedFiles }),
          ...(queuedQuotes.length > 0 && { quotes: queuedQuotes }),
          ...(queuedMessageQuotes.length > 0 && { messageQuotes: queuedMessageQuotes }),
        },
      ])
      if (usesComposerDraft) {
        setInput("")
        setAttachedFiles([])
        setPendingQuotes([])
        setPendingMessageQuotes([])
      }
      try {
        const incomingTurn = options?.queuedRequestId
          ? undefined
          : await buildIncomingTurnWire(canonicalText, canonicalTypedMentions)
        durableAttachments = await buildChatAttachments(
          rawText.trim(),
          queuedFiles,
          queuedQuotes,
          queuedMessageQuotes,
          queueSessionId,
          queueTransport,
          canonicalTypedMentions,
        )
        if (getExtraAttachments) {
          durableAttachments.push(...getExtraAttachments())
        }
        await ensureAttachmentCount(durableAttachments, queueTransport)
        await queueTransport.call<QueueTurnUserMessageResult>("queue_turn_user_message", {
          requestId,
          sessionId: queueSessionId,
          message: rawText.trim(),
          attachments: durableAttachments,
          displayText: options?.displayText,
          isPlanTrigger: options?.isPlanTrigger,
          goalTrigger: options?.goalTrigger,
          planComment: options?.planComment,
          planMode: options?.planMode,
          workflowMode: options?.workflowMode,
          incomingTurn,
        })
        await syncPendingSends(queueSessionId)
      } catch (error) {
        await Promise.allSettled(
          durableAttachments
            .map((attachment) => attachment.upload_id)
            .filter((id): id is string => !!id)
            .map((id) => queueTransport.discardChatAttachmentUpload(id)),
        )
        logger.error("chat", "useChatStream::queue", "Failed to persist pending message", error)
        updatePendingSends((prev) => prev.filter((item) => item.id !== requestId))
        if (usesComposerDraft) {
          const failedQueuedFiles = queuedFiles.map((draft) => ({
            ...draft,
            status: "error" as const,
            error: error instanceof Error ? error.message : String(error),
          }))
          if (currentSessionIdRef.current === queueSessionId) {
            // Do not overwrite text/files the user entered while the durable
            // save was in flight. Put the failed send back ahead of the newer
            // draft so nothing silently disappears.
            const current = inputRef.current
            const restored = mergeTypedMentionDrafts(
              rawText,
              typedMentionsForSend,
              current,
              typedMentionsRef.current,
            )
            replaceInputWithMentions(restored.text, restored.mentions)
            setAttachedFiles((existing) => [...failedQueuedFiles, ...existing])
            setPendingQuotes((existing) => [...queuedQuotes, ...existing])
            setPendingMessageQuotes((existing) => [...queuedMessageQuotes, ...existing])
          } else {
            // Session switches are allowed while the queue write is pending.
            // Restore the failed message into that session's draft cache so it
            // is waiting in the composer when the user returns.
            const key = inputDraftKey(queueSessionId)
            const existing = inputDraftsRef.current.get(key) ?? {
              input: "",
              typedMentions: [],
              attachedFiles: [],
              pendingQuotes: [],
              pendingMessageQuotes: [],
            }
            const restored = mergeTypedMentionDrafts(
              rawText,
              typedMentionsForSend,
              existing.input,
              existing.typedMentions,
            )
            saveInputDraft(key, {
              input: restored.text,
              typedMentions: restored.mentions,
              attachedFiles: [...failedQueuedFiles, ...existing.attachedFiles],
              pendingQuotes: [...queuedQuotes, ...existing.pendingQuotes],
              pendingMessageQuotes: [...queuedMessageQuotes, ...existing.pendingMessageQuotes],
            })
          }
        }
      }
      return
    }

    const text = canonicalText
    // `text` goes to the LLM; `displayed` is the user bubble. Slash-skill passThrough
    // uses this split so the UI shows "/drawio ..." while the LLM receives the expansion.
    const filesToSend = [...draftFiles]
    const quotesToSend = [...draftQuotes]
    const messageQuotesToSend = [...draftMessageQuotes]
    const displayed = options?.displayText?.trim() || text
    const sendSessionId = options?.sessionIdOverride ?? currentSessionId
    const chatRequestOwnerId = generateClientId()
    const preparationAbort = new AbortController()
    preparationAbortByRequestRef.current.set(chatRequestOwnerId, preparationAbort)
    let chatRequestOwnerKey = sendSessionId ?? "__pending__"
    const bindChatRequestOwner = (nextKey: string) => {
      if (
        chatRequestOwnerKey !== nextKey &&
        chatRequestOwnerBySessionRef.current.get(chatRequestOwnerKey) === chatRequestOwnerId
      ) {
        chatRequestOwnerBySessionRef.current.delete(chatRequestOwnerKey)
      }
      chatRequestOwnerKey = nextKey
      chatRequestOwnerBySessionRef.current.set(nextKey, chatRequestOwnerId)
    }
    const releaseChatRequestOwner = () => {
      if (chatRequestOwnerBySessionRef.current.get(chatRequestOwnerKey) === chatRequestOwnerId) {
        chatRequestOwnerBySessionRef.current.delete(chatRequestOwnerKey)
      }
    }
    const releasePreparationLoading = () => {
      if (chatRequestOwnerBySessionRef.current.get(chatRequestOwnerKey) === chatRequestOwnerId) {
        const nextLoading = loadingStateAfterPreparationRelease(
          chatRequestOwnerKey,
          currentSessionIdRef.current,
          loadingSessionsRef.current,
        )
        if (nextLoading !== undefined) setLoading(nextLoading)
      }
    }
    bindChatRequestOwner(chatRequestOwnerKey)
    // Attachment/mention preparation is part of the active send lifecycle.
    // Expose Stop immediately instead of waiting until optimistic messages are
    // created; handleStop aborts this request locally before startChat exists.
    setLoading(true)
    if (options?.sessionIdOverride) {
      currentSessionIdRef.current = options.sessionIdOverride
      setCurrentSessionId(options.sessionIdOverride)
    }
    const sendTransport = getTransport()
    let attachments: ChatAttachment[]
    let incomingTurn: Awaited<ReturnType<typeof buildIncomingTurnWire>> | undefined
    const sendingDraftIds = new Set(filesToSend.map((draft) => draft.id))
    if (usesComposerDraft && sendingDraftIds.size > 0) {
      setAttachedFiles((existing) =>
        existing.map((draft) =>
          sendingDraftIds.has(draft.id)
            ? { ...draft, status: "uploading", error: undefined }
            : draft,
        ),
      )
    }
    try {
      incomingTurn = options?.queuedRequestId
        ? undefined
        : await buildIncomingTurnWire(canonicalText, canonicalTypedMentions)
      attachments = options?.queuedRequestId
        ? []
        : await buildChatAttachments(
            text,
            filesToSend,
            quotesToSend,
            messageQuotesToSend,
            sendSessionId,
            sendTransport,
            canonicalTypedMentions,
            preparationAbort.signal,
          )
      if (getExtraAttachments && !options?.queuedRequestId) {
        attachments.push(...getExtraAttachments())
      }
      await ensureAttachmentCount(attachments, sendTransport, preparationAbort.signal)
    } catch (error) {
      preparationAbortByRequestRef.current.delete(chatRequestOwnerId)
      releasePreparationLoading()
      releaseChatRequestOwner()
      if (
        isChatPreparationCancelled(error) ||
        userStoppedRequestIdsRef.current.delete(chatRequestOwnerId)
      ) {
        userStoppedRequestIdsRef.current.delete(chatRequestOwnerId)
        if (usesComposerDraft && sendingDraftIds.size > 0) {
          setAttachedFiles((existing) =>
            existing.map((draft) =>
              sendingDraftIds.has(draft.id)
                ? { ...draft, status: "ready", error: undefined }
                : draft,
            ),
          )
        }
        return
      }
      logger.error("ui", "useChatStream::attachment", "Attachment upload failed", error)
      toast.error(
        t("attachments.uploadFailed", "Files were not sent. {{error}}", {
          error: error instanceof Error ? error.message : String(error),
        }),
      )
      if (usesComposerDraft) {
        setAttachedFiles((existing) =>
          existing.map((draft) =>
            sendingDraftIds.has(draft.id)
              ? {
                  ...draft,
                  status: "error",
                  error: error instanceof Error ? error.message : String(error),
                }
              : draft,
          ),
        )
      }
      options?.onPreparationError?.(error)
      return
    }
    preparationAbortByRequestRef.current.delete(chatRequestOwnerId)
    // Stop can arrive while files/quotes are still being staged, before the
    // backend knows this request. In that window the request id is the local
    // cancellation authority: discard leases and leave the composer intact.
    if (userStoppedRequestIdsRef.current.delete(chatRequestOwnerId)) {
      releasePreparationLoading()
      void Promise.allSettled(
        attachments
          .map((attachment) => attachment.upload_id)
          .filter((id): id is string => !!id)
          .map((id) => sendTransport.discardChatAttachmentUpload(id)),
      )
      if (usesComposerDraft && sendingDraftIds.size > 0) {
        setAttachedFiles((existing) =>
          existing.map((draft) =>
            sendingDraftIds.has(draft.id) ? { ...draft, status: "ready", error: undefined } : draft,
          ),
        )
      }
      releaseChatRequestOwner()
      return
    }
    const optimisticQuoteAttachments: MessageAttachment[] = quotesToSend.map((q) => ({
      name: q.name,
      mimeType: "text/plain",
      sizeBytes: 0,
      kind: "quote",
      quotePath: q.path,
      quoteLines: quoteLineLabel(q),
      quoteContent: q.content,
    }))
    const optimisticMessageQuoteAttachments: MessageAttachment[] = messageQuotesToSend.map((q) => ({
      name: "message-quote",
      mimeType: "text/plain",
      sizeBytes: 0,
      kind: "message_quote",
      quoteContent: q.content,
      messageQuoteRole: q.role,
    }))
    const optimisticAttachments = [
      ...filesToSend.map((draft) => optimisticAttachmentForFile(draft.file)),
      ...optimisticQuoteAttachments,
      ...optimisticMessageQuoteAttachments,
    ]
    if (usesComposerDraft) {
      setInput("")
      setAttachedFiles([])
      setPendingQuotes([])
      setPendingMessageQuotes([])
    }
    const restoreUnsentDraft = () => {
      if (!usesComposerDraft) return
      const restoredFiles = filesToSend.map((draft) => ({
        ...draft,
        status: "ready" as const,
        error: undefined,
      }))
      const merge = (current: InputDraft): InputDraft => {
        const restoredIds = new Set(restoredFiles.map((draft) => draft.id))
        const restored = mergeTypedMentionDrafts(
          rawText,
          typedMentionsForSend,
          current.input,
          current.typedMentions,
        )
        return {
          input: restored.text,
          typedMentions: restored.mentions,
          attachedFiles: [
            ...restoredFiles,
            ...current.attachedFiles.filter((draft) => !restoredIds.has(draft.id)),
          ],
          pendingQuotes: [...quotesToSend, ...current.pendingQuotes],
          pendingMessageQuotes: [...messageQuotesToSend, ...current.pendingMessageQuotes],
        }
      }
      const draftKey = inputDraftKey(sendSessionId)
      if (activeInputDraftKeyRef.current !== draftKey) {
        saveInputDraft(
          draftKey,
          merge(
            inputDraftsRef.current.get(draftKey) ?? {
              input: "",
              typedMentions: [],
              attachedFiles: [],
              pendingQuotes: [],
              pendingMessageQuotes: [],
            },
          ),
        )
        return
      }
      const current = inputRef.current
      const restored = mergeTypedMentionDrafts(
        rawText,
        typedMentionsForSend,
        current,
        typedMentionsRef.current,
      )
      replaceInputWithMentions(restored.text, restored.mentions)
      setAttachedFiles((current) => {
        const restoredIds = new Set(restoredFiles.map((draft) => draft.id))
        return [...restoredFiles, ...current.filter((draft) => !restoredIds.has(draft.id))]
      })
      setPendingQuotes((current) => [...quotesToSend, ...current])
      setPendingMessageQuotes((current) => [...messageQuotesToSend, ...current])
    }
    const now = new Date().toISOString()
    // Both placeholders get a `_clientId` up front so `mergeMessagesByDbId`
    // can transfer them to the DB-finalized rows after stream_end. Without
    // the user-side id, `getLatestUserTurnKey` would flip from
    // `ts:user:<iso>` to `db:<N>` at finalize, fooling the forceFollow
    // effect into a phantom "new user message" scroll-to-top.
    const optimisticUserClientId = generateClientId()
    const assistantPlaceholderClientId = generateClientId()
    const optimisticUserMessage = {
      role: "user" as const,
      content: displayed,
      timestamp: now,
      _clientId: optimisticUserClientId,
      ...(displayed === text && canonicalTypedMentions.length > 0
        ? { typedMentions: canonicalTypedMentions }
        : {}),
      ...(optimisticAttachments.length > 0 && { attachments: optimisticAttachments }),
      ...(options?.isPlanTrigger && { isPlanTrigger: true }),
      ...(options?.goalTrigger && { isGoalTrigger: true }),
      ...(options?.planComment && { planComment: options.planComment }),
    }
    const messagesBeforeEditedTurn = (current: Message[]): Message[] => {
      if (options?.editMessageId == null) return current
      const boundary = current.findIndex((message) => message.dbId === options.editMessageId)
      return boundary < 0 ? current : current.slice(0, boundary)
    }
    const sidForCap = sendSessionId ?? currentSessionIdRef.current
    setMessages((prev) => {
      const next = [...messagesBeforeEditedTurn(prev), optimisticUserMessage]
      return sidForCap && capMessagesForSession ? capMessagesForSession(sidForCap, next) : next
    })
    setLoading(true)

    // Empty assistant placeholder we'll stream into. `_clientId` was generated
    // alongside the user-side one above and survives the placeholder→DB
    // transition via `mergeMessagesByDbId`; see `messageStableId` for how the
    // row key consumes it.
    const assistantPlaceholderTimestamp = new Date().toISOString()
    setMessages((prev) => {
      const next: Message[] = [
        ...prev,
        {
          role: "assistant",
          content: "",
          timestamp: assistantPlaceholderTimestamp,
          _clientId: assistantPlaceholderClientId,
        },
      ]
      return sidForCap && capMessagesForSession ? capMessagesForSession(sidForCap, next) : next
    })

    let targetSessionId = sendSessionId ?? currentSessionId
    let chatResolved = false
    let keepExistingStreamLoading = false
    let dispatchAccepted = false
    const markDispatchAccepted = () => {
      if (dispatchAccepted) return
      dispatchAccepted = true
      options?.onDispatchAccepted?.()
    }

    try {
      const targetSid = () => targetSessionId || "__pending__"

      const handleSessionCreated = (event: Record<string, unknown>): boolean => {
        if (
          event.type !== "session_created" ||
          typeof event.session_id !== "string" ||
          !event.session_id
        ) {
          return false
        }

        targetSessionId = event.session_id
        bindChatRequestOwner(event.session_id)
        // Bridge the ref lag: `setCurrentSessionId` below only updates
        // `currentSessionIdRef` after React commits, but the user is already on
        // this freshly-materialized session. Set it eagerly so the mark-as-read
        // guard at turn end compares against the right session even on a very
        // fast first turn — and still flips away correctly if the user navigates
        // elsewhere (handleSwitchSession / the sync effect update the ref then).
        currentSessionIdRef.current = event.session_id
        const current = sessionCacheRef.current.get("__pending__")
        if (current) {
          sessionCacheRef.current.delete("__pending__")
          sessionCacheRef.current.set(event.session_id, current)
        }
        // Promote the freshly-created session into the LRU. Without this,
        // a new chat written via this rename path would skip the LRU
        // bookkeeping done by `handleSwitchSession` and could be evicted
        // before the user even sees the first response.
        touchSessionCacheLru?.(event.session_id)
        loadingSessionsRef.current.add(event.session_id)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        setCurrentSessionId(event.session_id)
        reloadSessions()
        return true
      }

      const handleTurnStartedEvent = (event: Record<string, unknown>): boolean => {
        if (
          event.type !== "turn_started" ||
          typeof event.session_id !== "string" ||
          typeof event.turn_id !== "string"
        ) {
          return false
        }
        handleTurnStarted(event.session_id, event.turn_id)
        if (options?.editMessageId != null) markDispatchAccepted()
        return true
      }

      const shouldDropStreamEvent = (event: Record<string, unknown>, sid: string): boolean => {
        const streamId = streamIdFromEvent(event)
        if (isStreamEnded(endedStreamIdsRef.current, sid, streamId)) return true

        // Primary path bumps the seq cursor so identical events arriving
        // later via the EventBus reattach listener are dropped.
        const seqRaw = event._oc_seq
        if (typeof seqRaw === "number" && sid !== "__pending__") {
          const cursorKey = streamCursorKey(sid, streamId)
          const prev = lastSeqRef.current.get(cursorKey) ?? 0
          if (seqRaw <= prev) return true
          lastSeqRef.current.set(cursorKey, seqRaw)
        }
        return false
      }

      const dispatchStreamEvent = (event: Record<string, unknown>) => {
        if (handleSessionCreated(event)) return
        if (handleTurnStartedEvent(event)) return

        const sid = targetSid()
        if (shouldDropStreamEvent(event, sid)) return

        if (event.type === "queued_user_message_inserted") {
          if (sid !== "__pending__") void syncPendingSends(sid).catch(() => undefined)
        } else if (event.type === "queued_user_message_blocked") {
          if (sid !== "__pending__") void syncPendingSends(sid).catch(() => undefined)
        }

        handleStreamEvent(event, sid, {
          updateSessionMessages,
          deltaBuffersRef,
          setShowCodexAuthExpired,
        })
      }

      const appendRawStreamText = (raw: string) => {
        const sid = targetSid()
        updateSessionMessages(sid, (prev) => {
          const updated = [...prev]
          const last = updated[updated.length - 1]
          if (last && last.role === "assistant") {
            updated[updated.length - 1] = {
              ...last,
              content: last.content + raw,
            }
          }
          return updated
        })
      }

      const onEvent = (raw: string) => {
        try {
          dispatchStreamEvent(JSON.parse(raw) as Record<string, unknown>)
        } catch {
          appendRawStreamText(raw)
        }
      }

      // Track loading state for this session. The cache write must mirror
      // what `setMessages` produced — without re-capping here, the first
      // streaming frame's `updateSessionMessages` would read the
      // uncapped array back and `setMessages` it, undoing the cap on
      // every send.
      const baseMessagesForSend = sendSessionId
        ? (sessionCacheRef.current.get(sendSessionId) ?? messages)
        : messages
      const freshMessages: Message[] = [
        ...messagesBeforeEditedTurn(baseMessagesForSend),
        optimisticUserMessage,
        {
          role: "assistant" as const,
          content: "",
          timestamp: assistantPlaceholderTimestamp,
          _clientId: assistantPlaceholderClientId,
        },
      ]
      const cappedFreshMessages =
        targetSessionId && capMessagesForSession
          ? capMessagesForSession(targetSessionId, freshMessages)
          : freshMessages
      if (targetSessionId) {
        loadingSessionsRef.current.add(targetSessionId)
        setLoadingSessionIds(new Set(loadingSessionsRef.current))
        sessionCacheRef.current.set(targetSessionId, cappedFreshMessages)
        touchSessionCacheLru?.(targetSessionId)
      } else {
        sessionCacheRef.current.set("__pending__", cappedFreshMessages)
      }

      const modelOverride = modelOverrideFromManualSelection(manualModelOverrideRef?.current)
      const effectivePlanMode = options?.planMode ?? planMode
      // This synchronous check+mark is the handoff linearization point. Stop
      // either cancelled the local request already, or it will observe backend
      // ownership and send `stop_chat` (which can latch before turn_started).
      beginChatBackendHandoff(
        chatRequestOwnerId,
        userStoppedRequestIdsRef.current,
        backendStartedRequestIdsRef.current,
      )
      await sendTransport.startChat(
        {
          message: text,
          incomingTurn,
          attachments,
          sessionId: sendSessionId,
          clientRequestId: chatRequestOwnerId,
          incognito: sendSessionId ? undefined : incognitoEnabled,
          sessionDefaults: sendSessionId
            ? undefined
            : {
                model: modelOverride,
                temperature: temperatureOverride ?? undefined,
                reasoningEffort: reasoningEffort ?? undefined,
              },
          agentId: currentAgentId,
          // Existing session: always send (the title-bar switcher persisted it).
          // New session: only send when the user explicitly changed it — otherwise
          // omit so the backend's create-time agent default (create_session_full)
          // wins instead of a seeded/stale value sent before seeding settled.
          permissionMode:
            sendSessionId || permissionModeDirtyRef.current ? permissionModeRef.current : undefined,
          sandboxMode:
            sendSessionId || sandboxModeDirtyRef.current ? sandboxModeRef.current : undefined,
          workflowMode: options?.workflowMode,
          planMode:
            effectivePlanMode && effectivePlanMode !== "off" ? effectivePlanMode : undefined,
          // Legacy top-level model/temperature/Think overrides remain available
          // to API clients as one-turn controls. The GUI uses sessionDefaults
          // above so draft choices are consumed only during materialization.
          displayText: options?.displayText?.trim() || undefined,
          queuedRequestId: options?.queuedRequestId,
          editMessageId: options?.editMessageId,
          isPlanTrigger: options?.isPlanTrigger,
          goalTrigger: options?.goalTrigger,
          initialGoal:
            !sendSessionId && options?.initialGoal
              ? {
                  objective: options.initialGoal.objective,
                  completionCriteria: options.initialGoal.completionCriteria,
                }
              : undefined,
          planComment: options?.planComment,
          workingDir: sendSessionId ? undefined : (draftWorkingDir ?? undefined),
          // Lazy project binding — send-time snapshot, only on the auto-create send.
          projectId: sendSessionId ? undefined : (draftProjectIdRef.current ?? undefined),
          projectBootstrap: sendSessionId ? undefined : (draftProjectBootstrap ?? undefined),
          // Send-time snapshot: only on the auto-create send, never incognito.
          kbAttachments:
            sendSessionId || incognitoEnabled
              ? undefined
              : draftKbAttachmentsRef.current.map((a) => ({
                  kbId: a.kbId,
                  access: a.access,
                })),
          ...(toolScope ? { toolScope } : {}),
          ...(uiSurface ? { uiSurface } : {}),
          // Anchor only matters on the auto-create send; mirrors kbAttachments.
          ...(toolScope && !sendSessionId && draftKbAnchorNote
            ? { kbAnchorNote: draftKbAnchorNote }
            : {}),
          // Design-space anchor: promote the new session into a project thread.
          ...(toolScope === "design" && !currentSessionId && draftDesignProjectId
            ? { designProjectId: draftDesignProjectId }
            : {}),
        },
        onEvent,
      )
      if (options?.editMessageId != null) markDispatchAccepted()
      chatResolved = true
    } catch (e) {
      const sid = targetSessionId || "__pending__"
      const ownsRequestLifecycleNow =
        chatRequestOwnerBySessionRef.current.get(sid) === chatRequestOwnerId
      const requestWasUserStopped = userStoppedRequestIdsRef.current.has(chatRequestOwnerId)
      const stoppedBeforePersistence = shouldRollbackNonPersistedStoppedSend(
        requestWasUserStopped,
        isPreflightStopError(e),
        isChatPreparationCancelled(e),
        isActiveStreamError(e),
      )
      await discardChatAttachmentUploads(attachments, sendTransport, !stoppedBeforePersistence)
      const bootstrapFailedBeforeSession =
        !sendSessionId && sid === "__pending__" && !!draftProjectBootstrap
      if (stoppedBeforePersistence) {
        updateSessionMessages(sid, (prev) =>
          prev.filter(
            (message) =>
              message._clientId !== assistantPlaceholderClientId &&
              message._clientId !== optimisticUserClientId,
          ),
        )
        if (!options?.queuedRequestId) restoreUnsentDraft()
      } else if (bootstrapFailedBeforeSession) {
        onProjectBootstrapFailure?.(e instanceof Error ? e.message : String(e))
        updateSessionMessages(sid, (prev) => {
          return prev.filter(
            (message) =>
              message._clientId !== assistantPlaceholderClientId &&
              message._clientId !== optimisticUserClientId,
          )
        })
        restoreUnsentDraft()
      } else if (
        (isActiveStreamError(e) || isQueuedMessageUnavailableError(e)) &&
        sid !== "__pending__"
      ) {
        // active_stream and a lost durable-queue CAS both reject before this
        // caller persists anything, so roll back its optimistic bubbles. Other
        // errors may already have saved server-side, so keep those visible.
        keepExistingStreamLoading = true
        updateSessionMessages(sid, (prev) => {
          return prev.filter(
            (message) =>
              message._clientId !== assistantPlaceholderClientId &&
              message._clientId !== optimisticUserClientId,
          )
        })
        restoreUnsentDraft()
        try {
          const state = await getTransport().call<SessionStreamState>("get_session_stream_state", {
            sessionId: sid,
          })
          keepExistingStreamLoading = state.active
          if (state.turnId && state.active) {
            activeTurnBySessionRef.current.set(sid, state.turnId)
          } else if (!state.active) {
            activeTurnBySessionRef.current.delete(sid)
          }
          if (state.status) {
            lastTurnStatusBySessionRef.current.set(sid, {
              status: state.status,
              interruptReason: state.interruptReason ?? null,
            })
            setExecutionStateBySession((prev) => new Map(prev).set(sid, state.status!))
          }
          const streamId = state.streamId || undefined
          markStreamActive(endedStreamIdsRef.current, sid, streamId)
          const cursorKey = streamCursorKey(sid, streamId)
          if (!lastSeqRef.current.has(cursorKey)) {
            lastSeqRef.current.set(cursorKey, Number(state.lastSeq) || 0)
          }
        } catch (stateError) {
          logger.warn(
            "chat",
            "useChatStream::active_stream",
            "Failed to refresh active stream state",
            stateError,
          )
        }
      } else {
        updateSessionMessages(sid, (prev) => {
          const updated = prev.filter(
            (message) => message._clientId !== assistantPlaceholderClientId,
          )
          // A late failure from an older request is already represented by
          // its durable terminal row.  Do not append that transient error
          // after a newer turn's optimistic messages.
          if (!ownsRequestLifecycleNow) return updated
          // isTurnError：可靠的失败信号（仅真抛错才 push，绝非 reconcile 未决的空成功）——
          // 让设计对话等消费端可给出一键重试，而不必用会误报的「空内容」启发式。
          updated.push({ role: "event", content: `${e}`, isTurnError: true })
          return updated
        })
      }
      if (options?.editMessageId != null && sid !== "__pending__") {
        try {
          const limit = Math.max(100, sessionCacheRef.current.get(sid)?.length ?? 0)
          const [rows] = await sendTransport.call<[SessionMessage[], number, boolean]>(
            "load_session_messages_latest_cmd",
            { sessionId: sid, limit },
          )
          updateSessionMessages(sid, () => parseSessionMessages(rows))
          if (!rows.some((row) => row.id === options.editMessageId)) {
            markDispatchAccepted()
          }
        } catch (reloadError) {
          logger.warn(
            "chat",
            "useChatStream::editMessage",
            "Failed to reconcile an edited message after dispatch error",
            reloadError,
          )
        }
        if (!dispatchAccepted) options.onPreparationError?.(e)
      }
      // Notify on error for non-current sessions
      if (
        ownsRequestLifecycleNow &&
        !keepExistingStreamLoading &&
        targetSessionId &&
        currentSessionIdRef.current !== targetSessionId
      ) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title
          // Only fall back to the latest user message when reply previews are
          // enabled; otherwise honor the privacy setting and never leak
          // conversation content into the system notification center.
          const userPreview =
            getCachedConfig()?.showChatContent !== false
              ? latestUserNotificationPreview(sessionCacheRef.current.get(targetSessionId) ?? [])
              : null
          const title = t("notification.chatError")
          notify(title, sessionTitle || userPreview || title)
        }
      }
    } finally {
      preparationAbortByRequestRef.current.delete(chatRequestOwnerId)
      backendStartedRequestIdsRef.current.delete(chatRequestOwnerId)
      const sid = targetSessionId || "__pending__"
      const ownsRequestLifecycle =
        chatRequestOwnerBySessionRef.current.get(sid) === chatRequestOwnerId
      const wasUserStopped = userStoppedRequestIdsRef.current.delete(chatRequestOwnerId)
      // Remove only this request's placeholder.  A watchdog can release the
      // session before this promise settles; using "last empty assistant"
      // here would delete a newer turn's placeholder.
      updateSessionMessages(sid, (prev) => {
        const index = prev.findIndex(
          (message) => message._clientId === assistantPlaceholderClientId,
        )
        if (index < 0) return prev
        const candidate = prev[index]
        if (
          candidate.role === "assistant" &&
          !candidate.content &&
          !candidate.toolCalls?.length &&
          !candidate.contentBlocks?.length
        ) {
          return [...prev.slice(0, index), ...prev.slice(index + 1)]
        }
        return prev
      })
      if (ownsRequestLifecycle) {
        if (keepExistingStreamLoading && sid !== "__pending__") {
          loadingSessionsRef.current.add(sid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
          if (currentSessionIdRef.current === sid) {
            setLoading(true)
          }
        } else {
          loadingSessionsRef.current.delete(sid)
          setLoadingSessionIds(new Set(loadingSessionsRef.current))
          if (currentSessionIdRef.current === sid) {
            setLoading(false)
          }
        }
      }
      // Notify on completion. Existing behavior: always notify for a
      // different session. For the active session, only surface an OS
      // notification when the app window is no longer foregrounded.
      if (ownsRequestLifecycle && !keepExistingStreamLoading && chatResolved && targetSessionId) {
        const agent = agents.find((a) => a.id === currentAgentId)
        if (isAgentNotifyEnabled(agent?.notifyOnComplete)) {
          const status = lastTurnStatusBySessionRef.current.get(targetSessionId)?.status
          const completed =
            status !== "failed" && status !== "interrupted" && status !== "cancelling"
          const sessionTitle = sessions.find((s) => s.id === targetSessionId)?.title || agentName
          const notification = chatCompletionNotificationPayload(
            sessionTitle,
            sessionCacheRef.current.get(targetSessionId) ??
              (currentSessionIdRef.current === targetSessionId ? messages : []),
            getCachedConfig()?.showChatContent !== false,
            t("notification.chatCompletedGenericTitle"),
            t("notification.chatCompletedGenericBody"),
          )
          if (completed && currentSessionIdRef.current !== targetSessionId) {
            void notify(notification.title, notification.body)
          } else if (completed) {
            void notifyIfBackground(notification.title, notification.body)
          }
        }
      }
      // Read state is advanced by the owning transcript only after the completed
      // database row has rendered and the tail is visible. Never mark the whole
      // session here: stream completion can beat React paint, which would clear
      // a reply the user has not actually seen yet.
      await reloadSessions()

      // SQLite remains authoritative. Reconcile the completed session, then
      // schedule exactly one FIFO row. If the user switched sessions we leave
      // that session's durable queue untouched instead of replaying it into the
      // newly selected conversation.
      if (targetSessionId) {
        const queue = await syncPendingSends(targetSessionId).catch(() => [])
        const queued = nextDispatchablePending(queue)
        const turnState = lastTurnStatusBySessionRef.current.get(targetSessionId)
        if (
          ownsRequestLifecycle &&
          shouldReplayNextPending(wasUserStopped, turnState) &&
          queued &&
          currentSessionIdRef.current === targetSessionId &&
          (queued.isPlanTrigger || queued.goalTrigger || autoSendPendingRef.current)
        ) {
          queuedReplayRef.current = {
            ...queued,
            options: {
              ...queued.options,
              sessionIdOverride: targetSessionId,
              queuedRequestId: queued.id,
            },
          }
          autoSendRef.current = true
        }
      }
      if (ownsRequestLifecycle) releaseChatRequestOwner()
    }
  }

  // Auto-send after React flushes loading=false. Durable queue replay always
  // carries queuedRequestId; the backend loads the authoritative payload.
  useEffect(() => {
    if (!autoSendRef.current || loading) return
    const replay = queuedReplayRef.current
    if (replay) {
      autoSendRef.current = false
      queuedReplayRef.current = null
      void handleSend(replay.text, replay.options)
    }
  }, [loading]) // eslint-disable-line react-hooks/exhaustive-deps

  const editPendingSend = useCallback(
    async (id: string, text: string): Promise<boolean> => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      const next = text.trim()
      if (!item || item.managedBy === "channel" || !next || item.status === "saving") return false
      const changed = await getTransport().call<boolean>("update_queued_turn_user_message", {
        sessionId: item.sessionId,
        requestId: id,
        message: next,
        displayText: item.options?.displayText ? next : undefined,
      })
      await syncPendingSends(item.sessionId)
      return changed
    },
    [syncPendingSends],
  )

  const discardPendingSend = useCallback(
    async (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (!item || item.managedBy === "channel") return
      if (item.status === "saving") {
        updatePendingSends((prev) => prev.filter((pending) => pending.id !== id))
        return
      }
      await getTransport().call<boolean>("delete_queued_turn_user_message", {
        sessionId: item.sessionId,
        requestId: id,
      })
      await syncPendingSends(item.sessionId)
    },
    [syncPendingSends, updatePendingSends],
  )

  const sendPendingSend = useCallback(
    async (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (
        !item ||
        item.managedBy === "channel" ||
        loading ||
        (item.status !== "queued" && item.status !== "fallback_after_reply")
      ) {
        return
      }
      await handleSend(item.text, {
        ...item.options,
        sessionIdOverride: item.sessionId,
        queuedRequestId: item.id,
      })
    },
    [loading], // eslint-disable-line react-hooks/exhaustive-deps
  )

  const forceInsertPendingSend = useCallback(
    async (id: string) => {
      const item = pendingSendsRef.current.find((pending) => pending.id === id)
      if (!item || !canForceInsertPending(item)) return
      const sid = currentSessionIdRef.current ?? currentSessionId
      const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
      if (!sid || !turnId) {
        return
      }
      try {
        const result = await getTransport().call<QueueTurnUserMessageResult>(
          "insert_queued_turn_user_message",
          {
            requestId: id,
            sessionId: sid,
            turnId,
          },
        )
        if (!result.queued) {
          logger.warn(
            "chat",
            "useChatStream::forceInsert",
            result.reason ?? "Queued message is no longer insertable",
          )
        }
        await syncPendingSends(sid)
      } catch (e) {
        logger.warn("chat", "useChatStream::forceInsert", "Failed to queue turn insertion", e)
        await syncPendingSends(sid).catch(() => undefined)
      }
    },
    [canForceInsertPending, currentSessionId, currentSessionIdRef, syncPendingSends],
  )

  const cancelForceInsertPendingSend = useCallback(
    async (id: string) => {
      const sid = currentSessionIdRef.current ?? currentSessionId
      const turnId = sid ? activeTurnBySessionRef.current.get(sid) : null
      if (sid && turnId) {
        const result = await getTransport()
          .call<CancelQueuedTurnUserMessageResult>("cancel_queued_turn_user_message", {
            sessionId: sid,
            turnId,
            requestId: id,
          })
          .catch(() => undefined)
        if (result && !result.cancelled) {
          logger.warn(
            "chat",
            "useChatStream::cancelForceInsert",
            result.reason ?? "Message already entered an insertion boundary",
          )
        }
        await syncPendingSends(sid).catch(() => undefined)
      }
    },
    [currentSessionId, currentSessionIdRef, syncPendingSends],
  )

  return {
    input,
    typedMentions: typedMentionsRef.current,
    setInput,
    setInputWithMention,
    appendInputMention,
    attachedFiles,
    setAttachedFiles,
    maxChatAttachmentBytes,
    pendingQuotes,
    setPendingQuotes,
    pendingMessageQuotes,
    setPendingMessageQuotes,
    pendingMessage,
    setPendingMessage,
    pendingSends,
    editPendingSend,
    discardPendingSend,
    sendPendingSend,
    forceInsertPendingSend,
    cancelForceInsertPendingSend,
    approvalRequests,
    showCodexAuthExpired,
    setShowCodexAuthExpired,
    permissionMode,
    setPermissionMode,
    setPermissionModeByUser,
    sandboxMode,
    setSandboxMode,
    setSandboxModeByUser,
    handleSend,
    handleStop,
    handleApprovalResponse,
    handleTurnStarted,
    handleTurnEnded,
    executionStateBySession,
  }
}
