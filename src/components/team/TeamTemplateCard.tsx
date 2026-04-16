import { Users } from "lucide-react"
import { cn } from "@/lib/utils"
import type { TeamTemplate } from "./teamTypes"

interface TeamTemplateCardProps {
  template: TeamTemplate
  selected?: boolean
  onSelect: () => void
}

export function TeamTemplateCard({
  template,
  selected,
  onSelect,
}: TeamTemplateCardProps) {
  const roleGroups = template.members.reduce(
    (acc, m) => {
      acc[m.role] = (acc[m.role] ?? 0) + 1
      return acc
    },
    {} as Record<string, number>,
  )

  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-2 rounded-lg border-2 p-3 text-left transition-colors",
        "hover:bg-accent/50",
        selected
          ? "border-primary bg-primary/5"
          : "border-border bg-background",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <span className="text-sm font-medium text-foreground">
          {template.name}
        </span>
        <div className="flex items-center gap-1 shrink-0 text-muted-foreground">
          <Users className="h-3 w-3" />
          <span className="text-[11px] tabular-nums">
            {template.members.length}
          </span>
        </div>
      </div>

      <p className="text-xs text-muted-foreground line-clamp-2 leading-relaxed">
        {template.description}
      </p>

      {/* Role breakdown */}
      <div className="flex flex-wrap gap-1.5">
        {Object.entries(roleGroups).map(([role, count]) => (
          <span
            key={role}
            className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground"
          >
            {count} {role}
          </span>
        ))}
      </div>

      {/* Member color dots */}
      <div className="flex gap-1">
        {template.members.map((m, i) => (
          <span
            key={i}
            className="h-2 w-2 rounded-full"
            style={{ backgroundColor: m.color }}
          />
        ))}
      </div>
    </button>
  )
}
