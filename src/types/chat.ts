export interface AgentInfo {
  id: string
  name: string
  emoji?: string | null
  avatar?: string | null
}

/** Structured media item emitted by tools (e.g. send_attachment) — richer than
 *  mediaUrls; carries filename, MIME, size, and a kind flag used by the UI to
 *  decide between image preview and file-card rendering.
 *
 *  URL resolution is transport-aware: use `getTransport().resolveMediaUrl(item)`
 *  rather than reading `url` / `localPath` directly.
 *  - Tauri: `localPath` is the absolute server-side path; run through
 *    `convertFileSrc` for `<img src>` / `<a href>`.
 *  - HTTP/Web: `url` is already `/api/attachments/...?token=...` after the
 *    server-side rewrite — prepend base URL and use directly. `localPath` is
 *    stripped by the HTTP sink and must not appear. */
export interface MediaItem {
  url: string
  /** Absolute server-side path. Present only in Tauri mode (HTTP sink strips it). */
  localPath?: string
  name: string
  mimeType: string
  sizeBytes: number
  kind: "image" | "file"
  /** Optional caption shown with the attachment (e.g. IM caption). */
  caption?: string
}

/**
 * Structured side-output emitted by file-mutating tools (write / edit /
 * apply_patch) and the file-reading tool (read). Drives the right-side diff
 * panel and the `+N -M` summary in tool call headers. `null` `before` /
 * `after` flags create / delete actions respectively. `truncated` indicates
 * the embedded snapshot was capped (single-side limit, currently 256KB on
 * the backend) — the line counts stay accurate either way.
 */
export interface FileChangeMetadata {
  kind: "file_change"
  path: string
  action: "create" | "edit" | "delete"
  linesAdded: number
  linesRemoved: number
  before: string | null
  after: string | null
  language: string
  truncated: boolean
}

/** Aggregate side-output for tools that touch multiple files in one call (apply_patch). */
export interface FileChangesMetadata {
  kind: "file_changes"
  changes: FileChangeMetadata[]
}

/** Side-output for the read tool — used to render the "已浏览 N 个文件" aggregate. */
export interface FileReadMetadata {
  kind: "file_read"
  path: string
  lines: number
}

export type ToolMetadata = FileChangeMetadata | FileChangesMetadata | FileReadMetadata

export interface ToolCall {
  callId: string
  name: string
  arguments: string
  result?: string
  isError?: boolean
  mediaUrls?: string[]
  mediaItems?: MediaItem[]
  durationMs?: number
  startedAtMs?: number
  /** Structured side-output (see {@link ToolMetadata}). Absent on legacy
   *  rows persisted before the diff-panel feature shipped. */
  metadata?: ToolMetadata
}

export interface MessageUsage {
  durationMs?: number
  /** Cumulative across tool-loop rounds — billing value, not context size. */
  inputTokens?: number
  outputTokens?: number
  cacheCreationInputTokens?: number
  cacheReadInputTokens?: number
  /** Last round's input tokens — the prompt size the model actually saw.
   *  Use for context-usage UI; fall back to `inputTokens` when undefined
   *  (pre-migration turns). */
  lastInputTokens?: number
}

/** Ordered content block within an assistant message */
export type ContentBlock =
  | { type: "thinking"; content: string; durationMs?: number; interrupted?: boolean }
  | { type: "text"; content: string; interrupted?: boolean }
  | { type: "tool_call"; tool: ToolCall }

export interface Message {
  role: "user" | "assistant" | "event"
  content: string
  contentBlocks?: ContentBlock[]
  toolCalls?: ToolCall[]
  thinking?: string
  timestamp?: string
  usage?: MessageUsage
  model?: string
  fallbackEvent?: FallbackEvent
  /** If set, this user message was sent by a parent agent (not a human) */
  fromAgentId?: string
  /** If true, this user message is a sub-agent result injected by the backend */
  isSubagentResult?: boolean
  /** The child agent ID that produced the sub-agent result */
  subagentResultAgentId?: string
  /** If true, this user message was triggered by a cron job */
  isCronTrigger?: boolean
  /** If true, this user message is a Plan Mode trigger (approve / resume) —
   *  sent to the LLM as a normal user turn but rendered as a system chip
   *  in the UI to distinguish it from real user input. */
  isPlanTrigger?: boolean
  /** If set, this is a plan inline-comment user message. The desktop GUI
   *  renders {@link PlanCommentBubble} from this structured payload instead
   *  of falling back to the markdown `content`. IM channels render the
   *  markdown content directly and ignore this field. */
  planComment?: {
    selectedText: string
    comment: string
  }
  /** If true, this message is a hidden skill prompt — sent to LLM but not shown in the UI */
  isMeta?: boolean
  /** The cron job name that triggered this message */
  cronJobName?: string
  /** If set, this user message came from an IM channel */
  channelInbound?: {
    channelId: string
    senderName?: string
  }
  /** Model picker data for rendering interactive model selection cards */
  modelPickerData?: {
    models: { providerId: string; providerName: string; modelId: string; modelName: string }[]
    activeProviderId?: string
    activeModelId?: string
  }
  /** Context window breakdown data for rendering the /context card */
  contextBreakdownData?: import("@/components/chat/slash-commands/types").ContextBreakdown
  /** Database row ID, used for deduplication during streaming append */
  dbId?: number
  /** If true, this message is currently being streamed (channel streaming) */
  isStreaming?: boolean
  /**
   * Client-only stable identity that survives the placeholder→finalized
   * transition. Assigned when a streaming placeholder is created; transferred
   * by `mergeMessagesByDbId` to the fresh DB-loaded message that replaces it
   * at stream_end. Lets `messageStableId` produce the same React row key
   * across the transition, so the row isn't unmounted/remounted (which would
   * force the markdown / shiki / katex subtree to rebuild and flicker).
   * Never persisted to DB, never sent over the wire. Pure runtime field.
   */
  _clientId?: string
}

export interface FallbackEvent {
  type?: string
  model: string
  from_model?: string
  reason?: string
  error?: string
  attempt?: number
  total?: number
  provider_id?: string
  model_id?: string
}

export interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  thinkingStyle?: "openai" | "anthropic" | "zai" | "qwen" | "none"
}

export interface ActiveModel {
  providerId: string
  modelId: string
}

/**
 * Per-session permission mode (permission system v2).
 *
 * - `default` — hardcoded edit-class approval (write/edit/apply_patch + edit
 *   commands matched in exec) plus the agent's optional custom approval list.
 * - `smart` — defers approval decisions to a configured judge model (or the
 *   model's `_confidence` self-tag).
 * - `yolo` — bypass approvals; protected paths and dangerous commands still
 *   audit-warn but execute. Plan Mode can still block.
 */
export type SessionMode = "default" | "smart" | "yolo"

export interface SessionMeta {
  id: string
  title?: string | null
  titleSource?: "first_message" | "llm" | "manual"
  agentId: string
  providerId?: string | null
  providerName?: string | null
  modelId?: string | null
  /** Session-scoped Think / reasoning effort override. */
  reasoningEffort?: string | null
  createdAt: string
  updatedAt: string
  messageCount: number
  unreadCount: number
  hasError: boolean
  /**
   * Number of pending interactions awaiting the user (sum of pending tool
   * approvals + pending ask_user_question groups). Drives the sidebar
   * "needs your response" indicator on non-active sessions.
   */
  pendingInteractionCount: number
  isCron: boolean
  parentSessionId?: string | null
  /**
   * Per-session permission mode. Persisted so the chat title bar's mode
   * switcher is restored when switching back to a historical session.
   */
  permissionMode?: SessionMode
  /**
   * When set, this session belongs to a Project — project-scoped memories
   * and shared files are automatically injected into its system prompt.
   */
  projectId?: string | null
  /** Session-scoped incognito mode: no passive memory/awareness injection or auto-extract. */
  incognito: boolean
  /**
   * User-selected working directory for this conversation. Injected into the
   * system prompt so the model defaults file operations to this path. In
   * server mode the path refers to the server machine's filesystem, not the
   * browser client.
   */
  workingDir?: string | null
  channelInfo?: {
    channelId: string
    accountId: string
    chatId: string
    chatType: string
    senderName?: string | null
  } | null
}

export interface SessionMessage {
  id: number
  sessionId: string
  role: string
  content: string
  timestamp: string
  attachmentsMeta?: string | null
  model?: string | null
  /** Cumulative across tool-loop rounds — see `MessageUsage.inputTokens`. */
  tokensIn?: number | null
  tokensOut?: number | null
  /** Last round's input tokens — see `MessageUsage.lastInputTokens`. */
  tokensInLast?: number | null
  /** Anthropic prompt-cache write tokens — see `MessageUsage.cacheCreationInputTokens`. */
  tokensCacheCreation?: number | null
  /** Prompt-cache read tokens — see `MessageUsage.cacheReadInputTokens`. */
  tokensCacheRead?: number | null
  toolCallId?: string | null
  toolName?: string | null
  toolArguments?: string | null
  toolResult?: string | null
  toolDurationMs?: number | null
  isError?: boolean | null
  thinking?: string | null
  /** JSON string with structured tool side-output (see {@link ToolMetadata}). */
  toolMetadata?: string | null
  /**
   * Streaming persistence state for thinking_block / text_block rows that
   * were inserted incrementally before the turn finalized. `streaming` =
   * write in progress; `completed` = clean finalize; `orphaned` = a previous
   * run died mid-stream and startup sweep marked it. Absent on legacy rows
   * (treat as `completed`).
   */
  streamStatus?: "streaming" | "completed" | "orphaned" | null
}

/**
 * A single message match from a full-text session search.
 *
 * `contentSnippet` may contain `<mark>...</mark>` tags wrapping matched
 * tokens. Render with care (whitelist `<mark>` only).
 */
export interface SessionSearchResult {
  messageId: number
  sessionId: string
  sessionTitle: string | null
  agentId: string
  messageRole: string
  contentSnippet: string
  timestamp: string
  relevanceRank: number
  isCron: boolean
  parentSessionId: string | null
  channelType: string | null
  channelChatType: string | null
}

export type SessionSearchType = "regular" | "cron" | "subagent" | "channel"

export interface AgentSummaryForSidebar {
  id: string
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  notifyOnComplete?: boolean | null
}

// ── Sub-Agent Types ─────────────────────────────────────────────

export interface SubagentEvent {
  eventType: "spawned" | "running" | "completed" | "error" | "killed" | "timeout" | "steered"
  runId: string
  parentSessionId: string
  childAgentId: string
  childSessionId: string
  taskPreview: string
  status: "spawning" | "running" | "completed" | "error" | "timeout" | "killed"
  resultPreview?: string
  resultFull?: string
  error?: string
  durationMs?: number
  label?: string
  inputTokens?: number
  outputTokens?: number
}

export interface SubagentRun {
  runId: string
  parentSessionId: string
  parentAgentId: string
  childAgentId: string
  childSessionId: string
  task: string
  status: "spawning" | "running" | "completed" | "error" | "timeout" | "killed"
  result?: string
  error?: string
  depth: number
  modelUsed?: string
  startedAt: string
  finishedAt?: string
  durationMs?: number
  label?: string
  attachmentCount?: number
  inputTokens?: number
  outputTokens?: number
}

export type TaskStatus = "pending" | "in_progress" | "completed"

export interface Task {
  id: number
  sessionId: string
  content: string
  activeForm?: string | null
  batchId?: string | null
  status: TaskStatus
  createdAt: string
  updatedAt: string
}

export interface ParentAgentStreamEvent {
  eventType: "started" | "delta" | "done" | "error"
  parentSessionId: string
  runId: string
  pushMessage?: string // only for "started"
  delta?: string // raw JSON delta string, only for "delta"
  error?: string // only for "error"
}

export interface SubagentConfig {
  enabled: boolean
  allowedAgents: string[]
  deniedAgents: string[]
  maxConcurrent: number
  defaultTimeoutSecs: number
  model?: string
  deniedTools: string[]
  maxSpawnDepth?: number
  maxBatchSize?: number
  archiveAfterMinutes?: number
  announceTimeoutSecs?: number
}

export function modelSupportsThinking(
  model?: Pick<AvailableModel, "reasoning" | "thinkingStyle">,
): boolean {
  if (!model) return true
  return model.reasoning && model.thinkingStyle !== "none"
}

export function getEffortOptionsForType(apiType: string | undefined, t: (key: string) => string) {
  const off = t("effort.off")
  const on = t("effort.on")
  const minimal = t("effort.minimal")
  const low = t("effort.low")
  const medium = t("effort.medium")
  const high = t("effort.high")
  const xhigh = t("effort.xhigh")
  switch (apiType) {
    case "openai-responses":
    case "codex":
      return [
        { value: "none", label: off },
        { value: "minimal", label: minimal },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
        { value: "xhigh", label: xhigh },
      ]
    case "anthropic":
    case "openai-chat":
      return [
        { value: "none", label: off },
        { value: "low", label: low },
        { value: "medium", label: medium },
        { value: "high", label: high },
      ]
    default:
      return [
        { value: "none", label: off },
        { value: "medium", label: on },
      ]
  }
}

export function getEffortOptionsForModel(
  model: Pick<AvailableModel, "apiType" | "reasoning" | "thinkingStyle"> | undefined,
  t: (key: string) => string,
) {
  if (!modelSupportsThinking(model)) {
    return [{ value: "none", label: t("effort.off") }]
  }
  return getEffortOptionsForType(model?.apiType, t)
}

export function normalizeEffortForModel(
  model: Pick<AvailableModel, "apiType" | "reasoning" | "thinkingStyle"> | undefined,
  effort: string,
  t: (key: string) => string,
): string {
  const validOptions = getEffortOptionsForModel(model, t)
  if (validOptions.some((opt) => opt.value === effort)) {
    return effort
  }
  return validOptions.some((opt) => opt.value === "medium") ? "medium" : "none"
}
