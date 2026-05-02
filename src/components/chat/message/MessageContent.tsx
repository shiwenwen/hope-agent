import React from "react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "./ToolCallBlock"
import ToolCallGroup from "./ToolCallGroup"
import ThinkingBlock from "./ThinkingBlock"
import TaskBlock from "./TaskBlock"
import SubagentGroup, { type SubagentGroupRun } from "@/components/chat/SubagentGroup"
import SubagentBlock from "@/components/chat/SubagentBlock"
import SkillProgressBlock from "@/components/chat/SkillProgressBlock"
import { AskUserQuestionResult, SubmitPlanResult } from "./PlanResultBlocks"
import { TASK_TOOL_NAMES } from "@/components/chat/tasks/taskProgress"
import type {
  ContentBlock,
  FileChangeMetadata,
  FileChangesMetadata,
  ToolCall,
} from "@/types/chat"
import type { Message } from "@/types/chat"

const NO_GROUP_TOOLS = new Set([
  "ask_user_question",
  "submit_plan",
  "task_create",
  "task_update",
  "task_list",
  // subagent spawns are handled by a dedicated SubagentGroup path below;
  // never let them fall into the generic tool-call group.
  "subagent",
  // skill activations get their own SkillProgressBlock renderer.
  "skill",
  // canvas has a dedicated reopen-card UI in ToolCallBlock; GroupItem
  // doesn't render it, so keep canvas out of the group path.
  "canvas",
])

/** Extract zero or more subagent runs from a tool_call block. Handles:
 *   - action=spawn            → 1 run (if runId present)
 *   - action=spawn_and_wait   → 1 run (foreground or backgrounded)
 *   - action=batch_spawn      → N runs from result.runs[] (only "spawned" entries)
 */
function extractSubagentRuns(tool: ToolCall): SubagentGroupRun[] {
  if (tool.name !== "subagent") return []
  if (!tool.result) return []
  let args: {
    action?: string
    agent_id?: string
    task?: string
    tasks?: Array<{ agent_id?: string; task?: string }>
  }
  try {
    args = JSON.parse(tool.arguments)
  } catch {
    return []
  }
  let result: unknown
  try {
    result = JSON.parse(tool.result)
  } catch {
    return []
  }
  if (!result || typeof result !== "object") return []

  if (args.action === "spawn" || args.action === "spawn_and_wait") {
    const runId = (result as { run_id?: unknown }).run_id
    if (typeof runId !== "string" || !runId) return []
    return [
      {
        runId,
        agentId: args.agent_id || "default",
        task: args.task || "",
      },
    ]
  }

  if (args.action === "batch_spawn") {
    const runs = (result as { runs?: unknown }).runs
    if (!Array.isArray(runs)) return []
    const taskDefs = Array.isArray(args.tasks) ? args.tasks : []
    const out: SubagentGroupRun[] = []
    for (let idx = 0; idx < runs.length; idx++) {
      const r = runs[idx]
      if (!r || typeof r !== "object") continue
      const obj = r as { status?: unknown; run_id?: unknown }
      if (obj.status !== "spawned") continue
      if (typeof obj.run_id !== "string" || !obj.run_id) continue
      const def = taskDefs[idx] || {}
      out.push({
        runId: obj.run_id,
        agentId: def.agent_id || "default",
        task: def.task || "",
      })
    }
    return out
  }

  return []
}

interface MessageContentProps {
  msg: Message
  loading: boolean
  isLast: boolean
  sessionId?: string | null
  onOpenPlanPanel?: () => void
  onSwitchSession?: (sessionId: string) => void
  /** Open the right-side diff panel for a file change payload. */
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
}

/** Renders assistant content blocks (thinking, text, tool calls) with grouping logic */
export function AssistantContentBlocks({
  msg,
  loading,
  isLast,
  sessionId,
  onOpenPlanPanel,
  onSwitchSession,
  onOpenDiff,
}: MessageContentProps) {
  const blocks = msg.contentBlocks!
  const elements: React.ReactNode[] = []

  // Pre-compute first task_* position + latest task_* tool with a result,
  // so all task_create / task_update / task_list calls in this message
  // collapse into a single TaskBlock showing the most recent snapshot
  // (each result is a full task-list snapshot, so the last one wins).
  let firstTaskIdx = -1
  let latestTaskTool: ToolCall | null = null
  for (let k = 0; k < blocks.length; k++) {
    const b = blocks[k]
    if (b.type !== "tool_call" || !TASK_TOOL_NAMES.has(b.tool.name)) continue
    if (firstTaskIdx === -1) firstTaskIdx = k
    if (b.tool.result) latestTaskTool = b.tool
  }
  if (firstTaskIdx !== -1 && !latestTaskTool) {
    // No tool has a result yet (first call still in-flight) — fall back to
    // the earliest one so we at least render the placeholder "no tasks".
    const first = blocks[firstTaskIdx]
    if (first.type === "tool_call") latestTaskTool = first.tool
  }

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
      // ask_user_question — passive indicator on the timeline. The actual
      // dialog is dispatched via a separate event channel, so the card here
      // is just for the user to see "model asked a question" while the answer
      // is still pending, then "answered" once the result arrives.
      if (block.tool.name === "ask_user_question") {
        elements.push(
          <AskUserQuestionResult
            key={block.tool.callId}
            result={block.tool.result}
            pending={!block.tool.result}
          />,
        )
        i++
        continue
      }
      if (TASK_TOOL_NAMES.has(block.tool.name)) {
        if (i === firstTaskIdx && latestTaskTool) {
          elements.push(<TaskBlock key={latestTaskTool.callId} tool={latestTaskTool} />)
        }
        i++
        continue
      }
      // submit_plan — render the card both in-flight (shimmer chip) and after
      // the result lands (full panel-opening card). The title is in arguments
      // so we can show it during the pending phase too.
      if (block.tool.name === "submit_plan") {
        let title = ""
        try {
          title = JSON.parse(block.tool.arguments)?.title || ""
        } catch { /* ignore */ }
        elements.push(
          <SubmitPlanResult
            key={block.tool.callId}
            title={title}
            sessionId={sessionId}
            onOpenPanel={onOpenPlanPanel}
            pending={!block.tool.result}
          />,
        )
        i++
        continue
      }
      // skill activation → dedicated Puzzle-iconed block (covers both inline
      // and fork modes; fork detection happens inside the component by
      // looking at the tool_result prefix).
      if (block.tool.name === "skill") {
        const isLastTool = loading && isLast && i === blocks.length - 1
        elements.push(
          <SkillProgressBlock key={block.tool.callId} tool={block.tool} shimmer={isLastTool} />,
        )
        i++
        continue
      }
      // subagent spawn / batch_spawn / spawn_and_wait → dedicated rendering
      if (block.tool.name === "subagent") {
        const firstRuns = extractSubagentRuns(block.tool)
        if (firstRuns.length > 0) {
          // Collect additional consecutive subagent blocks that also expose
          // one-or-more runs — covers "N parallel spawn calls" and "1 spawn
          // followed by 1 batch_spawn" alike.
          const runs: SubagentGroupRun[] = [...firstRuns]
          let j = i + 1
          while (j < blocks.length) {
            const nb = blocks[j]
            if (nb.type !== "tool_call" || nb.tool.name !== "subagent") break
            const nextRuns = extractSubagentRuns(nb.tool)
            if (nextRuns.length === 0) break
            runs.push(...nextRuns)
            j++
          }
          if (runs.length >= 2) {
            // Key on the concatenated runIds so React remounts the group
            // (instead of re-running effects) when the underlying run set
            // actually changes.
            const groupKey = `sgrp-${runs.map((r) => r.runId).join("|")}`
            elements.push(
              <SubagentGroup key={groupKey} runs={runs} onSwitchSession={onSwitchSession} />,
            )
          } else {
            // Single run (plain spawn, spawn_and_wait, or batch_spawn w/ 1 task)
            // → render SubagentBlock directly so batch_spawn's single case
            // also gets the rich UI (the legacy ToolCallBlock path only
            // detects action="spawn").
            const run = runs[0]
            elements.push(
              <SubagentBlock
                key={run.runId}
                runId={run.runId}
                agentId={run.agentId}
                task={run.task}
                onSwitchSession={onSwitchSession}
              />,
            )
          }
          i = j
          continue
        }
        // Non-spawn-like subagent action (check / list / kill / steer / etc)
        // or spawn still in-flight without a run_id yet → render individually.
        // NO_GROUP_TOOLS prevents it from falling into the generic tool-call
        // group below.
        elements.push(
          <ToolCallBlock key={block.tool.callId} tool={block.tool} onOpenDiff={onOpenDiff} />,
        )
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
            onOpenDiff={onOpenDiff}
          />,
        )
      } else {
        // Single tool — render individually
        elements.push(
          <ToolCallBlock
            key={block.tool.callId}
            tool={block.tool}
            shimmer={isLastToolGroup}
            onOpenDiff={onOpenDiff}
          />,
        )
      }
      i = j
    } else {
      i++
    }
  }

  // Loading dots only when the last block is a tool_call — text / thinking
  // blocks already render their own streaming visual (cursor / pulse) so
  // adding dots here would duplicate the indicator. Tool blocks settle on
  // their result and look static once done, so dots pick up the
  // between-rounds wait visual.
  if (loading && isLast) {
    const lastBlock = blocks[blocks.length - 1]
    if (lastBlock?.type === "tool_call") {
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
