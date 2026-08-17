// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import CronPreflightDialog from "./CronPreflightDialog"
import type { CronPreflightReport } from "./CronJobForm.types"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) =>
      vars ? `${key}:${Object.values(vars).join("/")}` : key,
  }),
}))

function report(overrides: Partial<CronPreflightReport["execution"]> = {}): CronPreflightReport {
  return {
    canProceed: true,
    nextRuns: [],
    issues: [],
    execution: {
      workspaceMode: "project",
      scheduler: { primary: true, runningTasks: 0, maxConcurrent: 5 },
      taskRunning: false,
      ...overrides,
    },
  }
}

afterEach(cleanup)

describe("CronPreflightDialog", () => {
  it("names the exact chat an existing-session task will post into", () => {
    render(
      <CronPreflightDialog
        report={report({
          conversation: { kind: "existingSession", sessionId: "chat-1", title: "Roadmap review" },
        })}
        confirmLabel="save"
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.getByText(/Roadmap review/)).toBeTruthy()
  })

  it("falls back to the chat id when the target title is unreadable", () => {
    render(
      <CronPreflightDialog
        report={report({ conversation: { kind: "existingSession", sessionId: "chat-9" } })}
        confirmLabel="save"
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.getByText(/chat-9/)).toBeTruthy()
  })

  it("shows delivery targets with the failing one attributed, and the scheduler posture", () => {
    render(
      <CronPreflightDialog
        report={report({
          conversation: { kind: "newSession" },
          deliveryTargets: [
            { channelId: "telegram", accountId: "a1", chatId: "c1", label: "telegram / ops" },
            {
              channelId: "slack",
              accountId: "a2",
              chatId: "c2",
              problem: "delivery_account_disabled",
            },
          ],
          scheduler: { primary: false, runningTasks: 2, maxConcurrent: 0 },
          taskRunning: true,
          taskRunningSince: "2026-08-15T01:00:00Z",
        })}
        confirmLabel="save"
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.getByText("telegram / ops")).toBeTruthy()
    expect(screen.getByText(/delivery account disabled/)).toBeTruthy()
    // A secondary process cannot execute, and "∞" stands in for an unlimited cap.
    expect(screen.getByText(/cron.schedulerSecondary/)).toBeTruthy()
    expect(screen.getByText(/2\/∞/)).toBeTruthy()
    expect(screen.getByText(/cron.runStatusRunning/)).toBeTruthy()
  })
})
