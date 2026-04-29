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
  Puzzle,
  GitCompare,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { FileChangeMetadata, FileChangesMetadata, ToolCall } from "@/types/chat"
import { IconTip } from "@/components/ui/tooltip"
import ToolMediaPreview from "@/components/chat/message/ToolMediaPreview"
import ExecToolResultCard from "@/components/chat/message/ExecToolResultCard"
import AsyncJobCancelCard from "@/components/chat/message/AsyncJobCancelCard"
import {
  getExecutionToolGroupLabelSegments,
  getExecutionToolGroupSegmentSeparator,
  getExecutionToolLabel,
  getFailedToolCount,
  getToolCategory,
  getToolExecutionState,
  type ExecutionToolGroupLabelKey,
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

const GROUP_ICONS: Record<ExecutionToolGroupLabelKey, React.ComponentType<{ className?: string }>> = {
  ...CATEGORY_ICONS,
  skill: Puzzle,
}

const DIGITS = Array.from("0123456789")

function RollingNumber({ value }: { value: string }) {
  const slotCount = Math.max(2, value.length)
  const padded = value.padStart(slotCount, "0")
  const firstVisibleIndex = padded.length - value.length

  return (
    <span className="tool-count-number" role="text" aria-label={value}>
      {Array.from(padded).map((digit, idx) => {
        const digitValue = Number(digit)
        const isPlaceholder = idx < firstVisibleIndex

        return (
          <span
            key={`${slotCount}-${idx}`}
            className={cn(
              "tool-count-digit",
              isPlaceholder && "tool-count-digit-placeholder",
            )}
            aria-hidden="true"
          >
            <span
              className="tool-count-digit-reel"
              style={{ transform: `translateY(-${digitValue}em)` }}
            >
              {DIGITS.map((item) => (
                <span key={item}>{item}</span>
              ))}
            </span>
          </span>
        )
      })}
    </span>
  )
}

function StableNumericLabel({ text }: { text: string }) {
  const parts = text.split(/(\d+)/g)

  return (
    <>
      {parts.map((part, idx) => {
        if (/^\d+$/.test(part)) {
          return <RollingNumber key={idx} value={part} />
        }
        return part
      })}
    </>
  )
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

/** Single item inside a group — shows label + expandable result */
function GroupItem({
  tool,
  onOpenDiff,
}: {
  tool: ToolCall
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
}) {
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
  const canExpand = tool.name === "exec" || (!isRunning && !!tool.result)

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

  useEffect(() => {
    if (!isRunning || !startedAtMs) return
    const timer = window.setInterval(() => setNow(Date.now()), 100)
    return () => window.clearInterval(timer)
  }, [isRunning, startedAtMs])

  return (
    <div className="text-[11px]">
      <button
        className="flex items-center gap-1.5 w-full px-1.5 py-0.5 text-left hover:bg-secondary/60 rounded transition-colors group/item"
        onClick={() => canExpand && setShowResult(!showResult)}
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 shrink-0 text-muted-foreground/40 transition-transform duration-150",
            showResult && "rotate-90",
            !canExpand && "opacity-40",
          )}
        />
        <span className="relative h-3 w-3 shrink-0">
          <CatIcon className="h-3 w-3 text-muted-foreground/40" />
          {isRunning && (
            <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60 ring-1 ring-card animate-pulse" />
          )}
        </span>
        <span
          className={cn(
            "font-medium shrink-0",
            isFailed ? "text-red-500" : "text-muted-foreground/80",
            isRunning && "animate-text-shimmer",
          )}
        >
          {toolLabel}
        </span>
        <span className="text-muted-foreground/60 truncate font-mono">{fullTarget}</span>
        {/* Inline result preview when collapsed */}
        {!showResult && preview && !fileChangeSummary && (
          <span className="text-muted-foreground/30 truncate ml-auto pl-2 max-w-[40%]">
            {preview}
          </span>
        )}
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
              onClick={(e) => {
                e.stopPropagation()
                onOpenDiff(fileChangeSummary.payload)
              }}
            >
              <GitCompare className="h-3 w-3" />
            </span>
          </IconTip>
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
      <AsyncJobCancelCard result={tool.result} className="ml-4" />
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
          showResult && (tool.name === "exec" || tool.result) ? "max-h-[420px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-4 mt-0.5 mb-1">
          {tool.name === "exec" ? (
            <ExecToolResultCard tool={tool} isRunning={isRunning} />
          ) : (
            <pre className="whitespace-pre-wrap text-muted-foreground/70 bg-secondary/40 rounded-md p-2 max-h-56 overflow-y-auto text-[11px] leading-relaxed border border-border/40">
              {tool.result}
            </pre>
          )}
        </div>
      </div>
    </div>
  )
}

interface ToolCallGroupProps {
  tools: ToolCall[]
  shimmer?: boolean
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
}

export default function ToolCallGroup({ tools, shimmer, onOpenDiff }: ToolCallGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const anyRunning = tools.some((tool) => getToolExecutionState(tool) === "running")
  const showActivity = anyRunning || shimmer
  const failedCount = getFailedToolCount(tools)

  const labelSegments = getExecutionToolGroupLabelSegments(tools, t, getSkillName)
  const labelSeparator = getExecutionToolGroupSegmentSeparator(labelSegments)

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
        {expanded ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        )}
        <span
          className={cn(
            "flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-muted-foreground font-medium",
            showActivity && "animate-text-shimmer",
          )}
        >
          {labelSegments.map((segment, idx) => {
            const SegmentIcon = GROUP_ICONS[segment.key]
            return (
              <span key={`${segment.key}-${idx}`} className="inline-flex min-w-0 items-center gap-1">
                {idx > 0 && (
                  <span className="text-muted-foreground/50">
                    {labelSeparator.trim()}
                  </span>
                )}
                <span className="relative h-3.5 w-3.5 shrink-0">
                  <SegmentIcon className="h-3.5 w-3.5 text-muted-foreground" />
                  {showActivity && idx === 0 && (
                    <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-muted-foreground/60 ring-1 ring-card animate-pulse" />
                  )}
                </span>
                <span className="whitespace-nowrap">
                  <StableNumericLabel text={segment.label} />
                </span>
              </span>
            )
          })}
        </span>
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
            <GroupItem key={tool.callId} tool={tool} onOpenDiff={onOpenDiff} />
          ))}
        </div>
      </div>
    </div>
  )
}
