import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { applyInlineHighlight, clearInlineHighlight } from "@/lib/inlineHighlight"
import { hasActiveTextSelection } from "@/lib/contextMenuGuard"
import { AnimatedCollapse, AnimatedPresenceBox } from "@/components/ui/animated-presence"
import {
  extractMessageFileAttachments,
  formatDuration,
  isCenteredSystemMessage,
  isUserAlignedMessage,
  type MessageFileAttachment,
} from "./chatUtils"
import { ChatWelcomeHero } from "./ChatWelcomeHero"
import { SkillMentionText } from "./skill-mention/SkillMentionText"
import MessageBubble from "./MessageBubble"
import {
  goalCompletionReportFromMessage,
  type GoalCompletionReport,
} from "./message/goalCompletionReport"
import MessageContextMenu, { type MessageContextMenuState } from "./message/MessageContextMenu"
import LoadMoreRow from "./LoadMoreRow"
import AskUserQuestionBlock from "./ask-user/AskUserQuestionBlock"
import PlanCardBlock from "./plan-mode/PlanCardBlock"
import { findMessageRowByKey, getLatestUserTurnKey, getMessageRowKey } from "./chatScrollKeys"
import type { AskUserQuestionGroup } from "./ask-user/AskUserQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"
import type { CompactResult } from "./sessionStatus"
import type {
  ChatDisplayMode,
  ChatTurnStatus,
  Message,
  AgentSummaryForSidebar,
  PendingMessageQuote,
} from "@/types/chat"
import type { PlanModeState } from "./plan-mode/usePlanMode"

interface MessageListProps {
  messages: Message[]
  historyLoading?: boolean
  loading: boolean
  executionState?: ChatTurnStatus | null
  agents: AgentSummaryForSidebar[]
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void | Promise<void>
  /** Whether the backend has more messages newer than the loaded window.
   *  True only after a search-jump landed the user on an around-window;
   *  false during normal latest-page browsing. */
  hasMoreAfter?: boolean
  loadingMoreAfter?: boolean
  onLoadMoreAfter?: () => void | Promise<void>
  /** Drop the partial around-window and reload the latest page. Wired
   *  to the jump-to-latest button when `hasMoreAfter` is true. */
  onResetToLatest?: () => void | Promise<void>
  sessionId?: string | null
  incognito?: boolean
  /**
   * When true, `ChatScreen` is rendering the empty-session greeting itself,
   * stacked above the centered hero composer — so MessageList must NOT also
   * render its own empty greeting (the two would overlap). See {@link ChatWelcomeHero}.
   */
  heroComposer?: boolean
  welcomeContext?: "chat" | "knowledge"
  projectName?: string | null
  onProjectSuggestion?: (prompt: string) => void
  /** Search-jump target + literal substrings to inline-highlight inside
   *  the matched bubble. `null` between jumps. The terms are painted via
   *  the CSS Custom Highlight API in `lib/inlineHighlight.ts`. */
  pendingScrollIntent?: { messageId: number; highlightTerms: string[] | null } | null
  onScrollTargetHandled?: () => void
  pendingQuestionGroup?: AskUserQuestionGroup | null
  onQuestionSubmitted?: () => void
  planCardData?: PlanCardData | null
  planState?: PlanModeState
  onOpenPlanPanel?: () => void
  onApprovePlan?: () => void
  onExitPlan?: () => void
  planSubagentRunning?: boolean
  onSwitchModel?: (providerId: string, modelId: string) => void
  onViewSystemPrompt?: () => void
  compacting?: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
  onViewChildSession?: (sessionId: string) => void
  onOpenDiff?: (
    metadata:
      | import("@/types/chat").FileChangeMetadata
      | import("@/types/chat").FileChangesMetadata,
  ) => void
  onResume?: (message: string) => void
  onForkFromMessage?: (messageId: number) => void
  onOpenMemorySettings?: () => void
  onOpenKnowledge?: () => void
  onAddQuickPrompt?: (content: string) => void
  onAddMessageQuote?: (quote: PendingMessageQuote) => void
  renderMessageActions?: (msg: Message, index: number) => ReactNode
  displayMode?: ChatDisplayMode
  autoCollapseCompletedTurns?: boolean
}

const AT_BOTTOM_THRESHOLD_PX = 48
const LOAD_MORE_THRESHOLD_PX = 200
const CHAT_CONTENT_MAX_WIDTH_CLASS = "max-w-[880px]"
// Windowed view: cap simultaneously-rendered messages so a long-running
// session that's been Load-More'd many times doesn't accumulate thousands of
// markdown / shiki / katex subtrees in DOM. `messages` itself is not trimmed
// — only the render slice. See `displayedStart`.
const MAX_DOM_MESSAGES = 200
const UNLOAD_BATCH = 30
const COMPACT_USER_ANCHOR_LEAD_PX = 32
const COMPACT_USER_REPLY_VISIBLE_MIN_PX = 56
const COMPACT_USER_ANCHOR_EXIT_MS = 200
const ASK_USER_FOLLOW_FRAMES = 16

interface MessageRenderItem {
  msg: Message
  originalIndex: number
  keyOverride?: string
  sourceDbId?: number
  footerFiles?: MessageFileAttachment[]
  hideOwnFooterFiles?: boolean
  goalCompletionReport?: GoalCompletionReport | null
  suppressGoalCompletionFooter?: boolean
}

interface CompletedTurnCollapseRow {
  kind: "completed-turn-collapse"
  key: string
  items: MessageRenderItem[]
  assistantCount: number
  elapsedMs?: number
  expanded: boolean
}

type MessageRenderRow = { kind: "message"; item: MessageRenderItem } | CompletedTurnCollapseRow

interface CompactUserAnchor {
  dbId?: number
  rowKey: string
  bodyStartRowKey: string
  bodyEndRowKey: string
  text: string
}

function preferredScrollBehavior(): ScrollBehavior {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return "smooth"
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth"
}

function shouldPassExecutionStateToBubble(
  isLast: boolean,
  loading: boolean,
  executionState: ChatTurnStatus | null | undefined,
): boolean {
  if (!isLast) return false
  if (!executionState || executionState === "completed") return false
  return loading || executionState !== "running"
}

function isHumanTurnStart(msg: Message): boolean {
  if (msg.fromAgentId) return false
  if (msg.role === "user" && !isCenteredSystemMessage(msg)) return true
  return msg.slashEvent?.displayAs === "user"
}

function findRenderWindowTurnStart(messages: Message[], start: number): number {
  if (messages.length === 0) return 0
  let i = Math.min(start, Math.max(0, messages.length - 1))
  while (i > 0) {
    const msg = messages[i]
    if (msg && !msg.isMeta && isHumanTurnStart(msg)) return i
    i -= 1
  }
  return 0
}

function timestampMs(msg: Message): number | null {
  if (!msg.timestamp) return null
  const ms = Date.parse(msg.timestamp)
  return Number.isFinite(ms) ? ms : null
}

function messageElapsedMs(msg: Message): number {
  let blockTotal = 0
  const blocks = msg.contentBlocks
  if (blocks && blocks.length > 0) {
    for (const block of blocks) {
      if (block.type === "thinking") blockTotal += block.durationMs ?? 0
      if (block.type === "tool_call") blockTotal += block.tool.durationMs ?? 0
    }
  } else if (msg.toolCalls) {
    for (const tool of msg.toolCalls) blockTotal += tool.durationMs ?? 0
  }
  return Math.max(msg.usage?.durationMs ?? 0, blockTotal)
}

function rowKeyForItem(item: MessageRenderItem): string {
  return item.keyOverride ?? getMessageRowKey(item.msg, item.originalIndex)
}

function itemMatchesMessageId(item: MessageRenderItem, messageId: number | null): boolean {
  if (messageId == null) return false
  return item.msg.dbId === messageId || item.sourceDbId === messageId
}

function textContentFromBlocks(blocks: NonNullable<Message["contentBlocks"]>): string {
  const texts: string[] = []
  for (const block of blocks) {
    if (block.type === "text" && block.content.trim().length > 0) {
      texts.push(block.content)
    }
  }
  return texts.join("\n\n")
}

function messageFileAttachmentKey(file: MessageFileAttachment): string {
  return file.kind === "media"
    ? `media:${file.item.localPath || file.item.url || file.item.name}`
    : `path:${file.path}`
}

function mergeMessageFileAttachments(
  ...groups: Array<readonly MessageFileAttachment[] | undefined>
): MessageFileAttachment[] {
  const merged = new Map<string, MessageFileAttachment>()
  for (const group of groups) {
    for (const file of group ?? []) {
      const key = messageFileAttachmentKey(file)
      const existing = merged.get(key)
      if (!existing) {
        merged.set(key, file)
      } else if (
        existing.kind === "path" &&
        file.kind === "path" &&
        !existing.language &&
        file.language
      ) {
        existing.language = file.language
      }
    }
  }
  return [...merged.values()]
}

function filesFromRenderItem(item: MessageRenderItem): MessageFileAttachment[] {
  const blocks = item.msg.contentBlocks
  return item.msg.role === "assistant" && blocks ? extractMessageFileAttachments(blocks) : []
}

function hideFooterFilesOnItems(items: MessageRenderItem[]): MessageRenderItem[] {
  return items.map((item) =>
    filesFromRenderItem(item).length > 0 || item.footerFiles?.length
      ? { ...item, hideOwnFooterFiles: true }
      : item,
  )
}

function assistantProcessBlockCount(blocks: NonNullable<Message["contentBlocks"]>): number {
  return blocks.filter(
    (block) => block.type === "thinking" || block.type === "tool_call" || block.type === "text",
  ).length
}

function appendToolText(parts: string[], tool: NonNullable<Message["toolCalls"]>[number]) {
  parts.push(tool.name, tool.arguments)
  if (tool.result) parts.push(tool.result)
}

function messageSearchText(msg: Message): string {
  const parts = [msg.content, msg.thinking ?? ""]
  if (msg.contentBlocks) {
    for (const block of msg.contentBlocks) {
      if (block.type === "text" || block.type === "thinking") {
        parts.push(block.content)
      } else if (block.type === "tool_call") {
        appendToolText(parts, block.tool)
      }
    }
  }
  if (msg.toolCalls) {
    for (const tool of msg.toolCalls) appendToolText(parts, tool)
  }
  return parts.filter(Boolean).join("\n")
}

function containsAnyHighlightTerm(
  text: string | null | undefined,
  terms: string[] | null,
): boolean {
  if (!terms || terms.length === 0 || !text) return false
  const haystack = text.toLowerCase()
  return terms.some((term) => term && haystack.includes(term.toLowerCase()))
}

function itemContainsAnyHighlightTerm(item: MessageRenderItem, terms: string[] | null): boolean {
  return containsAnyHighlightTerm(messageSearchText(item.msg), terms)
}

function compactAnchorTextForMessage(msg: Message): string | null {
  const text = (msg.planComment?.comment || msg.slashEvent?.command || msg.content)
    .replace(/\s+/g, " ")
    .trim()
  return text || null
}

function findActiveCompactUserAnchor(
  container: HTMLElement,
  anchors: CompactUserAnchor[],
): CompactUserAnchor | null {
  const containerRect = container.getBoundingClientRect()
  let active: CompactUserAnchor | null = null

  for (const anchor of anchors) {
    const bodyStart = findMessageRowByKey(container, anchor.bodyStartRowKey)
    const bodyEnd = findMessageRowByKey(container, anchor.bodyEndRowKey)
    if (!bodyStart || !bodyEnd) continue
    const bodyStartRect = bodyStart.getBoundingClientRect()
    const bodyEndRect = bodyEnd.getBoundingClientRect()
    const target = findMessageRowByKey(container, anchor.rowKey)
    const targetHasScrolledPast = target
      ? target.getBoundingClientRect().bottom < containerRect.top - COMPACT_USER_ANCHOR_LEAD_PX
      : true

    if (targetHasScrolledPast) {
      const bodyTop = bodyStartRect.top
      const bodyBottom = bodyEndRect.bottom
      const visibleHeight =
        Math.min(bodyBottom, containerRect.bottom) - Math.max(bodyTop, containerRect.top)
      const bodyHeight = Math.max(0, bodyBottom - bodyTop)
      const minVisibleHeight = Math.min(COMPACT_USER_REPLY_VISIBLE_MIN_PX, bodyHeight)
      if (visibleHeight < minVisibleHeight) continue
      active = anchor
      continue
    }
    break
  }

  return active
}

function findMessageScrollTarget(
  scope: ParentNode,
  targetId: number,
  highlightTerms: string[] | null,
): HTMLElement | null {
  const candidates = Array.from(
    scope.querySelectorAll<HTMLElement>(
      `[data-message-id="${targetId}"], [data-message-source-id="${targetId}"]`,
    ),
  )
  if (candidates.length === 0) return null
  const termMatch = candidates.find((candidate) =>
    containsAnyHighlightTerm(candidate.textContent, highlightTerms),
  )
  return termMatch ?? candidates[0]
}

function splitAssistantFinalAnswer(item: MessageRenderItem): {
  prefixItem: MessageRenderItem
  finalItem: MessageRenderItem
  prefixCount: number
} | null {
  const blocks = item.msg.contentBlocks
  if (item.msg.role !== "assistant" || !blocks || blocks.length < 2) return null

  let finalTextIndex = -1
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i]
    if (block.type === "text" && block.content.trim().length > 0) {
      finalTextIndex = i
      break
    }
  }
  if (finalTextIndex <= 0) return null

  const prefixBlocks = blocks.slice(0, finalTextIndex)
  const finalBlocks = blocks.slice(finalTextIndex)
  const prefixCount = assistantProcessBlockCount(prefixBlocks)
  if (prefixCount === 0) return null

  const baseKey = rowKeyForItem(item)
  const prefixContent = textContentFromBlocks(prefixBlocks)
  const finalContent = textContentFromBlocks(finalBlocks) || item.msg.content
  const hoistedGoalCompletionReport = goalCompletionReportFromMessage({
    ...item.msg,
    content: prefixContent,
    contentBlocks: prefixBlocks,
    toolCalls: undefined,
  })

  return {
    prefixCount,
    prefixItem: {
      msg: {
        ...item.msg,
        dbId: undefined,
        _clientId: item.msg._clientId ? `${item.msg._clientId}:prefix` : undefined,
        content: prefixContent,
        contentBlocks: prefixBlocks,
        toolCalls: undefined,
        usage: undefined,
      },
      originalIndex: item.originalIndex,
      keyOverride: `${baseKey}:prefix`,
      sourceDbId: item.msg.dbId,
      suppressGoalCompletionFooter: hoistedGoalCompletionReport != null,
    },
    finalItem: {
      msg: {
        ...item.msg,
        content: finalContent,
        contentBlocks: finalBlocks,
        toolCalls: undefined,
        thinking: undefined,
      },
      originalIndex: item.originalIndex,
      keyOverride: `${baseKey}:final`,
      ...(hoistedGoalCompletionReport ? { goalCompletionReport: hoistedGoalCompletionReport } : {}),
    },
  }
}

function completedTurnElapsedMs(
  turnItems: MessageRenderItem[],
  foldedItems: MessageRenderItem[],
  finalAssistantItem: MessageRenderItem,
): number | undefined {
  const start = timestampMs(turnItems[0]?.msg)
  const end = timestampMs(finalAssistantItem.msg)
  const wallClockMs = start != null && end != null && end > start ? end - start : 0
  const measuredMs = [...foldedItems, finalAssistantItem].reduce(
    (sum, item) => sum + messageElapsedMs(item.msg),
    0,
  )
  const elapsed = Math.max(wallClockMs, measuredMs)
  return elapsed > 0 ? elapsed : undefined
}

function completedTurnCollapseKey(
  userItem: MessageRenderItem,
  finalAssistantItem: MessageRenderItem,
): string {
  return ["completed-turn", rowKeyForItem(userItem), rowKeyForItem(finalAssistantItem)].join(":")
}

function isCurrentTurnStillRunning(
  finalAssistantItem: MessageRenderItem,
  messagesLength: number,
  loading: boolean,
  executionState: ChatTurnStatus | null | undefined,
): boolean {
  if (finalAssistantItem.originalIndex !== messagesLength - 1) return false
  return loading || executionState === "running" || executionState === "cancelling"
}

function buildMessageRenderRows(
  items: MessageRenderItem[],
  options: {
    enabled: boolean
    expandedKeys: Set<string>
    loading: boolean
    executionState: ChatTurnStatus | null | undefined
    messagesLength: number
  },
): MessageRenderRow[] {
  if (!options.enabled) return items.map((item) => ({ kind: "message", item }))

  const rows: MessageRenderRow[] = []
  let i = 0
  while (i < items.length) {
    const item = items[i]
    if (!isHumanTurnStart(item.msg)) {
      rows.push({ kind: "message", item })
      i += 1
      continue
    }

    let nextTurn = i + 1
    while (nextTurn < items.length && !isHumanTurnStart(items[nextTurn].msg)) {
      nextTurn += 1
    }

    const turnItems = items.slice(i, nextTurn)
    let finalAssistantPos = -1
    for (let j = turnItems.length - 1; j >= 1; j -= 1) {
      if (turnItems[j].msg.role === "assistant") {
        finalAssistantPos = j
        break
      }
    }

    const finalAssistantItem = finalAssistantPos >= 0 ? turnItems[finalAssistantPos] : undefined
    const finalAssistantSplit = finalAssistantItem
      ? splitAssistantFinalAnswer(finalAssistantItem)
      : null
    const foldedItems = finalAssistantPos > 1 ? turnItems.slice(1, finalAssistantPos) : []
    const assistantCount =
      foldedItems.filter((folded) => folded.msg.role === "assistant").length +
      (finalAssistantSplit?.prefixCount ?? 0)
    const canFold =
      finalAssistantItem &&
      assistantCount > 0 &&
      !isCurrentTurnStillRunning(
        finalAssistantItem,
        options.messagesLength,
        options.loading,
        options.executionState,
      )

    if (!canFold || !finalAssistantItem) {
      for (const turnItem of turnItems) rows.push({ kind: "message", item: turnItem })
      i = nextTurn
      continue
    }

    const rawCollapsedItems = finalAssistantSplit
      ? [...foldedItems, finalAssistantSplit.prefixItem]
      : foldedItems
    const hoistedFiles = mergeMessageFileAttachments(
      ...rawCollapsedItems.map(filesFromRenderItem),
      ...rawCollapsedItems.map((collapsedItem) => collapsedItem.footerFiles),
    )
    const collapsedItems = hideFooterFilesOnItems(rawCollapsedItems)
    const finalAssistantWithHoistedFiles: MessageRenderItem =
      hoistedFiles.length > 0
        ? {
            ...finalAssistantItem,
            footerFiles: mergeMessageFileAttachments(finalAssistantItem.footerFiles, hoistedFiles),
          }
        : finalAssistantItem
    const finalSplitItemWithHoistedFiles: MessageRenderItem | undefined =
      finalAssistantSplit && hoistedFiles.length > 0
        ? {
            ...finalAssistantSplit.finalItem,
            footerFiles: mergeMessageFileAttachments(
              finalAssistantSplit.finalItem.footerFiles,
              hoistedFiles,
            ),
          }
        : finalAssistantSplit?.finalItem

    const collapseKey = completedTurnCollapseKey(turnItems[0], finalAssistantItem)
    const expanded = options.expandedKeys.has(collapseKey)
    rows.push({ kind: "message", item: turnItems[0] })
    rows.push({
      kind: "completed-turn-collapse",
      key: collapseKey,
      items: collapsedItems,
      assistantCount,
      elapsedMs: completedTurnElapsedMs(
        turnItems,
        finalAssistantSplit ? foldedItems : collapsedItems,
        finalAssistantItem,
      ),
      expanded,
    })
    if (expanded) {
      for (const collapsedItem of collapsedItems) {
        rows.push({ kind: "message", item: collapsedItem })
      }
    }
    const tailItems = turnItems.slice(finalAssistantPos)
    if (finalAssistantSplit) {
      rows.push({
        kind: "message",
        item: finalSplitItemWithHoistedFiles ?? finalAssistantSplit.finalItem,
      })
      for (const tailItem of tailItems.slice(1)) rows.push({ kind: "message", item: tailItem })
    } else {
      for (let tailIdx = 0; tailIdx < tailItems.length; tailIdx += 1) {
        const tailItem = tailItems[tailIdx]
        if (!tailItem) continue
        rows.push({
          kind: "message",
          item: tailIdx === 0 ? finalAssistantWithHoistedFiles : tailItem,
        })
      }
    }
    i = nextTurn
  }
  return rows
}

function CompletedTurnCollapseSummary({
  row,
  onToggle,
}: {
  row: CompletedTurnCollapseRow
  onToggle: (key: string) => void
}) {
  const { t } = useTranslation()
  const duration = row.elapsedMs != null ? formatDuration(row.elapsedMs) : null
  const label = duration
    ? t("chat.completedTurnCollapsedWithDuration", {
        duration,
        count: row.assistantCount,
        defaultValue: `Processed for ${duration}, ${row.assistantCount} messages`,
      })
    : t("chat.completedTurnCollapsed", {
        count: row.assistantCount,
        defaultValue: `Processed ${row.assistantCount} messages`,
      })

  return (
    <div
      key={row.key}
      data-message-key={row.key}
      className="grid w-full min-w-0 grid-cols-1 justify-items-stretch pb-3"
    >
      <button
        type="button"
        aria-expanded={row.expanded}
        onClick={() => onToggle(row.key)}
        className="group flex h-9 w-full cursor-pointer items-center gap-1.5 border-b border-border/50 px-0 text-left text-sm font-medium text-muted-foreground/75 transition-colors hover:text-muted-foreground"
      >
        <span className="truncate">{label}</span>
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 transition-transform duration-200",
            row.expanded && "rotate-90",
          )}
        />
      </button>
    </div>
  )
}

export default function MessageList({
  messages,
  historyLoading = false,
  loading,
  executionState,
  agents,
  hasMore,
  loadingMore,
  onLoadMore,
  hasMoreAfter = false,
  loadingMoreAfter = false,
  onLoadMoreAfter,
  onResetToLatest,
  sessionId,
  incognito = false,
  heroComposer = false,
  welcomeContext = "chat",
  projectName,
  onProjectSuggestion,
  pendingScrollIntent,
  onScrollTargetHandled,
  pendingQuestionGroup,
  onQuestionSubmitted,
  planCardData,
  planState,
  onOpenPlanPanel,
  onApprovePlan,
  onExitPlan,
  planSubagentRunning,
  onSwitchModel,
  onViewSystemPrompt,
  compacting,
  onCompactContext,
  onOpenDashboardTab,
  onViewChildSession,
  onOpenDiff,
  onResume,
  onForkFromMessage,
  onOpenMemorySettings,
  onOpenKnowledge,
  onAddQuickPrompt,
  onAddMessageQuote,
  renderMessageActions,
  displayMode = "bubble",
  autoCollapseCompletedTurns = true,
}: MessageListProps) {
  const { t } = useTranslation()
  const rootRef = useRef<HTMLDivElement | null>(null)
  const containerRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const sessionKey = sessionId ?? "draft-session"

  const [hoveredMsgIndex, setHoveredMsgIndex] = useState<number | null>(null)
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const [highlightMessageId, setHighlightMessageId] = useState<number | null>(null)
  const [searchExpandedUserMessageId, setSearchExpandedUserMessageId] = useState<number | null>(
    null,
  )
  const [compactUserAnchor, setCompactUserAnchor] = useState<CompactUserAnchor | null>(null)
  const [compactUserAnchorFrame, setCompactUserAnchorFrame] = useState<{
    left: number
    width: number
  } | null>(null)
  const [compactUserAnchorVisible, setCompactUserAnchorVisible] = useState(false)
  const [compactUserAnchorMounted, setCompactUserAnchorMounted] = useState(false)
  const [expandedCompletedTurns, setExpandedCompletedTurns] = useState<Set<string>>(() => new Set())
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const highlightTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const askUserFollowRafRef = useRef<number | null>(null)
  const lastAskUserFollowKeyRef = useRef<string | null>(null)
  const [contextMenu, setContextMenu] = useState<MessageContextMenuState | null>(null)

  // Single source of truth: are we at (or following) the bottom?
  // Default true so the first paint after mount/session swap aligns to bottom.
  const [atBottom, setAtBottom] = useState(true)
  const atBottomRef = useRef(true)
  // User-intent flag: when true, auto-follow (useLayoutEffect + ResizeObserver)
  // is suspended even if streaming would otherwise pin to bottom. Set by user
  // gestures (wheel / touch / arrow keys) and cleared when the user reaches
  // bottom on their own or clicks jump-to-latest.
  const userScrollLockRef = useRef(false)

  // Windowed view start: only `messages.slice(displayedStart)` is rendered.
  // Advances when at bottom + DOM bloated (drops top); decrements when user
  // scrolls near top + local older messages exist (restores them); falls
  // through to remote `onLoadMore` only when the window is fully expanded.
  // Reset on session swap via the prop-derived state pattern below.
  const [displayedStart, setDisplayedStart] = useState(0)
  const [displayedStartSession, setDisplayedStartSession] = useState(sessionKey)
  const [displayedStartMessagesLength, setDisplayedStartMessagesLength] = useState(messages.length)
  // Tracks the previous `messages[0]` so a length change can be classified
  // as prepend (Load More) vs append (streaming) vs reload (cap-rebuild).
  const [prevFirstMessage, setPrevFirstMessage] = useState<Message | null>(messages[0] ?? null)
  if (displayedStartSession !== sessionKey) {
    setDisplayedStartSession(sessionKey)
    setDisplayedStartMessagesLength(messages.length)
    setDisplayedStart(0)
    setPrevFirstMessage(messages[0] ?? null)
  } else if (displayedStartMessagesLength !== messages.length) {
    // Message content streaming reuses the same item, so this only runs on append/reload/prepend.
    const prevLength = displayedStartMessagesLength
    const prependCount = messages.length - prevLength
    const prevFirst = prevFirstMessage
    setDisplayedStartMessagesLength(messages.length)
    setPrevFirstMessage(messages[0] ?? null)
    if (
      displayedStart !== 0 &&
      (messages.length < prevLength || displayedStart >= messages.length)
    ) {
      // Snap to tail; on a head-trim resetting to 0 would mount every
      // surviving bubble in a single frame.
      setDisplayedStart(Math.max(0, messages.length - MAX_DOM_MESSAGES))
    } else if (
      // Prepend (Load More) detected: push the window forward by the
      // newly-prepended count so `items[0]` stays the same DOM node.
      // Without this, items.slice(0) would mount every prepended bubble
      // in a single commit; their async-rendered subtrees (KaTeX,
      // Mermaid, Shiki, images) finalize their heights over the next
      // several frames, but `[overflow-anchor:none]` (line 589) opted
      // out of browser auto-anchoring and the useLayoutEffect below
      // only compensates once with stale dimensions — leaving scrollTop
      // pinned at the macOS WebKit rubber-band overscroll value (~-9)
      // while scrollHeight balloons, so the viewport reads blank until
      // the user scrolls and triggers a layout flush.
      //
      // Identity check below confirms it's an actual prepend, not a
      // reload that happens to grow the array. `dbId` is the stable
      // identity (database row id) — `chatUtils.mergeMessagesByDbId`
      // replaces in-place with new object references when the backend
      // re-sends the same row, so a pure reference compare
      // (`messages[prependCount] === prevFirst`) silently fails. Fall
      // back to reference compare only when neither side has a dbId
      // (streaming placeholders, never the case at messages[0] in
      // practice). `!atBottom` skips the case where streaming
      // append raced past in the same tick (no actual prepend; user is
      // at bottom anyway). State (not ref) here because render must read
      // a render-stable snapshot — the scroll handler always sets state +
      // ref together, so the lag is at most one frame.
      prependCount > 0 &&
      !atBottom &&
      prevFirst != null &&
      messages[prependCount] != null &&
      (prevFirst.dbId != null && messages[prependCount].dbId != null
        ? messages[prependCount].dbId === prevFirst.dbId
        : messages[prependCount] === prevFirst)
    ) {
      setDisplayedStart((s) => s + prependCount)
    }
  }
  // Refs mirror state/props for the scroll listener which is bound in an
  // effect with deps {sessionKey, hasMore, loadingMore, onLoadMore} — keeping
  // these out of deps avoids re-binding on every token / window step.
  const displayedStartRef = useRef(displayedStart)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  displayedStartRef.current = displayedStart
  const messagesRef = useRef(messages)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  messagesRef.current = messages
  // After-pagination state mirrored into refs so the scroll listener (whose
  // deps are deliberately narrow) can read fresh values without re-binding
  // on every loading flip / window step.
  const hasMoreAfterRef = useRef(hasMoreAfter)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  hasMoreAfterRef.current = hasMoreAfter
  const loadingMoreAfterRef = useRef(loadingMoreAfter)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  loadingMoreAfterRef.current = loadingMoreAfter
  const onLoadMoreAfterRef = useRef(onLoadMoreAfter)
  // eslint-disable-next-line react-hooks/refs -- ref-as-snapshot
  onLoadMoreAfterRef.current = onLoadMoreAfter

  // Filter isMeta but preserve originalIndex for MessageBubble props. Slice
  // starts at `displayedStart` so older messages outside the window aren't
  // mounted at all.
  const items = useMemo(() => {
    const out: { msg: Message; originalIndex: number }[] = []
    const requestedStart = Math.min(displayedStart, Math.max(0, messages.length - 1))
    const start = findRenderWindowTurnStart(messages, requestedStart)
    for (let i = start; i < messages.length; i++) {
      const msg = messages[i]
      if (!msg.isMeta) out.push({ msg, originalIndex: i })
    }
    return out
  }, [messages, displayedStart])

  const renderRows = useMemo(
    () =>
      buildMessageRenderRows(items, {
        enabled: autoCollapseCompletedTurns,
        expandedKeys: expandedCompletedTurns,
        loading,
        executionState,
        messagesLength: messages.length,
      }),
    [
      autoCollapseCompletedTurns,
      expandedCompletedTurns,
      executionState,
      items,
      loading,
      messages.length,
    ],
  )
  const pendingQuestionRequestId = pendingQuestionGroup?.requestId ?? null

  useEffect(() => {
    setExpandedCompletedTurns(new Set())
  }, [sessionKey])

  const toggleCompletedTurn = useCallback((key: string) => {
    setExpandedCompletedTurns((prev) => {
      const next = new Set(prev)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }, [])

  const isTimelineMode = displayMode === "timeline"
  const compactUserAnchors = useMemo(() => {
    if (!isTimelineMode) return []
    const anchors: CompactUserAnchor[] = []
    let pendingAnchor: Omit<CompactUserAnchor, "bodyStartRowKey" | "bodyEndRowKey"> | null = null
    let pendingBodyStartRowKey: string | null = null
    let pendingBodyEndRowKey: string | null = null

    const finishPendingAnchor = () => {
      if (pendingAnchor && pendingBodyStartRowKey && pendingBodyEndRowKey) {
        anchors.push({
          ...pendingAnchor,
          bodyStartRowKey: pendingBodyStartRowKey,
          bodyEndRowKey: pendingBodyEndRowKey,
        })
      }
      pendingAnchor = null
      pendingBodyStartRowKey = null
      pendingBodyEndRowKey = null
    }

    for (const row of renderRows) {
      const rowKey = row.kind === "message" ? rowKeyForItem(row.item) : row.key
      if (row.kind !== "message") {
        if (pendingAnchor) {
          pendingBodyStartRowKey = pendingBodyStartRowKey ?? rowKey
          pendingBodyEndRowKey = rowKey
        }
        continue
      }

      const { msg } = row.item
      if (!msg.fromAgentId && !isCenteredSystemMessage(msg) && isUserAlignedMessage(msg)) {
        finishPendingAnchor()
        const text = compactAnchorTextForMessage(msg)
        if (text) {
          pendingAnchor = {
            dbId: msg.dbId,
            rowKey,
            text,
          }
        }
      } else if (pendingAnchor) {
        pendingBodyStartRowKey = pendingBodyStartRowKey ?? rowKey
        pendingBodyEndRowKey = rowKey
      }
    }

    finishPendingAnchor()
    return anchors
  }, [isTimelineMode, renderRows])

  const updateCompactUserAnchor = useCallback(() => {
    const root = rootRef.current
    const el = containerRef.current
    const content = contentRef.current
    if (root && content) {
      const rootRect = root.getBoundingClientRect()
      const contentRect = content.getBoundingClientRect()
      const nextFrame = {
        left: contentRect.left - rootRect.left,
        width: contentRect.width,
      }
      setCompactUserAnchorFrame((prev) =>
        prev &&
        Math.abs(prev.left - nextFrame.left) < 0.5 &&
        Math.abs(prev.width - nextFrame.width) < 0.5
          ? prev
          : nextFrame,
      )
    } else {
      setCompactUserAnchorFrame(null)
    }

    if (!el || compactUserAnchors.length === 0) {
      setCompactUserAnchorVisible(false)
      return
    }
    const active = findActiveCompactUserAnchor(el, compactUserAnchors)
    if (active) {
      setCompactUserAnchor((prev) =>
        prev?.rowKey === active.rowKey && prev.text === active.text ? prev : active,
      )
    }
    const visible = active != null
    setCompactUserAnchorVisible((prev) => (prev === visible ? prev : visible))
  }, [compactUserAnchors])

  useLayoutEffect(() => {
    setCompactUserAnchor(null)
    setCompactUserAnchorVisible(false)
    setCompactUserAnchorMounted(false)
  }, [sessionKey, isTimelineMode])

  useLayoutEffect(() => {
    updateCompactUserAnchor()
  }, [renderRows, updateCompactUserAnchor])

  useEffect(() => {
    if (compactUserAnchorVisible) {
      setCompactUserAnchorMounted(true)
      return
    }

    const timer = setTimeout(() => setCompactUserAnchorMounted(false), COMPACT_USER_ANCHOR_EXIT_MS)
    return () => clearTimeout(timer)
  }, [compactUserAnchorVisible])

  // Baseline for entrance animation: only messages appended *after* this
  // session was opened animate in. The initial set renders statically — no
  // distracting cascade when entering an existing conversation. Render-time
  // prop-derived state per React docs: rebase on session swap and after async
  // history hydration completes.
  const [animationBaseline, setAnimationBaseline] = useState(messages.length)
  const [animationBaselineSession, setAnimationBaselineSession] = useState(sessionKey)
  const [animationBaselineHistoryLoading, setAnimationBaselineHistoryLoading] =
    useState(historyLoading)
  if (
    animationBaselineSession !== sessionKey ||
    animationBaselineHistoryLoading !== historyLoading
  ) {
    setAnimationBaselineSession(sessionKey)
    setAnimationBaselineHistoryLoading(historyLoading)
    setAnimationBaseline(messages.length)
  }

  // Top-anchor fallback: when `items[0]` reference changes (Load More
  // prepended remote rows OR window decremented to restore local rows) and
  // scrollHeight grows while the user is not at bottom, manually compensate
  // `scrollTop` by the height delta. `overflow-anchor: auto` covers this in
  // most browsers but Safari is occasionally imprecise; this is the
  // belt-and-suspenders. Skipped at advance (window dropped top, scrollHeight
  // shrinks instead of grows) and at streaming append (items[0] unchanged).
  const prevScrollHeightRef = useRef(0)
  const prevFirstItemMsgRef = useRef<Message | null>(items[0]?.msg ?? null)
  useLayoutEffect(() => {
    const el = containerRef.current
    if (!el) return
    const oldHeight = prevScrollHeightRef.current
    const newHeight = el.scrollHeight
    const oldFirst = prevFirstItemMsgRef.current
    const newFirst = items[0]?.msg ?? null
    if (
      newFirst &&
      oldFirst &&
      newFirst !== oldFirst &&
      newHeight > oldHeight &&
      oldHeight > 0 &&
      !atBottomRef.current
    ) {
      el.scrollTop += newHeight - oldHeight
    }
    // Defensive clamp. Two failure modes covered:
    //   1. macOS WebKit/Tauri rubber-band: an upward overscroll at the top
    //      can leave scrollTop at a small negative value (e.g. -9). With
    //      `[overflow-anchor:none]` the browser doesn't auto-correct, and
    //      the viewport reads the gap as blank.
    //   2. Window advance (`displayedStart` increment) shrinks scrollHeight
    //      below the prior scrollTop on the next commit; without a clamp,
    //      scrollTop sticks above the new max until the next user scroll.
    const maxTop = Math.max(0, newHeight - el.clientHeight)
    if (el.scrollTop < 0 || el.scrollTop > maxTop) {
      el.scrollTop = Math.max(0, Math.min(el.scrollTop, maxTop))
    }
    prevScrollHeightRef.current = newHeight
    prevFirstItemMsgRef.current = newFirst
  }, [items])

  // Follow bottom: any messages change while we're tracking bottom keeps the
  // viewport pinned. Session swap re-arms atBottomRef synchronously here so
  // the same effect run scrolls to the new session's bottom — running this in
  // a separate useEffect would leave the first paint of the new session
  // tracking the *previous* session's atBottomRef value.
  const lastSessionKeyRef = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (lastSessionKeyRef.current !== sessionKey) {
      lastSessionKeyRef.current = sessionKey
      atBottomRef.current = true
      userScrollLockRef.current = false
    }
    const el = containerRef.current
    if (!el) return
    // Defensive unlock: stale lock can survive edge timing (gesture + stream
    // frame ordering) even after we've effectively returned to bottom.
    // Keeping the lock in this state disables follow-bottom permanently.
    if (atBottomRef.current && userScrollLockRef.current) {
      userScrollLockRef.current = false
    }
    if (!atBottomRef.current || userScrollLockRef.current) return
    el.scrollTop = el.scrollHeight
  }, [messages, sessionKey])

  // ask_user_question renders in the footer, not as a new message row. When
  // the card appears, `messages` may be unchanged, so the regular follow-bottom
  // effect above does not run. Force the pending interaction into view and
  // keep pinning through the collapse animation frames.
  useLayoutEffect(() => {
    if (!pendingQuestionRequestId) {
      lastAskUserFollowKeyRef.current = null
      if (askUserFollowRafRef.current !== null) {
        cancelAnimationFrame(askUserFollowRafRef.current)
        askUserFollowRafRef.current = null
      }
      return
    }

    const followKey = `${sessionKey}:${pendingQuestionRequestId}`
    if (lastAskUserFollowKeyRef.current === followKey) return
    lastAskUserFollowKeyRef.current = followKey

    const pinToBottom = () => {
      const el = containerRef.current
      if (!el) return
      el.scrollTop = el.scrollHeight
    }

    if (askUserFollowRafRef.current !== null) {
      cancelAnimationFrame(askUserFollowRafRef.current)
      askUserFollowRafRef.current = null
    }

    userScrollLockRef.current = false
    atBottomRef.current = true
    setAtBottom(true)
    pinToBottom()

    let framesLeft = ASK_USER_FOLLOW_FRAMES
    const followFrame = () => {
      pinToBottom()
      framesLeft -= 1
      if (framesLeft > 0) {
        askUserFollowRafRef.current = requestAnimationFrame(followFrame)
      } else {
        askUserFollowRafRef.current = null
      }
    }
    askUserFollowRafRef.current = requestAnimationFrame(followFrame)

    return () => {
      if (askUserFollowRafRef.current !== null) {
        cancelAnimationFrame(askUserFollowRafRef.current)
        askUserFollowRafRef.current = null
      }
    }
  }, [pendingQuestionRequestId, sessionKey])

  // Sync state to ref on session swap (state lags ref by one effect tick,
  // only affects jump-to-latest button paint).
  useEffect(() => {
    setAtBottom(true)
  }, [sessionKey])

  // ResizeObserver: re-pin to bottom whenever the layout changes while we're
  // tracking bottom. Two targets:
  //   - contentRef: content total height grows from async-rendered subtrees
  //     (markdown, shiki, katex, mermaid, images).
  //   - containerRef: scroll container height shrinks/grows when siblings
  //     (memory toast, ChatInput textarea expanding) take/return space —
  //     without this, sibling-resize hides the bottom of the conversation
  //     because the browser doesn't auto-adjust scrollTop.
  // Re-attach on sessionKey change because outer `<div key={sessionKey}>`
  // remounts both refs to fresh DOM nodes.
  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return
    const el = containerRef.current
    const content = contentRef.current
    if (!el || !content) return
    const ro = new ResizeObserver(() => {
      if (atBottomRef.current && !userScrollLockRef.current) {
        el.scrollTop = el.scrollHeight
      }
      updateCompactUserAnchor()
    })
    ro.observe(content)
    ro.observe(el)
    return () => ro.disconnect()
  }, [sessionKey, updateCompactUserAnchor])

  // Scroll listener: track atBottom + trigger load-more near top.
  // The user-intent listeners (wheel/touch/keyboard) below set
  // userScrollLockRef synchronously, before the streaming useLayoutEffect
  // could pin the viewport back to bottom — without this lock, scroll-pin
  // races with user gestures and the user can never actually scroll up.
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    let raf = 0
    const onScroll = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        const dist = el.scrollHeight - el.scrollTop - el.clientHeight
        const at = dist < AT_BOTTOM_THRESHOLD_PX
        if (at !== atBottomRef.current) {
          atBottomRef.current = at
          setAtBottom(at)
        }
        // Reaching bottom (by hand or via auto-follow) clears the user lock so
        // streaming follows again.
        if (at) userScrollLockRef.current = false

        // Windowed view advance: at bottom + DOM exceeds cap → drop top.
        // `overflow-anchor: auto` keeps the user's bottom-aligned position
        // stable when the top messages disappear.
        const totalLen = messagesRef.current.length
        const renderedCount = totalLen - displayedStartRef.current
        if (at && renderedCount > MAX_DOM_MESSAGES) {
          setDisplayedStart((prev) => Math.min(Math.max(0, totalLen - 1), prev + UNLOAD_BATCH))
        }

        // Near top: restore local older messages first; fall through to remote
        // onLoadMore only when the window is fully expanded.
        if (el.scrollTop < LOAD_MORE_THRESHOLD_PX) {
          if (displayedStartRef.current > 0) {
            setDisplayedStart((prev) => Math.max(0, prev - UNLOAD_BATCH))
          } else if (hasMore && !loadingMore) {
            void onLoadMore()
          }
        }

        // Forward twin of the load-more-on-near-top branch above: keeps
        // walking the conversation when a search-jump left the view on a
        // partial around-window (hasMoreAfter === true).
        if (
          hasMoreAfterRef.current &&
          !loadingMoreAfterRef.current &&
          dist < LOAD_MORE_THRESHOLD_PX &&
          onLoadMoreAfterRef.current
        ) {
          void onLoadMoreAfterRef.current()
        }
        updateCompactUserAnchor()
      })
    }
    const arrowKeys = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"])
    const lockOnIntent = () => {
      // Skip locking when already at bottom: scrolling down from bottom is
      // a no-op (scrollTop pinned at max), so no `scroll` event fires to
      // ever clear the lock — auto-follow then stays suspended forever and
      // the jump-to-latest button never appears (atBottom state is still
      // true). When the user actually drags up, the scroll handler flips
      // atBottomRef false on the next tick and subsequent gestures lock.
      if (atBottomRef.current) return
      userScrollLockRef.current = true
    }
    const onKey = (e: KeyboardEvent) => {
      if (arrowKeys.has(e.key)) lockOnIntent()
    }
    el.addEventListener("scroll", onScroll, { passive: true })
    el.addEventListener("wheel", lockOnIntent, { passive: true })
    el.addEventListener("touchmove", lockOnIntent, { passive: true })
    el.addEventListener("keydown", onKey)
    return () => {
      el.removeEventListener("scroll", onScroll)
      el.removeEventListener("wheel", lockOnIntent)
      el.removeEventListener("touchmove", lockOnIntent)
      el.removeEventListener("keydown", onKey)
      if (raf) cancelAnimationFrame(raf)
    }
    // `sessionKey` is part of the deps because the outer `<div key={sessionKey}>`
    // remounts the scroll container on session swap — without re-running this
    // effect, the listeners would stay bound to the old (detached) DOM node.
  }, [sessionKey, hasMore, loadingMore, onLoadMore, updateCompactUserAnchor])

  // forceFollow on lastUserKey change (user sent a new message): jump to the
  // live tail and re-arm follow-bottom so the assistant stream stays visible.
  const lastUserKey = useMemo(() => getLatestUserTurnKey(messages), [messages])
  const lastSeenUserKeyRef = useRef<string | null>(lastUserKey)
  const lastSeenUserSessionRef = useRef(sessionKey)
  useLayoutEffect(() => {
    if (lastSeenUserSessionRef.current !== sessionKey) {
      lastSeenUserSessionRef.current = sessionKey
      lastSeenUserKeyRef.current = lastUserKey
      return
    }
    if (!lastUserKey || lastUserKey === lastSeenUserKeyRef.current) return
    lastSeenUserKeyRef.current = lastUserKey

    const msgs = messagesRef.current
    let userIdx = -1
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i]
      if (m.role === "user" && !m.isMeta) {
        userIdx = i
        break
      }
    }
    if (userIdx < 0) return

    const el = containerRef.current
    if (!el) return
    // User just sent a message — they want the latest turn, not the historic
    // scroll position. Use an immediate jump so generated smooth-scroll
    // events cannot briefly mark us as "not at bottom" and disable tailing.
    // Clear any prior scroll-lock from earlier history reading.
    userScrollLockRef.current = false
    atBottomRef.current = true
    setAtBottom(true)
    el.scrollTop = el.scrollHeight
  }, [lastUserKey, sessionKey])

  // Search-result jump: scroll target dbId into view + 2s highlight pulse.
  // If the target is outside the windowed slice (`displayedStart > targetIdx`),
  // expand the window first and let the effect re-run on next render. If the
  // DOM node still hasn't materialised on this tick (markdown / shiki async
  // mount), retry on the next two animation frames before giving up — without
  // the retry the jump silently no-ops on cold renders.
  const handledScrollTargetRef = useRef<number | null>(null)
  const scrollRetryRafRef = useRef<number | null>(null)
  useEffect(() => {
    if (pendingScrollIntent == null) {
      handledScrollTargetRef.current = null
      if (scrollRetryRafRef.current != null) {
        cancelAnimationFrame(scrollRetryRafRef.current)
        scrollRetryRafRef.current = null
      }
      return
    }
    const { messageId: targetId, highlightTerms } = pendingScrollIntent
    if (handledScrollTargetRef.current === targetId) return

    if (autoCollapseCompletedTurns) {
      const collapsedTarget = renderRows.find(
        (row): row is CompletedTurnCollapseRow =>
          row.kind === "completed-turn-collapse" &&
          !row.expanded &&
          row.items.some(
            (item) =>
              item.msg.dbId === targetId ||
              (item.sourceDbId === targetId && itemContainsAnyHighlightTerm(item, highlightTerms)),
          ),
      )
      if (collapsedTarget) {
        setExpandedCompletedTurns((prev) => {
          if (prev.has(collapsedTarget.key)) return prev
          const next = new Set(prev)
          next.add(collapsedTarget.key)
          return next
        })
        return
      }
    }

    const targetIdx = messagesRef.current.findIndex((m) => m.dbId === targetId)
    if (targetIdx >= 0 && targetIdx < displayedStart) {
      setDisplayedStart(0)
      return
    }
    const targetMessage = messagesRef.current[targetIdx]
    if (
      targetMessage?.role === "user" &&
      !isCenteredSystemMessage(targetMessage) &&
      searchExpandedUserMessageId !== targetId
    ) {
      setSearchExpandedUserMessageId(targetId)
      return
    }

    const el = containerRef.current
    if (!el) return

    const tryScroll = (attemptsLeft: number): void => {
      const target = findMessageScrollTarget(el, targetId, highlightTerms)
      if (!target) {
        if (attemptsLeft > 0) {
          scrollRetryRafRef.current = requestAnimationFrame(() => tryScroll(attemptsLeft - 1))
          return
        }
        // Give up after a few frames — typically the target dbId is not in
        // the loaded window (cache vs DB drift). Surface to logs.db instead
        // of silent no-op so agent self-repair has a breadcrumb.
        logger.warn(
          "session",
          "MessageList::scrollToTarget",
          "Pending scroll target not found in DOM after retries",
          {
            sessionId,
            targetDbId: targetId,
            messagesInWindow: messagesRef.current.length - displayedStart,
          },
        )
        handledScrollTargetRef.current = targetId
        onScrollTargetHandled?.()
        return
      }
      handledScrollTargetRef.current = targetId
      target.scrollIntoView({ block: "center", behavior: preferredScrollBehavior() })
      setHighlightMessageId(targetId)
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      // Inline-highlight via CSS Custom Highlight API — doesn't mutate the
      // Streamdown / Shiki / KaTeX subtrees that re-mount on every render.
      if (highlightTerms && highlightTerms.length > 0) {
        applyInlineHighlight(target, highlightTerms)
      } else {
        clearInlineHighlight()
      }
      highlightTimerRef.current = setTimeout(() => {
        setHighlightMessageId(null)
        clearInlineHighlight()
      }, 2000)
      onScrollTargetHandled?.()
    }
    tryScroll(2)
    return () => {
      if (scrollRetryRafRef.current != null) {
        cancelAnimationFrame(scrollRetryRafRef.current)
        scrollRetryRafRef.current = null
      }
    }
  }, [
    autoCollapseCompletedTurns,
    pendingScrollIntent,
    onScrollTargetHandled,
    displayedStart,
    renderRows,
    sessionId,
    searchExpandedUserMessageId,
  ])

  useEffect(
    () => () => {
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
      // Drop any lingering inline highlight on unmount / session swap so
      // ranges from the previous bubble don't bleed into the new one.
      clearInlineHighlight()
    },
    [],
  )

  useEffect(() => {
    if (!contextMenu) return
    const close = () => setContextMenu(null)
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close()
    }
    window.addEventListener("pointerdown", close)
    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("resize", close)
    window.addEventListener("blur", close)
    window.addEventListener("scroll", close, true)
    return () => {
      window.removeEventListener("pointerdown", close)
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("resize", close)
      window.removeEventListener("blur", close)
      window.removeEventListener("scroll", close, true)
    }
  }, [contextMenu])

  const handleJumpToLatest = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    // Clear the user-intent lock so auto-follow resumes once we land at
    // bottom. atBottomRef already true here lets ResizeObserver tail any
    // height changes during the smooth scroll. Don't touch atBottom state —
    // let scroll listener flip it when the scroll actually reaches bottom,
    // otherwise the button blinks.
    userScrollLockRef.current = false
    atBottomRef.current = true
    if (hasMoreAfter && onResetToLatest) {
      // The user sits on a partial around-window from a search jump; the
      // tail of `messages` is mid-conversation, not the live tail. Reload
      // the latest page first so scrolling-to-bottom actually shows the
      // newest message. The reload swaps the `messages` array, the
      // useLayoutEffect that follows-bottom + ResizeObserver re-pin to the
      // real bottom on the next frame.
      void onResetToLatest()
      return
    }
    el.scrollTo({ top: el.scrollHeight, behavior: preferredScrollBehavior() })
  }, [hasMoreAfter, onResetToLatest])

  const handleCompactUserAnchorClick = useCallback(() => {
    const el = containerRef.current
    const rowKey = compactUserAnchor?.rowKey
    if (!el || !rowKey) return
    const target = findMessageRowByKey(el, rowKey)
    if (!target) return

    userScrollLockRef.current = true
    atBottomRef.current = false
    setAtBottom(false)
    target.scrollIntoView({ block: "start", behavior: preferredScrollBehavior() })

    if (compactUserAnchor.dbId != null) {
      if (highlightTimerRef.current) clearTimeout(highlightTimerRef.current)
      setHighlightMessageId(compactUserAnchor.dbId)
      highlightTimerRef.current = setTimeout(() => setHighlightMessageId(null), 1600)
    }
  }, [compactUserAnchor])

  const handleContextMenu = useCallback((e: React.MouseEvent, index: number) => {
    const msg = messagesRef.current[index]
    const selectionActive = hasActiveTextSelection(e.target)
    if (selectionActive) {
      if (
        (msg.role !== "user" && msg.role !== "assistant") ||
        msg.isMeta ||
        isCenteredSystemMessage(msg)
      ) {
        return
      }
      const selection = window.getSelection()
      const container = e.currentTarget instanceof HTMLElement ? e.currentTarget : null
      if (
        !selection ||
        !container ||
        !selection.anchorNode ||
        !selection.focusNode ||
        !container.contains(selection.anchorNode) ||
        !container.contains(selection.focusNode)
      ) {
        // Cross-message selections keep the native menu so exact copy remains
        // available; only a single-bubble excerpt can become a message quote.
        return
      }
      const selectedText = selection.toString()
      if (!selectedText.trim()) return
      e.preventDefault()
      setContextMenu({
        x: Math.max(8, Math.min(e.clientX, window.innerWidth - 176)),
        y: Math.max(8, Math.min(e.clientY, window.innerHeight - 92)),
        index,
        selectedText,
        quoteRole: msg.role,
      })
      return
    }

    // Preserve the existing whole-message menu for assistant prose only.
    if (msg.role !== "assistant" || !msg.content) return
    e.preventDefault()
    setContextMenu({
      x: Math.max(8, Math.min(e.clientX, window.innerWidth - 176)),
      y: Math.max(8, Math.min(e.clientY, window.innerHeight - 52)),
      index,
    })
  }, [])

  const handleCopyMessage = useCallback((content: string, index: number) => {
    navigator.clipboard
      .writeText(content)
      .then(() => {
        if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
        setCopiedIndex(index)
        copiedTimerRef.current = setTimeout(() => setCopiedIndex(null), 1500)
      })
      .catch(() => {})
  }, [])

  const planCardVisible = Boolean(
    planCardData && planState && planState !== "off" && planState !== "planning",
  )
  const showEmpty = items.length === 0
  const showHistoryLoading = showEmpty && historyLoading
  const hasFooterContent = Boolean(
    pendingQuestionGroup ||
    planCardVisible ||
    planSubagentRunning ||
    (showEmpty && !historyLoading),
  )
  // Show whenever user is scrolled away from bottom — independent of loading
  // state. Lets the user always have a one-click way back to latest.
  // Also surface it whenever a search-jump has detached the view from the
  // live tail so the user has an obvious way to re-anchor regardless of
  // scroll position.
  const showJumpToLatest = Boolean((!atBottom && items.length > 0) || hasMoreAfter)

  return (
    <div ref={rootRef} className="relative flex-1 min-h-0 min-w-0 overflow-hidden">
      <div
        ref={containerRef}
        key={sessionKey}
        // `overflow-anchor: none` opts out of the browser's default scroll-
        // anchoring. Otherwise the browser tries to keep visible elements at
        // their viewport position when content above grows (e.g. Load More
        // prepend), and the `useLayoutEffect` top-anchor below tries to do
        // the same — the result is double-compensation, which the user reads
        // as "the scroll keeps moving by itself after the load finished".
        // `overscroll-behavior-y: none` disables macOS WebKit/Tauri rubber-
        // band overscroll, which on a long Load-More'd conversation can
        // leave scrollTop sitting at a small negative value (e.g. -41) past
        // the gesture's end — a bug we observed where the negative gap +
        // async KaTeX/Mermaid/Shiki layout settling created a multi-frame
        // blank viewport even after the messages had committed to the DOM.
        className={cn(
          "h-full overflow-y-auto overflow-x-hidden px-4 [overflow-anchor:none] [overscroll-behavior-y:none]",
          isTimelineMode && "px-5 sm:px-6",
        )}
      >
        <div ref={contentRef} className={cn("mx-auto w-full pt-4", CHAT_CONTENT_MAX_WIDTH_CLASS)}>
          {hasMore && displayedStart === 0 && (
            <div className="pt-6">
              <LoadMoreRow loadingMore={loadingMore} onLoadMore={onLoadMore} />
            </div>
          )}

          {renderRows.map((row) => {
            if (row.kind === "completed-turn-collapse") {
              return (
                <CompletedTurnCollapseSummary
                  key={row.key}
                  row={row}
                  onToggle={toggleCompletedTurn}
                />
              )
            }

            const { msg, originalIndex } = row.item
            const rowKey = rowKeyForItem(row.item)
            const isLast = originalIndex === messages.length - 1
            // Only the last bubble cares about the `loading` prop (drives
            // streaming-bubble class, dots placeholder, MarkdownRenderer
            // streaming hint). Pass false to all others so global loading
            // flips don't re-render the entire list — that's the source of
            // the post-stream "flicker" (markdown / shiki / katex subtree
            // rebuilds when each bubble's loading prop changes).
            const bubbleLoading = isLast ? loading : false
            const bubbleExecutionState = shouldPassExecutionStateToBubble(
              isLast,
              bubbleLoading,
              executionState,
            )
              ? executionState
              : null
            const forceExpandUserContent =
              msg.dbId != null && searchExpandedUserMessageId === msg.dbId
            return (
              <div
                key={rowKey}
                data-message-key={rowKey}
                data-message-id={msg.dbId ?? undefined}
                data-message-source-id={row.item.sourceDbId ?? undefined}
                className={cn(
                  "grid w-full min-w-0 grid-cols-1 rounded-lg transition-colors",
                  itemMatchesMessageId(row.item, highlightMessageId) && "message-hit-pulse",
                  isTimelineMode
                    ? isCenteredSystemMessage(msg)
                      ? "justify-items-center pb-4"
                      : isUserAlignedMessage(msg) && !msg.fromAgentId
                        ? "justify-items-end pb-4"
                        : msg.role === "assistant"
                          ? "justify-items-stretch pb-0"
                          : "justify-items-start pb-4"
                    : cn(
                        "pb-4",
                        isCenteredSystemMessage(msg)
                          ? "justify-items-center"
                          : isUserAlignedMessage(msg) && !msg.fromAgentId
                            ? "justify-items-end"
                            : "justify-items-start",
                      ),
                  isLast && originalIndex >= animationBaseline && "animate-fade-slide-in",
                )}
              >
                <MessageBubble
                  msg={msg}
                  index={originalIndex}
                  isLast={isLast}
                  loading={bubbleLoading}
                  executionState={bubbleExecutionState}
                  agents={agents}
                  isHovered={hoveredMsgIndex === originalIndex}
                  onHover={setHoveredMsgIndex}
                  onContextMenu={handleContextMenu}
                  isCopied={copiedIndex === originalIndex}
                  onCopy={handleCopyMessage}
                  onAddQuickPrompt={onAddQuickPrompt}
                  sessionId={sessionId}
                  onOpenPlanPanel={onOpenPlanPanel}
                  onViewChildSession={onViewChildSession}
                  onSwitchModel={onSwitchModel}
                  onViewSystemPrompt={onViewSystemPrompt}
                  compacting={compacting}
                  onCompactContext={onCompactContext}
                  onOpenDashboardTab={onOpenDashboardTab}
                  onOpenDiff={onOpenDiff}
                  onResume={onResume}
                  onForkFromMessage={onForkFromMessage}
                  onOpenMemorySettings={onOpenMemorySettings}
                  onOpenKnowledge={onOpenKnowledge}
                  displayMode={displayMode}
                  footerFiles={row.item.footerFiles}
                  hideOwnFooterFiles={row.item.hideOwnFooterFiles}
                  goalCompletionReportOverride={row.item.goalCompletionReport}
                  suppressGoalCompletionFooter={row.item.suppressGoalCompletionFooter}
                  forceExpandUserContent={forceExpandUserContent}
                  onForceExpandedUserContentDismiss={
                    forceExpandUserContent
                      ? () =>
                          setSearchExpandedUserMessageId((current) =>
                            current === msg.dbId ? null : current,
                          )
                      : undefined
                  }
                />
                {renderMessageActions?.(msg, originalIndex)}
              </div>
            )
          })}

          {hasMoreAfter && (
            <div className="pt-2 pb-1">
              <LoadMoreRow loadingMore={loadingMoreAfter} onLoadMore={onLoadMoreAfter} />
            </div>
          )}

          <AnimatedCollapse open={hasFooterContent} durationMs={220}>
            <div className="flex flex-col gap-4 pt-2 pb-6">
              {pendingQuestionGroup && (
                <div className="w-full">
                  <AskUserQuestionBlock
                    key={pendingQuestionGroup.requestId}
                    group={pendingQuestionGroup}
                    onSubmitted={onQuestionSubmitted}
                  />
                </div>
              )}
              {planCardVisible && planCardData && (
                <div className="flex justify-start">
                  <div className="max-w-[85%] w-full">
                    <PlanCardBlock
                      data={planCardData}
                      planState={planState ?? "off"}
                      onOpenPanel={onOpenPlanPanel}
                      onApprove={onApprovePlan}
                      onExit={onExitPlan}
                    />
                  </div>
                </div>
              )}
              {planSubagentRunning && (
                <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/5 border border-blue-500/20 text-sm text-blue-600 dark:text-blue-400 animate-in fade-in slide-in-from-bottom-2 duration-300">
                  <span className="animate-spin h-3.5 w-3.5 border-2 border-current border-t-transparent rounded-full shrink-0" />
                  <span>{t("planMode.planningInProgress")}</span>
                </div>
              )}
              {showEmpty && !historyLoading && !heroComposer && (
                <div className="flex min-h-[50vh] items-center justify-center animate-in fade-in-0 duration-300">
                  <ChatWelcomeHero
                    incognito={incognito}
                    context={welcomeContext}
                    projectName={projectName}
                    onProjectSuggestion={onProjectSuggestion}
                  />
                </div>
              )}
            </div>
          </AnimatedCollapse>
        </div>
      </div>

      {showHistoryLoading && (
        <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center px-4">
          <div className="flex items-center gap-2 rounded-lg border border-border/70 bg-surface-floating px-3 py-2 text-sm text-muted-foreground shadow-sm">
            <span className="h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent" />
            <span>{t("chat.loadingSession", "正在加载会话…")}</span>
          </div>
        </div>
      )}

      {compactUserAnchor && (compactUserAnchorVisible || compactUserAnchorMounted) && (
        <div
          style={
            compactUserAnchorFrame
              ? {
                  left: compactUserAnchorFrame.left,
                  width: compactUserAnchorFrame.width,
                }
              : undefined
          }
          className={cn(
            "pointer-events-none absolute top-2 z-30 flex justify-end transition-all duration-200 ease-out",
            !compactUserAnchorFrame && "inset-x-0 px-4 sm:px-6",
            compactUserAnchorVisible
              ? "translate-y-0 opacity-100 animate-in fade-in-0 slide-in-from-top-1"
              : "-translate-y-1 opacity-0",
          )}
        >
          <button
            type="button"
            onClick={handleCompactUserAnchorClick}
            className={cn(
              "pointer-events-auto flex h-9 max-w-[min(720px,85%)] cursor-pointer items-center rounded-full border border-border-soft bg-surface-floating/95 px-3.5 text-right text-sm font-medium text-foreground shadow-panel backdrop-blur transition-colors hover:bg-surface-subtle supports-[backdrop-filter]:bg-surface-floating/85",
              !compactUserAnchorVisible && "pointer-events-none",
            )}
          >
            <span className="min-w-0 flex-1 truncate">
              <SkillMentionText text={compactUserAnchor.text} />
            </span>
          </button>
        </div>
      )}

      <AnimatedPresenceBox
        open={showJumpToLatest}
        className="pointer-events-none absolute inset-x-0 bottom-4 z-20 flex justify-center px-4"
        enterClassName="translate-y-0 scale-100 opacity-100"
        exitClassName="translate-y-2 scale-95 opacity-0 pointer-events-none"
      >
        <button
          type="button"
          onClick={handleJumpToLatest}
          className="pointer-events-auto inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-full border border-border/70 bg-background/95 text-foreground shadow-lg shadow-black/10 backdrop-blur transition-colors hover:bg-muted"
          aria-label={t("chat.scrollToBottom")}
        >
          <ArrowDown className="h-4 w-4" />
        </button>
      </AnimatedPresenceBox>

      <MessageContextMenu
        contextMenu={contextMenu}
        onCopy={(index, selectedText) => {
          const msg = messages[index]
          const content = selectedText ?? msg?.content
          if (content) handleCopyMessage(content, index)
        }}
        onAddToChat={onAddMessageQuote}
        onClose={() => setContextMenu(null)}
      />
    </div>
  )
}
