import React from "react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import ToolCallBlock from "./ToolCallBlock"
import ToolCallGroup from "./ToolCallGroup"
import ThinkingBlock from "./ThinkingBlock"
import TaskBlock from "./TaskBlock"
import ProcessedBlockGroup from "./ProcessedBlockGroup"
import InterruptedMark from "./InterruptedMark"
import SubagentGroup, { type SubagentGroupRun } from "@/components/chat/SubagentGroup"
import SubagentBlock from "@/components/chat/SubagentBlock"
import SkillProgressBlock from "@/components/chat/SkillProgressBlock"
import { AskUserQuestionResult, SubmitPlanResult } from "./PlanResultBlocks"
import { TASK_TOOL_NAMES } from "@/components/chat/tasks/taskProgress"
import { getFailedToolCount, getToolExecutionState } from "./executionStatus"
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

// Synthesize ContentBlock[] from legacy `msg.thinking` / `msg.toolCalls` /
// `msg.content` when the server hasn't sent finalized contentBlocks yet
// (during streaming). This lets MessageBubble always render through the
// content-blocks code path, avoiding the unmount/remount flicker that used
// to happen at stream_end when the renderer switched from
// AssistantLegacyContent to AssistantContentBlocks.
function synthesizeBlocks(msg: Message): ContentBlock[] {
  const blocks: ContentBlock[] = []
  if (msg.thinking) blocks.push({ type: "thinking", content: msg.thinking })
  if (msg.toolCalls && msg.toolCalls.length > 0) {
    for (const tool of msg.toolCalls) blocks.push({ type: "tool_call", tool })
  }
  if (msg.content) blocks.push({ type: "text", content: msg.content })
  return blocks
}

interface RenderUnit {
  key: string
  node: React.ReactNode
  processTools?: ToolCall[]
  isProcessComplete?: boolean
}

function processUnitToolsComplete(tools: ToolCall[]): boolean {
  return tools.every((tool) => getToolExecutionState(tool) !== "running")
}

function collapseProcessedUnits(units: RenderUnit[]): React.ReactNode[] {
  const nodes: React.ReactNode[] = []
  let i = 0

  while (i < units.length) {
    const unit = units[i]
    if (!unit.isProcessComplete) {
      nodes.push(unit.node)
      i++
      continue
    }

    const group: RenderUnit[] = [unit]
    let j = i + 1
    while (j < units.length && units[j].isProcessComplete) {
      group.push(units[j])
      j++
    }

    if (group.length >= 2) {
      const tools = group.flatMap((item) => item.processTools || [])
      nodes.push(
        <ProcessedBlockGroup
          key={`processed-${group[0].key}`}
          failedCount={getFailedToolCount(tools)}
        >
          {group.map((item) => (
            <React.Fragment key={item.key}>{item.node}</React.Fragment>
          ))}
        </ProcessedBlockGroup>,
      )
    } else {
      nodes.push(unit.node)
    }

    i = j
  }

  return nodes
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
  const blocks =
    msg.contentBlocks && msg.contentBlocks.length > 0
      ? msg.contentBlocks
      : synthesizeBlocks(msg)

  // Streaming pre-first-token: no content yet, no tool call yet → show dots
  // placeholder. Dimensions are tuned to match the box of a one-line `<p>`
  // that the markdown renderer will draw the moment the first token lands —
  // `h-[1.625em]` mirrors text-sm leading-relaxed line-height (22.75px) and
  // `my-1.5` mirrors `.markdown-content p { margin-block: 0.375rem }`. Dot
  // size + gap shrunk so the loading-state bubble width is closer to a
  // first-char text bubble, minimizing the perceptible width "shrink" at
  // the dots → first-token transition.
  if (blocks.length === 0) {
    if (loading && isLast) {
      return (
        <div className="flex items-center gap-1 h-[1.625em] my-1.5">
          <span className="block w-1.5 h-1.5 aspect-square rounded-full bg-foreground/70 animate-bounce-pulse" />
          <span className="block w-1.5 h-1.5 aspect-square rounded-full bg-foreground/70 animate-bounce-pulse [animation-delay:200ms]" />
          <span className="block w-1.5 h-1.5 aspect-square rounded-full bg-foreground/70 animate-bounce-pulse [animation-delay:400ms]" />
        </div>
      )
    }
    return null
  }

  const units: RenderUnit[] = []
  const isStreamingMessage = loading && isLast

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
      units.push({
        key: `thinking-${i}`,
        isProcessComplete: !isStreamingMessage,
        node: (
          <ThinkingBlock
            key={i}
            content={block.content}
            isStreaming={loading && isLast && isLastBlock}
            durationMs={block.durationMs}
            interrupted={block.interrupted}
          />
        ),
      })
      i++
    } else if (block.type === "text") {
      units.push({
        key: `text-${i}`,
        node: (
          <div key={i}>
            <MarkdownRenderer
              content={block.content}
              isStreaming={loading && isLast && i === blocks.length - 1}
            />
            {block.interrupted ? <InterruptedMark /> : null}
          </div>
        ),
      })
      i++
    } else if (block.type === "tool_call") {
      // ask_user_question — passive indicator on the timeline. The actual
      // dialog is dispatched via a separate event channel, so the card here
      // is just for the user to see "model asked a question" while the answer
      // is still pending, then "answered" once the result arrives.
      if (block.tool.name === "ask_user_question") {
        units.push({
          key: block.tool.callId,
          node: (
            <AskUserQuestionResult
              key={block.tool.callId}
              result={block.tool.result}
              pending={!block.tool.result}
            />
          ),
        })
        i++
        continue
      }
      if (TASK_TOOL_NAMES.has(block.tool.name)) {
        if (i === firstTaskIdx && latestTaskTool) {
          units.push({
            key: latestTaskTool.callId,
            node: <TaskBlock key={latestTaskTool.callId} tool={latestTaskTool} />,
          })
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
        units.push({
          key: block.tool.callId,
          node: (
            <SubmitPlanResult
              key={block.tool.callId}
              title={title}
              sessionId={sessionId}
              onOpenPanel={onOpenPlanPanel}
              pending={!block.tool.result}
            />
          ),
        })
        i++
        continue
      }
      // skill activation → dedicated Puzzle-iconed block (covers both inline
      // and fork modes; fork detection happens inside the component by
      // looking at the tool_result prefix).
      if (block.tool.name === "skill") {
        const isLastTool = loading && isLast && i === blocks.length - 1
        units.push({
          key: block.tool.callId,
          node: (
            <SkillProgressBlock key={block.tool.callId} tool={block.tool} shimmer={isLastTool} />
          ),
        })
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
            units.push({
              key: groupKey,
              node: (
                <SubagentGroup key={groupKey} runs={runs} onSwitchSession={onSwitchSession} />
              ),
            })
          } else {
            // Single run (plain spawn, spawn_and_wait, or batch_spawn w/ 1 task)
            // → render SubagentBlock directly so batch_spawn's single case
            // also gets the rich UI (the legacy ToolCallBlock path only
            // detects action="spawn").
            const run = runs[0]
            units.push({
              key: run.runId,
              node: (
                <SubagentBlock
                  key={run.runId}
                  runId={run.runId}
                  agentId={run.agentId}
                  task={run.task}
                  onSwitchSession={onSwitchSession}
                />
              ),
            })
          }
          i = j
          continue
        }
        // Non-spawn-like subagent action (check / list / kill / steer / etc)
        // or spawn still in-flight without a run_id yet → render individually.
        // NO_GROUP_TOOLS prevents it from falling into the generic tool-call
        // group below.
        units.push({
          key: block.tool.callId,
          node: (
            <ToolCallBlock key={block.tool.callId} tool={block.tool} onOpenDiff={onOpenDiff} />
          ),
        })
        i++
        continue
      }
      if (NO_GROUP_TOOLS.has(block.tool.name)) {
        units.push({
          key: block.tool.callId,
          node: (
            <ToolCallBlock key={block.tool.callId} tool={block.tool} onOpenDiff={onOpenDiff} />
          ),
        })
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
        units.push({
          key: `grp-${tools[0].callId}`,
          processTools: tools,
          isProcessComplete: !isStreamingMessage && processUnitToolsComplete(tools),
          node: (
            <ToolCallGroup
              key={`grp-${tools[0].callId}`}
              tools={tools}
              shimmer={isLastToolGroup}
              onOpenDiff={onOpenDiff}
            />
          ),
        })
      } else {
        // Single tool — render individually
        units.push({
          key: block.tool.callId,
          processTools: [block.tool],
          isProcessComplete:
            !isStreamingMessage && getToolExecutionState(block.tool) !== "running",
          node: (
            <ToolCallBlock
              key={block.tool.callId}
              tool={block.tool}
              shimmer={isLastToolGroup}
              onOpenDiff={onOpenDiff}
            />
          ),
        })
      }
      i = j
    } else {
      i++
    }
  }

  // text / thinking blocks render their own streaming visual; tool_call blocks
  // go static once result lands, so dots fill the between-rounds wait there.
  if (loading && isLast) {
    const lastBlock = blocks[blocks.length - 1]
    if (lastBlock?.type === "tool_call") {
      units.push({
        key: "__loading__",
        node: (
          <div key="__loading__" className="flex items-center gap-1 py-1 px-2">
            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse" />
            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:300ms]" />
            <span className="block w-1.5 h-1.5 rounded-full bg-foreground/50 animate-pulse [animation-delay:600ms]" />
          </div>
        ),
      })
    }
  }

  return <>{collapseProcessedUnits(units)}</>
}
