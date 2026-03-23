import { useState, useRef, useEffect } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { Copy, Check, Info, Network } from "lucide-react"
import { formatTokens, formatDuration, formatMessageTime } from "./chatUtils"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "@/components/chat/ToolCallBlock"
import ThinkingBlock from "@/components/chat/ThinkingBlock"
import FallbackBanner from "@/components/chat/FallbackBanner"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"

interface MessageBubbleProps {
  msg: Message
  index: number
  isLast: boolean
  loading: boolean
  agents: AgentSummaryForSidebar[]
  // Hover & interaction state
  hoveredMsgIndex: number | null
  onHover: (index: number | null) => void
  onContextMenu: (e: React.MouseEvent, index: number) => void
  // Copy
  copiedIndex: number | null
  onCopy: (content: string, index: number) => void
  // Edit
  editingIndex: number | null
  editContent: string
  onEditContentChange: (content: string) => void
  onSaveEdit: (index: number) => void
  onCancelEdit: () => void
}

export default function MessageBubble({
  msg,
  index,
  isLast,
  loading,
  agents,
  hoveredMsgIndex,
  onHover,
  onContextMenu,
  copiedIndex,
  onCopy,
  editingIndex,
  editContent,
  onEditContentChange,
  onSaveEdit,
  onCancelEdit,
}: MessageBubbleProps) {
  const { t } = useTranslation()
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)
  const editTextareaRef = useRef<HTMLTextAreaElement>(null)

  // Auto-focus textarea when entering edit mode
  useEffect(() => {
    if (editingIndex === index) {
      editTextareaRef.current?.focus()
    }
  }, [editingIndex, index])

  const fromAgent = msg.fromAgentId ? agents.find((a) => a.id === msg.fromAgentId) : undefined

  if (msg.role === "event") {
    return (
      <div className="max-w-[80%] px-3 py-1.5 rounded-lg text-xs text-muted-foreground bg-muted/50 border border-border/50 text-center">
        {msg.content}
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
              src={
                fromAgent.avatar.startsWith("/") ? convertFileSrc(fromAgent.avatar) : fromAgent.avatar
              }
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
          {msg.role === "assistant" && editingIndex === index ? (
            // Edit mode
            <div className="space-y-2">
              <textarea
                ref={editTextareaRef}
                value={editContent}
                onChange={(e) => onEditContentChange(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") onCancelEdit()
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) onSaveEdit(index)
                }}
                className="w-full min-h-[80px] rounded-lg border border-border bg-background p-2 text-sm text-foreground resize-y focus:outline-none focus:ring-1 focus:ring-ring"
              />
              <div className="flex items-center justify-end gap-2">
                <button
                  onClick={onCancelEdit}
                  className="px-2.5 py-1 rounded-md text-xs text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors"
                >
                  {t("common.cancel")}
                </button>
                <button
                  onClick={() => onSaveEdit(index)}
                  className="px-2.5 py-1 rounded-md text-xs bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
                >
                  {t("common.save")}
                </button>
              </div>
            </div>
          ) : msg.role === "assistant" && msg.contentBlocks && msg.contentBlocks.length > 0 ? (
            // Render content blocks in order (thinking → tool → text)
            msg.contentBlocks
              .map((block, blockIdx) => {
                if (block.type === "thinking") {
                  const isLastBlock = blockIdx === msg.contentBlocks!.length - 1
                  return (
                    <ThinkingBlock
                      key={blockIdx}
                      content={block.content}
                      isStreaming={
                        loading && isLast && isLastBlock && !msg.content.trim()
                      }
                    />
                  )
                }
                if (block.type === "tool_call") {
                  return <ToolCallBlock key={block.tool.callId} tool={block.tool} />
                }
                if (block.type === "text") {
                  return (
                    <MarkdownRenderer
                      key={blockIdx}
                      content={block.content}
                      isStreaming={
                        loading && isLast && blockIdx === msg.contentBlocks!.length - 1
                      }
                    />
                  )
                }
                return null
              })
              .concat(
                // Show loading dots between tool rounds
                loading && isLast
                  ? (() => {
                      const lastBlock = msg.contentBlocks![msg.contentBlocks!.length - 1]
                      const waitingForNextRound =
                        lastBlock.type === "tool_call" && lastBlock.tool.result !== undefined
                      if (!waitingForNextRound) return null
                      return (
                        <div key="__loading__" className="flex items-center gap-1 py-1 px-2">
                          <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse" />
                          <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:300ms]" />
                          <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:600ms]" />
                        </div>
                      )
                    })()
                  : null,
              )
          ) : msg.role === "assistant" ? (
            // Legacy fallback path for old messages without contentBlocks
            <>
              {msg.thinking && (
                <ThinkingBlock
                  content={msg.thinking}
                  isStreaming={loading && isLast && !msg.content}
                />
              )}
              {msg.toolCalls?.map((tool) => (
                <ToolCallBlock key={tool.callId} tool={tool} />
              ))}
              {msg.content ? (
                <MarkdownRenderer content={msg.content} isStreaming={loading && isLast} />
              ) : (
                !msg.toolCalls?.length && (
                  <div className="flex items-center gap-1.5 h-6 px-2 relative top-1">
                    <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse" />
                    <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse [animation-delay:200ms]" />
                    <span className="block w-2 h-2 aspect-square rounded-full bg-foreground animate-bounce-pulse [animation-delay:400ms]" />
                  </div>
                )
              )}
            </>
          ) : (
            // User message content
            msg.content
          )}
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
          <TooltipProvider>
            <div
              className={cn(
                "flex items-center gap-0.5 mt-0.5 h-6",
                msg.role === "user" ? "justify-end" : "justify-start",
                !(hoveredMsgIndex === index || copiedIndex === index || detailsIndex === index) &&
                  "invisible",
              )}
            >
              <IconTip label={t("chat.copy")}>
                <button
                  onClick={() => onCopy(msg.content, index)}
                  className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors"
                >
                  {copiedIndex === index ? (
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
                    <div className="absolute bottom-full mb-1 z-50 min-w-[180px] rounded-lg border border-border bg-popover p-2.5 shadow-lg left-0">
                      <div className="space-y-1.5 text-xs">
                        {msg.model && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground">{t("chat.statusModel")}</span>
                            <span
                              className="font-medium text-foreground truncate max-w-[160px]"
                              title={msg.model}
                            >
                              {msg.model}
                            </span>
                          </div>
                        )}
                        {msg.model && msg.usage?.inputTokens != null && (
                          <div className="border-t border-border" />
                        )}
                        {msg.usage?.inputTokens != null && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground">{t("chat.inputTokens")}</span>
                            <span className="font-medium text-foreground tabular-nums">
                              {formatTokens(msg.usage.inputTokens)}
                            </span>
                          </div>
                        )}
                        {msg.usage?.outputTokens != null && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground">{t("chat.outputTokens")}</span>
                            <span className="font-medium text-foreground tabular-nums">
                              {formatTokens(msg.usage.outputTokens)}
                            </span>
                          </div>
                        )}
                        {msg.usage?.inputTokens != null && msg.usage?.outputTokens != null && (
                          <>
                            <div className="border-t border-border" />
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground">{t("chat.totalTokens")}</span>
                              <span className="font-medium text-foreground tabular-nums">
                                {formatTokens(msg.usage.inputTokens + msg.usage.outputTokens)}
                              </span>
                            </div>
                          </>
                        )}
                        {msg.usage?.durationMs != null && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground">{t("chat.duration")}</span>
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
          </TooltipProvider>
        )}
      </div>
    </div>
  )
}
