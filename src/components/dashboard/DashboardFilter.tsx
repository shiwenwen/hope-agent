import { useState, useEffect, useCallback, useMemo, type ReactNode } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import { X } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DashboardFilter as DashboardFilterState } from "./types"

interface Agent {
  id: string
  name: string
  emoji?: string | null
  avatar?: string | null
}

interface Provider {
  id: string
  name: string
}

export type DashboardRangeKey = "today" | "7d" | "30d" | "90d" | "all" | "custom"

export interface DashboardFilterFields {
  date: boolean
  agent: boolean
  provider: boolean
  usageKind: boolean
}

const USAGE_KIND_VALUES = [
  "__all__",
  "chat",
  "side_query",
  "embedding",
  "stt",
  "judge",
  "summarize",
  "web_search",
  "image_generation",
  "audio_generation",
  "provider_test",
  "vision",
] as const

export function computeDashboardDateRange(key: DashboardRangeKey): {
  start: string | null
  end: string | null
} {
  if (key === "all") return { start: null, end: null }
  const now = new Date()
  const end = now.toISOString()
  const start = new Date(now)
  switch (key) {
    case "today":
      start.setHours(0, 0, 0, 0)
      break
    case "7d":
      start.setDate(start.getDate() - 7)
      break
    case "30d":
      start.setDate(start.getDate() - 30)
      break
    case "90d":
      start.setDate(start.getDate() - 90)
      break
    default:
      return { start: null, end: null }
  }
  return { start: start.toISOString(), end }
}

function dateInputValue(value: string | null): string {
  if (!value) return ""
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? "" : parsed.toISOString().slice(0, 10)
}

interface DashboardFilterProps {
  filter: DashboardFilterState
  onChange: (filter: DashboardFilterState) => void
  fields: DashboardFilterFields
  rangeKey: DashboardRangeKey
  onRangeKeyChange: (rangeKey: DashboardRangeKey) => void
  children?: ReactNode
  className?: string
}

export default function DashboardFilter({
  filter,
  onChange,
  fields,
  rangeKey,
  onRangeKeyChange,
  children,
  className,
}: DashboardFilterProps) {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<Agent[]>([])
  const [providers, setProviders] = useState<Provider[]>([])
  const [customStart, setCustomStart] = useState("")
  const [customEnd, setCustomEnd] = useState("")

  useEffect(() => {
    let alive = true
    Promise.all([
      getTransport().call<Agent[]>("list_agents"),
      getTransport().call<Provider[]>("get_providers"),
    ])
      .then(([agentList, providerList]) => {
        if (!alive) return
        setAgents(agentList)
        setProviders(providerList)
      })
      .catch(() => {
        // ignore - lists may be empty
      })
    return () => {
      alive = false
    }
  }, [])

  useEffect(() => {
    if (rangeKey !== "custom") return
    setCustomStart(dateInputValue(filter.startDate))
    setCustomEnd(dateInputValue(filter.endDate))
  }, [filter.endDate, filter.startDate, rangeKey])

  const handleRangeChange = useCallback(
    (key: DashboardRangeKey) => {
      onRangeKeyChange(key)
      if (key !== "custom") {
        const { start, end } = computeDashboardDateRange(key)
        onChange({ ...filter, startDate: start, endDate: end })
      }
    },
    [filter, onChange, onRangeKeyChange],
  )

  const handleCustomApply = useCallback(() => {
    onChange({
      ...filter,
      startDate: customStart ? new Date(customStart).toISOString() : null,
      endDate: customEnd ? new Date(customEnd + "T23:59:59").toISOString() : null,
    })
  }, [filter, onChange, customStart, customEnd])

  const handleClearFilters = useCallback(() => {
    onRangeKeyChange("30d")
    const { start, end } = computeDashboardDateRange("30d")
    onChange({
      startDate: start,
      endDate: end,
      agentId: null,
      providerId: null,
      modelId: null,
      usageKind: null,
      operation: null,
    })
  }, [onChange, onRangeKeyChange])

  const hasActiveFilters = useMemo(
    () =>
      (fields.agent && filter.agentId) ||
      (fields.provider && (filter.providerId || filter.modelId)) ||
      (fields.usageKind && (filter.usageKind || filter.operation)),
    [
      fields.agent,
      fields.provider,
      fields.usageKind,
      filter.agentId,
      filter.modelId,
      filter.operation,
      filter.providerId,
      filter.usageKind,
    ],
  )
  const selectedAgent = agents.find((a) => a.id === filter.agentId)

  const rangeKeys: DashboardRangeKey[] = ["today", "7d", "30d", "90d", "all", "custom"]
  const hasDimensionFilters = fields.agent || fields.provider || fields.usageKind

  return (
    <div
      className={cn(
        "shrink-0 rounded-lg bg-muted/50 p-3 flex items-center gap-3 flex-wrap",
        className,
      )}
    >
      {/* Time range quick picks */}
      {fields.date && (
        <div className="flex gap-1">
          {rangeKeys.map((key) => (
            <Button
              key={key}
              variant={rangeKey === key ? "secondary" : "ghost"}
              size="sm"
              className="text-xs h-7"
              onClick={() => handleRangeChange(key)}
            >
              {t(`dashboard.range.${key}`)}
            </Button>
          ))}
        </div>
      )}

      {/* Custom date inputs */}
      {fields.date && rangeKey === "custom" && (
        <div className="flex items-center gap-2">
          <Input
            type="date"
            value={customStart}
            onChange={(e) => setCustomStart(e.target.value)}
            className="h-7 w-auto px-2 text-xs"
          />
          <span className="text-xs text-muted-foreground">-</span>
          <Input
            type="date"
            value={customEnd}
            onChange={(e) => setCustomEnd(e.target.value)}
            className="h-7 w-auto px-2 text-xs"
          />
          <Button variant="secondary" size="sm" className="text-xs h-7" onClick={handleCustomApply}>
            {t("dashboard.filter.apply")}
          </Button>
        </div>
      )}

      {/* Separator */}
      {fields.date && hasDimensionFilters && <div className="w-px h-5 bg-border" />}

      {/* Agent filter */}
      {fields.agent && (
        <Select
          value={filter.agentId ?? "__all__"}
          onValueChange={(v) => onChange({ ...filter, agentId: v === "__all__" ? null : v })}
        >
          <SelectTrigger className="h-7 w-36 text-xs">
            {selectedAgent ? (
              <AgentSelectDisplay agent={selectedAgent} size="xs" />
            ) : (
              <SelectValue placeholder={t("dashboard.filter.allAgents")} />
            )}
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t("dashboard.filter.allAgents")}</SelectItem>
            {agents.map((a) => (
              <SelectItem key={a.id} value={a.id} textValue={a.name}>
                <AgentSelectDisplay agent={a} size="xs" />
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      {fields.provider && (
        <Select
          value={filter.providerId ?? "__all__"}
          onValueChange={(v) => onChange({ ...filter, providerId: v === "__all__" ? null : v })}
        >
          <SelectTrigger className="h-7 w-36 text-xs">
            <SelectValue placeholder={t("dashboard.filter.allProviders")} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t("dashboard.filter.allProviders")}</SelectItem>
            {providers.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      {fields.usageKind && (
        <Select
          value={filter.usageKind ?? "__all__"}
          onValueChange={(v) => onChange({ ...filter, usageKind: v === "__all__" ? null : v })}
        >
          <SelectTrigger className="h-7 w-40 text-xs">
            <SelectValue placeholder={t("dashboard.usageKind.all")} />
          </SelectTrigger>
          <SelectContent>
            {USAGE_KIND_VALUES.map((value) => (
              <SelectItem key={value} value={value}>
                {value === "__all__"
                  ? t("dashboard.usageKind.all")
                  : t(`dashboard.usageKind.${value}`, value)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      {/* Active model filter indicator */}
      {fields.provider && filter.modelId && (
        <div className="flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-xs">
          <span className="text-muted-foreground">{t("dashboard.filter.model")}:</span>
          <span className="font-medium">{filter.modelId}</span>
          <button
            onClick={() => onChange({ ...filter, modelId: null })}
            className="ml-1 hover:text-foreground text-muted-foreground"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      )}

      {/* Active operation (purpose tag) filter indicator — drill-down only,
          no dropdown; set by clicking a row in the Tokens tab's operation
          table. */}
      {fields.usageKind && filter.operation && (
        <div className="flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-xs">
          <span className="text-muted-foreground">{t("dashboard.token.operation")}:</span>
          <span className="font-mono font-medium">{filter.operation}</span>
          <button
            onClick={() => onChange({ ...filter, operation: null })}
            className="ml-1 hover:text-foreground text-muted-foreground"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      )}

      {/* Clear filters */}
      {hasActiveFilters && (
        <Button
          variant="ghost"
          size="sm"
          className={cn("text-xs h-7 text-muted-foreground")}
          onClick={handleClearFilters}
        >
          <X className="h-3 w-3 mr-1" />
          {t("dashboard.filter.clear")}
        </Button>
      )}

      {children}
    </div>
  )
}
