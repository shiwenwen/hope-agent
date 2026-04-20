import React from "react"
import { cn } from "@/lib/utils"

export interface MetricCardProps {
  icon: React.ElementType
  label: string
  value: string
  subValue?: string
  colorClass: string
  bgClass: string
}

export default function MetricCard({
  icon: Icon,
  label,
  value,
  subValue,
  colorClass,
  bgClass,
}: MetricCardProps) {
  return (
    <div className="flex items-center gap-3 p-3 rounded-lg bg-muted/50">
      <div
        className={cn(
          "h-9 w-9 rounded-full flex items-center justify-center shrink-0",
          bgClass,
        )}
      >
        <Icon className={cn("h-4 w-4", colorClass)} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold truncate">{value}</div>
        <div className="text-[11px] text-muted-foreground truncate">{label}</div>
        {subValue && (
          <div className="text-[10px] text-muted-foreground/70 truncate">
            {subValue}
          </div>
        )}
      </div>
    </div>
  )
}
