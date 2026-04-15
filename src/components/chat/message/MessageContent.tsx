import React from "react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "./ToolCallBlock"
import ToolCallGroup from "./ToolCallGroup"
import ThinkingBlock from "./ThinkingBlock"
import TaskBlock from "./TaskBlock"
import SubagentGroup, { type SubagentGroupRun } from "@/components/chat/SubagentGroup"
import { AskUserQuestionResult, SubmitPlanResult } from "./PlanResultBlocks"
import type { ContentBlock, ToolCall } from "@/types/chat"
import type { Message } from "@/types/chat"

const TASK_TOOL_NAMES = new Set(["task_create", "task_update", "task_list"])
const NO_GROUP_TOOLS = new Set([
  "ask_user_question",
  "submit_plan",
  "task_create",
  "task_update",
  "task_list",
  // subagent spawns are handled by a dedicated SubagentGroup path below;
  // never let them fall into the generic tool-call group.
  "subagent",
])

/** Parse a tool_call block as a subagent spawn with a resolved run_id. */
function parseSubagentSpawn(tool: ToolCall): SubagentGroupRun | null {
  if (tool.name !== "subagent") return null
  let args: { action?: string; agent_id?: string; task?: string } = {}
  try {
    args = JSON.parse(tool.arguments)
  } catch {
    return null
  }
  if (args.action !== "spawn") return null
  if (!tool.result) return null
  let runId: string | undefined
  try {
    runId = JSON.parse(tool.result).run_id
  } catch {
    return null
  }
  if (!runId) return null
  return {
    runId,
    agentId: args.agent_id || "default",
    task: args.task || "",
  }
}

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
      if (TASK_TOOL_NAMES.has(block.tool.name)) {
        elements.push(<TaskBlock key={block.tool.callId} tool={block.tool} />)
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
      // subagent spawn → collect consecutive spawns into a dedicated group
      if (block.tool.name === "subagent") {
        const parsed = parseSubagentSpawn(block.tool)
        if (parsed) {
          const runs: SubagentGroupRun[] = [parsed]
          let j = i + 1
          while (j < blocks.length) {
            const nb = blocks[j]
            if (nb.type !== "tool_call" || nb.tool.name !== "subagent") break
            const nextParsed = parseSubagentSpawn(nb.tool)
            if (!nextParsed) break
            runs.push(nextParsed)
            j++
          }
          if (runs.length >= 2) {
            // Key on the concatenated runIds so React remounts the group
            // (instead of re-running effects) when the underlying run set
            // actually changes.
            const groupKey = `sgrp-${runs.map((r) => r.runId).join("|")}`
            elements.push(<SubagentGroup key={groupKey} runs={runs} />)
            i = j
            continue
          }
          // Single subagent spawn → fall through to the default ToolCallBlock
          // path which already renders SubagentBlock.
          elements.push(<ToolCallBlock key={block.tool.callId} tool={block.tool} />)
          i++
          continue
        }
        // Non-spawn subagent action (check / list / kill) or still in-flight
        // → render individually. NO_GROUP_TOOLS prevents it from joining the
        // generic tool-call group below.
        elements.push(<ToolCallBlock key={block.tool.callId} tool={block.tool} />)
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
        if (NO_GROUP_TOOLS.has(tb.tool.name)) break
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
