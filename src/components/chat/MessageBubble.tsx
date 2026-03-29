import { useState, useMemo } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { invoke } from "@tauri-apps/api/core"
import { Copy, Check, Info, Network, Timer, MessageSquare, ChevronRight, ClipboardList, FolderOpen, PanelRight } from "lucide-react"
import { formatTokens, formatDuration, formatMessageTime, extractModifiedFiles } from "./chatUtils"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "@/components/chat/ToolCallBlock"
import ToolCallGroup from "@/components/chat/ToolCallGroup"
import type { ContentBlock } from "@/types/chat"
import ThinkingBlock from "@/components/chat/ThinkingBlock"
import FallbackBanner from "@/components/chat/FallbackBanner"
import FileAttachments from "@/components/chat/FileAttachments"
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
  // Plan mode
  sessionId?: string | null
  onOpenPlanPanel?: () => void
}

/** Collapsible Q&A summary for plan_question tool results */
function PlanQuestionResult({ result }: { result: string }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const items = useMemo(() => {
    try {
      const data = JSON.parse(result) as {
        answers: Array<{ question: string; selected: string[]; customInput?: string }>
      }
      return data.answers || []
    } catch {
      return []
    }
  }, [result])

  if (items.length === 0) return null

  return (
    <div className="my-2 rounded-lg border border-green-500/20 bg-green-500/5">
      <button
        className="flex items-center gap-2 w-full px-4 py-2.5 text-sm text-green-600 hover:bg-green-500/5 transition-colors cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Check className="h-4 w-4" />
        <span className="font-medium">{t("planMode.question.answered")}</span>
      </button>
      {expanded && (
        <div className="px-4 pb-3 space-y-2 border-t border-green-500/10 pt-2">
          {items.map((item, i) => (
            <div key={i} className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground">{item.question}</span>
              <div className="mt-0.5 pl-2">
                {item.selected.map((s, j) => (
                  <div key={j}>- {s}</div>
                ))}
                {item.customInput && <div>- {item.customInput}</div>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/** Compact inline card for submit_plan tool results */
function SubmitPlanResult({
  title,
  sessionId,
  onOpenPanel,
}: {
  title: string
  sessionId?: string | null
  onOpenPanel?: () => void
}) {
  const { t } = useTranslation()

  const handleRevealFile = async () => {
    if (!sessionId) return
    try {
      const filePath = await invoke<string | null>("get_plan_file_path", { sessionId })
      if (filePath) {
        await invoke("reveal_in_folder", { path: filePath })
      }
    } catch { /* ignore */ }
  }

  return (
    <div
      className="my-2 rounded-lg border border-purple-500/20 bg-purple-500/5 px-4 py-3 flex items-center gap-3 cursor-pointer hover:bg-purple-500/10 transition-colors"
      onClick={onOpenPanel}
    >
      <ClipboardList className="h-4 w-4 text-purple-600 shrink-0" />
      <span className="text-sm font-medium truncate flex-1">
        {title || t("planMode.panelTitle")}
      </span>
      <div className="flex items-center gap-1.5 shrink-0">
        <PanelRight className="h-3.5 w-3.5 text-muted-foreground" />
        <button
          onClick={(e) => { e.stopPropagation(); handleRevealFile() }}
          className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-secondary transition-colors cursor-pointer"
        >
          <FolderOpen className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  )
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
  sessionId,
  onOpenPlanPanel,
}: MessageBubbleProps) {
  const { t } = useTranslation()
  const [detailsIndex, setDetailsIndex] = useState<number | null>(null)

  const modifiedFiles = useMemo(
    () =>
      msg.role === "assistant" && msg.contentBlocks
        ? extractModifiedFiles(msg.contentBlocks)
        : [],
    [msg.role, msg.contentBlocks],
  )

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
          {msg.role === "assistant" && msg.contentBlocks && msg.contentBlocks.length > 0 ? (
            // Render content blocks with consecutive same-category tool calls grouped
            (() => {
              const blocks = msg.contentBlocks!
              const elements: React.ReactNode[] = []

              let i = 0
              while (i < blocks.length) {
                const block = blocks[i]

                if (block.type === "thinking") {
                  const isLastBlock = i === blocks.length - 1
                  elements.push(
                    <ThinkingBlock
                      key={i}
                      content={block.content}
                      isStreaming={loading && isLast && isLastBlock && !msg.content.trim()}
                    />,
                  )
                  i++
                } else if (block.type === "text") {
                  elements.push(
                    <MarkdownRenderer
                      key={i}
                      content={block.content}
                      isStreaming={loading && isLast && i === blocks.length - 1}
                    />,
                  )
                  i++
                } else if (block.type === "tool_call") {
                  // Render plan_question as Q&A summary card (result contains formatted answers)
                  if (block.tool.name === "plan_question") {
                    if (block.tool.result) {
                      elements.push(
                        <PlanQuestionResult key={block.tool.callId} result={block.tool.result} />,
                      )
                    }
                    i++
                    continue
                  }
                  // Render submit_plan inline as a compact plan card
                  if (block.tool.name === "submit_plan") {
                    if (block.tool.result) {
                      let title = ""
                      try {
                        title = JSON.parse(block.tool.arguments)?.title || ""
                      } catch { /* ignore */ }
                      elements.push(
                        <SubmitPlanResult key={block.tool.callId} title={title} sessionId={sessionId} onOpenPanel={onOpenPlanPanel} />,
                      )
                    }
                    i++
                    continue
                  }
                  // Collect ALL consecutive tool_call blocks (regardless of category)
                  const group: ContentBlock[] = [block]
                  let j = i + 1
                  while (
                    j < blocks.length &&
                    blocks[j].type === "tool_call"
                  ) {
                    const tb = blocks[j] as { type: "tool_call"; tool: { name: string } }
                    if (tb.tool.name === "plan_question" || tb.tool.name === "submit_plan") break // stop grouping at plan tools
                    group.push(blocks[j])
                    j++
                  }

                  if (group.length >= 2) {
                    // Render as a collapsed group
                    const tools = group.map(
                      (b) => (b as { type: "tool_call"; tool: typeof block.tool }).tool,
                    )
                    elements.push(
                      <ToolCallGroup
                        key={`grp-${tools[0].callId}`}
                        tools={tools}
                      />,
                    )
                  } else {
                    // Single tool — render individually
                    elements.push(<ToolCallBlock key={block.tool.callId} tool={block.tool} />)
                  }
                  i = j
                } else {
                  i++
                }
              }

              // Loading dots during/between tool rounds
              if (loading && isLast) {
                const lastBlock = blocks[blocks.length - 1]
                if (lastBlock.type === "tool_call") {
                  elements.push(
                    <div key="__loading__" className="flex items-center gap-1 py-1 px-2">
                      <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse" />
                      <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:300ms]" />
                      <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:600ms]" />
                    </div>,
                  )
                }
              }

              return elements
            })()
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
                    <div className="absolute bottom-full mb-1 z-50 min-w-[220px] rounded-lg border border-border bg-popover p-2.5 shadow-lg left-0">
                      <div className="space-y-1.5 text-xs">
                        {msg.model && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground whitespace-nowrap shrink-0">{t("chat.statusModel")}</span>
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
                            <span className="text-muted-foreground whitespace-nowrap shrink-0">{t("chat.inputTokens")}</span>
                            <span className="font-medium text-foreground tabular-nums">
                              {formatTokens(msg.usage.inputTokens)}
                            </span>
                          </div>
                        )}
                        {msg.usage?.outputTokens != null && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground whitespace-nowrap shrink-0">{t("chat.outputTokens")}</span>
                            <span className="font-medium text-foreground tabular-nums">
                              {formatTokens(msg.usage.outputTokens)}
                            </span>
                          </div>
                        )}
                        {msg.usage?.inputTokens != null && msg.usage?.outputTokens != null && (
                          <>
                            <div className="border-t border-border" />
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground whitespace-nowrap shrink-0">{t("chat.totalTokens")}</span>
                              <span className="font-medium text-foreground tabular-nums">
                                {formatTokens(msg.usage.inputTokens + msg.usage.outputTokens)}
                              </span>
                            </div>
                          </>
                        )}
                        {msg.usage?.durationMs != null && (
                          <div className="flex items-center justify-between gap-3">
                            <span className="text-muted-foreground whitespace-nowrap shrink-0">{t("chat.duration")}</span>
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
