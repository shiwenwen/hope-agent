import { describe, expect, it } from "vitest"

import { dashboardFilterFields, normalizeInitialTab, showsGlobalOverview } from "./dashboardTabs"

describe("DashboardView tab compatibility", () => {
  it("maps the legacy plans tab to the control-plane page", () => {
    expect(normalizeInitialTab("plans")).toBe("control-plane")
  })

  it("keeps the automation tab wire value compatible", () => {
    expect(normalizeInitialTab("tasks")).toBe("tasks")
  })

  it("defaults unknown tabs to insights", () => {
    expect(normalizeInitialTab("unknown")).toBe("insights")
  })

  it("keeps global overview cards in the insights tab only", () => {
    expect(showsGlobalOverview("control-plane")).toBe(false)
    expect(showsGlobalOverview("tokens")).toBe(false)
    expect(showsGlobalOverview("system")).toBe(false)
    expect(showsGlobalOverview("insights")).toBe(true)
  })

  it("scopes filters to tabs that consume them", () => {
    expect(dashboardFilterFields("tokens")).toEqual({
      date: true,
      agent: true,
      provider: true,
      usageKind: true,
    })
    expect(dashboardFilterFields("control-plane")).toEqual({
      date: true,
      agent: true,
      provider: false,
      usageKind: false,
    })
    expect(dashboardFilterFields("system")).toBeNull()
    expect(dashboardFilterFields("dreaming")).toBeNull()
    expect(dashboardFilterFields("evaluation")).toBeNull()
  })
})
