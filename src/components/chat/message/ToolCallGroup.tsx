import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  ChevronDown,
  FileText,
  FilePen,
  Search,
  Globe,
  Brain,
  Wrench,
  Info,
  AlertCircle,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import { IconTip } from "@/components/ui/tooltip"
import ToolMediaPreview from "@/components/chat/message/ToolMediaPreview"
import {
  getExecutionToolGroupLabel,
  getExecutionToolLabel,
  getFailedToolCount,
  getToolCategory,
  getToolExecutionState,
  type ToolCategory,
} from "./executionStatus"

function formatElapsed(ms: number): string {
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}m ${seconds}s`
}

/** Icon per category */
const CATEGORY_ICONS: Record<ToolCategory, React.ComponentType<{ className?: string }>> = {
  browse: FileText,
  edit: FilePen,
  search: Search,
  web: Globe,
  memory: Brain,
  other: Wrench,
}

/** Check if a read tool call targets a SKILL.md file, return skill name if so */
function getSkillName(tool: ToolCall): string | null {
  if (tool.name !== "read") return null
  try {
    const parsed = JSON.parse(tool.arguments)
    const path: string = parsed.path || ""
    if (path.endsWith("/SKILL.md") || path.endsWith("\\SKILL.md")) {
      const parts = path.replace(/\\/g, "/").split("/")
      return parts.length >= 2 ? parts[parts.length - 2] : "skill"
    }
  } catch { /* ignore */ }
  return null
}

/** Extract the full target path/URL/query from tool arguments */
function getFullTarget(tool: ToolCall): string {
  try {
    const parsed = JSON.parse(tool.arguments)
    return (
      parsed.path || parsed.url || parsed.query || parsed.pattern || parsed.title || parsed.key || tool.name
    )
  } catch {
    return tool.name
  }
}

/** Get a one-line result preview (first non-empty line, truncated) */
function getResultPreview(result: string | undefined, maxLen = 80): string | null {
  if (!result) return null
  const firstLine = result.split("\n").find((l) => l.trim())
  if (!firstLine) return null
  return firstLine.length > maxLen ? firstLine.slice(0, maxLen) + "…" : firstLine
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

/** Get the icon for the most frequent category in a mixed group */
function getPrimaryCategory(tools: ToolCall[]): ToolCategory {
  const counts = new Map<ToolCategory, number>()
  for (const tool of tools) {
    const cat = getToolCategory(tool.name)
    counts.set(cat, (counts.get(cat) || 0) + 1)
  }
  let maxCat: ToolCategory = "other"
  let maxCount = 0
  for (const [cat, count] of counts) {
    if (count > maxCount) {
      maxCat = cat
      maxCount = count
    }
  }
  return maxCat
}

/** Single item inside a group — shows label + expandable result */
function GroupItem({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation()
  const [showResult, setShowResult] = useState(false)
  const [showRaw, setShowRaw] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const state = getToolExecutionState(tool)
  const isRunning = state === "running"
  const isFailed = state === "failed"
  const skillName = getSkillName(tool)
  const fullTarget = skillName ? "" : getFullTarget(tool)
  const toolLabel = getExecutionToolLabel({ t, tool, skillName })
  const preview = skillName ? null : getResultPreview(tool.result)
  const cat = getToolCategory(tool.name)
  const CatIcon = CATEGORY_ICONS[cat]
  const startedAtMs = tool.startedAtMs || 0
  const elapsedMs = tool.durationMs ?? (isRunning && startedAtMs ? now - startedAtMs : undefined)
  const elapsedText = useMemo(
    () => (elapsedMs != null && elapsedMs >= 0 ? formatElapsed(elapsedMs) : null),
    [elapsedMs],
  )

  useEffect(() => {
    if (!isRunning || !startedAtMs) return
    const timer = window.setInterval(() => setNow(Date.now()), 100)
    return () => window.clearInterval(timer)
  }, [isRunning, startedAtMs])

  return (
    <div className="text-[11px]">
      <button
        className="flex items-center gap-1.5 w-full px-1.5 py-0.5 text-left hover:bg-secondary/60 rounded transition-colors group/item"
        onClick={() => !isRunning && tool.result && setShowResult(!showResult)}
      >
        {isRunning ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0 text-muted-foreground/60" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground/40 transition-transform duration-150",
              showResult && "rotate-90",
            )}
          />
        )}
        <CatIcon className="h-3 w-3 shrink-0 text-muted-foreground/40" />
        <span
          className={cn(
            "font-medium shrink-0",
            isFailed ? "text-red-500" : "text-muted-foreground/80",
          )}
        >
          {toolLabel}
        </span>
        <span className="text-muted-foreground/60 truncate font-mono">{fullTarget}</span>
        {/* Inline result preview when collapsed */}
        {!showResult && preview && (
          <span className="text-muted-foreground/30 truncate ml-auto pl-2 max-w-[40%]">
            {preview}
          </span>
        )}
        {elapsedText && (
          <span className="ml-auto shrink-0 text-[10px] text-muted-foreground/60 tabular-nums">
            {t("tools.elapsed", { time: elapsedText })}
          </span>
        )}
        <IconTip label={t("tools.rawCall", "查看原始调用")}>
          <span
            role="button"
            className="shrink-0 p-0.5 rounded hover:bg-secondary text-muted-foreground/40 hover:text-muted-foreground/80 transition-colors opacity-0 group-hover/item:opacity-100"
            onClick={(e) => {
              e.stopPropagation()
              setShowRaw(!showRaw)
            }}
          >
            <Info className="h-3 w-3" />
          </span>
        </IconTip>
      </button>
      <ToolMediaPreview tool={tool} className="ml-4" />
      {/* Raw tool call */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          showRaw ? "max-h-[400px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-4 mt-0.5 mb-1">
          <pre className="whitespace-pre-wrap text-muted-foreground/70 bg-muted/50 rounded-md p-2 max-h-56 overflow-y-auto text-[11px] leading-relaxed border border-border/30 font-mono select-all">
            {formatRawCall(tool)}
          </pre>
        </div>
      </div>
      {/* Full result */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          showResult && tool.result ? "max-h-[400px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-4 mt-0.5 mb-1">
          <pre className="whitespace-pre-wrap text-muted-foreground/70 bg-secondary/40 rounded-md p-2 max-h-56 overflow-y-auto text-[11px] leading-relaxed border border-border/40">
            {tool.result}
          </pre>
        </div>
      </div>
    </div>
  )
}

interface ToolCallGroupProps {
  tools: ToolCall[]
  shimmer?: boolean
}

export default function ToolCallGroup({ tools, shimmer }: ToolCallGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const anyRunning = tools.some((tool) => getToolExecutionState(tool) === "running")
  const failedCount = getFailedToolCount(tools)

  const primaryCategory = getPrimaryCategory(tools)
  const HeaderIcon = CATEGORY_ICONS[primaryCategory]
  const label = getExecutionToolGroupLabel(tools, t, getSkillName)

  // Calculate total elapsed time across all tools in the group
  const totalElapsedMs = useMemo(() => {
    let total = 0
    let hasAny = false
    for (const tool of tools) {
      const isRunning = tool.result === undefined
      const ms = tool.durationMs ?? (isRunning && tool.startedAtMs ? now - tool.startedAtMs : undefined)
      if (ms != null && ms >= 0) {
        total += ms
        hasAny = true
      }
    }
    return hasAny ? total : undefined
  }, [tools, now])

  const totalElapsedText = useMemo(
    () => (totalElapsedMs != null ? formatElapsed(totalElapsedMs) : null),
    [totalElapsedMs],
  )

  // Live-update timer while any tool is still running
  useEffect(() => {
    if (!anyRunning) return
    const timer = window.setInterval(() => setNow(Date.now()), 100)
    return () => window.clearInterval(timer)
  }, [anyRunning])

  return (
    <div className="my-1 text-xs">
      {/* Group header */}
      <button
        className="flex items-center gap-1.5 w-full px-1 py-1 text-left hover:bg-secondary/60 rounded-md transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        {anyRunning ? (
          <span className="animate-spin h-3.5 w-3.5 border-[1.5px] border-current border-t-transparent rounded-full shrink-0 text-muted-foreground" />
        ) : expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        )}
        <HeaderIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className={cn("text-muted-foreground font-medium", (anyRunning || shimmer) && "animate-text-shimmer")}>{label}</span>
        {failedCount > 0 && (
          <span className="shrink-0 rounded-full bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-500">
            <span className="inline-flex items-center gap-0.5">
              <AlertCircle className="h-3 w-3" />
              {t("executionStatus.tool.group.failedCount", { count: failedCount })}
            </span>
          </span>
        )}
        {totalElapsedText && (
          <span className="ml-auto shrink-0 text-[10px] text-muted-foreground/60 tabular-nums">
            {t("tools.elapsed", { time: totalElapsedText })}
          </span>
        )}
      </button>

      {/* Expanded: show each item with inline result access */}
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded ? "max-h-[3000px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-3 border-l border-border/40 pl-0.5">
          {tools.map((tool) => (
            <GroupItem key={tool.callId} tool={tool} />
          ))}
        </div>
      </div>
    </div>
  )
}
