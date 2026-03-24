import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  ChevronDown,
  FileText,
  FilePen,
  Search,
  Globe,
  Brain,
  Terminal,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"

/** Grouping categories */
export type ToolCategory = "browse" | "edit" | "search" | "web" | "memory" | "other"

const CATEGORY_MAP: Record<string, ToolCategory> = {
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
}

export function getToolCategory(name: string): ToolCategory {
  return CATEGORY_MAP[name] || "other"
}

/** Icon per category */
const CATEGORY_ICONS: Record<ToolCategory, React.ComponentType<{ className?: string }>> = {
  browse: FileText,
  edit: FilePen,
  search: Search,
  web: Globe,
  memory: Brain,
  other: Terminal,
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

/** Extract a short filename from a full path */
function getShortName(full: string): string {
  if (full.includes("/")) {
    return full.split("/").pop() || full
  }
  return full
}

/** Get a one-line result preview (first non-empty line, truncated) */
function getResultPreview(result: string | undefined, maxLen = 80): string | null {
  if (!result) return null
  const firstLine = result.split("\n").find((l) => l.trim())
  if (!firstLine) return null
  return firstLine.length > maxLen ? firstLine.slice(0, maxLen) + "…" : firstLine
}

/** Single item inside a group — shows label + expandable result */
function GroupItem({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation()
  const [showResult, setShowResult] = useState(false)
  const isRunning = tool.result === undefined
  const fullTarget = getFullTarget(tool)
  const toolLabel = t(`tools.${tool.name}`, tool.name)
  const preview = getResultPreview(tool.result)

  return (
    <div className="text-[11px]">
      <button
        className="flex items-center gap-1.5 w-full px-1.5 py-0.5 text-left hover:bg-secondary/60 rounded transition-colors"
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
        <span className="text-muted-foreground/80 font-medium shrink-0">{toolLabel}</span>
        <span className="text-muted-foreground/60 truncate font-mono">{fullTarget}</span>
        {/* Inline result preview when collapsed */}
        {!showResult && preview && (
          <span className="text-muted-foreground/30 truncate ml-auto pl-2 max-w-[40%]">
            {preview}
          </span>
        )}
      </button>
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
  category: ToolCategory
  tools: ToolCall[]
}

export default function ToolCallGroup({ category, tools }: ToolCallGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const anyRunning = tools.some((tc) => tc.result === undefined)

  const Icon = CATEGORY_ICONS[category]
  const count = tools.length
  const label = t(`toolGroup.${category}`, { count })

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
        <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground font-medium">{label}</span>
      </button>

      {/* Collapsed: show target file/URL list directly */}
      {!expanded && (
        <div className="ml-6 mt-0.5 space-y-px">
          {tools.map((tool) => {
            const target = getFullTarget(tool)
            const short = getShortName(target)
            const isRunning = tool.result === undefined
            return (
              <div
                key={tool.callId}
                className="flex items-center gap-1.5 text-[11px] text-muted-foreground/70 py-px"
              >
                {isRunning ? (
                  <span className="animate-spin h-2.5 w-2.5 border border-current border-t-transparent rounded-full shrink-0" />
                ) : (
                  <Icon className="h-2.5 w-2.5 shrink-0 text-muted-foreground/40" />
                )}
                <span className="font-medium truncate max-w-[180px]" title={target}>
                  {short}
                </span>
                {short !== target && (
                  <span className="text-muted-foreground/30 truncate text-[10px]" title={target}>
                    {target}
                  </span>
                )}
              </div>
            )
          })}
        </div>
      )}

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
