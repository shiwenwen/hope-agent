import { useState, useEffect, useCallback, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { X } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DashboardFilter as DashboardFilterState } from "./types"

interface Agent {
  id: string
  name: string
}

interface Provider {
  id: string
  name: string
}

type RangeKey = "today" | "7d" | "30d" | "90d" | "all" | "custom"

function computeDateRange(key: RangeKey): { start: string | null; end: string | null } {
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

interface DashboardFilterProps {
  filter: DashboardFilterState
  onChange: (filter: DashboardFilterState) => void
}

export default function DashboardFilter({ filter, onChange }: DashboardFilterProps) {
  const { t } = useTranslation()
  const [rangeKey, setRangeKey] = useState<RangeKey>("30d")
  const [agents, setAgents] = useState<Agent[]>([])
  const [providers, setProviders] = useState<Provider[]>([])
  const [customStart, setCustomStart] = useState("")
  const [customEnd, setCustomEnd] = useState("")

  const loadOptions = useCallback(async () => {
    try {
      const [agentList, providerList] = await Promise.all([
        invoke<Agent[]>("list_agents"),
        invoke<Provider[]>("get_providers"),
      ])
      setAgents(agentList)
      setProviders(providerList)
    } catch {
      // ignore - lists may be empty
    }
  }, [])

  useEffect(() => {
    loadOptions()
  }, [loadOptions])

  const handleRangeChange = useCallback(
    (key: RangeKey) => {
      setRangeKey(key)
      if (key !== "custom") {
        const { start, end } = computeDateRange(key)
        onChange({ ...filter, startDate: start, endDate: end })
      }
    },
    [filter, onChange],
  )

  const handleCustomApply = useCallback(() => {
    onChange({
      ...filter,
      startDate: customStart ? new Date(customStart).toISOString() : null,
      endDate: customEnd ? new Date(customEnd + "T23:59:59").toISOString() : null,
    })
  }, [filter, onChange, customStart, customEnd])

  const handleClearFilters = useCallback(() => {
    setRangeKey("30d")
    const { start, end } = computeDateRange("30d")
    onChange({
      startDate: start,
      endDate: end,
      agentId: null,
      providerId: null,
      modelId: null,
    })
  }, [onChange])

  const hasActiveFilters = useMemo(
    () => filter.agentId || filter.providerId || filter.modelId,
    [filter.agentId, filter.providerId, filter.modelId],
  )

  const rangeKeys: RangeKey[] = ["today", "7d", "30d", "90d", "all", "custom"]

  return (
    <div className="shrink-0 mx-6 mt-3 bg-muted/50 rounded-lg p-3 flex items-center gap-3 flex-wrap">
      {/* Time range quick picks */}
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

      {/* Custom date inputs */}
      {rangeKey === "custom" && (
        <div className="flex items-center gap-2">
          <input
            type="date"
            value={customStart}
            onChange={(e) => setCustomStart(e.target.value)}
            className="h-7 rounded-md border border-input bg-transparent px-2 text-xs"
          />
          <span className="text-xs text-muted-foreground">-</span>
          <input
            type="date"
            value={customEnd}
            onChange={(e) => setCustomEnd(e.target.value)}
            className="h-7 rounded-md border border-input bg-transparent px-2 text-xs"
          />
          <Button variant="secondary" size="sm" className="text-xs h-7" onClick={handleCustomApply}>
            {t("dashboard.filter.apply")}
          </Button>
        </div>
      )}

      {/* Separator */}
      <div className="w-px h-5 bg-border" />

      {/* Agent filter */}
      <Select
        value={filter.agentId ?? "__all__"}
        onValueChange={(v) => onChange({ ...filter, agentId: v === "__all__" ? null : v })}
      >
        <SelectTrigger className="h-7 w-36 text-xs">
          <SelectValue placeholder={t("dashboard.filter.allAgents")} />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__all__">{t("dashboard.filter.allAgents")}</SelectItem>
          {agents.map((a) => (
            <SelectItem key={a.id} value={a.id}>
              {a.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      {/* Provider filter */}
      <Select
        value={filter.providerId ?? "__all__"}
        onValueChange={(v) =>
          onChange({ ...filter, providerId: v === "__all__" ? null : v })
        }
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

      {/* Active model filter indicator */}
      {filter.modelId && (
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
    </div>
  )
}
