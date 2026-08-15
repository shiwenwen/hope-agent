import { describe, it, expect } from "vitest"
import {
  cronAttention,
  cronSearchHaystack,
  runLogDotColor,
  runStatusDisplay,
} from "./cronHelpers"
import type { CronJob } from "./CronJobForm.types"

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
    expect(runLogDotColor("queued", "active")).toBe("bg-amber-500")
    expect(runLogDotColor("cancelling", "active")).toBe("bg-amber-500")
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
    expect(runStatusDisplay("queued")).toMatchObject({
      className: "text-amber-500",
      labelKey: "common.statusValues.queued",
    })
    expect(runStatusDisplay("cancelling")).toMatchObject({
      className: "text-amber-500",
      labelKey: "common.statusValues.cancelling",
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

describe("cronAttention", () => {
  const job = (overrides: Partial<CronJob> = {}): CronJob => ({
    id: "job-1",
    revision: 1,
    name: "Daily summary",
    workspacePolicy: { mode: "project" },
    schedule: { type: "cron", expression: "0 0 9 * * *" },
    payload: { type: "agentTurn", prompt: "summarize" },
    status: "active",
    consecutiveFailures: 0,
    maxFailures: 5,
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
    notifyOnComplete: true,
    deliveryTargets: [],
    ...overrides,
  })
  const lastRun = (status: string, error?: string) => ({
    runLogId: 7,
    sessionId: "session-1",
    status,
    startedAt: "2026-08-14T09:00:00Z",
    error: error ?? null,
  })

  it("leaves a healthy task alone", () => {
    expect(cronAttention(job())).toBeNull()
    expect(cronAttention(job({ lastRun: lastRun("success") }))).toBeNull()
    // A user pause is a decision, not a problem to fix.
    expect(cronAttention(job({ status: "paused" }))).toBeNull()
  })

  it("ranks an auto-disabled task above a merely failing one", () => {
    expect(
      cronAttention(
        job({ status: "disabled", consecutiveFailures: 5, lastRun: lastRun("error", "boom") }),
      ),
    ).toMatchObject({ kind: "autoDisabled", failures: 5, error: "boom" })
    expect(cronAttention(job({ consecutiveFailures: 2 }))).toMatchObject({ kind: "failing" })
  })

  it("surfaces a failed last run, a missed occurrence, and a stale delivery target", () => {
    expect(cronAttention(job({ lastRun: lastRun("timeout") }))).toMatchObject({
      kind: "runFailed",
      runLogId: 7,
    })
    expect(cronAttention(job({ status: "missed" }))).toMatchObject({ kind: "missed" })
    expect(
      cronAttention(
        job({
          deliveryTargets: [{ channelId: "telegram", accountId: "a", chatId: "c", stale: true }],
        }),
      ),
    ).toMatchObject({ kind: "deliveryStale" })
  })

  it("keeps the last run's error searchable alongside name and description", () => {
    const haystack = cronSearchHaystack(
      job({
        description: "posts to the ops channel",
        lastRun: lastRun("error", "provider rate limited"),
      }),
    )
    expect(haystack).toContain("ops channel")
    expect(haystack).toContain("provider rate limited")
    expect(haystack).toBe(haystack.toLowerCase())
  })
})
