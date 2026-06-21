import React, { useState, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
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
} from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import {
  formatTokens,
  formatDuration,
  formatMessageTime,
  extractMessageFileAttachments,
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
  // Plan mode
  sessionId?: string | null
  onOpenPlanPanel?: () => void
  // Session switching (used by SubagentBlock's "jump to child session" button)
  onSwitchSession?: (sessionId: string) => void
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
  displayMode?: ChatDisplayMode
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
  sessionId,
  onOpenPlanPanel,
  onSwitchSession,
  onSwitchModel,
  onViewSystemPrompt,
  compacting,
  onCompactContext,
  onOpenDashboardTab,
  onOpenDiff,
  onResume,
  displayMode = "bubble",
}: MessageBubbleProps) {
  const { t } = useTranslation()
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)
  const [resultExpanded, setResultExpanded] = useState(false)
  const [contentRenderMode, setContentRenderMode] = useState<ContentRenderMode>("markdown")

  const messageFiles = useMemo(
    () =>
      msg.role === "assistant" && msg.contentBlocks
        ? extractMessageFileAttachments(msg.contentBlocks)
        : [],
    [msg.role, msg.contentBlocks],
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
  const hasTextContent = hasRenderableTextContent(msg)
  const hasDetails = msg.role === "assistant" && !!(msg.usage || msg.model)
  const hasToolbarActions = hasTextContent || hasDetails
  // Always-visible total turn duration, shown at the message bottom once the
  // assistant turn has finished (the per-step / per-group times live above).
  const totalDurationText =
    msg.role === "assistant" && !(loading && isLast) && msg.usage?.durationMs != null
      ? formatDuration(msg.usage.durationMs)
      : null
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
          onSwitchSession={onSwitchSession}
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
            onSwitchSession={onSwitchSession}
            onOpenDiff={onOpenDiff}
            displayMode="timeline"
            contentRenderMode={contentRenderMode}
          />
          {messageFiles.length > 0 && (
            <div className="ml-7">
              <FileAttachments files={messageFiles} sessionId={sessionId} />
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
            {renderToggleButton}
            {detailsButton}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className={cn("relative max-w-[95%]", msg.fromAgentId && "flex items-start gap-2")}
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
      <div>
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
            "px-4 py-2.5 rounded-xl text-sm leading-relaxed overflow-hidden break-words select-text",
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
              onSwitchSession={onSwitchSession}
              onOpenDiff={onOpenDiff}
              contentRenderMode={contentRenderMode}
            />
          ) : (
            <>
              <UserAttachments attachments={msg.attachments} />
              {contentRenderMode === "markdown" ? (
                <MarkdownRenderer content={msg.content} />
              ) : (
                <PlainTextRenderer content={msg.content} />
              )}
            </>
          )}
          {/* URL Previews (only for non-streaming messages) */}
          {msg.content && !(loading && isLast) && (
            <MessageUrlPreviews content={msg.content} isStreaming={loading && isLast} />
          )}
          {messageFiles.length > 0 && (
            <FileAttachments files={messageFiles} sessionId={sessionId} />
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
          {renderToggleButton}
          {detailsButton}
        </div>
      </div>
    </div>
  )
}

const MessageBubble = React.memo(MessageBubbleInner)
export default MessageBubble
