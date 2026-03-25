import React from "react"
import { useTranslation } from "react-i18next"
import {
  MessageSquare,
  MessagesSquare,
  Coins,
  DollarSign,
  Wrench,
  AlertTriangle,
  Bot,
  Clock,
} from "lucide-react"
import { cn } from "@/lib/utils"
import type { OverviewStats } from "./types"
import { formatNumber, formatCost } from "./types"

interface OverviewCardsProps {
  data: OverviewStats | null
  loading: boolean
}

interface CardConfig {
  key: string
  icon: React.ElementType
  colorClass: string
  bgClass: string
  getValue: (data: OverviewStats) => string
}

const cards: CardConfig[] = [
  {
    key: "totalSessions",
    icon: MessageSquare,
    colorClass: "text-blue-500",
    bgClass: "bg-blue-500/10",
    getValue: (d) => formatNumber(d.totalSessions),
  },
  {
    key: "totalMessages",
    icon: MessagesSquare,
    colorClass: "text-green-500",
    bgClass: "bg-green-500/10",
    getValue: (d) => formatNumber(d.totalMessages),
  },
  {
    key: "totalTokens",
    icon: Coins,
    colorClass: "text-purple-500",
    bgClass: "bg-purple-500/10",
    getValue: (d) => formatNumber(d.totalInputTokens + d.totalOutputTokens),
  },
  {
    key: "estimatedCost",
    icon: DollarSign,
    colorClass: "text-amber-500",
    bgClass: "bg-amber-500/10",
    getValue: (d) => formatCost(d.estimatedCostUsd),
  },
  {
    key: "toolCalls",
    icon: Wrench,
    colorClass: "text-cyan-500",
    bgClass: "bg-cyan-500/10",
    getValue: (d) => formatNumber(d.totalToolCalls),
  },
  {
    key: "errors",
    icon: AlertTriangle,
    colorClass: "text-red-500",
    bgClass: "bg-red-500/10",
    getValue: (d) => formatNumber(d.totalErrors),
  },
  {
    key: "activeAgents",
    icon: Bot,
    colorClass: "text-indigo-500",
    bgClass: "bg-indigo-500/10",
    getValue: (d) => formatNumber(d.activeAgents),
  },
  {
    key: "cronJobs",
    icon: Clock,
    colorClass: "text-orange-500",
    bgClass: "bg-orange-500/10",
    getValue: (d) => formatNumber(d.activeCronJobs),
  },
]

function SkeletonCard() {
  return (
    <div className="bg-card border rounded-xl p-4 space-y-3">
      <div className="flex items-center gap-3">
        <div className="h-9 w-9 rounded-full bg-muted animate-pulse" />
        <div className="space-y-1.5 flex-1">
          <div className="h-5 w-16 bg-muted animate-pulse rounded" />
          <div className="h-3 w-24 bg-muted animate-pulse rounded" />
        </div>
      </div>
    </div>
  )
}

const OverviewCards = React.memo(function OverviewCards({ data, loading }: OverviewCardsProps) {
  const { t } = useTranslation()

  if (loading && !data) {
    return (
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {Array.from({ length: 8 }).map((_, i) => (
          <SkeletonCard key={i} />
        ))}
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
      {cards.map((card) => {
        const Icon = card.icon
        const value = data ? card.getValue(data) : "-"
        return (
          <div key={card.key} className="bg-card border rounded-xl p-4">
            <div className="flex items-center gap-3">
              <div className={cn("h-9 w-9 rounded-full flex items-center justify-center", card.bgClass)}>
                <Icon className={cn("h-4.5 w-4.5", card.colorClass)} />
              </div>
              <div className="min-w-0">
                <div className="text-xl font-bold truncate">{value}</div>
                <div className="text-xs text-muted-foreground truncate">
                  {t(`dashboard.overview.${card.key}`)}
                </div>
              </div>
            </div>
          </div>
        )
      })}
    </div>
  )
})

export default OverviewCards
