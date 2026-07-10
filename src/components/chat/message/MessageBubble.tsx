import React, { useState, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { AnimatedCollapse, AnimatedPresenceBox } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import {
  Copy,
  Check,
  Info,
  Network,
  Timer,
  AlarmClock,
  PlayCircle,
  ChevronDown,
  Code2,
  Type,
  Hash,
  Brain,
  Settings,
  Ban,
  Pencil,
  X,
} from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import {
  formatTokens,
  formatDuration,
  formatMessageTime,
  extractMessageFileAttachments,
  type MessageFileAttachment,
  isUserAlignedMessage,
} from "../chatUtils"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import PlainTextRenderer from "@/components/common/PlainTextRenderer"
import FileAttachments from "./FileAttachments"
import UserAttachments from "./UserAttachments"
import FallbackBanner from "@/components/chat/FallbackBanner"
import ProfileRotationBanner from "@/components/chat/ProfileRotationBanner"
import ContextCompactedBanner from "@/components/chat/ContextCompactedBanner"
import RoundLimitReachedBanner from "@/components/chat/RoundLimitReachedBanner"
import MessageUrlPreviews from "./MessageUrlPreviews"
import { AssistantContentBlocks } from "./MessageContent"
import { PlanCommentBubble } from "./PlanCommentBubble"
import type {
  ChatDisplayMode,
  ContentRenderMode,
  Message,
  AgentSummaryForSidebar,
  ProfileRotationEvent,
  ContextCompactedEvent,
  ContextCompactionProgressEvent,
  ChatTurnStatus,
  RoundLimitReachedEvent,
  ActiveMemoryRecall,
  UsedMemoryRef,
  RetrievalPlannerTrace,
} from "@/types/chat"
import ModelPickerCard from "@/components/chat/ModelPickerCard"
import ContextBreakdownCard from "@/components/chat/context-view/ContextBreakdownCard"
import type { CompactResult } from "@/components/chat/sessionStatus"
import RecapProgressCard from "@/components/chat/RecapProgressCard"
import SkillForkStatusCard from "@/components/chat/SkillForkStatusCard"
import {
  parseSubagentResultDetail,
  parseSubagentResultStatus,
  parseToolJobPayload,
  TOOL_JOB_AGENT_PREFIX,
  TOOL_JOB_STATUSES,
} from "./asyncResultPayload"
import { isQuickPromptEligibleUserMessage } from "../quick-prompts/messageQuickPrompts"
import {
  requestMemoryFocus,
  type MemoryFocusTarget,
} from "@/components/settings/memory-panel/memoryFocus"
import {
  requestKnowledgeFocus,
  type KnowledgeFocusTarget,
} from "@/components/knowledge/knowledgeFocus"
import {
  memoryKindLabel,
  memoryTraceErrorDescription,
  memoryLocationLabel,
  memoryMetricLabels,
  memoryOriginLabel,
  memoryReasonText,
  memoryRoleLabel,
  memorySourceLabel,
  retrievalLayerDetailParts,
  retrievalLayerLabel,
  retrievalIntentLabel,
  retrievalTraceStatusLabel,
  retrievalTraceSummary,
  retrievalTraceTitle,
  isMemoryCandidateRole,
  shouldRenderMemoryTracePanel,
} from "./memoryTraceFormat"

const USER_MESSAGE_COLLAPSE_CHARS = 900
const USER_MESSAGE_COLLAPSE_LINES = 12

function shouldCollapseUserMessage(content: string): boolean {
  if (!content) return false
  if (content.length > USER_MESSAGE_COLLAPSE_CHARS) return true
  return content.split(/\r\n|\r|\n/).length > USER_MESSAGE_COLLAPSE_LINES
}

function collapsedUserMessagePreview(content: string): string {
  const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
  const lineLimited = normalized
    .split("\n")
    .slice(0, USER_MESSAGE_COLLAPSE_LINES)
    .join("\n")
  const charLimited =
    lineLimited.length > USER_MESSAGE_COLLAPSE_CHARS
      ? lineLimited.slice(0, USER_MESSAGE_COLLAPSE_CHARS)
      : lineLimited
  const trimmed = charLimited.trimEnd()
  return trimmed ? `${trimmed}...` : "..."
}

function UserMessageContent({
  content,
  renderMode,
  fadeToClassName,
  forceExpanded = false,
  onForceExpandedDismiss,
}: {
  content: string
  renderMode: ContentRenderMode
  fadeToClassName: string
  forceExpanded?: boolean
  onForceExpandedDismiss?: () => void
}) {
  const { t } = useTranslation()
  const [expandedState, setExpandedState] = useState(() => ({ content, expanded: false }))
  const collapsible = useMemo(() => shouldCollapseUserMessage(content), [content])
  const preview = useMemo(() => collapsedUserMessagePreview(content), [content])
  const expanded =
    forceExpanded || (expandedState.content === content ? expandedState.expanded : false)

  const rendered =
    renderMode === "markdown" ? (
      <MarkdownRenderer content={content} />
    ) : (
      <PlainTextRenderer content={content} />
    )
  const handleToggle = () => {
    if (expanded) onForceExpandedDismiss?.()
    setExpandedState({ content, expanded: !expanded })
  }

  if (!collapsible) return rendered

  return (
    <div>
      <div className="relative">
        {expanded ? rendered : <PlainTextRenderer content={preview} />}
        {!expanded && (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-b from-transparent",
              fadeToClassName,
            )}
          />
        )}
      </div>
      <div className="mt-1.5 flex justify-end">
        <button
          type="button"
          aria-expanded={expanded}
          onClick={handleToggle}
          className="inline-flex items-center gap-1 rounded-md bg-background/45 px-2 py-1 text-xs font-medium text-foreground/60 transition-colors hover:bg-background/70 hover:text-foreground"
        >
          <ChevronDown
            className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-180")}
          />
          <span>
            {expanded
              ? t("chat.collapseMessage", { defaultValue: "Collapse message" })
              : t("chat.expandMessage", { defaultValue: "Expand message" })}
          </span>
        </button>
      </div>
    </div>
  )
}

export interface MessageBubbleProps {
  msg: Message
  index: number
  isLast: boolean
  loading: boolean
  executionState?: ChatTurnStatus | null
  agents: AgentSummaryForSidebar[]
  // Hover & interaction state
  isHovered: boolean
  onHover: (index: number | null) => void
  onContextMenu: (e: React.MouseEvent, index: number) => void
  // Copy
  isCopied: boolean
  onCopy: (content: string, index: number) => void
  onAddQuickPrompt?: (content: string) => void
  // Plan mode
  sessionId?: string | null
  onOpenPlanPanel?: () => void
  // Child-session preview (used by SubagentBlock's "view child session" button)
  onViewChildSession?: (sessionId: string) => void
  // Model switching
  onSwitchModel?: (providerId: string, modelId: string) => void
  // View system prompt (triggered from context breakdown card)
  onViewSystemPrompt?: () => void
  compacting?: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  // Open a dashboard tab from structured slash command cards.
  onOpenDashboardTab?: (tab: string, initialReportId?: string | null) => void
  // Open the right-side diff panel for a file change payload.
  onOpenDiff?: (
    metadata:
      | import("@/types/chat").FileChangeMetadata
      | import("@/types/chat").FileChangesMetadata,
  ) => void
  onResume?: (message: string) => void
  onOpenMemorySettings?: () => void
  onOpenKnowledge?: () => void
  displayMode?: ChatDisplayMode
  footerFiles?: MessageFileAttachment[]
  hideOwnFooterFiles?: boolean
  forceExpandUserContent?: boolean
  onForceExpandedUserContentDismiss?: () => void
}

function messageFileAttachmentKey(file: MessageFileAttachment): string {
  return file.kind === "media"
    ? `media:${file.item.localPath || file.item.url || file.item.name}`
    : `path:${file.path}`
}

function mergeMessageFileAttachments(
  ownFiles: MessageFileAttachment[],
  footerFiles: MessageFileAttachment[] | undefined,
): MessageFileAttachment[] {
  if (!footerFiles?.length) return ownFiles
  const merged = new Map<string, MessageFileAttachment>()
  for (const file of ownFiles) merged.set(messageFileAttachmentKey(file), file)
  for (const file of footerFiles) {
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
  return [...merged.values()]
}

function hasRenderableTextContent(msg: Message): boolean {
  return (
    !!msg.content || !!msg.contentBlocks?.some((block) => block.type === "text" && !!block.content)
  )
}

function getSubagentResultDisplay(
  msg: Message,
  t: TFunction,
): { name: string; status: string; statusText: string; isToolJob: boolean; detail?: string } {
  const agentId = msg.subagentResultAgentId
  const name = String(t("chat.asyncToolJobFallbackName"))
  if (agentId?.startsWith(TOOL_JOB_AGENT_PREFIX)) {
    const payload = parseToolJobPayload(msg.content)
    const status =
      payload?.status && TOOL_JOB_STATUSES.has(payload.status) ? payload.status : "completed"
    return {
      name,
      status,
      statusText: String(
        t(`chat.asyncToolJobStatuses.${status}`, {
          defaultValue: t("chat.asyncToolJobStatuses.completed"),
        }),
      ),
      isToolJob: true,
      detail: payload?.detail || String(t("tools.execPanel.noOutput")),
    }
  }

  const status = parseSubagentResultStatus(msg.content)
  return {
    name,
    status,
    statusText: String(
      t(`chat.asyncToolJobStatuses.${status}`, {
        defaultValue: t("chat.asyncToolJobStatuses.completed"),
      }),
    ),
    isToolJob: false,
    detail: parseSubagentResultDetail(msg.content),
  }
}

function getAsyncResultTone(status: string): {
  chip: string
  icon: string
  label: string
  separator: string
  detail: string
} {
  switch (status) {
    case "completed":
      return {
        chip: "bg-emerald-500/8 border-emerald-500/20 text-emerald-700 hover:bg-emerald-500/15 dark:text-emerald-400",
        icon: "text-emerald-600 dark:text-emerald-400",
        label: "text-emerald-600 dark:text-emerald-400",
        separator: "text-emerald-500/50",
        detail: "bg-emerald-500/5 border-emerald-500/15",
      }
    case "failed":
      return {
        chip: "bg-red-500/8 border-red-500/20 text-red-700 hover:bg-red-500/15 dark:text-red-400",
        icon: "text-red-600 dark:text-red-400",
        label: "text-red-600 dark:text-red-400",
        separator: "text-red-500/50",
        detail: "bg-red-500/5 border-red-500/15",
      }
    case "timed_out":
    case "cancelled":
    case "interrupted":
      return {
        chip: "bg-amber-500/8 border-amber-500/20 text-amber-700 hover:bg-amber-500/15 dark:text-amber-400",
        icon: "text-amber-600 dark:text-amber-400",
        label: "text-amber-600 dark:text-amber-400",
        separator: "text-amber-500/50",
        detail: "bg-amber-500/5 border-amber-500/15",
      }
    default:
      return {
        chip: "bg-sky-500/8 border-sky-500/20 text-sky-700 hover:bg-sky-500/15 dark:text-sky-400",
        icon: "text-sky-600 dark:text-sky-400",
        label: "text-sky-600 dark:text-sky-400",
        separator: "text-sky-500/50",
        detail: "bg-sky-500/5 border-sky-500/15",
      }
  }
}

function CronTriggerBubble({ msg, t }: { msg: Message; t: (key: string) => string }) {
  const [expanded, setExpanded] = useState(false)
  return (
    <div className="flex flex-col items-center gap-1 max-w-[80%]">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-amber-500/8 border border-amber-500/20 text-xs text-amber-400/80 hover:bg-amber-500/15 transition-colors cursor-pointer"
      >
        <Timer className="w-3 h-3 shrink-0 text-amber-500" />
        <span className="font-medium text-amber-500">
          {msg.cronJobName || t("chat.cronTrigger")}
        </span>
        <span className="text-amber-400/50">·</span>
        <span>{t("chat.cronTaskStarted")}</span>
        <svg
          className={cn(
            "w-3 h-3 shrink-0 text-amber-500/60 transition-transform duration-200",
            expanded && "rotate-180",
          )}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
      <AnimatedCollapse open={expanded}>
        <div className="w-full px-3 py-2 rounded-lg bg-amber-500/5 border border-amber-500/15 text-xs text-foreground/80 whitespace-pre-wrap break-words animate-in fade-in-0 slide-in-from-top-1 duration-150">
          {msg.content}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function WakeupTriggerBubble({ t }: { t: (key: string) => string }) {
  // Static chip only — no expand. `msg.content` is the LLM-facing `<wakeup>…
  // <note>…</note></wakeup>` scaffolding with XML-escaped entities; rendering it
  // raw would show internal tags and literal `&lt;` to the user. The chip alone
  // conveys "the agent resumed from a scheduled wakeup".
  return (
    <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-violet-500/8 border border-violet-500/20 text-xs text-violet-400/80 max-w-[80%]">
      <AlarmClock className="w-3 h-3 shrink-0 text-violet-500" />
      <span className="font-medium text-violet-500">{t("chat.wakeupTrigger")}</span>
      <span className="text-violet-400/50">·</span>
      <span>{t("chat.wakeupResumed")}</span>
    </div>
  )
}

function decodeXmlText(value: string): string {
  return value
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
}

function xmlTag(content: string, tag: string): string | null {
  const match = content.match(new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`))
  return match?.[1] ? decodeXmlText(match[1].trim()) : null
}

function parseProcessNotification(content: string): {
  processId: string | null
  status: string
  summary: string
  detail: string | null
} {
  const processId = xmlTag(content, "process-id")
  const status = xmlTag(content, "status") || "completed"
  const summary = xmlTag(content, "summary") || content
  const tail = xmlTag(content, "output-tail")
  const detail = tail ? tail : content.includes("<process-notification>") ? null : content
  return { processId, status, summary, detail }
}

function ProcessNotificationBubble({ msg, t }: { msg: Message; t: TFunction }) {
  const [expanded, setExpanded] = useState(false)
  const parsed = useMemo(() => parseProcessNotification(msg.content), [msg.content])
  const tone =
    parsed.status === "completed"
      ? "bg-emerald-500/8 border-emerald-500/20 text-emerald-700 dark:text-emerald-400"
      : "bg-red-500/8 border-red-500/20 text-red-700 dark:text-red-400"
  return (
    <div className="flex w-full max-w-[80%] flex-col items-center gap-1">
      <button
        type="button"
        aria-expanded={parsed.detail ? expanded : undefined}
        disabled={!parsed.detail}
        onClick={() => parsed.detail && setExpanded((v) => !v)}
        className={cn(
          "flex max-w-full items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs transition-colors",
          parsed.detail && "cursor-pointer",
          !parsed.detail && "disabled:cursor-default",
          tone,
        )}
      >
        <Code2 className="h-3 w-3 shrink-0" />
        <span className="font-medium">
          {t("chat.processNotification", "进程已结束")}
        </span>
        {parsed.processId && (
          <>
            <span className="opacity-50">·</span>
            <span className="font-mono">{parsed.processId}</span>
          </>
        )}
        <span className="opacity-70">·</span>
        <span>{parsed.status}</span>
        {parsed.detail && (
          <ChevronDown
            className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-180")}
          />
        )}
      </button>
      <div className="max-w-full truncate px-3 text-[11px] text-muted-foreground/75">
        {parsed.summary}
      </div>
      {parsed.detail && (
        <AnimatedCollapse open={expanded}>
          <pre className="max-h-[360px] w-full overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/40 bg-secondary/40 px-3 py-2 font-mono text-[11px] text-foreground/85">
            {parsed.detail}
          </pre>
        </AnimatedCollapse>
      )}
    </div>
  )
}

function memoryRefKey(ref: UsedMemoryRef): string {
  return [ref.origin ?? "memory", ref.role ?? "", ref.kind, ref.id].join(":")
}

function isHighlightedMemoryRef(ref: UsedMemoryRef, selected: UsedMemoryRef | null): boolean {
  return (
    !!selected &&
    (ref.origin ?? "") === (selected.origin ?? "") &&
    ref.kind === selected.kind &&
    ref.id === selected.id
  )
}

function focusTargetFromMemoryRef(ref: UsedMemoryRef): MemoryFocusTarget | null {
  if (ref.kind === "memory") {
    const id = Number(ref.id)
    return Number.isFinite(id) ? { kind: "memory", id } : null
  }
  if (ref.kind === "claim" && ref.id) {
    return { kind: "claim", id: ref.id }
  }
  if (ref.kind === "profile") {
    return { kind: "profile", id: ref.id }
  }
  if ((ref.kind === "episode" || ref.kind === "procedure") && ref.id) {
    return { kind: ref.kind, id: ref.id }
  }
  return null
}

function focusTargetFromKnowledgeRef(ref: UsedMemoryRef): KnowledgeFocusTarget | null {
  if (ref.kind !== "knowledge" || !ref.path) return null
  const [kbId] = ref.id.split(":")
  if (!kbId) return null
  return {
    kbId,
    path: ref.path,
    ...(ref.line && ref.line > 0 ? { line: ref.line } : {}),
    ...(ref.col != null ? { col: ref.col } : {}),
    ...(ref.headingPath ? { headingPath: ref.headingPath } : {}),
    ...(ref.blockId ? { blockId: ref.blockId } : {}),
  }
}

function memoryTraceMarkdown(
  refs: UsedMemoryRef[],
  retrievalPlanner: RetrievalPlannerTrace | undefined,
  memory: ActiveMemoryRecall | undefined,
  t: TFunction,
): string {
  const traceTitle = retrievalTraceTitle(refs.length, retrievalPlanner, t)
  const traceSummary =
    memory?.summary || retrievalTraceSummary(refs.length, retrievalPlanner, t)
  const lines: string[] = [
    `# ${traceTitle}`,
    "",
    traceSummary,
    "",
  ]

  if (retrievalPlanner) {
    lines.push(
      `- ${t("chat.memoryTrace.traceStatusLabel", "Status")}: ${retrievalTraceStatusLabel(
        retrievalPlanner.status,
        t,
      )}`,
      `- totalRefs=${retrievalPlanner.totalRefs}`,
      ...(retrievalPlanner.intent
        ? [
            `- ${t("chat.memoryTrace.intentLabel", "Detected task")}: ${retrievalIntentLabel(
              retrievalPlanner.intent,
              t,
            )}`,
          ]
        : []),
      ...(retrievalPlanner.rankingVersion
        ? [`- ranking=${retrievalPlanner.rankingVersion}`]
        : []),
      ...(typeof retrievalPlanner.maxTraceRefs === "number"
        ? [
            `- budget=${retrievalPlanner.maxTraceRefs}, perSource=${retrievalPlanner.maxCandidatesPerOrigin ?? "?"}`,
          ]
        : []),
      "",
    )
  }

  if (retrievalPlanner?.layers.length) {
    lines.push(`## ${t("chat.memoryTrace.whyTitle", "Why this context appeared")}`, "")
    for (const layer of retrievalPlanner.layers) {
      const parts = retrievalLayerDetailParts(layer, t)
      lines.push(`- ${retrievalLayerLabel(layer.layer, t)}: ${parts.join(" · ")}`)
    }
    lines.push("")
  }

  if (refs.length) {
    lines.push(`## ${t("chat.memoryTrace.sources", "Memory sources")}`, "")
    refs.forEach((ref, index) => {
      const labels = [
        memoryKindLabel(ref, t),
        ref.id ? `id=${ref.id}` : null,
        ref.origin ? memoryOriginLabel(ref.origin, t) : null,
        memoryRoleLabel(ref.role, t),
        memorySourceLabel(ref, t) || null,
      ].filter(Boolean)
      lines.push(`${index + 1}. ${labels.join(" · ")}`)
      lines.push(`   - ${memoryReasonText(ref, t)}`)
      const metricLabels = memoryMetricLabels(ref, t)
      if (metricLabels.length) lines.push(`   - ${metricLabels.join(" · ")}`)
      if (ref.path) lines.push(`   - path: ${ref.path}`)
      const locationLabel = memoryLocationLabel(ref)
      if (locationLabel) lines.push(`   - location: ${locationLabel}`)
      if (ref.preview) lines.push(`   - preview: ${ref.preview}`)
    })
    lines.push("")
  }

  return lines.join("\n").trimEnd()
}

type MemorySourceCorrectionState = Record<string, "saving" | "done">

interface MemoryQuickEditRecord {
  content: string
  tags?: string[]
}

interface MemoryQuickEditState {
  key: string
  id: number
  draft: string
  tags: string[]
  status: "loading" | "editing" | "saving"
}

function retrievalTraceStatusClass(status: string | undefined): string {
  switch (status) {
    case "partial":
    case "degraded":
      return "bg-destructive/10 text-destructive"
    case "disabled":
    case "no_context":
      return "bg-muted text-muted-foreground"
    default:
      return "bg-primary/8 text-primary/80"
  }
}

function ActiveMemoryTrace({
  memory,
  usedMemoryRefs,
  retrievalPlanner,
  onOpenMemorySettings,
  onOpenKnowledge,
}: {
  memory?: ActiveMemoryRecall
  usedMemoryRefs?: UsedMemoryRef[]
  retrievalPlanner?: RetrievalPlannerTrace
  onOpenMemorySettings?: () => void
  onOpenKnowledge?: () => void
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [showAllRefs, setShowAllRefs] = useState(false)
  const [traceCopied, setTraceCopied] = useState(false)
  const [pendingForgetMemoryId, setPendingForgetMemoryId] = useState<string | null>(null)
  const [claimCorrections, setClaimCorrections] = useState<MemorySourceCorrectionState>({})
  const [memoryCorrections, setMemoryCorrections] = useState<MemorySourceCorrectionState>({})
  const [quickEdit, setQuickEdit] = useState<MemoryQuickEditState | null>(null)
  const [editedMemoryPreviews, setEditedMemoryPreviews] = useState<Record<string, string>>({})
  const refs =
    usedMemoryRefs && usedMemoryRefs.length > 0
      ? usedMemoryRefs
      : memory?.candidates.map((candidate) => ({
          ...candidate,
          origin: "active_memory",
          role:
            memory.selected?.kind === candidate.kind && memory.selected.id === candidate.id
              ? "selected"
              : "candidate",
        })) ?? []
  const displayRefs = useMemo(
    () =>
      refs.map((ref) => {
        const preview = editedMemoryPreviews[memoryRefKey(ref)]
        return preview ? { ...ref, preview } : ref
      }),
    [editedMemoryPreviews, refs],
  )
  const selected =
    displayRefs.find((ref) => ref.role === "selected") ??
    displayRefs.find((ref) => ref.role === "injected") ??
    displayRefs[0] ??
    null
  const traceTitle = retrievalTraceTitle(displayRefs.length, retrievalPlanner, t)
  const traceSummary =
    memory?.summary || retrievalTraceSummary(displayRefs.length, retrievalPlanner, t)
  const traceStatusLabel = retrievalPlanner
    ? retrievalTraceStatusLabel(retrievalPlanner.status, t)
    : null
  const showHeaderStatus = !!retrievalPlanner && retrievalPlanner.status !== "used"
  const visibleRefs = showAllRefs ? displayRefs : displayRefs.slice(0, 4)
  const traceStats = useMemo(() => {
    const originCounts = new Map<string, number>()
    const roleCounts = new Map<string, number>()
    for (const ref of displayRefs) {
      const origin = ref.origin || "unknown"
      const role = ref.role || "related"
      originCounts.set(origin, (originCounts.get(origin) ?? 0) + 1)
      roleCounts.set(role, (roleCounts.get(role) ?? 0) + 1)
    }
    const candidateCount = displayRefs.filter((ref) => isMemoryCandidateRole(ref.role)).length
    return {
      origins: [...originCounts.entries()],
      injected: roleCounts.get("injected") ?? 0,
      selected: roleCounts.get("selected") ?? 0,
      candidates: candidateCount,
    }
  }, [displayRefs])

  const markClaimDoNotUse = async (ref: UsedMemoryRef) => {
    if (ref.kind !== "claim" || !ref.id || claimCorrections[ref.id]) return
    setClaimCorrections((prev) => ({ ...prev, [ref.id]: "saving" }))
    try {
      await getTransport().call("claim_forget", {
        id: ref.id,
        permanent: false,
        note:
          isMemoryCandidateRole(ref.role)
            ? "User dismissed this candidate memory source from an answer memory chip."
            : "User marked this memory source as no longer valid from an answer memory chip.",
      })
      setClaimCorrections((prev) => ({ ...prev, [ref.id]: "done" }))
      toast.success(
        isMemoryCandidateRole(ref.role)
          ? t(
              "chat.memoryTrace.claimCandidateDismissedToast",
              "This structured memory will not be suggested again.",
            )
          : t(
              "chat.memoryTrace.claimMarkedDoNotUseToast",
              "This structured memory will not be used again.",
            ),
      )
    } catch (error) {
      setClaimCorrections((prev) => {
        const next = { ...prev }
        delete next[ref.id]
        return next
      })
      const description = memoryTraceErrorDescription(error, t)
      toast.error(
        t("chat.memoryTrace.correctionFailedToast", "Couldn't update this memory source."),
        description ? { description } : undefined,
      )
    }
  }

  const requestMemoryDoNotUse = (ref: UsedMemoryRef) => {
    if (ref.kind !== "memory" || !ref.id || memoryCorrections[ref.id]) return
    const key = memoryRefKey(ref)
    if (quickEdit?.key === key && quickEdit.status === "saving") return
    setQuickEdit((current) => (current?.key === key ? null : current))
    setPendingForgetMemoryId((current) => (current === ref.id ? null : ref.id))
  }

  const markMemoryDoNotUse = async (ref: UsedMemoryRef) => {
    if (ref.kind !== "memory" || !ref.id || memoryCorrections[ref.id]) return
    const id = Number(ref.id)
    if (!Number.isFinite(id)) return
    const key = memoryRefKey(ref)
    setMemoryCorrections((prev) => ({ ...prev, [ref.id]: "saving" }))
    setQuickEdit((current) => (current?.key === key ? null : current))
    try {
      await getTransport().call("memory_delete", { id })
      setPendingForgetMemoryId((current) => (current === ref.id ? null : current))
      setMemoryCorrections((prev) => ({ ...prev, [ref.id]: "done" }))
      toast.success(t("chat.memoryTrace.memoryDeletedToast", "Memory deleted."))
    } catch (error) {
      setPendingForgetMemoryId((current) => (current === ref.id ? null : current))
      setMemoryCorrections((prev) => {
        const next = { ...prev }
        delete next[ref.id]
        return next
      })
      const description = memoryTraceErrorDescription(error, t)
      toast.error(
        t("chat.memoryTrace.correctionFailedToast", "Couldn't update this memory source."),
        description ? { description } : undefined,
      )
    }
  }

  const startMemoryQuickEdit = async (ref: UsedMemoryRef) => {
    if (ref.kind !== "memory" || !ref.id || memoryCorrections[ref.id]) return
    const id = Number(ref.id)
    if (!Number.isFinite(id)) return
    const key = memoryRefKey(ref)
    setPendingForgetMemoryId(null)
    setQuickEdit({ key, id, draft: "", tags: [], status: "loading" })
    try {
      const entry = await getTransport().call<MemoryQuickEditRecord | null>("memory_get", { id })
      if (!entry) {
        throw new Error(t("chat.memoryTrace.quickEditUnavailable", "Memory no longer exists."))
      }
      setQuickEdit({
        key,
        id,
        draft: entry.content,
        tags: Array.isArray(entry.tags) ? entry.tags : [],
        status: "editing",
      })
    } catch (error) {
      setQuickEdit((current) => (current?.key === key ? null : current))
      const description = memoryTraceErrorDescription(error, t)
      toast.error(
        t("chat.memoryTrace.quickEditLoadFailed", "Couldn't load this memory."),
        description ? { description } : undefined,
      )
    }
  }

  const saveMemoryQuickEdit = async () => {
    const current = quickEdit
    if (!current || current.status === "loading" || current.status === "saving") return
    const content = current.draft.trim()
    if (!content) {
      toast.error(t("chat.memoryTrace.quickEditEmpty", "Memory content cannot be empty."))
      return
    }
    setQuickEdit({ ...current, status: "saving" })
    try {
      await getTransport().call("memory_update", {
        id: current.id,
        content,
        tags: current.tags,
      })
      setEditedMemoryPreviews((prev) => ({ ...prev, [current.key]: content }))
      setQuickEdit(null)
      toast.success(t("chat.memoryTrace.quickEditSaved", "Memory updated."))
    } catch (error) {
      setQuickEdit({ ...current, status: "editing" })
      const description = memoryTraceErrorDescription(error, t)
      toast.error(
        t("chat.memoryTrace.quickEditSaveFailed", "Couldn't save this memory."),
        description ? { description } : undefined,
      )
    }
  }

  const copyTrace = async () => {
    try {
      await navigator.clipboard.writeText(
        memoryTraceMarkdown(displayRefs, retrievalPlanner, memory, t),
      )
      setTraceCopied(true)
      window.setTimeout(() => setTraceCopied(false), 1600)
    } catch (error) {
      const description = memoryTraceErrorDescription(error, t)
      toast.error(
        t("chat.memoryTrace.copyFailed", "Failed to copy memory diagnostics"),
        description ? { description } : undefined,
      )
    }
  }

  return (
    <div className="mt-2 rounded-lg border border-primary/15 bg-primary/6 text-xs">
      <button
        type="button"
        className="flex w-full min-w-0 items-center gap-2 px-2.5 py-1.5 text-left text-primary transition-colors hover:bg-primary/8"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
      >
        <Brain className="h-3.5 w-3.5 shrink-0" />
        <span className="shrink-0 font-medium">{traceTitle}</span>
        {selected && (
          <span className="min-w-0 truncate text-primary/75">
            {memorySourceLabel(selected, t)}
          </span>
        )}
        {showHeaderStatus && traceStatusLabel && (
          <span
            className={cn(
              "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium",
              retrievalTraceStatusClass(retrievalPlanner.status),
            )}
          >
            {traceStatusLabel}
          </span>
        )}
        <ChevronDown
          className={cn(
            "ml-auto h-3.5 w-3.5 shrink-0 transition-transform",
            expanded && "rotate-180",
          )}
        />
      </button>
      <AnimatedCollapse open={expanded}>
        <div className="space-y-2 border-t border-primary/10 px-2.5 py-2 text-foreground/80">
          <p className="m-0 leading-relaxed">{traceSummary}</p>
          <div className="rounded-md border border-primary/10 bg-background/45 px-2 py-1.5">
            <div className="flex items-center gap-1.5 text-[11px] font-medium text-foreground/75">
              <Info className="h-3.5 w-3.5 text-primary/70" />
              {t("chat.memoryTrace.whyTitle", "Why this context appeared")}
            </div>
            <div className="mt-1.5 flex flex-wrap gap-1.5 text-[10px] text-muted-foreground">
              {retrievalPlanner && traceStatusLabel && (
                <span
                  className={cn(
                    "rounded px-1.5 py-0.5 font-medium",
                    retrievalTraceStatusClass(retrievalPlanner.status),
                  )}
                >
                  {t("chat.memoryTrace.traceStatusLabel", "Status")}: {traceStatusLabel}
                </span>
              )}
              {retrievalPlanner?.intent && (
                <span className="rounded bg-primary/8 px-1.5 py-0.5 text-primary/80">
                  {t("chat.memoryTrace.intentLabel", "Detected task")}: {" "}
                  {retrievalIntentLabel(retrievalPlanner.intent, t)}
                </span>
              )}
              {traceStats.injected > 0 && (
                <span className="rounded bg-primary/8 px-1.5 py-0.5 text-primary/80">
                  {t("chat.memoryTrace.count.injected", {
                    count: traceStats.injected,
                    defaultValue: "{{count}} injected",
                  })}
                </span>
              )}
              {traceStats.selected > 0 && (
                <span className="rounded bg-primary/8 px-1.5 py-0.5 text-primary/80">
                  {t("chat.memoryTrace.count.selected", {
                    count: traceStats.selected,
                    defaultValue: "{{count}} selected",
                  })}
                </span>
              )}
              {traceStats.candidates > 0 && (
                <span className="rounded bg-muted px-1.5 py-0.5">
                  {t("chat.memoryTrace.count.candidate", {
                    count: traceStats.candidates,
                    defaultValue: "{{count}} candidates",
                  })}
                </span>
              )}
              {traceStats.origins.map(([origin, count]) => (
                <span key={origin} className="rounded bg-muted px-1.5 py-0.5">
                  {memoryOriginLabel(origin, t)} · {count}
                </span>
              ))}
            </div>
            {retrievalPlanner?.layers.length ? (
              <div className="mt-1.5 flex flex-wrap gap-1.5 text-[10px] text-muted-foreground">
                {retrievalPlanner.layers.map((layer) => {
                  const parts = retrievalLayerDetailParts(layer, t)
                  return (
                    <span key={layer.layer} className="rounded bg-background/70 px-1.5 py-0.5">
                      {retrievalLayerLabel(layer.layer, t)}:{" "}
                      {parts.join(" · ")}
                    </span>
                  )
                })}
              </div>
            ) : null}
          </div>
          {visibleRefs.length > 0 && (
            <div className="space-y-1.5">
              {visibleRefs.map((candidate) => {
                const refKey = memoryRefKey(candidate)
                const focusTarget = focusTargetFromMemoryRef(candidate)
                const knowledgeTarget = focusTargetFromKnowledgeRef(candidate)
                const roleLabel = memoryRoleLabel(candidate.role, t)
                const originLabel = memoryOriginLabel(candidate.origin, t)
                const metricLabels = memoryMetricLabels(candidate, t)
                const locationLabel = memoryLocationLabel(candidate)
                const correctionState =
                  candidate.kind === "claim"
                    ? claimCorrections[candidate.id]
                    : candidate.kind === "memory"
                      ? memoryCorrections[candidate.id]
                      : undefined
                const isPendingMemoryForget =
                  candidate.kind === "memory" &&
                  pendingForgetMemoryId === candidate.id &&
                  !correctionState
                const isCandidateRef = isMemoryCandidateRole(candidate.role)
                const memoryDismissLabel = isCandidateRef
                  ? t("chat.memoryTrace.doNotSuggestCandidate", "Do not suggest this candidate")
                  : t("chat.memoryTrace.doNotUse", "Do not use this memory")
                const memoryDismissDoneLabel = isCandidateRef
                  ? t("chat.memoryTrace.markedCandidateDoNotSuggest", "Marked as no longer suggested")
                  : t("chat.memoryTrace.markedDoNotUse", "Marked as no longer used")
                const confirmMemoryDismissLabel = isCandidateRef
                  ? t(
                      "chat.memoryTrace.confirmForgetCandidateAction",
                      "Confirm not suggesting this candidate",
                    )
                  : t("chat.memoryTrace.confirmForgetMemoryAction", "Confirm delete this memory")
                const opensMemoryCenter = !!focusTarget && !!onOpenMemorySettings
                const canEditMemoryRef =
                  opensMemoryCenter && (candidate.kind === "memory" || candidate.kind === "claim")
                const sourceActionLabel = canEditMemoryRef
                  ? t("chat.memoryTrace.editMemory", "Edit this memory")
                  : t("chat.memoryTrace.openSource", "Open source")
                const SourceActionIcon = canEditMemoryRef ? Pencil : Settings
                const quickEditActive = quickEdit?.key === refKey
                const quickEditSaving = quickEditActive && quickEdit?.status === "saving"
                const previewText = editedMemoryPreviews[refKey] ?? candidate.preview
                return (
                  <div
                    key={refKey}
                    className={cn(
                      "rounded-md border px-2 py-1.5",
                      isHighlightedMemoryRef(candidate, selected)
                        ? "border-primary/20 bg-primary/8"
                        : "border-border/60 bg-background/40",
                    )}
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase text-muted-foreground">
                        {memoryKindLabel(candidate, t)}
                      </span>
                      {roleLabel && (
                        <span className="shrink-0 rounded bg-primary/8 px-1.5 py-0.5 text-[10px] text-primary/80">
                          {roleLabel}
                        </span>
                      )}
                      <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                        {memorySourceLabel(candidate, t)}
                      </span>
                      {((focusTarget && onOpenMemorySettings) ||
                        (knowledgeTarget && onOpenKnowledge)) && (
                        <IconTip label={sourceActionLabel}>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation()
                              if (knowledgeTarget && onOpenKnowledge) {
                                requestKnowledgeFocus(knowledgeTarget)
                                onOpenKnowledge()
                                return
                              }
                              if (focusTarget && onOpenMemorySettings) {
                                requestMemoryFocus(focusTarget)
                                onOpenMemorySettings()
                              }
                            }}
                            className="shrink-0 rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground"
                            aria-label={sourceActionLabel}
                          >
                            <SourceActionIcon className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      )}
                      {candidate.kind === "memory" && (
                        <IconTip label={t("chat.memoryTrace.quickEdit", "Quick edit memory")}>
                          <button
                            type="button"
                            disabled={!!correctionState || quickEdit?.status === "saving"}
                            onClick={(e) => {
                              e.stopPropagation()
                              if (quickEditActive) {
                                setQuickEdit(null)
                              } else {
                                void startMemoryQuickEdit(candidate)
                              }
                            }}
                            className={cn(
                              "shrink-0 rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-background/70 hover:text-foreground disabled:pointer-events-none disabled:opacity-60",
                              quickEditActive && "bg-primary/8 text-primary",
                            )}
                            aria-label={t("chat.memoryTrace.quickEdit", "Quick edit memory")}
                          >
                            <Type className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      )}
                      {(candidate.kind === "claim" || candidate.kind === "memory") && (
                        <IconTip
                          label={
                            correctionState === "done"
                              ? memoryDismissDoneLabel
                              : correctionState === "saving"
                                ? t("common.loading")
                                : isPendingMemoryForget
                                  ? confirmMemoryDismissLabel
                                  : memoryDismissLabel
                          }
                        >
                          <button
                            type="button"
                            disabled={!!correctionState || quickEditSaving}
                            onClick={(e) => {
                              e.stopPropagation()
                              if (candidate.kind === "claim") {
                                void markClaimDoNotUse(candidate)
                              } else {
                                requestMemoryDoNotUse(candidate)
                              }
                            }}
                            className={cn(
                              "shrink-0 rounded p-0.5 text-muted-foreground/70 transition-colors hover:bg-destructive/10 hover:text-destructive disabled:pointer-events-none",
                              isPendingMemoryForget && "bg-destructive/10 text-destructive",
                              correctionState === "done" && "text-destructive/70",
                            )}
                            aria-label={memoryDismissLabel}
                          >
                            <Ban className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      )}
                    </div>
                    <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] text-muted-foreground">
                      <span>{originLabel}</span>
                      <span>{memoryReasonText(candidate, t)}</span>
                      {metricLabels.map((label) => (
                        <span key={label} className="font-mono">
                          {label}
                        </span>
                      ))}
                      {locationLabel && <span className="font-mono">{locationLabel}</span>}
                    </div>
                    {previewText && (
                      <div className="mt-1 line-clamp-2 leading-relaxed text-foreground/75">
                        {previewText}
                      </div>
                    )}
                    {quickEditActive && (
                      <div className="mt-1.5 rounded-md border border-primary/15 bg-background/70 p-2">
                        {quickEdit.status === "loading" ? (
                          <div className="text-[10px] text-muted-foreground">
                            {t("chat.memoryTrace.quickEditLoading", "Loading memory...")}
                          </div>
                        ) : (
                          <div className="space-y-1.5">
                            <textarea
                              value={quickEdit.draft}
                              disabled={quickEdit.status === "saving"}
                              onChange={(event) =>
                                setQuickEdit((current) =>
                                  current?.key === refKey
                                    ? { ...current, draft: event.target.value }
                                    : current,
                                )
                              }
                              className="min-h-20 w-full resize-y rounded-md border border-border/70 bg-background px-2 py-1.5 text-[11px] leading-relaxed text-foreground outline-none focus:border-primary/40 disabled:opacity-70"
                              placeholder={t(
                                "chat.memoryTrace.quickEditPlaceholder",
                                "Rewrite this memory...",
                              )}
                            />
                            <div className="flex justify-end gap-1.5">
                              <button
                                type="button"
                                disabled={quickEdit.status === "saving"}
                                onClick={() => setQuickEdit(null)}
                                className="inline-flex h-6 items-center gap-1 rounded border border-border/70 bg-background/80 px-1.5 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:pointer-events-none disabled:opacity-60"
                              >
                                <X className="h-3 w-3" />
                                {t("common.cancel")}
                              </button>
                              <button
                                type="button"
                                disabled={quickEdit.status === "saving"}
                                onClick={() => void saveMemoryQuickEdit()}
                                className="inline-flex h-6 items-center gap-1 rounded bg-primary px-1.5 text-[10px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-70"
                              >
                                <Check className="h-3 w-3" />
                                {quickEdit.status === "saving" ? t("common.loading") : t("common.save")}
                              </button>
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                    {isPendingMemoryForget && (
                      <div className="mt-1.5 flex flex-wrap items-center gap-1.5 rounded-md border border-destructive/20 bg-destructive/5 px-2 py-1.5 text-[10px] text-destructive">
                        <span className="min-w-[160px] flex-1">
                          {isCandidateRef
                            ? t(
                                "chat.memoryTrace.confirmForgetCandidateInline",
                                "这会删除这条长期记忆，让它之后不再作为候选出现，并留下审计记录。",
                              )
                            : t(
                                "chat.memoryTrace.confirmForgetMemoryInline",
                                "这会删除这条长期记忆，并留下审计记录。",
                              )}
                        </span>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation()
                            setPendingForgetMemoryId(null)
                          }}
                          className="inline-flex h-6 items-center gap-1 rounded border border-border/70 bg-background/80 px-1.5 font-medium text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                        >
                          <X className="h-3 w-3" />
                          {t("common.cancel")}
                        </button>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation()
                            void markMemoryDoNotUse(candidate)
                          }}
                          className="inline-flex h-6 items-center gap-1 rounded bg-destructive px-1.5 font-medium text-destructive-foreground transition-colors hover:bg-destructive/90"
                        >
                          <Check className="h-3 w-3" />
                          {t("common.delete")}
                        </button>
                      </div>
                    )}
                  </div>
                )
              })}
              {displayRefs.length > 4 && (
                <button
                  type="button"
                  onClick={() => setShowAllRefs((v) => !v)}
                  className="rounded px-1 py-0.5 text-left text-[10px] font-medium text-muted-foreground transition-colors hover:bg-background/60 hover:text-foreground"
                >
                  {showAllRefs
                    ? t("common.showLess", "折叠显示")
                    : t("chat.memoryTrace.more", {
                        count: displayRefs.length - visibleRefs.length,
                        defaultValue: "还有 {{count}} 条长期上下文未展开显示。",
                      })}
                </button>
              )}
            </div>
          )}
          <div className="flex flex-wrap gap-1.5">
            <button
              type="button"
              onClick={() => void copyTrace()}
              className="inline-flex items-center gap-1.5 rounded-md border border-border/60 bg-background/70 px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
            >
              {traceCopied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
              {traceCopied
                ? t("common.copied", "Copied")
                : t("chat.memoryTrace.copyTrace", "Copy diagnostics")}
            </button>
            {onOpenMemorySettings && (
              <button
                type="button"
                onClick={onOpenMemorySettings}
                className="inline-flex items-center gap-1.5 rounded-md border border-border/60 bg-background/70 px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground"
              >
                <Settings className="h-3.5 w-3.5" />
                {t("settings.memoryTabs.manage")}
              </button>
            )}
          </div>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function MessageBubbleInner({
  msg,
  index,
  isLast,
  loading,
  executionState,
  agents,
  isHovered,
  onHover,
  onContextMenu,
  isCopied,
  onCopy,
  onAddQuickPrompt,
  sessionId,
  onOpenPlanPanel,
  onViewChildSession,
  onSwitchModel,
  onViewSystemPrompt,
  compacting,
  onCompactContext,
  onOpenDashboardTab,
  onOpenDiff,
  onResume,
  onOpenMemorySettings,
  onOpenKnowledge,
  displayMode = "bubble",
  footerFiles,
  hideOwnFooterFiles = false,
  forceExpandUserContent = false,
  onForceExpandedUserContentDismiss,
}: MessageBubbleProps) {
  const { t } = useTranslation()
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)
  const [resultExpanded, setResultExpanded] = useState(false)
  const [contentRenderMode, setContentRenderMode] = useState<ContentRenderMode>("markdown")

  const ownMessageFiles = useMemo(
    () =>
      !hideOwnFooterFiles && msg.role === "assistant" && msg.contentBlocks
        ? extractMessageFileAttachments(msg.contentBlocks)
        : [],
    [hideOwnFooterFiles, msg.role, msg.contentBlocks],
  )
  const messageFiles = useMemo(
    () => mergeMessageFileAttachments(ownMessageFiles, footerFiles),
    [footerFiles, ownMessageFiles],
  )

  const fromAgent = msg.fromAgentId ? agents.find((a) => a.id === msg.fromAgentId) : undefined
  const eventPayload = useMemo(() => {
    if (msg.role !== "event") return null
    try {
      return JSON.parse(msg.content) as Record<string, unknown>
    } catch {
      return null
    }
  }, [msg.content, msg.role])
  const isUserAligned = isUserAlignedMessage(msg)
  const userMessageFadeToClassName =
    isUserAligned && !msg.fromAgentId
      ? "to-[var(--color-user-bubble)]"
      : msg.fromAgentId
        ? "to-purple-500/10"
        : "to-card"
  const hasTextContent = hasRenderableTextContent(msg)
  const hasDetails = msg.role === "assistant" && !!(msg.usage || msg.model)
  const canAddQuickPrompt = !!onAddQuickPrompt && isQuickPromptEligibleUserMessage(msg)
  const hasToolbarActions = hasTextContent || hasDetails || canAddQuickPrompt
  // Always-visible total turn duration, shown at the message bottom once the
  // assistant turn has finished (the per-step / per-group times live above).
  const totalDurationText =
    msg.role === "assistant" && !(loading && isLast) && msg.usage?.durationMs != null
      ? formatDuration(msg.usage.durationMs)
      : null
  const memoryTraceRefCount =
    msg.usedMemoryRefs?.length ?? msg.activeMemory?.candidates.length ?? 0
  const shouldShowMemoryTrace = shouldRenderMemoryTracePanel(
    memoryTraceRefCount,
    msg.retrievalPlanner,
  )
  const toolbarButtonClass =
    "flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors"
  const renderToggleLabel =
    contentRenderMode === "markdown"
      ? String(t("chat.showPlainText", { defaultValue: "Show plain text" }))
      : String(t("chat.renderMarkdown", { defaultValue: "Render Markdown" }))
  const renderToggleButton = hasTextContent ? (
    <IconTip label={renderToggleLabel}>
      <button
        type="button"
        aria-label={renderToggleLabel}
        aria-pressed={contentRenderMode === "markdown"}
        onClick={() =>
          setContentRenderMode((mode) => (mode === "markdown" ? "text" : "markdown"))
        }
        className={toolbarButtonClass}
      >
        {contentRenderMode === "markdown" ? (
          <Type className="h-3.5 w-3.5" />
        ) : (
          <Code2 className="h-3.5 w-3.5" />
        )}
      </button>
    </IconTip>
  ) : null
  const addQuickPromptButton = canAddQuickPrompt ? (
    <IconTip label={t("chat.quickPrompts.addAction")}>
      <button
        type="button"
        onClick={() => onAddQuickPrompt?.(msg.content)}
        className={toolbarButtonClass}
      >
        <Hash className="h-3.5 w-3.5" />
      </button>
    </IconTip>
  ) : null
  const detailsButton = hasDetails ? (
    <div className="relative flex h-6 w-6 items-center justify-center">
      <IconTip label={t("chat.details")}>
        <button
          type="button"
          onClick={() => setDetailsIndex(detailsIndex === index ? null : index)}
          className={cn(
            toolbarButtonClass,
            detailsIndex === index && "text-foreground bg-muted/80",
          )}
        >
          <Info className="h-3.5 w-3.5" />
        </button>
      </IconTip>
      <AnimatedPresenceBox
        open={detailsIndex === index}
        className="absolute bottom-full left-0 z-50 mb-1 w-64 max-w-[calc(100vw-2rem)] origin-bottom-left rounded-lg border border-border bg-popover p-2.5 shadow-lg"
        enterClassName="translate-y-0 scale-100 opacity-100"
        exitClassName="translate-y-1 scale-[0.98] opacity-0 pointer-events-none"
      >
        <div className="space-y-1.5 text-xs">
          {msg.model && (
            <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
              <span className="text-muted-foreground whitespace-nowrap shrink-0">
                {t("chat.statusModel")}
              </span>
              <IconTip label={msg.model}>
                <span className="block truncate text-right font-medium text-foreground">
                  {msg.model}
                </span>
              </IconTip>
            </div>
          )}
          {msg.model && msg.usage?.inputTokens != null && (
            <div className="border-t border-border" />
          )}
          {(() => {
            const inputTokens = msg.usage?.inputTokens
            const lastInputTokens = msg.usage?.lastInputTokens
            const showLastInput =
              inputTokens != null &&
              lastInputTokens != null &&
              lastInputTokens !== inputTokens
            if (inputTokens == null) return null
            return (
              <>
                <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
                  <span className="text-muted-foreground whitespace-nowrap shrink-0">
                    {showLastInput
                      ? t("chat.inputTokensCumulative")
                      : t("chat.inputTokens")}
                  </span>
                  <span className="justify-self-end whitespace-nowrap text-right font-medium text-foreground tabular-nums">
                    {formatTokens(inputTokens)}
                  </span>
                </div>
                {showLastInput && lastInputTokens != null && (
                  <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
                    <span className="text-muted-foreground whitespace-nowrap shrink-0">
                      {t("chat.lastRoundInputTokens")}
                    </span>
                    <span className="justify-self-end whitespace-nowrap text-right font-medium text-foreground tabular-nums">
                      {formatTokens(lastInputTokens)}
                    </span>
                  </div>
                )}
              </>
            )
          })()}
          {msg.usage?.outputTokens != null && (
            <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
              <span className="text-muted-foreground whitespace-nowrap shrink-0">
                {t("chat.outputTokens")}
              </span>
              <span className="justify-self-end whitespace-nowrap text-right font-medium text-foreground tabular-nums">
                {formatTokens(msg.usage.outputTokens)}
              </span>
            </div>
          )}
          {msg.usage?.inputTokens != null && msg.usage?.outputTokens != null && (
            <>
              <div className="border-t border-border" />
              <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
                <span className="text-muted-foreground whitespace-nowrap shrink-0">
                  {t("chat.totalTokens")}
                </span>
                <span className="justify-self-end whitespace-nowrap text-right font-medium text-foreground tabular-nums">
                  {formatTokens(msg.usage.inputTokens + msg.usage.outputTokens)}
                </span>
              </div>
            </>
          )}
          {msg.usage?.durationMs != null && (
            <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-3">
              <span className="text-muted-foreground whitespace-nowrap shrink-0">
                {t("chat.duration")}
              </span>
              <span className="justify-self-end whitespace-nowrap text-right font-medium text-foreground tabular-nums">
                {formatDuration(msg.usage.durationMs)}
              </span>
            </div>
          )}
        </div>
      </AnimatedPresenceBox>
    </div>
  ) : null

  if (msg.role === "event" && !isUserAligned) {
    // Interactive model picker card
    if (msg.modelPickerData) {
      return (
        <ModelPickerCard
          data={msg.modelPickerData}
          onSelect={(providerId, modelId) => onSwitchModel?.(providerId, modelId)}
        />
      )
    }
    // Context window breakdown card
    if (msg.contextBreakdownData) {
      return (
        <ContextBreakdownCard
          data={msg.contextBreakdownData}
          sessionId={sessionId}
          compacting={compacting}
          onCompactContext={onCompactContext}
          onViewSystemPrompt={onViewSystemPrompt}
        />
      )
    }
    if (msg.recapCardData) {
      return (
        <RecapProgressCard
          reportId={msg.recapCardData.reportId}
          onOpenDashboardTab={onOpenDashboardTab}
        />
      )
    }
    if (msg.skillForkData) {
      return (
        <SkillForkStatusCard
          runId={msg.skillForkData.runId}
          skillName={msg.skillForkData.skillName}
          onViewChildSession={onViewChildSession}
        />
      )
    }
    if (eventPayload?.type === "thinking_auto_disabled") {
      return (
        <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center">
          {t("chat.thinkingAutoDisabled", {
            provider: String(eventPayload.provider_name || t("chat.unknownProvider")),
            model: String(eventPayload.model_id || ""),
          })}
        </div>
      )
    }
    if (eventPayload?.type === "vision_auto_disabled") {
      return (
        <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center">
          {t("chat.visionAutoDisabled", {
            provider: String(eventPayload.provider_name || t("chat.unknownProvider")),
            model: String(eventPayload.model_id || ""),
          })}
        </div>
      )
    }
    if (eventPayload?.type === "vision_bridge") {
      return (
        <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center">
          {eventPayload.status === "unavailable"
            ? t("chat.visionBridgeUnavailable")
            : t("chat.visionBridgeEngaged", {
                model: String(eventPayload.model_id || ""),
              })}
        </div>
      )
    }
    if (eventPayload?.type === "profile_rotation") {
      return <ProfileRotationBanner event={eventPayload as ProfileRotationEvent} />
    }
    if (
      eventPayload?.type === "context_compacted" ||
      eventPayload?.type === "context_compaction_progress"
    ) {
      const data = (eventPayload.data ?? eventPayload) as ContextCompactedEvent
      return (
        <ContextCompactedBanner
          event={data as ContextCompactedEvent & ContextCompactionProgressEvent}
        />
      )
    }
    if (eventPayload?.type === "round_limit_reached") {
      return (
        <RoundLimitReachedBanner
          event={eventPayload as RoundLimitReachedEvent}
          onResume={onResume}
        />
      )
    }
    return (
      <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center [&_p]:m-0">
        <MarkdownRenderer content={msg.content} />
      </div>
    )
  }

  if (msg.isSubagentResult) {
    const resultDisplay = getSubagentResultDisplay(msg, t)
    const hasDetail = !!resultDisplay.detail
    const resultTone = getAsyncResultTone(resultDisplay.status)
    return (
      <div className="flex flex-col items-center gap-1 w-full max-w-[80%]">
        <button
          type="button"
          disabled={!hasDetail}
          aria-expanded={hasDetail ? resultExpanded : undefined}
          aria-label={hasDetail ? t("chat.details") : undefined}
          onClick={() => {
            if (hasDetail) setResultExpanded((v) => !v)
          }}
          className={cn(
            "flex flex-wrap items-center gap-1.5 max-w-full px-3 py-1.5 rounded-full border text-xs transition-colors",
            hasDetail && "cursor-pointer",
            resultTone.chip,
            !hasDetail && "disabled:cursor-default",
          )}
        >
          <Timer className={cn("w-3 h-3 shrink-0", resultTone.icon)} />
          <span className={cn("font-medium", resultTone.label)}>
            {resultDisplay.name}
          </span>
          <span className={resultTone.separator}>
            ·
          </span>
          <span>{resultDisplay.statusText}</span>
          {hasDetail && (
            <ChevronDown
              className={cn(
                "w-3 h-3 shrink-0 transition-transform duration-200",
                resultExpanded && "rotate-180",
                resultTone.icon,
              )}
            />
          )}
        </button>
        {hasDetail && (
          <AnimatedCollapse open={resultExpanded}>
            <div
              className={cn(
                "w-full max-h-[360px] overflow-auto px-3 py-2 rounded-lg border text-xs text-foreground/85 whitespace-pre-wrap break-words animate-in fade-in-0 slide-in-from-top-1 duration-150",
                resultDisplay.isToolJob
                  ? cn(resultTone.detail, "font-mono text-[11px]")
                  : "bg-purple-500/5 border-purple-500/15",
              )}
            >
              {resultDisplay.detail}
            </div>
          </AnimatedCollapse>
        )}
      </div>
    )
  }

  if (msg.isCronTrigger) {
    return <CronTriggerBubble msg={msg} t={t} />
  }

  if (msg.isWakeupTrigger) {
    return <WakeupTriggerBubble t={t} />
  }

  if (msg.isProcessNotification) {
    return <ProcessNotificationBubble msg={msg} t={t} />
  }

  if (msg.isPlanTrigger) {
    return (
      <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-sky-500/8 border border-sky-500/20 text-xs text-sky-700 dark:text-sky-400 max-w-[80%]">
        <PlayCircle className="w-3 h-3 shrink-0 text-sky-600 dark:text-sky-400" />
        <span>{msg.content}</span>
      </div>
    )
  }

  // Plan inline-comment user message — bespoke layered card (header chip /
  // quoted selection / comment body) instead of the generic user bubble.
  // The markdown displayText still lives in `msg.content` as a fallback for
  // IM channels and historical sessions where the metadata wasn't captured.
  if (msg.planComment) {
    return (
      <PlanCommentBubble
        selectedText={msg.planComment.selectedText}
        comment={msg.planComment.comment}
      />
    )
  }

  if (displayMode === "timeline" && msg.role === "assistant") {
    return (
      <div
        className={cn("relative w-full max-w-4xl", msg.fromAgentId && "flex items-start gap-2")}
        onMouseEnter={() => onHover(index)}
        onMouseLeave={() => {
          onHover(null)
          setDetailsIndex((prev) => (prev === index ? null : prev))
        }}
        onContextMenu={(e) => onContextMenu(e, index)}
      >
        {msg.fromAgentId && (
          <div className="w-6 h-6 rounded-full bg-purple-500/15 flex items-center justify-center text-purple-500 shrink-0 mt-1 text-[10px] overflow-hidden">
            {fromAgent?.avatar ? (
              <img
                src={getTransport().resolveAssetUrl(fromAgent.avatar) ?? fromAgent.avatar}
                className="w-full h-full object-cover"
                alt=""
              />
            ) : fromAgent?.emoji ? (
              <span>{fromAgent.emoji}</span>
            ) : (
              <Network className="w-3 h-3" />
            )}
          </div>
        )}
        <div className="min-w-0 flex-1">
          {msg.fromAgentId && (
            <div className="mb-0.5 text-[10px] font-medium text-purple-500">
              {fromAgent?.name || msg.fromAgentId}
            </div>
          )}
          {msg.channelInbound && (
            <div className="mb-0.5 flex items-center gap-1 text-[10px] font-medium text-blue-500">
              <ChannelIcon channelId={msg.channelInbound.channelId} className="w-2.5 h-2.5" />
              <span>{msg.channelInbound.channelId}</span>
              {msg.channelInbound.senderName && (
                <span className="text-blue-400">· {msg.channelInbound.senderName}</span>
              )}
            </div>
          )}
          {msg.fallbackEvent && (
            <div className="mb-1">
              <FallbackBanner event={msg.fallbackEvent} />
            </div>
          )}
          <AssistantContentBlocks
            msg={msg}
            loading={loading}
            isLast={isLast}
            executionState={executionState}
            sessionId={sessionId}
            onOpenPlanPanel={onOpenPlanPanel}
            onViewChildSession={onViewChildSession}
            onOpenDiff={onOpenDiff}
            displayMode="timeline"
            contentRenderMode={contentRenderMode}
          />
          {messageFiles.length > 0 && (
            <div className="ml-7">
              <FileAttachments files={messageFiles} sessionId={sessionId} />
            </div>
          )}
          {shouldShowMemoryTrace && (
            <div className="ml-7">
              <ActiveMemoryTrace
                memory={msg.activeMemory}
                usedMemoryRefs={msg.usedMemoryRefs}
                retrievalPlanner={msg.retrievalPlanner}
                onOpenMemorySettings={onOpenMemorySettings}
                onOpenKnowledge={onOpenKnowledge}
              />
            </div>
          )}
          {(msg.timestamp || totalDurationText) && (
            <div className="ml-7 mt-0.5 text-[10px] leading-none text-muted-foreground/60 select-none">
              {msg.timestamp ? formatMessageTime(msg.timestamp) : null}
              {totalDurationText && (
                <span className="text-muted-foreground/50">
                  {msg.timestamp ? " · " : ""}
                  {t("tools.elapsed", { time: totalDurationText })}
                </span>
              )}
            </div>
          )}
          <div
            className={cn(
              "ml-7 mt-0.5 flex h-6 items-center gap-0.5",
              (!hasToolbarActions || !(isHovered || isCopied || detailsIndex === index)) &&
                "invisible",
            )}
          >
            {msg.content && (
              <IconTip label={t("chat.copy")}>
                <button
                  type="button"
                  onClick={() => onCopy(msg.content, index)}
                  className={toolbarButtonClass}
                >
                  {isCopied ? (
                    <Check className="h-3.5 w-3.5 text-green-500" />
                  ) : (
                    <Copy className="h-3.5 w-3.5" />
                  )}
                </button>
              </IconTip>
            )}
            {addQuickPromptButton}
            {renderToggleButton}
            {detailsButton}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className={cn("relative min-w-0 max-w-[95%]", msg.fromAgentId && "flex items-start gap-2")}
      onMouseEnter={() => onHover(index)}
      onMouseLeave={() => {
        onHover(null)
        setDetailsIndex((prev) => (prev === index ? null : prev))
      }}
      onContextMenu={(e) => onContextMenu(e, index)}
    >
      {/* Parent agent avatar for delegated messages */}
      {msg.fromAgentId && (
        <div className="w-6 h-6 rounded-full bg-purple-500/15 flex items-center justify-center text-purple-500 shrink-0 mt-1 text-[10px] overflow-hidden">
          {fromAgent?.avatar ? (
            <img
              src={getTransport().resolveAssetUrl(fromAgent.avatar) ?? fromAgent.avatar}
              className="w-full h-full object-cover"
              alt=""
            />
          ) : fromAgent?.emoji ? (
            <span>{fromAgent.emoji}</span>
          ) : (
            <Network className="w-3 h-3" />
          )}
        </div>
      )}
      <div className="min-w-0 max-w-full">
        {msg.fromAgentId && (
          <div className="text-[10px] text-purple-500 mb-0.5 font-medium">
            {fromAgent?.name || msg.fromAgentId}
          </div>
        )}
        {msg.channelInbound && (
          <div className="flex items-center gap-1 text-[10px] text-blue-500 mb-0.5 font-medium justify-end">
            <ChannelIcon channelId={msg.channelInbound.channelId} className="w-2.5 h-2.5" />
            <span>{msg.channelInbound.channelId}</span>
            {msg.channelInbound.senderName && (
              <span className="text-blue-400">· {msg.channelInbound.senderName}</span>
            )}
          </div>
        )}
        {msg.role === "assistant" && msg.fallbackEvent && (
          <FallbackBanner event={msg.fallbackEvent} />
        )}
        <div
          className={cn(
            "max-w-full min-w-0 px-4 py-2.5 rounded-xl text-sm leading-relaxed overflow-hidden break-words select-text",
            isUserAligned && !msg.fromAgentId
              ? "bg-[var(--color-user-bubble)] text-foreground"
              : msg.fromAgentId
                ? "bg-purple-500/10 border border-purple-500/20 text-foreground"
                : "bg-card text-foreground/80",
            contentRenderMode === "markdown"
              ? "message-markdown-content"
              : "message-plain-content",
            msg.role === "assistant" &&
              !msg.content &&
              !msg.toolCalls?.length &&
              !msg.contentBlocks?.length &&
              "animate-pulse",
            msg.role === "assistant" && loading && isLast && "streaming-bubble",
          )}
        >
          {msg.role === "assistant" ? (
            // Always go through AssistantContentBlocks, even when contentBlocks
            // is missing — it synthesizes blocks from msg.thinking / toolCalls /
            // content as a fallback. This keeps the React component type stable
            // across stream_end (server-sent finalized contentBlocks merge into
            // state), preventing unmount/remount flicker of the markdown subtree.
            <AssistantContentBlocks
              msg={msg}
              loading={loading}
              isLast={isLast}
              executionState={executionState}
              sessionId={sessionId}
              onOpenPlanPanel={onOpenPlanPanel}
              onViewChildSession={onViewChildSession}
              onOpenDiff={onOpenDiff}
              contentRenderMode={contentRenderMode}
            />
          ) : (
            <>
              <UserAttachments attachments={msg.attachments} sessionId={sessionId} />
              <UserMessageContent
                content={msg.content}
                renderMode={contentRenderMode}
                fadeToClassName={userMessageFadeToClassName}
                forceExpanded={forceExpandUserContent}
                onForceExpandedDismiss={onForceExpandedUserContentDismiss}
              />
            </>
          )}
          {/* URL Previews (only for non-streaming messages) */}
          {msg.content && !(loading && isLast) && (
            <MessageUrlPreviews content={msg.content} isStreaming={loading && isLast} />
          )}
          {messageFiles.length > 0 && (
            <FileAttachments files={messageFiles} sessionId={sessionId} />
          )}
          {shouldShowMemoryTrace && (
            <ActiveMemoryTrace
              memory={msg.activeMemory}
              usedMemoryRefs={msg.usedMemoryRefs}
              retrievalPlanner={msg.retrievalPlanner}
              onOpenMemorySettings={onOpenMemorySettings}
              onOpenKnowledge={onOpenKnowledge}
            />
          )}
          {(msg.timestamp || totalDurationText) && (
            <div
              className={cn(
                "mt-1 text-[10px] leading-none select-none",
                isUserAligned ? "text-foreground/40 text-right" : "text-muted-foreground/60",
              )}
            >
              {msg.timestamp ? formatMessageTime(msg.timestamp) : null}
              {totalDurationText && (
                <span className="text-muted-foreground/50">
                  {msg.timestamp ? " · " : ""}
                  {t("tools.elapsed", { time: totalDurationText })}
                </span>
              )}
            </div>
          )}
        </div>
        {/* Hover toolbar — always reserve height (h-6 + mt-0.5 ≈ 26px) so a
         * loading bubble (msg.content === "") and a filled bubble both leave
         * the same gap to the row's bottom. Without the placeholder height
         * the gap to bottom jumps from 16px to 42px the moment the first
         * token arrives. */}
        <div
          className={cn(
            "flex items-center gap-0.5 mt-0.5 h-6",
            isUserAligned ? "justify-end" : "justify-start",
            (!hasToolbarActions || !(isHovered || isCopied || detailsIndex === index)) &&
              "invisible",
          )}
        >
          {msg.content && (
            <IconTip label={t("chat.copy")}>
              <button
                type="button"
                onClick={() => onCopy(msg.content, index)}
                className={toolbarButtonClass}
              >
                {isCopied ? (
                  <Check className="h-3.5 w-3.5 text-green-500" />
                ) : (
                  <Copy className="h-3.5 w-3.5" />
                )}
              </button>
            </IconTip>
          )}
          {addQuickPromptButton}
          {renderToggleButton}
          {detailsButton}
        </div>
      </div>
    </div>
  )
}

const MessageBubble = React.memo(MessageBubbleInner)
export default MessageBubble
