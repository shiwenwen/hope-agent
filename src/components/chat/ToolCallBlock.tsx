import { useState } from "react"
import { ChevronDown, ChevronRight, Terminal } from "lucide-react"
import type { ToolCall } from "@/types/chat"

export default function ToolCallBlock({ tool }: { tool: ToolCall }) {
  const [expanded, setExpanded] = useState(false)
  const isRunning = tool.result === undefined
  const displayArgs = (() => {
    try {
      const parsed = JSON.parse(tool.arguments)
      if (tool.name === "exec") return parsed.command
      if (tool.name === "read_file" || tool.name === "list_dir")
        return parsed.path || "."
      if (tool.name === "write_file") return parsed.path
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
        ) : expanded ? (
          <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
        )}
        <Terminal className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-foreground">{tool.name}</span>
        <span className="text-muted-foreground truncate">{displayArgs}</span>
      </button>
      {expanded && tool.result && (
        <div className="px-2.5 pb-2 pt-0.5">
          <pre className="whitespace-pre-wrap text-muted-foreground bg-background rounded p-2 max-h-48 overflow-y-auto text-[11px] leading-relaxed">
            {tool.result}
          </pre>
        </div>
      )}
    </div>
  )
}
