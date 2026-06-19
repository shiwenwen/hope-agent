import { ChevronRight } from "lucide-react"
import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

interface SidebarSectionHeaderProps {
  title: string
  count?: number
  expanded: boolean
  onToggle: () => void
  action?: ReactNode
  className?: string
}

export default function SidebarSectionHeader({
  title,
  count,
  expanded,
  onToggle,
  action,
  className,
}: SidebarSectionHeaderProps) {
  return (
    <div className={cn("mb-2 flex items-center gap-1", className)}>
      <button
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-[11px] font-bold tracking-normal text-foreground/75 transition-colors hover:text-foreground"
      >
        <ChevronRight
          className={cn(
            "h-3 w-3 shrink-0 transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
        <span className="truncate">{title}</span>
        {count !== undefined && (
          <span className="shrink-0 font-normal text-muted-foreground/60">· {count}</span>
        )}
      </button>
      {action && <div className="ml-auto flex items-center">{action}</div>}
    </div>
  )
}
