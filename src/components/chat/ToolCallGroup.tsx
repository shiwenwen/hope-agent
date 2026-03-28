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
  Info,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import { IconTip } from "@/components/ui/tooltip"

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
  memory_get: "memory",
  image: "browse",
  pdf: "browse",
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

/** Format the raw tool call as `name(args)` for display */
function formatRawCall(tool: ToolCall): string {
  try {
    const pretty = JSON.stringify(JSON.parse(tool.arguments), null, 2)
    return `${tool.name}(${pretty})`
  } catch {
    return `${tool.name}(${tool.arguments})`
  }
}

/** Build a comma-separated summary label from mixed tool categories */
function buildSummaryLabel(
  tools: ToolCall[],
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  // Count tools per category, preserving first-seen order
  // Skill reads get their own "skill" pseudo-category
  type LabelKey = ToolCategory | "skill"
  const order: LabelKey[] = []
  const counts = new Map<LabelKey, number>()
  const skillNames: string[] = []

  for (const tool of tools) {
    const sn = getSkillName(tool)
    const key: LabelKey = sn ? "skill" : getToolCategory(tool.name)
    if (sn) skillNames.push(sn)
    if (!counts.has(key)) {
      order.push(key)
    }
    counts.set(key, (counts.get(key) || 0) + 1)
  }

  return order
    .map((key) => {
      if (key === "skill") {
        const count = counts.get(key)!
        if (count === 1 && skillNames.length === 1) {
          return t("tools.loadingSkill", { name: skillNames[0] })
        }
        return t("toolGroup.skill", { count })
      }
      return t(`toolGroup.${key}`, { count: counts.get(key)! })
    })
    .join(", ")
}

/** Get the icon for the most frequent category in a mixed group */
function getPrimaryIcon(tools: ToolCall[]): React.ComponentType<{ className?: string }> {
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
  return CATEGORY_ICONS[maxCat]
}

/** Single item inside a group — shows label + expandable result */
function GroupItem({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation()
  const [showResult, setShowResult] = useState(false)
  const [showRaw, setShowRaw] = useState(false)
  const isRunning = tool.result === undefined
  const skillName = getSkillName(tool)
  const fullTarget = skillName ? "" : getFullTarget(tool)
  const toolLabel = skillName
    ? t("tools.loadingSkill", { name: skillName })
    : t(`tools.${tool.name}`, tool.name)
  const preview = skillName ? null : getResultPreview(tool.result)
  const cat = getToolCategory(tool.name)
  const CatIcon = CATEGORY_ICONS[cat]

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
        <span className="text-muted-foreground/80 font-medium shrink-0">{toolLabel}</span>
        <span className="text-muted-foreground/60 truncate font-mono">{fullTarget}</span>
        {/* Inline result preview when collapsed */}
        {!showResult && preview && (
          <span className="text-muted-foreground/30 truncate ml-auto pl-2 max-w-[40%]">
            {preview}
          </span>
        )}
        <IconTip label={t("tools.rawCall", "查看原始调用")}>
          <span
            role="button"
            className="ml-auto shrink-0 p-0.5 rounded hover:bg-secondary text-muted-foreground/40 hover:text-muted-foreground/80 transition-colors opacity-0 group-hover/item:opacity-100"
            onClick={(e) => {
              e.stopPropagation()
              setShowRaw(!showRaw)
            }}
          >
            <Info className="h-3 w-3" />
          </span>
        </IconTip>
      </button>
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
}

export default function ToolCallGroup({ tools }: ToolCallGroupProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const anyRunning = tools.some((tc) => tc.result === undefined)

  const Icon = getPrimaryIcon(tools)
  const label = buildSummaryLabel(tools, t)

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
            const sn = getSkillName(tool)
            const target = sn || getFullTarget(tool)
            const short = sn || getShortName(target)
            const isRunning = tool.result === undefined
            const cat = getToolCategory(tool.name)
            const CatIcon = CATEGORY_ICONS[cat]
            return (
              <div
                key={tool.callId}
                className="flex items-center gap-1.5 text-[11px] text-muted-foreground/70 py-px"
              >
                {isRunning ? (
                  <span className="animate-spin h-2.5 w-2.5 border border-current border-t-transparent rounded-full shrink-0" />
                ) : (
                  <CatIcon className="h-2.5 w-2.5 shrink-0 text-muted-foreground/40" />
                )}
                <span className="font-medium truncate max-w-[180px]" title={target}>
                  {short}
                </span>
                {!sn && short !== target && (
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
