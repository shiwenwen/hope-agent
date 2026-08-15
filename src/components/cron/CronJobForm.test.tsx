// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import CronJobForm from "./CronJobForm"
import type { CronJob } from "./CronJobForm.types"

const calls: Array<{ command: string; args?: unknown }> = []

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/hooks/useDockerStatus", () => ({
  useDockerStatus: () => ({ status: null, checking: false, ready: true, refresh: () => {} }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: async (command: string, args?: unknown) => {
      calls.push({ command, args })
      if (command === "list_sessions_cmd") {
        return [
          [
            {
              id: "chat-1",
              title: "Roadmap review",
              agentId: "ha-main",
              createdAt: "2026-08-01T00:00:00Z",
              updatedAt: "2026-08-01T00:00:00Z",
              messageCount: 4,
              unreadCount: 0,
              channelUnreadCount: 0,
              hasError: false,
              pendingInteractionCount: 0,
              isCron: false,
              incognito: false,
              kind: "regular",
            },
          ],
          1,
        ]
      }
      if (command === "cron_preflight") {
        return {
          canProceed: true,
          nextRuns: [],
          issues: [],
          execution: { workspaceMode: "project" },
        }
      }
      if (command === "list_agents" || command === "list_projects_cmd") return []
      if (command === "channel_list_accounts") return []
      return null
    },
    listen: () => () => {},
  }),
}))

const existingJob: CronJob = {
  id: "job-1",
  revision: 2,
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
}

beforeEach(() => {
  calls.length = 0
})

afterEach(cleanup)

describe("CronJobForm conversation target", () => {
  it("schedules into any existing chat picked from the global create form", async () => {
    render(<CronJobForm onSave={() => {}} onCancel={() => {}} />)

    fireEvent.click(screen.getByText("cron.conversationTargetExisting"))
    fireEvent.click(screen.getByText("cron.sessionTargetPlaceholder"))
    await waitFor(() => expect(screen.getByText("Roadmap review")).toBeTruthy())
    fireEvent.click(screen.getByText("Roadmap review"))

    fireEvent.change(screen.getByPlaceholderText("cron.namePlaceholder"), {
      target: { value: "Weekly nudge" },
    })
    fireEvent.change(screen.getByPlaceholderText("cron.messagePlaceholder"), {
      target: { value: "check in" },
    })
    fireEvent.click(screen.getByText("cron.create"))

    await waitFor(() => expect(calls.some((call) => call.command === "cron_preflight")).toBe(true))
    const request = (
      calls.find((call) => call.command === "cron_preflight")?.args as {
        request: { operation: string; job: Record<string, unknown> }
      }
    ).request
    expect(request.operation).toBe("create")
    expect(request.job.payload).toEqual({
      type: "sessionTurn",
      sessionId: "chat-1",
      prompt: "check in",
    })
    // An existing-chat task follows the chat's live context; it never carries a
    // task-owned Project / workspace / permission copy.
    expect(request.job.projectId).toBeNull()
    expect(request.job.workspacePolicy).toEqual({ mode: "project", baseRef: null })
    expect(request.job.permissionModeOverride).toBeNull()
  })

  it("refuses to save an existing-chat task before a chat is picked", async () => {
    render(<CronJobForm onSave={() => {}} onCancel={() => {}} />)

    fireEvent.click(screen.getByText("cron.conversationTargetExisting"))
    fireEvent.change(screen.getByPlaceholderText("cron.namePlaceholder"), {
      target: { value: "Weekly nudge" },
    })
    fireEvent.change(screen.getByPlaceholderText("cron.messagePlaceholder"), {
      target: { value: "check in" },
    })
    fireEvent.click(screen.getByText("cron.create"))

    await waitFor(() => expect(screen.getByText("cron.errorSessionTargetRequired")).toBeTruthy())
    expect(calls.some((call) => call.command === "cron_preflight")).toBe(false)
  })

  it("keeps the target immutable when editing and when opened from a chat", () => {
    const { unmount } = render(
      <CronJobForm job={existingJob} onSave={() => {}} onCancel={() => {}} />,
    )
    expect(screen.queryByText("cron.conversationTarget")).toBeNull()
    unmount()

    render(
      <CronJobForm
        sessionTarget={{ id: "chat-9", title: "Handoff" }}
        onSave={() => {}}
        onCancel={() => {}}
      />,
    )
    expect(screen.queryByText("cron.conversationTarget")).toBeNull()
    expect(screen.getByText(/cron.sessionTurnTarget/)).toBeTruthy()
  })

  it("seeds a new draft from a retained task instead of editing it", () => {
    render(
      <CronJobForm
        seedJob={{ ...existingJob, name: "Deleted daily" }}
        onSave={() => {}}
        onCancel={() => {}}
      />,
    )

    expect(screen.getByText("cron.copyAsNewTask")).toBeTruthy()
    expect((screen.getByPlaceholderText("cron.namePlaceholder") as HTMLInputElement).value).toBe(
      "Deleted daily",
    )
    expect(
      (screen.getByPlaceholderText("cron.messagePlaceholder") as HTMLTextAreaElement).value,
    ).toBe("summarize")
    // Create, not update: the seed's revision must never reach a CAS write.
    expect(screen.getByText("cron.create")).toBeTruthy()
  })
})
