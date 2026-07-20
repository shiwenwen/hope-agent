export type DashboardRangeKey = "today" | "7d" | "30d" | "90d" | "all" | "custom"

export interface DashboardFilterFields {
  date: boolean
  agent: boolean
  provider: boolean
  usageKind: boolean
}

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
