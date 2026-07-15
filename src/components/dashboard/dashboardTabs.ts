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
])

export function normalizeInitialTab(tab?: string): string {
  if (tab === "plans") return "control-plane"
  return tab && DASHBOARD_TAB_VALUES.has(tab) ? tab : "insights"
}

export function showsGlobalOverview(tab: string): boolean {
  return tab !== "control-plane"
}
