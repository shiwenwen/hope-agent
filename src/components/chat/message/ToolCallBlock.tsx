import { useState, useMemo, useCallback, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { convertFileSrc } from "@tauri-apps/api/core"
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
  Info,
  ExternalLink,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import { IconTip } from "@/components/ui/tooltip"
import SubagentBlock from "@/components/chat/SubagentBlock"
import { useLightbox } from "@/components/common/ImageLightbox"

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

/** Check if a read tool call targets a SKILL.md file, return skill name if so */
function getSkillName(name: string, args: string): string | null {
  if (name !== "read") return null
  try {
    const parsed = JSON.parse(args)
    const path: string = parsed.path || ""
    if (path.endsWith("/SKILL.md") || path.endsWith("\\SKILL.md")) {
      // Extract skill name from parent directory: .../skills/apple-notes/SKILL.md → apple-notes
      const parts = path.replace(/\\/g, "/").split("/")
      return parts.length >= 2 ? parts[parts.length - 2] : "skill"
    }
  } catch { /* ignore */ }
  return null
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

/** Format the raw tool call as `name(args)` for display */
function formatRawCall(tool: ToolCall): string {
  try {
    const pretty = JSON.stringify(JSON.parse(tool.arguments), null, 2)
    return `${tool.name}(${pretty})`
  } catch {
    return `${tool.name}(${tool.arguments})`
  }
}

export default function ToolCallBlock({ tool, shimmer }: { tool: ToolCall; shimmer?: boolean }) {
  const { t } = useTranslation()
  const { openLightbox } = useLightbox()
  const [expanded, setExpanded] = useState(false)
  const [showRaw, setShowRaw] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const isRunning = tool.result === undefined
  const startedAtMs = tool.startedAtMs || 0
  const elapsedMs = tool.durationMs ?? (isRunning && startedAtMs ? now - startedAtMs : undefined)
  const elapsedText = useMemo(() => {
    if (elapsedMs == null || elapsedMs < 0) return null
    if (elapsedMs < 60_000) return `${(elapsedMs / 1000).toFixed(1)}s`
    const totalSeconds = Math.floor(elapsedMs / 1000)
    const minutes = Math.floor(totalSeconds / 60)
    const seconds = totalSeconds % 60
    return `${minutes}m ${seconds}s`
  }, [elapsedMs])

  useEffect(() => {
    if (!isRunning || !startedAtMs) return
    const timer = window.setInterval(() => setNow(Date.now()), 100)
    return () => window.clearInterval(timer)
  }, [isRunning, startedAtMs])

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

  const skillName = getSkillName(tool.name, tool.arguments)
  const Icon = skillName ? FileCode : (TOOL_ICONS[tool.name] || Terminal)
  const toolLabel = skillName
    ? t("tools.loadingSkill", { name: skillName })
    : t(`tools.${tool.name}`, tool.name)
  const displayArgs = skillName ? "" : getDisplayArgs(tool.name, tool.arguments)

  // Canvas reopen logic
  const canvasInfo = useMemo(() => {
    if (tool.name !== "canvas") return null
    try {
      const args = JSON.parse(tool.arguments)
      const action = args.action
      if (!["create", "update", "show", "restore"].includes(action)) return null
      let projectId: string | null = null
      let title = args.title || ""
      const contentType = args.content_type || ""
      // For create, project_id and title may be in the result
      if (action === "create" && tool.result) {
        try {
          const res = JSON.parse(tool.result)
          projectId = res.project_id || null
          if (res.title) title = res.title
        } catch { /* ignore */ }
      } else {
        projectId = args.project_id || null
      }
      if (!projectId) return null
      return { projectId, title, contentType }
    } catch { return null }
  }, [tool.name, tool.arguments, tool.result])

  const handleOpenCanvas = useCallback(async () => {
    if (!canvasInfo) return
    try {
      await getTransport().call("show_canvas_panel", { projectId: canvasInfo.projectId })
    } catch {
      // Project may have been deleted
    }
  }, [canvasInfo])

  if (subagentSpawn?.runId) {
    return (
      <SubagentBlock
        runId={subagentSpawn.runId}
        agentId={subagentSpawn.agentId}
        task={subagentSpawn.task}
      />
    )
  }

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
        <span className={cn("text-muted-foreground font-medium shrink-0 whitespace-nowrap", (isRunning || shimmer) && "animate-text-shimmer")}>{toolLabel}</span>
        <span className="text-muted-foreground/60 truncate font-mono text-[11px]">
          {displayArgs}
        </span>
        {elapsedText && (
          <span className="ml-auto shrink-0 text-[10px] text-muted-foreground/60 tabular-nums">
            {t("tools.elapsed", { time: elapsedText })}
          </span>
        )}

        <IconTip label={t("tools.rawCall", "查看原始调用")}>
          <span
            role="button"
            className="shrink-0 p-0.5 rounded hover:bg-secondary text-muted-foreground/40 hover:text-muted-foreground/80 transition-colors opacity-0 group-hover:opacity-100"
            onClick={(e) => {
              e.stopPropagation()
              setShowRaw(!showRaw)
            }}
          >
            <Info className="h-3 w-3" />
          </span>
        </IconTip>
      </button>
      {/* Media images (e.g. from image_generate) */}
      {tool.mediaUrls && tool.mediaUrls.length > 0 && (
        <div className="ml-5 mt-1.5 mb-1 flex flex-wrap gap-2">
          {tool.mediaUrls.map((url, i) => (
            <button
              key={i}
              type="button"
              onClick={() => openLightbox(convertFileSrc(url), `Generated image ${i + 1}`)}
              className="block rounded-lg overflow-hidden border border-border/50 hover:border-primary/40 transition-colors cursor-zoom-in"
            >
              <img
                src={convertFileSrc(url)}
                alt={`Generated image ${i + 1}`}
                className="max-w-72 max-h-72 object-contain bg-secondary/30"
                loading="lazy"
              />
            </button>
          ))}
        </div>
      )}
      {/* Canvas preview card */}
      {canvasInfo && !isRunning && (
        <div className="ml-5 mt-1.5 mb-1">
          <button
            type="button"
            onClick={handleOpenCanvas}
            className="flex items-center gap-2.5 px-3 py-2 rounded-lg border border-border/50 hover:border-primary/40 bg-secondary/30 hover:bg-secondary/50 transition-colors cursor-pointer group/canvas"
          >
            <PanelRight className="h-4 w-4 shrink-0 text-primary/70" />
            <div className="flex flex-col items-start gap-0.5 min-w-0">
              <span className="text-xs font-medium text-foreground truncate max-w-[200px]">
                {canvasInfo.title || "Canvas"}
              </span>
              {canvasInfo.contentType && (
                <span className="text-[10px] text-muted-foreground/60 uppercase tracking-wider">
                  {canvasInfo.contentType}
                </span>
              )}
            </div>
            <ExternalLink className="h-3 w-3 shrink-0 text-muted-foreground/40 group-hover/canvas:text-primary/60 transition-colors ml-auto" />
          </button>
        </div>
      )}
      {/* Raw tool call */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          showRaw ? "max-h-[400px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-5 mt-0.5 mb-1">
          <pre className="whitespace-pre-wrap text-muted-foreground/70 bg-muted/50 rounded-md p-2.5 max-h-64 overflow-y-auto text-[11px] leading-relaxed border border-border/30 font-mono select-all">
            {formatRawCall(tool)}
          </pre>
        </div>
      </div>
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
