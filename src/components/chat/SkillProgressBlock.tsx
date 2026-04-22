import { memo, useMemo, useState } from "react"
import { ChevronRight, Puzzle, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { ToolCall } from "@/types/chat"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { getToolExecutionState } from "@/components/chat/message/executionStatus"

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
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const { name: skillName, args } = useMemo(() => parseSkillArgs(tool.arguments), [tool.arguments])
  const state = getToolExecutionState(tool)
  const running = state === "running"
  const failed = state === "failed"
  const forkMode = isForkResult(tool.result, skillName)
  const body = tool.result || ""
  const title = t(`executionStatus.skill.${state}`, {
    name: skillName || "skill",
  })

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
    <div
      className={cn(
        "my-1.5 rounded-lg border text-xs",
        failed
          ? "border-red-500/30 bg-red-500/5"
          : "border-amber-500/30 bg-amber-500/5",
      )}
    >
      <button
        type="button"
        className={cn(
          "flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left transition-colors",
          !running && (failed ? "hover:bg-red-500/10" : "hover:bg-amber-500/10"),
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
        <span
          className={cn(
            "font-medium truncate max-w-[55%]",
            failed ? "text-red-500" : "text-foreground",
          )}
        >
          {title}
        </span>
        <span
          className={cn(
            "hidden sm:inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] leading-none shrink-0",
            failed
              ? "bg-red-500/10 text-red-500"
              : "bg-amber-500/10 text-muted-foreground",
          )}
        >
          {t(`executionStatus.skill.mode.${forkMode ? "fork" : "inline"}`)}
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
