import { memo, useMemo, useState } from "react"
import { ChevronRight, Puzzle, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface SkillProgressBlockProps {
  tool: ToolCall
  /** Show shimmer while the tool call is still in-flight. */
  shimmer?: boolean
}

function parseSkillArgs(raw: string): { name: string; args?: string } {
  try {
    const parsed = JSON.parse(raw || "{}") as { name?: string; args?: string }
    return { name: parsed.name || "", args: parsed.args }
  } catch {
    return { name: "" }
  }
}

// Detect whether the tool_result came from a fork (extract_fork_result format)
// vs an inline SKILL.md dump. The fork formatter always prefixes
// "Skill '<name>' completed." and the inline path returns raw markdown.
function isForkResult(result: string | undefined, skillName: string): boolean {
  if (!result || !skillName) return false
  return result.startsWith(`Skill '${skillName}' completed.`)
}

function SkillProgressBlockImpl({ tool, shimmer }: SkillProgressBlockProps) {
  const [expanded, setExpanded] = useState(false)
  const { name: skillName, args } = useMemo(() => parseSkillArgs(tool.arguments), [tool.arguments])
  const running = !tool.result
  const forkMode = isForkResult(tool.result, skillName)
  const body = tool.result || ""

  // Strip the "Skill 'xxx' completed.\n\nResult:\n" envelope for nicer fork display.
  const displayBody = useMemo(() => {
    if (!body) return ""
    if (forkMode) {
      const marker = "\n\nResult:\n"
      const idx = body.indexOf(marker)
      if (idx >= 0) return body.slice(idx + marker.length)
    }
    return body
  }, [body, forkMode])

  return (
    <div className="my-1.5 rounded-lg border border-amber-500/30 bg-amber-500/5 text-xs">
      <button
        type="button"
        className={cn(
          "flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors",
          !running && "hover:bg-amber-500/10",
          shimmer && "animate-pulse",
        )}
        onClick={() => !running && setExpanded(!expanded)}
        disabled={running}
        aria-expanded={running ? undefined : expanded}
      >
        {running ? (
          <Loader2 className="h-3 w-3 shrink-0 animate-spin text-amber-600" />
        ) : (
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90",
            )}
          />
        )}
        <Puzzle className="h-3 w-3 shrink-0 text-amber-600" />
        <span className="font-medium text-foreground truncate max-w-[40%]">
          {skillName || "skill"}
        </span>
        <span className="text-[10px] text-muted-foreground shrink-0 hidden sm:inline">
          {forkMode ? "skill · fork" : "skill · inline"}
        </span>
        {args && (
          <span className="text-muted-foreground truncate flex-1 min-w-0" title={args}>
            {args}
          </span>
        )}
        {tool.durationMs !== undefined && (
          <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">
            {(tool.durationMs / 1000).toFixed(1)}s
          </span>
        )}
      </button>
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          expanded && displayBody ? "max-h-[600px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="px-2.5 pb-2 pt-0.5 max-h-[600px] overflow-y-auto">
          {displayBody && (
            <div className="bg-background rounded p-2 text-[11px] leading-relaxed">
              <MarkdownRenderer content={displayBody} />
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

const SkillProgressBlock = memo(SkillProgressBlockImpl)
export default SkillProgressBlock
