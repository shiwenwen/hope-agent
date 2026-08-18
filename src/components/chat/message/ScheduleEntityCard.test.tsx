// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import ScheduleEntityCard from "./ScheduleEntityCard"
import { CRON_TASK_DRAFT_EVENT } from "@/components/cron/cronNavigation"
import type { CronJobSnapshot, CronRunLog } from "@/components/cron/CronJobForm.types"
import type { ScheduleEntityMetadata } from "@/types/chat"

const calls: Array<{ command: string; args?: unknown }> = []
let snapshot: CronJobSnapshot | null = null
let runLogs: CronRunLog[] = []

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: async (command: string, args?: unknown) => {
      calls.push({ command, args })
      if (command === "cron_get_job_snapshot") return snapshot
      if (command === "cron_get_run_logs") return runLogs
      if (command === "cron_preflight") {
        return {
          canProceed: true,
          nextRuns: [],
          issues: [],
          execution: { workspaceMode: "project" },
        }
      }
      if (command === "cron_run_now") return { status: "started" }
      return null
    },
    listen: () => () => {},
  }),
}))

const metadata: ScheduleEntityMetadata = {
  kind: "schedule_entity",
  entityType: "cronTask",
  entityId: "job-1",
  title: "Daily summary",
  state: "active",
  workspaceMode: "project",
}

function job(overrides: Record<string, unknown> = {}) {
  return {
    id: "job-1",
    revision: 3,
    name: "Daily summary",
    workspacePolicy: { mode: "project" as const },
    schedule: { type: "cron" as const, expression: "0 0 9 * * *" },
    payload: { type: "agentTurn" as const, prompt: "summarize" },
    status: "active" as const,
    consecutiveFailures: 0,
    maxFailures: 5,
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
    notifyOnComplete: true,
    deliveryTargets: [],
    ...overrides,
  }
}

beforeEach(() => {
  calls.length = 0
  snapshot = null
  runLogs = []
})

afterEach(cleanup)

describe("ScheduleEntityCard", () => {
  it("resolves the task through the tombstone-aware snapshot lookup", async () => {
    snapshot = { job: job(), deleted: false } as CronJobSnapshot
    render(<ScheduleEntityCard metadata={metadata} />)

    await waitFor(() => expect(screen.getByText("fileActions.open")).toBeTruthy())
    expect(calls.some((call) => call.command === "cron_get_job_snapshot")).toBe(true)
    expect(calls.some((call) => call.command === "cron_get_job")).toBe(false)
    expect(screen.queryByText("cron.copyAsNewTask")).toBeNull()
  })

  it("shows a deleted task as deleted and copies it into a new-task draft", async () => {
    snapshot = { job: job({ status: "paused" }), deleted: true } as CronJobSnapshot
    const drafts: unknown[] = []
    const listener = (event: Event) => drafts.push((event as CustomEvent).detail?.seed)
    window.addEventListener(CRON_TASK_DRAFT_EVENT, listener)

    render(<ScheduleEntityCard metadata={metadata} />)
    await waitFor(() => expect(screen.getByText("cron.copyAsNewTask")).toBeTruthy())
    expect(screen.getByText("common.deleted")).toBeTruthy()
    expect(screen.queryByText("fileActions.open")).toBeNull()
    // A deleted task keeps its history: no run-log poll, no live scheduling.
    expect(calls.some((call) => call.command === "cron_get_run_logs")).toBe(false)

    fireEvent.click(screen.getByText("cron.copyAsNewTask"))
    window.removeEventListener(CRON_TASK_DRAFT_EVENT, listener)
    expect(drafts).toHaveLength(1)
    expect((drafts[0] as { id: string }).id).toBe("job-1")
  })

  it("offers a retry for a failed last run and starts it through preflight", async () => {
    snapshot = { job: job(), deleted: false } as CronJobSnapshot
    runLogs = [
      {
        id: 9,
        jobId: "job-1",
        sessionId: "session-1",
        status: "error",
        startedAt: "2026-08-14T09:00:00Z",
      },
    ]
    render(<ScheduleEntityCard metadata={metadata} />)

    await waitFor(() => expect(screen.getByText("cron.runAgain")).toBeTruthy())
    fireEvent.click(screen.getByText("cron.runAgain"))
    await waitFor(() => expect(calls.some((call) => call.command === "cron_preflight")).toBe(true))
    expect(calls.find((call) => call.command === "cron_preflight")?.args).toEqual({
      request: { operation: "runNow", jobId: "job-1", expectedRevision: 3 },
    })
  })

  it("keeps a successful run free of the retry action", async () => {
    snapshot = { job: job(), deleted: false } as CronJobSnapshot
    runLogs = [
      {
        id: 10,
        jobId: "job-1",
        sessionId: "session-1",
        status: "success",
        startedAt: "2026-08-14T09:00:00Z",
      },
    ]
    render(<ScheduleEntityCard metadata={metadata} />)

    await waitFor(() => expect(screen.getByText("fileActions.open")).toBeTruthy())
    expect(screen.queryByText("cron.runAgain")).toBeNull()
  })
})
