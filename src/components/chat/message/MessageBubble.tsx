import React, { useState, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Copy, Check, Info, Network, Timer } from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import { formatTokens, formatDuration, formatMessageTime, extractModifiedFiles } from "../chatUtils"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import FileAttachments from "./FileAttachments"
import FallbackBanner from "@/components/chat/FallbackBanner"
import MessageUrlPreviews from "./MessageUrlPreviews"
import { AssistantContentBlocks, AssistantLegacyContent } from "./MessageContent"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"
import ModelPickerCard from "@/components/chat/ModelPickerCard"
import ContextBreakdownCard from "@/components/chat/context-view/ContextBreakdownCard"

export interface MessageBubbleProps {
  msg: Message
  index: number
  isLast: boolean
  loading: boolean
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
      {expanded && (
        <div className="w-full px-3 py-2 rounded-lg bg-amber-500/5 border border-amber-500/15 text-xs text-foreground/80 whitespace-pre-wrap break-words animate-in fade-in-0 slide-in-from-top-1 duration-150">
          {msg.content}
        </div>
      )}
    </div>
  )
}

function MessageBubbleInner({
  msg,
  index,
  isLast,
  loading,
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
}: MessageBubbleProps) {
  const { t } = useTranslation()
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)

  const modifiedFiles = useMemo(
    () =>
      msg.role === "assistant" && msg.contentBlocks ? extractModifiedFiles(msg.contentBlocks) : [],
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

  if (msg.role === "event") {
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
          onViewSystemPrompt={onViewSystemPrompt}
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
    return (
      <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center [&_p]:m-0">
        <MarkdownRenderer content={msg.content} />
      </div>
    )
  }

  if (msg.isSubagentResult) {
    return (
      <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-purple-500/8 border border-purple-500/20 text-xs text-purple-400/80 max-w-[80%]">
        <Network className="w-3 h-3 shrink-0 text-purple-500" />
        <span className="font-medium text-purple-500">
          {agents.find((a) => a.id === msg.subagentResultAgentId)?.name ||
            msg.subagentResultAgentId ||
            "Sub-agent"}
        </span>
        <span className="text-purple-400/50">·</span>
        <span>任务完成，结果已注入</span>
      </div>
    )
  }

  if (msg.isCronTrigger) {
    return <CronTriggerBubble msg={msg} t={t} />
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
            msg.role === "user" && !msg.fromAgentId
              ? "bg-[var(--color-user-bubble)] text-foreground whitespace-pre-wrap"
              : msg.fromAgentId
                ? "bg-purple-500/10 border border-purple-500/20 text-foreground whitespace-pre-wrap"
                : "bg-card text-foreground/80",
            msg.role === "assistant" &&
              !msg.content &&
              !msg.toolCalls?.length &&
              !msg.contentBlocks?.length &&
              "animate-pulse",
            msg.role === "assistant" && loading && isLast && "streaming-bubble",
          )}
        >
          {msg.role === "assistant" && msg.contentBlocks && msg.contentBlocks.length > 0 ? (
            <AssistantContentBlocks
              msg={msg}
              loading={loading}
              isLast={isLast}
              sessionId={sessionId}
              onOpenPlanPanel={onOpenPlanPanel}
              onSwitchSession={onSwitchSession}
            />
          ) : msg.role === "assistant" ? (
            <AssistantLegacyContent msg={msg} loading={loading} isLast={isLast} />
          ) : (
            // User message content
            msg.content
          )}
          {/* URL Previews (only for non-streaming messages) */}
          {msg.content && !(loading && isLast) && (
            <MessageUrlPreviews content={msg.content} isStreaming={loading && isLast} />
          )}
          {modifiedFiles.length > 0 && <FileAttachments files={modifiedFiles} />}
          {msg.timestamp && (
            <div
              className={cn(
                "mt-1 text-[10px] leading-none select-none",
                msg.role === "user" ? "text-foreground/40 text-right" : "text-muted-foreground/60",
              )}
            >
              {formatMessageTime(msg.timestamp)}
            </div>
          )}
        </div>
        {/* Hover toolbar */}
        {msg.content && (
          <div
            className={cn(
              "flex items-center gap-0.5 mt-0.5 h-6",
              msg.role === "user" ? "justify-end" : "justify-start",
              !(isHovered || isCopied || detailsIndex === index) && "invisible",
            )}
          >
            <IconTip label={t("chat.copy")}>
              <button
                onClick={() => onCopy(msg.content, index)}
                className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors"
              >
                {isCopied ? (
                  <Check className="h-3.5 w-3.5 text-green-500" />
                ) : (
                  <Copy className="h-3.5 w-3.5" />
                )}
              </button>
            </IconTip>
            {msg.role === "assistant" && (msg.usage || msg.model) && (
              <div className="relative">
                <IconTip label={t("chat.details")}>
                  <button
                    onClick={() => setDetailsIndex(detailsIndex === index ? null : index)}
                    className={cn(
                      "p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors",
                      detailsIndex === index && "text-foreground bg-muted/80",
                    )}
                  >
                    <Info className="h-3.5 w-3.5" />
                  </button>
                </IconTip>
                {detailsIndex === index && (
                  <div className="absolute bottom-full mb-1 z-50 min-w-[220px] rounded-lg border border-border bg-popover p-2.5 shadow-lg left-0">
                    <div className="space-y-1.5 text-xs">
                      {msg.model && (
                        <div className="flex items-center justify-between gap-3">
                          <span className="text-muted-foreground whitespace-nowrap shrink-0">
                            {t("chat.statusModel")}
                          </span>
                          <IconTip label={msg.model}>
                            <span className="font-medium text-foreground truncate max-w-[160px]">
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
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground whitespace-nowrap shrink-0">
                                {showLastInput
                                  ? t("chat.inputTokensCumulative")
                                  : t("chat.inputTokens")}
                              </span>
                              <span className="font-medium text-foreground tabular-nums">
                                {formatTokens(inputTokens)}
                              </span>
                            </div>
                            {showLastInput && lastInputTokens != null && (
                              <div className="flex items-center justify-between gap-3">
                                <span className="text-muted-foreground whitespace-nowrap shrink-0">
                                  {t("chat.lastRoundInputTokens")}
                                </span>
                                <span className="font-medium text-foreground tabular-nums">
                                  ⚡️{t("chat.statusCacheHit")} {formatTokens(lastInputTokens)}
                                </span>
                              </div>
                            )}
                          </>
                        )
                      })()}
                      {msg.usage?.outputTokens != null && (
                        <div className="flex items-center justify-between gap-3">
                          <span className="text-muted-foreground whitespace-nowrap shrink-0">
                            {t("chat.outputTokens")}
                          </span>
                          <span className="font-medium text-foreground tabular-nums">
                            {formatTokens(msg.usage.outputTokens)}
                          </span>
                        </div>
                      )}
                      {msg.usage?.inputTokens != null && msg.usage?.outputTokens != null && (
                        <>
                          <div className="border-t border-border" />
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground whitespace-nowrap shrink-0">
                              {t("chat.totalTokens")}
                            </span>
                            <span className="font-medium text-foreground tabular-nums">
                              {formatTokens(msg.usage.inputTokens + msg.usage.outputTokens)}
                            </span>
                          </div>
                        </>
                      )}
                      {msg.usage?.durationMs != null && (
                        <div className="flex items-center justify-between gap-3">
                          <span className="text-muted-foreground whitespace-nowrap shrink-0">
                            {t("chat.duration")}
                          </span>
                          <span className="font-medium text-foreground tabular-nums">
                            {formatDuration(msg.usage.durationMs)}
                          </span>
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

const MessageBubble = React.memo(MessageBubbleInner)
export default MessageBubble
