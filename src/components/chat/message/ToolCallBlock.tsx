import { useState, useMemo, useCallback, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import {
  ChevronRight,
  SquareTerminal,
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
  HelpCircle,
  Check,
  Timer,
  ListChecks,
  Settings,
  Wrench,
  Paperclip,
  Cable,
  Plug,
  GitCompare,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { parseMcpToolName } from "@/lib/mcp"
import type { FileChangeMetadata, FileChangesMetadata, ToolCall } from "@/types/chat"
import { IconTip } from "@/components/ui/tooltip"
import SubagentBlock from "@/components/chat/SubagentBlock"
import ToolMediaPreview from "@/components/chat/message/ToolMediaPreview"
import ExecToolResultCard from "@/components/chat/message/ExecToolResultCard"
import AsyncJobCancelCard from "@/components/chat/message/AsyncJobCancelCard"
import { getExecutionToolLabel, getToolExecutionState } from "./executionStatus"

/** Map tool name → Lucide icon component */
const TOOL_ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  read: FileText,
  write: FilePen,
  edit: FilePen,
  ls: FolderOpen,
  exec: SquareTerminal,
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
  task_create: ListChecks,
  task_update: ListChecks,
  task_list: ListChecks,
  get_settings: Settings,
  update_settings: Settings,
  send_attachment: Paperclip,
  acp_spawn: Cable,
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

function truncateEllipsis(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n)}...` : s
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
      case "ask_user_question":
        return parsed.context || `${(parsed.questions || []).length} question(s)`
      case "send_attachment":
        return parsed.display_name || parsed.path || args
      case "task_create": {
        const arr = Array.isArray(parsed.tasks) ? parsed.tasks : []
        if (arr.length === 0) return args
        const first = String(arr[0]?.content ?? "")
        return arr.length === 1
          ? `1 task: ${truncateEllipsis(first, 50)}`
          : `${arr.length} tasks: ${truncateEllipsis(first, 40)}`
      }
      case "task_update": {
        const parts: string[] = [`#${parsed.id}`]
        if (parsed.status) parts.push(`→ ${parsed.status}`)
        return parts.join(" ")
      }
      default:
        return args
    }
  } catch {
    return args
  }
}

interface AskUserAnswer {
  question: string
  selected: string[]
  customInput?: string | null
}

/** Parse the JSON result returned by ask_user_question. */
function parseAskUserAnswers(
  result: string | undefined,
): { answers: AskUserAnswer[]; timedOut: boolean; cancelled: boolean } | null {
  if (!result) return null
  const trimmed = result.trim()
  if (trimmed.startsWith("The user cancelled")) {
    return { answers: [], timedOut: false, cancelled: true }
  }
  try {
    const parsed = JSON.parse(trimmed)
    if (!Array.isArray(parsed?.answers)) return null
    return {
      answers: parsed.answers as AskUserAnswer[],
      timedOut: !!parsed.timedOut,
      cancelled: false,
    }
  } catch {
    return null
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

export interface ToolCallBlockProps {
  tool: ToolCall
  shimmer?: boolean
  /**
   * Open the right-side diff panel for a `file_change` / `file_changes`
   * payload coming from this tool call. Wired up by ChatScreen so the
   * existing folded ToolCallBlock + tool group rendering remain untouched
   * for tools that don't carry diff metadata.
   */
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
}

export default function ToolCallBlock({ tool, shimmer, onOpenDiff }: ToolCallBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [showRaw, setShowRaw] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const state = getToolExecutionState(tool)
  const isRunning = state === "running"
  const isFailed = state === "failed"
  const showActivity = isRunning || shimmer
  const canExpand = tool.name === "exec" || !isRunning
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
  const isMcpTool = parseMcpToolName(tool.name) !== null
  const Icon = skillName
    ? FileCode
    : isMcpTool
      ? Plug
      : TOOL_ICONS[tool.name] || Wrench
  const toolLabel = getExecutionToolLabel({ t, tool, skillName })
  const displayArgs = skillName ? "" : getDisplayArgs(tool.name, tool.arguments)

  // Diff summary and "open diff" button surface only when the tool emitted
  // structured side-output. Legacy rows persisted before the diff panel
  // shipped have `metadata === undefined` and stay on the original layout.
  const fileChangeSummary = useMemo<{
    linesAdded: number
    linesRemoved: number
    payload: FileChangeMetadata | FileChangesMetadata
  } | null>(() => {
    const meta = tool.metadata
    if (!meta) return null
    if (meta.kind === "file_change") {
      return {
        linesAdded: meta.linesAdded,
        linesRemoved: meta.linesRemoved,
        payload: meta,
      }
    }
    if (meta.kind === "file_changes") {
      const totals = meta.changes.reduce(
        (acc, c) => {
          acc.linesAdded += c.linesAdded
          acc.linesRemoved += c.linesRemoved
          return acc
        },
        { linesAdded: 0, linesRemoved: 0 },
      )
      return { ...totals, payload: meta }
    }
    return null
  }, [tool.metadata])

  const handleOpenDiff = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      if (fileChangeSummary && onOpenDiff) {
        onOpenDiff(fileChangeSummary.payload)
      }
    },
    [fileChangeSummary, onOpenDiff],
  )

  const askUserOutcome = useMemo(
    () =>
      tool.name === "ask_user_question"
        ? parseAskUserAnswers(tool.result)
        : null,
    [tool.name, tool.result],
  )

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
        className="flex items-center gap-1.5 w-full pl-0 pr-1 py-1 text-left hover:bg-secondary/60 rounded-md transition-colors group"
        onClick={() => canExpand && setExpanded(!expanded)}
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-200",
            expanded && "rotate-90",
            !canExpand && "opacity-40",
          )}
        />
        <span className="relative h-3.5 w-3.5 shrink-0">
          <Icon className="h-3.5 w-3.5 text-muted-foreground" />
          {showActivity && (
            <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60 ring-1 ring-card animate-pulse" />
          )}
        </span>
        <span
          className={cn(
            "font-medium shrink-0 whitespace-nowrap",
            isFailed ? "text-red-500" : "text-muted-foreground",
            showActivity && "animate-text-shimmer",
          )}
        >
          {toolLabel}
        </span>
        <span className="text-muted-foreground/60 truncate font-mono text-[11px]">
          {displayArgs}
        </span>
        {fileChangeSummary && (
          <span className="ml-auto flex shrink-0 items-center gap-1.5 text-[10px] tabular-nums">
            <span className="text-emerald-600 dark:text-emerald-400">
              +{fileChangeSummary.linesAdded}
            </span>
            <span className="text-rose-600 dark:text-rose-400">
              -{fileChangeSummary.linesRemoved}
            </span>
          </span>
        )}
        {elapsedText && (
          <span
            className={cn(
              "shrink-0 text-[10px] text-muted-foreground/60 tabular-nums",
              !fileChangeSummary && "ml-auto",
            )}
          >
            {t("tools.elapsed", { time: elapsedText })}
          </span>
        )}

        {fileChangeSummary && onOpenDiff && (
          <IconTip label={t("diffPanel.openDiff", "查看 diff")}>
            <span
              role="button"
              className="shrink-0 p-0.5 rounded hover:bg-secondary text-muted-foreground/60 hover:text-muted-foreground transition-colors"
              onClick={handleOpenDiff}
            >
              <GitCompare className="h-3 w-3" />
            </span>
          </IconTip>
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
      <AsyncJobCancelCard result={tool.result} className="ml-5" />
      <ToolMediaPreview tool={tool} className="ml-5" />
      {/* ask_user_question answers card */}
      {askUserOutcome && !isRunning && (
        <div className="ml-5 mt-1.5 mb-1 rounded-md border border-blue-500/20 bg-blue-500/5 px-3 py-2 space-y-1.5 text-xs">
          {askUserOutcome.cancelled ? (
            <div className="text-muted-foreground italic">
              {t("tools.ask_user.cancelled")}
            </div>
          ) : askUserOutcome.answers.length === 0 ? (
            <div className="text-muted-foreground italic">
              {t("tools.ask_user.no_answers")}
            </div>
          ) : (
            askUserOutcome.answers.map((a, i) => {
              const parts: string[] = [...a.selected]
              if (a.customInput) parts.push(a.customInput)
              return (
                <div key={i} className="flex items-start gap-2">
                  <HelpCircle className="h-3 w-3 mt-0.5 text-blue-500 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <div className="text-muted-foreground">{a.question}</div>
                    {parts.length > 0 && (
                      <div className="mt-0.5 flex flex-wrap gap-1">
                        {parts.map((p, j) => (
                          <span
                            key={j}
                            className="inline-flex items-center gap-1 rounded-full bg-blue-500/10 text-blue-600 px-2 py-0.5"
                          >
                            <Check className="h-2.5 w-2.5" />
                            {p}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              )
            })
          )}
          {askUserOutcome.timedOut && (
            <div className="text-[10px] text-amber-600 flex items-center gap-1 pt-0.5">
              <Timer className="h-2.5 w-2.5" />
              {t("tools.ask_user.timed_out")}
            </div>
          )}
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
          expanded && (tool.name === "exec" || tool.result) ? "max-h-[420px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-5 mt-0.5 mb-1">
          {tool.name === "exec" ? (
            <ExecToolResultCard tool={tool} isRunning={isRunning} />
          ) : (
            <pre className="whitespace-pre-wrap text-muted-foreground/80 bg-secondary/40 rounded-md p-2.5 max-h-64 overflow-y-auto text-[11px] leading-relaxed border border-border/50">
              {tool.result}
            </pre>
          )}
        </div>
      </div>
    </div>
  )
}
