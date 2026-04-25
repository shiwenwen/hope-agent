import type { TFunction } from "i18next"
import type { ToolCall } from "@/types/chat"
import { parseMcpToolName } from "@/lib/mcp"

export type ExecutionState = "running" | "completed" | "failed"
export type ToolCategory = "browse" | "edit" | "search" | "web" | "memory" | "other"
export type ExecutionToolGroupLabelKey = ToolCategory | "skill"

export interface ExecutionToolGroupLabelSegment {
  key: ExecutionToolGroupLabelKey
  label: string
}

const TOOL_CATEGORY_MAP: Record<string, ToolCategory> = {
  read: "browse",
  ls: "browse",
  write: "edit",
  edit: "edit",
  apply_patch: "edit",
  grep: "search",
  find: "search",
  web_search: "web",
  web_fetch: "web",
  save_memory: "memory",
  recall_memory: "memory",
  update_memory: "memory",
  delete_memory: "memory",
  memory_get: "memory",
  image: "browse",
  pdf: "browse",
}

const KNOWN_TOOL_STATUS_NAMES = new Set([
  "exec",
  "process",
  "read",
  "write",
  "edit",
  "ls",
  "grep",
  "find",
  "apply_patch",
  "web_search",
  "web_fetch",
  "save_memory",
  "recall_memory",
  "update_memory",
  "delete_memory",
  "manage_cron",
  "browser",
  "send_notification",
  "subagent",
  "memory_get",
  "agents_list",
  "sessions_list",
  "session_status",
  "sessions_history",
  "sessions_send",
  "image",
  "image_generate",
  "pdf",
  "canvas",
  "task_create",
  "task_update",
  "task_list",
  "get_settings",
  "update_settings",
  "send_attachment",
  "ask_user_question",
  "tool_search",
  "acp_spawn",
  "submit_plan",
  "amend_plan",
  "update_plan_step",
])

export function hasToolError(
  tool: Pick<ToolCall, "isError" | "result">,
): boolean {
  if (tool.isError === true) return true
  if (tool.isError === false) return false
  return typeof tool.result === "string" && tool.result.startsWith("Tool error:")
}

export function getToolExecutionState(
  tool: Pick<ToolCall, "isError" | "result">,
): ExecutionState {
  if (tool.result === undefined) return "running"
  return hasToolError(tool) ? "failed" : "completed"
}

export function getFailedToolCount(tools: ToolCall[]): number {
  return tools.filter((tool) => getToolExecutionState(tool) === "failed").length
}

export function getToolCategory(name: string): ToolCategory {
  return TOOL_CATEGORY_MAP[name] || "other"
}

export function getToolDisplayName(t: TFunction, toolName: string): string {
  // MCP tools are dynamic — format the namespaced identifier into
  // `<server> · <tool>` so the chat UI stays readable.
  const mcp = parseMcpToolName(toolName)
  if (mcp) return `${mcp.serverName} · ${mcp.tool}`
  return String(t(`tools.${toolName}`, { defaultValue: toolName }))
}

export function getExecutionToolLabel(params: {
  t: TFunction
  tool: ToolCall
  skillName?: string | null
}): string {
  const { t, tool, skillName } = params
  const state = getToolExecutionState(tool)

  if (skillName) {
    return String(t(`executionStatus.tool.single.skill_read.${state}`, { name: skillName }))
  }

  const keyBase = KNOWN_TOOL_STATUS_NAMES.has(tool.name)
    ? `executionStatus.tool.single.${tool.name}`
    : "executionStatus.tool.single.fallback"

  return String(
    t(`${keyBase}.${state}`, {
      name: getToolDisplayName(t, tool.name),
    }),
  )
}

function stripRepeatedChineseStatusPrefix(
  segment: string,
  firstSegment: string,
): string {
  if (firstSegment.startsWith("正在") && segment.startsWith("正在")) {
    return segment.slice("正在".length)
  }
  if (firstSegment.startsWith("已") && segment.startsWith("已")) {
    return segment.slice("已".length)
  }
  return segment
}

function joinExecutionToolGroupSegments(segments: string[]): string {
  if (segments.length <= 1) return segments[0] || ""

  return segments.join(getExecutionToolGroupSegmentSeparator(segments))
}

export function getExecutionToolGroupSegmentSeparator(
  segments: readonly string[] | readonly ExecutionToolGroupLabelSegment[],
): string {
  const firstSegment = segments[0]
  const firstLabel = typeof firstSegment === "string" ? firstSegment : firstSegment?.label
  return firstLabel?.startsWith("正在") || firstLabel?.startsWith("已") ? "，" : ", "
}

export function getExecutionToolGroupLabelSegments(
  tools: ToolCall[],
  t: TFunction,
  getSkillName: (tool: ToolCall) => string | null,
): ExecutionToolGroupLabelSegment[] {
  const state: Exclude<ExecutionState, "failed"> = tools.some(
    (tool) => getToolExecutionState(tool) === "running",
  )
    ? "running"
    : "completed"

  const order: ExecutionToolGroupLabelKey[] = []
  const counts = new Map<ExecutionToolGroupLabelKey, number>()
  const skillNames: string[] = []

  for (const tool of tools) {
    const skillName = getSkillName(tool)
    const key: ExecutionToolGroupLabelKey = skillName ? "skill" : getToolCategory(tool.name)
    if (skillName) skillNames.push(skillName)
    if (!counts.has(key)) order.push(key)
    counts.set(key, (counts.get(key) || 0) + 1)
  }

  const rawSegments = order.map((key): ExecutionToolGroupLabelSegment => {
    if (key === "skill") {
      const count = counts.get(key) || 0
      if (count === 1 && skillNames.length === 1) {
        return {
          key,
          label: String(
            t(`executionStatus.tool.group.skill_single.${state}`, {
              count,
              name: skillNames[0],
            }),
          ),
        }
      }
      return {
        key,
        label: String(t(`executionStatus.tool.group.skill.${state}`, { count })),
      }
    }
    return {
      key,
      label: String(t(`executionStatus.tool.group.${key}.${state}`, { count: counts.get(key) || 0 })),
    }
  })

  const firstLabel = rawSegments[0]?.label || ""
  return rawSegments.map((segment, idx) => ({
    ...segment,
    label: idx === 0 ? segment.label : stripRepeatedChineseStatusPrefix(segment.label, firstLabel),
  }))
}

export function getExecutionToolGroupLabel(
  tools: ToolCall[],
  t: TFunction,
  getSkillName: (tool: ToolCall) => string | null,
): string {
  return joinExecutionToolGroupSegments(
    getExecutionToolGroupLabelSegments(tools, t, getSkillName).map((segment) => segment.label),
  )
}
