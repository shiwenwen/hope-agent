import type { DashboardFilterFields } from "./DashboardFilter"

const DASHBOARD_TAB_VALUES = new Set([
  "insights",
  "control-plane",
  "tokens",
  "tools",
  "sessions",
  "errors",
  "tasks",
  "plans",
  "system",
  "local-models",
  "recap",
  "learning",
  "dreaming",
  "evaluation",
])

export const FILTERED_DASHBOARD_TABS = [
  "insights",
  "control-plane",
  "tokens",
  "tools",
  "sessions",
  "errors",
  "tasks",
  "local-models",
] as const

export type FilteredDashboardTab = (typeof FILTERED_DASHBOARD_TABS)[number]

const FILTER_FIELDS: Record<FilteredDashboardTab, DashboardFilterFields> = {
  insights: { date: true, agent: true, provider: true, usageKind: true },
  "control-plane": { date: true, agent: true, provider: false, usageKind: false },
  tokens: { date: true, agent: true, provider: true, usageKind: true },
  tools: { date: true, agent: true, provider: false, usageKind: false },
  sessions: { date: true, agent: true, provider: false, usageKind: false },
  errors: { date: true, agent: true, provider: true, usageKind: false },
  tasks: { date: true, agent: true, provider: false, usageKind: false },
  "local-models": { date: true, agent: true, provider: false, usageKind: false },
}

export function normalizeInitialTab(tab?: string): string {
  if (tab === "plans") return "control-plane"
  return tab && DASHBOARD_TAB_VALUES.has(tab) ? tab : "insights"
}

export function showsGlobalOverview(tab: string): boolean {
  return tab === "insights"
}

export function isFilteredDashboardTab(tab: string): tab is FilteredDashboardTab {
  return FILTERED_DASHBOARD_TABS.some((candidate) => candidate === tab)
}

export function dashboardFilterFields(tab: string): DashboardFilterFields | null {
  return isFilteredDashboardTab(tab) ? FILTER_FIELDS[tab] : null
}
