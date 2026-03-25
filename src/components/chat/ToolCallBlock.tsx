import { useState, useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  Terminal,
  FileText,
  FilePen,
  FolderOpen,
  Search,
  FileSearch,
  FileCode,
  Globe,
  Brain,
  Clock,
  Monitor,
  Bell,
  Network,
  Cpu,
  MessageSquare,
  List,
  History,
  Activity,
  Users,
  Image,
  ImagePlus,
  PanelRight,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import SubagentBlock from "@/components/chat/SubagentBlock"

/** Map tool name → Lucide icon component */
const TOOL_ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  read: FileText,
  write: FilePen,
  edit: FilePen,
  ls: FolderOpen,
  exec: Terminal,
  process: Cpu,
  grep: Search,
  find: FileSearch,
  apply_patch: FileCode,
  web_search: Globe,
  web_fetch: Globe,
  save_memory: Brain,
  recall_memory: Brain,
  update_memory: Brain,
  delete_memory: Brain,
  manage_cron: Clock,
  browser: Monitor,
  send_notification: Bell,
  subagent: Network,
  memory_get: Brain,
  agents_list: Users,
  sessions_list: List,
  session_status: Activity,
  sessions_history: History,
  sessions_send: MessageSquare,
  image: Image,
  image_generate: ImagePlus,
  pdf: FileText,
  canvas: PanelRight,
}

/** Extract a short, human-friendly summary of tool arguments */
function getDisplayArgs(name: string, args: string): string {
  try {
    const parsed = JSON.parse(args)
    switch (name) {
      case "exec":
        return parsed.command || args
      case "read":
      case "ls":
        return parsed.path || "."
      case "write":
      case "edit":
        return parsed.path || args
      case "find":
        return parsed.pattern
          ? `${parsed.path || "."} → ${parsed.pattern}`
          : parsed.path || args
      case "grep":
        return parsed.pattern
          ? `"${parsed.pattern}"${parsed.path ? ` in ${parsed.path}` : ""}`
          : args
      case "apply_patch":
        return parsed.path || args
      case "web_search":
        return parsed.query || args
      case "web_fetch":
        return parsed.url || args
      case "save_memory":
      case "update_memory":
        return parsed.title || parsed.key || args
      case "recall_memory":
        return parsed.query || args
      case "delete_memory":
        return parsed.id || parsed.key || args
      case "manage_cron":
        return parsed.action || args
      case "browser":
        return parsed.action || args
      case "send_notification":
        return parsed.title || args
      case "subagent":
        return `${parsed.action}${parsed.run_id ? ` ${parsed.run_id}` : ""}`
      case "memory_get":
        return `id: ${parsed.id}`
      case "agents_list":
        return ""
      case "sessions_list":
        return parsed.agent_id ? `agent: ${parsed.agent_id}` : "all"
      case "session_status":
      case "sessions_history":
        return parsed.session_id || args
      case "sessions_send":
        return parsed.session_id || args
      case "image":
        return parsed.path || args
      case "image_generate":
        return parsed.prompt
          ? parsed.prompt.length > 60
            ? `${parsed.prompt.slice(0, 60)}...`
            : parsed.prompt
          : args
      case "pdf":
        return parsed.path || args
      case "canvas":
        return `${parsed.action || ""}${parsed.title ? ` "${parsed.title}"` : ""}${parsed.project_id ? ` (${parsed.project_id.slice(0, 8)})` : ""}`
      default:
        return args
    }
  } catch {
    return args
  }
}

export default function ToolCallBlock({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const isRunning = tool.result === undefined

  // Detect subagent spawn — render SubagentBlock instead
  const subagentSpawn = useMemo(() => {
    if (tool.name !== "subagent") return null
    try {
      const args = JSON.parse(tool.arguments)
      if (args.action !== "spawn") return null
      let runId: string | undefined
      if (tool.result) {
        try {
          const res = JSON.parse(tool.result)
          runId = res.run_id
        } catch {
          /* ignore */
        }
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

  const Icon = TOOL_ICONS[tool.name] || Terminal
  const toolLabel = t(`tools.${tool.name}`, tool.name)
  const displayArgs = getDisplayArgs(tool.name, tool.arguments)

  return (
    <div className="my-1 text-xs">
      <button
        className="flex items-center gap-1.5 w-full px-1 py-1 text-left hover:bg-secondary/60 rounded-md transition-colors group"
        onClick={() => !isRunning && setExpanded(!expanded)}
      >
        {isRunning ? (
          <span className="animate-spin h-3.5 w-3.5 border-[1.5px] border-current border-t-transparent rounded-full shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-200",
              expanded && "rotate-90",
            )}
          />
        )}
        <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground font-medium">{toolLabel}</span>
        <span className="text-muted-foreground/60 truncate font-mono text-[11px]">
          {displayArgs}
        </span>
      </button>
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded && tool.result ? "max-h-[400px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-5 mt-0.5 mb-1">
          <pre className="whitespace-pre-wrap text-muted-foreground/80 bg-secondary/40 rounded-md p-2.5 max-h-64 overflow-y-auto text-[11px] leading-relaxed border border-border/50">
            {tool.result}
          </pre>
        </div>
      </div>
    </div>
  )
}
