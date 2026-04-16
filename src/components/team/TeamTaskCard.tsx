import { useMemo } from "react"
import { AlertTriangle } from "lucide-react"
import { cn } from "@/lib/utils"
import type { TeamTask, TeamMember } from "./teamTypes"

interface TeamTaskCardProps {
  task: TeamTask
  members: TeamMember[]
}

export function TeamTaskCard({ task, members }: TeamTaskCardProps) {
  const owner = useMemo(
    () => members.find((m) => m.memberId === task.ownerMemberId),
    [members, task.ownerMemberId],
  )

  const isHighPriority = task.priority < 100

  return (
    <div
      className={cn(
        "rounded-md border border-border bg-background p-2.5 text-sm transition-colors hover:bg-accent/50",
        isHighPriority && "border-l-2 border-l-orange-400",
      )}
    >
      {/* Content - truncated to 2 lines */}
      <p className="line-clamp-2 text-xs leading-relaxed text-foreground">
        {task.content}
      </p>

      {/* Footer: owner + priority */}
      <div className="mt-2 flex items-center justify-between">
        {owner ? (
          <div className="flex items-center gap-1.5">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ backgroundColor: owner.color }}
            />
            <span className="truncate text-[11px] text-muted-foreground">
              {owner.name}
            </span>
          </div>
        ) : (
          <span className="text-[11px] text-muted-foreground/60">--</span>
        )}

        {isHighPriority && (
          <AlertTriangle className="h-3 w-3 shrink-0 text-orange-400" />
        )}
      </div>
    </div>
  )
}
