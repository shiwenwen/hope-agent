import React from "react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "./ToolCallBlock"
import ToolCallGroup from "./ToolCallGroup"
import ThinkingBlock from "./ThinkingBlock"
import { AskUserQuestionResult, SubmitPlanResult } from "./PlanResultBlocks"
import type { ContentBlock } from "@/types/chat"
import type { Message } from "@/types/chat"

interface MessageContentProps {
  msg: Message
  loading: boolean
  isLast: boolean
  sessionId?: string | null
  onOpenPlanPanel?: () => void
}

/** Renders assistant content blocks (thinking, text, tool calls) with grouping logic */
export function AssistantContentBlocks({
  msg,
  loading,
  isLast,
  sessionId,
  onOpenPlanPanel,
}: MessageContentProps) {
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
          isStreaming={loading && isLast && isLastBlock}
          durationMs={block.durationMs}
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
      // Render ask_user_question as Q&A summary card (result contains formatted answers)
      if (block.tool.name === "ask_user_question") {
        if (block.tool.result) {
          elements.push(
            <AskUserQuestionResult key={block.tool.callId} result={block.tool.result} />,
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
        if (tb.tool.name === "ask_user_question" || tb.tool.name === "submit_plan") break // stop grouping at plan tools
        group.push(blocks[j])
        j++
      }

      const isLastToolGroup = loading && isLast && j === blocks.length
      if (group.length >= 2) {
        // Render as a collapsed group
        const tools = group.map(
          (b) => (b as { type: "tool_call"; tool: typeof block.tool }).tool,
        )
        elements.push(
          <ToolCallGroup
            key={`grp-${tools[0].callId}`}
            tools={tools}
            shimmer={isLastToolGroup}
          />,
        )
      } else {
        // Single tool — render individually
        elements.push(<ToolCallBlock key={block.tool.callId} tool={block.tool} shimmer={isLastToolGroup} />)
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

  return <>{elements}</>
}

/** Legacy fallback path for old messages without contentBlocks */
export function AssistantLegacyContent({
  msg,
  loading,
  isLast,
}: {
  msg: Message
  loading: boolean
  isLast: boolean
}) {
  return (
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
  )
}
