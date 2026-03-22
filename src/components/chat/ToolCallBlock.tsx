import { useState, useMemo } from "react"
import { ChevronRight, Terminal } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import SubagentBlock from "@/components/chat/SubagentBlock"

export default function ToolCallBlock({ tool }: { tool: ToolCall }) {
  const [expanded, setExpanded] = useState(false)
  const isRunning = tool.result === undefined

  // Detect subagent spawn — render SubagentBlock instead
  const subagentSpawn = useMemo(() => {
    if (tool.name !== "subagent") return null
    try {
      const args = JSON.parse(tool.arguments)
      if (args.action !== "spawn") return null
      // Extract run_id from tool result
      let runId: string | undefined
      if (tool.result) {
        try {
          const res = JSON.parse(tool.result)
          runId = res.run_id
        } catch { /* ignore */ }
      }
      return { agentId: args.agent_id || "default", task: args.task || "", runId }
    } catch {
      return null
    }
  }, [tool.name, tool.arguments, tool.result])

  if (subagentSpawn?.runId) {
    return (
      <SubagentBlock
        runId={subagentSpawn.runId}
        agentId={subagentSpawn.agentId}
        task={subagentSpawn.task}
      />
    )
  }

  const displayArgs = (() => {
    try {
      const parsed = JSON.parse(tool.arguments)
      if (tool.name === "exec") return parsed.command
      if (tool.name === "read_file" || tool.name === "list_dir")
        return parsed.path || "."
      if (tool.name === "write_file") return parsed.path
      if (tool.name === "subagent") return `${parsed.action}${parsed.run_id ? ` ${parsed.run_id}` : ""}`
      return tool.arguments
    } catch {
      return tool.arguments
    }
  })()

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/50 text-xs">
      <button
        className="flex items-center gap-1.5 w-full px-2.5 py-1.5 text-left hover:bg-secondary/80 rounded-lg transition-colors"
        onClick={() => !isRunning && setExpanded(!expanded)}
      >
        {isRunning ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90"
            )}
          />
        )}
        <Terminal className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-foreground">{tool.name}</span>
        <span className="text-muted-foreground truncate">{displayArgs}</span>
      </button>
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded && tool.result ? "max-h-[300px] opacity-100" : "max-h-0 opacity-0"
        )}
      >
        <div className="px-2.5 pb-2 pt-0.5">
          <pre className="whitespace-pre-wrap text-muted-foreground bg-background rounded p-2 max-h-48 overflow-y-auto text-[11px] leading-relaxed">
            {tool.result}
          </pre>
        </div>
      </div>
    </div>
  )
}
