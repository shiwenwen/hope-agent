import { describe, expect, it } from "vitest"

import { normalizeInitialTab, showsGlobalOverview } from "./dashboardTabs"

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

  it("keeps global overview cards out of the control-plane scope", () => {
    expect(showsGlobalOverview("control-plane")).toBe(false)
    expect(showsGlobalOverview("insights")).toBe(true)
  })
})
