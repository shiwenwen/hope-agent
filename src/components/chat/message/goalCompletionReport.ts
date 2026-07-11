import type { Message } from "@/types/chat"

export interface GoalCompletionReport {
  status?: string
  state?: string
  summary?: string
  usage?: {
    tokensUsed?: number
    elapsedSecs?: number
    turnsUsed?: number
  }
  evidenceCount?: number
}

export function parseGoalCompletionReportFromToolResult(
  result: string | undefined,
): GoalCompletionReport | null {
  if (!result) return null
  try {
    const parsed = JSON.parse(result) as {
      ok?: boolean
      status?: string
      report?: GoalCompletionReport
    }
    if (parsed.ok !== true || parsed.status !== "completed" || !parsed.report) return null
    return parsed.report
  } catch {
    return null
  }
}

export function goalCompletionReportFromMessage(msg: Message): GoalCompletionReport | null {
  if (msg.role !== "assistant") return null
  const candidates = [
    ...(msg.toolCalls ?? []),
    ...(msg.contentBlocks ?? [])
      .filter((block) => block.type === "tool_call")
      .map((block) => block.tool),
  ]
  for (let i = candidates.length - 1; i >= 0; i -= 1) {
    const tool = candidates[i]
    if (tool.name !== "goal_finish_request") continue
    const report = parseGoalCompletionReportFromToolResult(tool.result)
    if (report) return report
  }
  return null
}
