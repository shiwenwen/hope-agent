import { describe, it, expect } from "vitest"
import { runLogDotColor, runStatusDisplay } from "./cronHelpers"

describe("runLogDotColor (C21)", () => {
  it("colors success / failure run-logs distinctly", () => {
    expect(runLogDotColor("success", "active")).toBe("bg-emerald-500")
    expect(runLogDotColor("error", "active")).toBe("bg-red-500")
    expect(runLogDotColor("timeout", "active")).toBe("bg-red-500")
  })

  it("does NOT paint empty / cancelled / running as failure (red) — the C21 fix", () => {
    expect(runLogDotColor("empty", "active")).toBe("bg-muted-foreground")
    expect(runLogDotColor("cancelled", "active")).toBe("bg-muted-foreground")
    expect(runLogDotColor("running", "active")).toBe("bg-blue-500")
  })

  it("falls back to the job status color when there is no run log (future occurrence)", () => {
    expect(runLogDotColor(undefined, "active")).toBe("bg-blue-500")
    expect(runLogDotColor(undefined, "paused")).toBe("bg-amber-500")
  })
})

describe("runStatusDisplay (C21)", () => {
  it("labels empty / cancelled / running as themselves, not a red Error", () => {
    expect(runStatusDisplay("success")).toMatchObject({
      className: "text-emerald-500",
      labelKey: "cron.runStatusSuccess",
    })
    expect(runStatusDisplay("running")).toMatchObject({
      className: "text-blue-500",
      labelKey: "cron.runStatusRunning",
    })
    expect(runStatusDisplay("empty")).toMatchObject({
      className: "text-muted-foreground",
      labelKey: "cron.runStatusEmpty",
    })
    // cancelled reuses common.cancel, matching CronJobDetail.
    expect(runStatusDisplay("cancelled")).toMatchObject({
      className: "text-muted-foreground",
      labelKey: "common.cancel",
    })
  })

  it("treats error / timeout / unknown as failure", () => {
    expect(runStatusDisplay("error")).toMatchObject({
      className: "text-red-500",
      labelKey: "cron.runStatusError",
    })
    expect(runStatusDisplay("timeout")).toMatchObject({
      className: "text-red-500",
      labelKey: "cron.runStatusError",
    })
  })
})
